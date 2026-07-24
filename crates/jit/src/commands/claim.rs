//! Claim coordination command implementations.
//!
//! Provides CLI interface to the lease-based claim coordination system.

use crate::config::ConfigLoader;
use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{
    ClaimAcquireLimits, ClaimCoordinator, FileLocker, IssueStore, Lease, StorageWarning,
};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

/// True when the current directory is inside a git working tree, regardless
/// of whether any commit exists yet.
///
/// Used by [`get_current_branch`] to tell apart "no git repository" from "git
/// repository with no commits" when `git rev-parse --abbrev-ref HEAD` fails —
/// both fail identically, but only `--is-inside-work-tree` still succeeds in
/// the latter case (it doesn't need a resolvable `HEAD`).
fn is_inside_git_work_tree() -> bool {
    Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Get current git branch name.
///
/// # Errors
///
/// Returns [`crate::errors::ClaimRequiresGitError`] when the current directory
/// is not inside a git repository, when git is not available, or when the
/// directory is a git repository with no commits yet (so `HEAD` doesn't
/// resolve to a branch) — the error's [`GitRequirementGap`](crate::errors::GitRequirementGap)
/// distinguishes the two so the hint matches the actual gap (REQ-04). This
/// typed error allows callers to classify the failure as an external
/// dependency failure (exit code 10) rather than a generic error.
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to execute git command for branch detection")?;

    if !output.status.success() {
        let gap = if is_inside_git_work_tree() {
            crate::errors::GitRequirementGap::NoCommits
        } else {
            crate::errors::GitRequirementGap::NoRepository
        };
        return Err(crate::errors::ClaimRequiresGitError::new(gap).into());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Execute `jit claim acquire` command.
///
/// Acquires an exclusive lease on an issue for the specified agent.
///
/// # Returns
///
/// The acquired lease id paired with any non-fatal [`StorageWarning`]s observed
/// while loading the worktree identity (e.g. a relocation). The caller's output
/// layer decides how to render the warnings; this function never writes to
/// stderr itself.
///
/// # Errors
///
/// Returns an error if the issue cannot be resolved or loaded, or if the lease
/// cannot be acquired (e.g. it is already held by another agent).
pub fn execute_claim_acquire<S: IssueStore + crate::storage::RepositoryStateStore>(
    storage: &S,
    issue_id: &str,
    ttl_secs: u64,
    agent_id: Option<&str>,
    reason: Option<&str>,
) -> Result<(String, Vec<StorageWarning>)> {
    use crate::agent_config::resolve_agent_id;

    // Resolve short ID to full ID
    let full_id = storage.resolve_issue_id(issue_id)?;

    // Validate issue exists
    let _issue = storage
        .load_issue(&full_id)
        .with_context(|| format!("Issue {} not found", full_id))?;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch
    let branch = get_current_branch()?;

    // Load or generate worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // Resolve agent ID using proper priority: CLI flag > JIT_AGENT_ID > ~/.config/jit/agent.toml > error
    let agent = resolve_agent_id(agent_id.map(|s| s.to_string()))?;

    // Load config for policy limits
    let config = ConfigLoader::new()
        .with_repo_config(&paths.local_jit)
        .unwrap_or_else(|_| ConfigLoader::new())
        .build();
    let coord_config = config.coordination();

    // Create file locker with the default lock-acquisition timeout
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));

    // Create claim coordinator
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        locker,
        identity.worktree_id.clone(),
        agent.clone(),
    );

    // Initialize control plane if needed
    coordinator.init()?;

    // Acquire the lease AND synchronize the repository-side issue/event state
    // under one held coordinator lock, in the mandatory order coordinator ->
    // bootstrap -> repository -> events. The coordinator receives the CANONICAL
    // full id (never the raw user spelling), and the repository transition is
    // idempotent and convergent over both the captured issue assignment and the
    // captured event-log tail, so a crash-interrupted prior attempt converges
    // without emitting a duplicate claim event (@/inv/event-log).
    let agent_assignee: crate::domain::Assignee = agent.parse()?;
    let worktree_root = paths.worktree_root.clone();
    let data_root = storage.root().to_path_buf();
    let (lease, ()) = coordinator.acquire_claim_synchronized(
        &full_id,
        ttl_secs,
        reason,
        ClaimAcquireLimits::new(
            coord_config.max_indefinite_leases_per_agent(),
            coord_config.max_indefinite_leases_per_repo(),
        ),
        |persisted_id| storage.resolve_issue_id(persisted_id),
        |_lease| {
            synchronize_claim_repository_state(
                storage,
                &worktree_root,
                &data_root,
                &full_id,
                &agent_assignee,
            )
        },
    )?;

    Ok((lease.lease_id, warnings))
}

/// Publish the repository-side issue assignment and claim event through one
/// recovered mutation session.
///
/// Invoked under the held coordinator lock, so the guard order is coordinator ->
/// bootstrap -> repository -> events. The transition is derived from the captured
/// image and is convergent: a claim already reflected in both the issue
/// assignment and the event-log tail is a complete no-op; an issue assigned
/// without its claim event emits exactly the missing event; a duplicate event is
/// never emitted, preserving `@/inv/event-log`.
fn synchronize_claim_repository_state<S>(
    storage: &S,
    worktree_root: &Path,
    data_root: &Path,
    full_id: &str,
    agent: &crate::domain::Assignee,
) -> Result<()>
where
    S: IssueStore + crate::storage::RepositoryStateStore,
{
    use super::{capture_or_retry, with_mutation_session, SessionStep};
    use crate::repository_state::{
        finalize, CaptureBudget, CaptureSpec, MutationContext, MutationIntent, VirtualPath,
    };
    use crate::storage::discover_repository_layout;

    let layout = discover_repository_layout(worktree_root, data_root)?;
    let intents = [MutationIntent::ClaimIssue {
        issue_id: full_id.to_string(),
        agent: agent.clone(),
    }];
    let budget = CaptureBudget {
        max_paths: 16,
        max_listings: 0,
        max_bytes: 16 * 1024 * 1024,
        max_depth: 6,
    };
    let build_spec = || -> Result<CaptureSpec> {
        Ok(CaptureSpec::phase_one(
            [
                VirtualPath::data(format!("issues/{full_id}.json"))?,
                VirtualPath::EVENTS,
            ],
            budget,
        )?)
    };
    // Operation-scoped: a capture/apply retry must keep claim audit identity and
    // transition time stable while opening a fresh recovered session.
    let context = MutationContext::production();
    with_mutation_session(
        storage,
        &layout,
        "claim repository synchronization",
        |session| {
            let Some(image) = capture_or_retry(session.capture(build_spec()?))? else {
                return Ok(SessionStep::Retry);
            };
            let plan = finalize(&layout, &image, &context, &intents)?;
            Ok(SessionStep::Apply(plan, ()))
        },
    )
}

/// Execute `jit claim heartbeat` command.
///
/// Sends a heartbeat for an indefinite lease to prevent staleness.
///
/// # Returns
///
/// Any non-fatal [`StorageWarning`]s observed while loading the worktree
/// identity (e.g. a relocation), for the caller's output layer to render.
///
/// # Errors
///
/// Returns an error if worktree context cannot be detected or the heartbeat
/// cannot be recorded (e.g. the lease does not exist).
pub fn execute_claim_heartbeat(lease_id: &str) -> Result<Vec<StorageWarning>> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // Resolve agent ID
    let agent = resolve_agent_id(None)?;

    // Create coordinator
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent);

    // Send heartbeat
    coordinator.heartbeat(lease_id)?;

    Ok(warnings)
}

/// Details of a lease released via `jit claim release <issue-id>`.
///
/// Returned by [`execute_claim_release_by_issue`] so the caller can report which
/// lease was released, on which issue, and which identity performed the release
/// (the actor recorded in the audit trail).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReleasedLeaseInfo {
    /// The lease ID that was released (resolved from the issue, not supplied by the user).
    pub lease_id: String,
    /// The full issue ID whose lease was released.
    pub issue_id: String,
    /// The prior owner of the released lease.
    pub previous_owner: String,
    /// The acting identity recorded in the eviction audit trail.
    pub actor: String,
}

/// Build a valid `human:{identifier}` actor id from an optional git user name.
///
/// Whitespace in the name is collapsed to `-` so any returned value satisfies the
/// agent-identity invariant (see `agent_config::validate_agent_id`): it contains
/// a `:`, has a non-empty type and identifier, and no whitespace in either part.
/// Returns `None` when there is no usable name (absent, empty, or whitespace
/// only), so callers can treat the actor as unattributable rather than
/// fabricating a placeholder identity.
fn sanitize_actor(name: Option<&str>) -> Option<String> {
    name.map(|n| n.split_whitespace().collect::<Vec<_>>().join("-"))
        .filter(|s| !s.is_empty())
        .map(|id| format!("human:{}", id))
}

/// Resolve the acting identity for an owner-bypass release.
///
/// Prefers the configured agent id (`JIT_AGENT_ID` > `~/.config/jit/agent.toml`,
/// already validated), falling back to the git `user.name` (sanitized via
/// [`sanitize_actor`]). The returned string is always a valid
/// `{type}:{identifier}` agent identity, so it can safely be used as the
/// `ClaimCoordinator` agent id and recorded in the eviction audit trail.
///
/// # Errors
///
/// Returns an error (see [`crate::errors::no_acting_identity`]) when neither a
/// configured agent id nor a git `user.name` is available. A release must be
/// attributable, so no placeholder identity is invented.
fn resolve_release_actor() -> Result<String> {
    use crate::agent_config::resolve_agent_id;

    if let Ok(agent) = resolve_agent_id(None) {
        return Ok(agent);
    }

    let git_name = Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());

    sanitize_actor(git_name.as_deref())
        .ok_or_else(|| anyhow::anyhow!("{}", crate::errors::no_acting_identity()))
}

/// Execute `jit claim release <issue-id>` command.
///
/// Resolves the issue's active lease and releases it regardless of the lease
/// owner, recording the acting identity in the eviction audit trail. Reuses the
/// admin/owner-bypass [`ClaimCoordinator::force_evict_lease`] path so a non-owner
/// (or an actor with no configured agent identity) can release the lease.
///
/// The positional argument is an ISSUE ID (short ids are resolved), not a lease
/// UUID. If the issue holds more than one active lease (which should not happen
/// under exclusive claiming), every active lease is released; the returned
/// [`ReleasedLeaseInfo`] reports the last one released.
///
/// # Errors
///
/// Returns an error if the issue id cannot be resolved, if the issue has no
/// active lease (see [`crate::errors::no_active_lease`]), or if the underlying
/// eviction fails.
pub fn execute_claim_release_by_issue<S: IssueStore>(
    storage: &S,
    issue_id: &str,
) -> Result<(ReleasedLeaseInfo, Vec<StorageWarning>)> {
    // Resolve short ID to full ID (errors if the issue does not exist).
    let full_id = storage
        .resolve_issue_id(issue_id)
        .with_context(|| format!("Failed to resolve issue id {}", issue_id))?;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // Resolve the acting identity for the audit trail BEFORE evicting anything,
    // so a release never happens without an attributable actor. This is
    // independent of lease ownership: any identified actor may release by issue id.
    let actor = resolve_release_actor()
        .context("Cannot release lease without an attributable acting identity")?;

    // Create file locker and coordinator using the actor as the agent id.
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        locker,
        identity.worktree_id.clone(),
        actor.clone(),
    );
    coordinator.init().with_context(|| {
        format!(
            "Failed to initialize claim coordinator for issue {}",
            full_id
        )
    })?;

    // Resolve the issue's active (non-expired) lease(s).
    let leases = coordinator
        .get_active_leases(Some(&full_id), None)
        .with_context(|| format!("Failed to look up active leases for issue {}", full_id))?;

    let released = leases
        .into_iter()
        .map(|lease| {
            let reason = format!("released by {} via claim release", actor);
            coordinator
                .force_evict_lease(&lease.lease_id, &reason)
                .with_context(|| {
                    format!(
                        "Failed to release lease {} for issue {}",
                        lease.lease_id, full_id
                    )
                })?;
            Ok(ReleasedLeaseInfo {
                lease_id: lease.lease_id,
                issue_id: lease.issue_id,
                previous_owner: lease.agent_id,
                actor: actor.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let info = released.into_iter().next_back().ok_or_else(|| {
        crate::errors::LeaseNotFoundError::new(&crate::errors::no_active_lease(&full_id))
    })?;
    Ok((info, warnings))
}

/// Renews an existing lease, extending its expiry time.
///
/// # Arguments
///
/// * `lease_id` - ID of the lease to renew
/// * `extension_secs` - How many seconds to extend the lease by
///
/// # Returns
///
/// The renewed lease with updated expiry time, paired with any non-fatal
/// [`StorageWarning`]s observed while loading the worktree identity (e.g. a
/// relocation), for the caller's output layer to render. This function never
/// writes to stderr itself.
///
/// # Errors
///
/// Returns an error if worktree context cannot be detected or the lease cannot
/// be renewed (e.g. it does not exist).
pub fn execute_claim_renew<S: IssueStore>(
    lease_id: &str,
    extension_secs: u64,
) -> Result<(Lease, Vec<StorageWarning>)> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch
    let branch = get_current_branch()?;

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // Resolve agent ID
    let agent = resolve_agent_id(None)?;

    // Create locker and coordinator
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        locker,
        identity.worktree_id.clone(),
        agent.clone(),
    );

    // Renew the lease
    let lease = coordinator.renew_lease(lease_id, extension_secs)?;
    Ok((lease, warnings))
}

/// Shows status of active leases.
///
/// # Arguments
///
/// * `issue_id` - Optional filter by issue ID
/// * `agent_id` - Optional filter by agent ID
///
/// # Returns
///
/// Vector of active leases matching the filters, paired with any non-fatal
/// [`StorageWarning`]s observed while loading the worktree identity (e.g. a
/// relocation), for the caller's output layer to render. This function never
/// writes to stderr itself.
///
/// # Errors
///
/// Returns an error if worktree context cannot be detected or the leases cannot
/// be read.
pub fn execute_claim_status<S: IssueStore>(
    issue_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<(Vec<Lease>, Vec<StorageWarning>)> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // Resolve current agent ID using proper priority: JIT_AGENT_ID > ~/.config/jit/agent.toml > error
    let current_agent_id = resolve_agent_id(None)?;

    // Create claim coordinator
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(
        paths,
        locker,
        identity.worktree_id,
        current_agent_id.clone(),
    );
    coordinator.init()?;

    // Get active leases (default to current agent if no filters specified)
    let filter_agent = if agent_id.is_none() && issue_id.is_none() {
        Some(current_agent_id.as_str())
    } else {
        agent_id
    };

    let leases = coordinator.get_active_leases(issue_id, filter_agent)?;
    Ok((leases, warnings))
}

/// Lists all active leases across all agents and worktrees.
///
/// # Arguments
///
/// * `_storage` - Issue storage (unused but kept for consistency)
///
/// # Returns
///
/// Vector of all active leases, paired with any non-fatal [`StorageWarning`]s
/// observed while loading the worktree identity (e.g. a relocation), for the
/// caller's output layer to render. This function never writes to stderr itself.
///
/// # Errors
///
/// Returns an error if worktree context cannot be detected or the leases cannot
/// be read.
pub fn execute_claim_list() -> Result<(Vec<Lease>, Vec<StorageWarning>)> {
    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // We need an agent ID for coordinator, but it doesn't matter which one for listing
    let agent = "system:list".to_string();

    // Create claim coordinator
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent);
    coordinator.init()?;

    // Get all active leases (no filters)
    let leases = coordinator.get_active_leases(None, None)?;
    Ok((leases, warnings))
}

/// Check if an issue has an active lease held by another agent.
///
/// This is an internal helper with no output of its own; it returns any
/// non-fatal [`StorageWarning`]s (e.g. a worktree relocation observed while
/// loading the identity) alongside the lease so the calling command can surface
/// them at its own boundary.
///
/// # Returns
///
/// `(None, _)` if no conflicting lease exists, `(Some(lease), _)` if a lease is
/// held by a different agent. Identity/branch detection failures degrade to
/// `(None, _)` (no lease system active) rather than erroring.
///
/// # Errors
///
/// Returns an error only if active leases cannot be read once the coordinator
/// is initialized.
pub fn check_issue_lease(
    issue_id: &str,
    current_agent: Option<&str>,
) -> Result<(Option<Lease>, Vec<StorageWarning>)> {
    // Try to detect worktree context - if not in a git repo, no leases are active
    let paths = match WorktreePaths::detect() {
        Ok(p) => p,
        Err(_) => return Ok((None, Vec::new())), // Not in a git repo, no lease system active
    };

    // Get current branch for identity
    let branch = match get_current_branch() {
        Ok(b) => b,
        Err(_) => return Ok((None, Vec::new())), // Can't determine branch, skip lease check
    };

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = match load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    ) {
        Ok(loaded) => loaded,
        Err(_) => return Ok((None, Vec::new())), // Can't load identity, skip lease check
    };

    // Create coordinator to check leases
    let agent = current_agent.unwrap_or("system:check").to_string();
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent.clone());

    // Don't fail if control plane doesn't exist yet
    if coordinator.init().is_err() {
        return Ok((None, warnings));
    }

    // Get leases for this issue
    let leases = coordinator.get_active_leases(Some(issue_id), None)?;

    // Check if any lease is held by a different agent
    for lease in leases {
        if current_agent.is_none() || Some(lease.agent_id.as_str()) != current_agent {
            return Ok((Some(lease), warnings));
        }
    }

    Ok((None, warnings))
}

/// Force-evicts a lease (admin operation).
///
/// # Arguments
///
/// * `lease_id` - ID of the lease to evict
/// * `reason` - Reason for eviction (for audit trail)
///
/// # Returns
///
/// Any non-fatal [`StorageWarning`]s observed while loading the worktree
/// identity (e.g. a relocation), for the caller's output layer to render. This
/// function never writes to stderr itself.
///
/// # Errors
///
/// Returns an error if worktree context cannot be detected or the lease cannot
/// be evicted.
pub fn execute_claim_force_evict<S: IssueStore>(
    lease_id: &str,
    reason: &str,
) -> Result<Vec<StorageWarning>> {
    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // For force-evict, we use a system agent (admin operation)
    let agent = "system:admin".to_string();

    // Create claim coordinator
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent);
    coordinator.init()?;

    // Force evict the lease
    coordinator.force_evict_lease(lease_id, reason)?;
    Ok(warnings)
}

/// Report of recovery actions taken.
#[derive(Debug, Clone, Default)]
pub struct RecoveryReport {
    /// Number of stale locks cleaned up
    pub stale_locks_cleaned: usize,
    /// Whether the claims index was rebuilt
    pub index_rebuilt: bool,
    /// Number of expired leases evicted
    pub expired_leases_evicted: usize,
    /// Number of orphaned temp files removed
    pub temp_files_removed: usize,
    /// Non-fatal warnings observed during recovery, for the output layer to
    /// render. Storage returns these instead of writing to stderr directly.
    pub warnings: Vec<StorageWarning>,
}

/// Execute recovery routines to fix common issues.
///
/// Performs automatic recovery operations:
/// - Cleans up stale locks from crashed processes (PID check)
/// - Rebuilds corrupted claims index from append-only log
/// - Evicts expired leases
/// - Removes orphaned temp files (older than 1 hour)
///
/// Safe to run at any time - only removes provably stale data.
pub fn execute_recover<S: IssueStore>(_storage: &S) -> Result<RecoveryReport> {
    use crate::storage::lock_cleanup;
    use crate::storage::temp_cleanup;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity (surfacing any relocation as a typed warning)
    let (identity, identity_warnings) = load_or_create_worktree_identity_with_warnings(
        &paths.local_jit,
        &paths.worktree_root,
        &branch,
    )?;

    // Create claim coordinator
    let agent = "system:recovery".to_string();
    let locker = FileLocker::new(Duration::from_secs(
        crate::runtime_defaults::LOCK_TIMEOUT_SECS,
    ));
    let coordinator = ClaimCoordinator::new(paths.clone(), locker, identity.worktree_id, agent);
    coordinator.init()?;

    let mut report = RecoveryReport::default();
    report.warnings.extend(identity_warnings);

    // 1. Clean up stale locks
    let lock_dir = paths.shared_jit.join("locks");
    if lock_dir.exists() {
        // Count locks before cleanup
        let locks_before = std::fs::read_dir(&lock_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "lock"))
                    .count()
            })
            .unwrap_or(0);

        report
            .warnings
            .extend(lock_cleanup::cleanup_stale_locks(&lock_dir)?);

        let locks_after = std::fs::read_dir(&lock_dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "lock"))
                    .count()
            })
            .unwrap_or(0);

        report.stale_locks_cleaned = locks_before.saturating_sub(locks_after);
    }

    // 2. Rebuild index if corrupted
    let (consistent, verify_warnings) = coordinator.verify_index_consistency()?;
    report.warnings.extend(verify_warnings);
    if !consistent {
        report.warnings.push(StorageWarning::IndexRebuilt);
        let index = coordinator.rebuild_index_from_log()?;
        report.warnings.extend(index.warnings());
        coordinator.write_index_atomic(&index)?;
        report.index_rebuilt = true;
    }

    // 3. Evict expired leases
    let mut index = coordinator.load_claims_index()?;
    let leases_before = index.leases.len();
    coordinator.evict_expired(&mut index)?;
    coordinator.write_index_atomic(&index)?;
    report.expired_leases_evicted = leases_before.saturating_sub(index.leases.len());

    // 4. Clean up orphaned temp files. Best-effort: a failure is surfaced as a
    // warning rather than aborting recovery.
    let jit_data_dir = &paths.local_jit;
    match temp_cleanup::cleanup_orphaned_temp_files(
        jit_data_dir,
        crate::runtime_defaults::TEMP_CLEANUP_THRESHOLD_SECS,
    ) {
        Ok(removed) => report.temp_files_removed = removed,
        Err(e) => report.warnings.push(StorageWarning::TempCleanupFailed {
            reason: e.to_string(),
        }),
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::storage::claim_coordinator::Lease;
    // Test helpers below load identity with explicit paths and don't surface
    // warnings; the production paths use the `_with_warnings` variant instead.
    use crate::storage::worktree_identity::load_or_create_worktree_identity;

    use crate::storage::{ClaimCoordinator, FileLocker, JsonFileStorage};
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    // Use shared test utilities
    use crate::test_utils::{create_test_paths, setup_test_repo};

    /// Execute claim acquire with manually constructed paths (bypassing WorktreePaths::detect)
    fn execute_claim_acquire_test(
        temp: &TempDir,
        storage: &JsonFileStorage,
        issue_id: &str,
        ttl_secs: u64,
        agent_id: &str,
    ) -> Result<String> {
        // Resolve short ID to full ID
        let full_id = storage.resolve_issue_id(issue_id)?;

        // Validate issue exists
        let _issue = storage.load_issue(&full_id)?;

        // Get test paths
        let paths = create_test_paths(temp);

        // Get or create worktree identity
        let branch = "test-branch".to_string();
        let identity =
            load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

        // Create coordinator
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths,
            locker,
            identity.worktree_id.clone(),
            agent_id.to_string(),
        );
        coordinator.init()?;

        // Acquire claim
        let lease = coordinator.acquire_claim(issue_id, ttl_secs)?;
        Ok(lease.lease_id)
    }

    /// Execute claim release with manually constructed paths (for testing)
    fn execute_claim_release_test(temp: &TempDir, lease_id: &str, agent_id: &str) -> Result<()> {
        let paths = create_test_paths(temp);

        // Create coordinator
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator =
            ClaimCoordinator::new(paths, locker, "wt:test".to_string(), agent_id.to_string());

        // Release lease
        coordinator.release_lease(lease_id)?;
        Ok(())
    }

    /// Execute claim release-by-issue with manually constructed paths (for testing).
    ///
    /// Mirrors [`execute_claim_release_by_issue`] but takes explicit test paths
    /// and an explicit actor (simulating the resolved identity) so tests can act
    /// as an arbitrary identity without touching `JIT_AGENT_ID` / config.
    fn execute_claim_release_by_issue_test(
        temp: &TempDir,
        storage: &JsonFileStorage,
        issue_id: &str,
        actor: &str,
    ) -> Result<ReleasedLeaseInfo> {
        let full_id = storage.resolve_issue_id(issue_id)?;
        let paths = create_test_paths(temp);

        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator =
            ClaimCoordinator::new(paths, locker, "wt:test".to_string(), actor.to_string());
        coordinator.init()?;

        let leases = coordinator.get_active_leases(Some(&full_id), None)?;

        let released = leases
            .into_iter()
            .map(|lease| {
                let reason = format!("released by {} via claim release", actor);
                coordinator.force_evict_lease(&lease.lease_id, &reason)?;
                Ok(ReleasedLeaseInfo {
                    lease_id: lease.lease_id,
                    issue_id: lease.issue_id,
                    previous_owner: lease.agent_id,
                    actor: actor.to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        released.into_iter().next_back().ok_or_else(|| {
            crate::errors::LeaseNotFoundError::new(&crate::errors::no_active_lease(&full_id)).into()
        })
    }

    /// Assert a string satisfies the agent-identity invariant: has a `:`,
    /// non-empty type and identifier, and no whitespace in either part.
    fn assert_valid_actor(actor: &str) {
        assert!(actor.contains(':'), "actor must contain ':': {}", actor);
        assert!(
            !actor.chars().any(char::is_whitespace),
            "actor must not contain whitespace: {}",
            actor
        );
        let (type_part, id_part) = actor.split_once(':').unwrap();
        assert!(
            !type_part.is_empty(),
            "type part must be non-empty: {}",
            actor
        );
        assert!(
            !id_part.is_empty(),
            "identifier part must be non-empty: {}",
            actor
        );
    }

    #[test]
    fn test_sanitize_actor_collapses_whitespace_in_git_name() {
        let actor = sanitize_actor(Some("Alice Example")).expect("name yields an identity");
        assert_eq!(actor, "human:Alice-Example");
        assert_valid_actor(&actor);
    }

    #[test]
    fn test_sanitize_actor_handles_multiple_and_trailing_whitespace() {
        let actor = sanitize_actor(Some("  Bob   Q.  Smith  ")).expect("name yields an identity");
        assert_eq!(actor, "human:Bob-Q.-Smith");
        assert_valid_actor(&actor);
    }

    #[test]
    fn test_sanitize_actor_returns_none_without_usable_name() {
        // No release identity may be fabricated: these must all yield None so
        // the caller errors rather than attributing the release to a placeholder.
        assert_eq!(sanitize_actor(None), None);
        assert_eq!(sanitize_actor(Some("")), None);
        assert_eq!(sanitize_actor(Some("   ")), None);
    }

    #[test]
    fn test_no_acting_identity_error_is_actionable() {
        // The mapping `sanitize_actor(None) -> Err(no_acting_identity())` used by
        // resolve_release_actor must surface a clear, actionable error so a
        // release is never attributed to a fabricated identity.
        let err: Result<String> = sanitize_actor(None)
            .ok_or_else(|| anyhow::anyhow!("{}", crate::errors::no_acting_identity()));
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(msg.contains("no acting identity"));
        assert!(msg.contains("JIT_AGENT_ID"));
        assert!(msg.contains("git config user.name"));
    }

    #[test]
    fn test_release_by_issue_succeeds_for_different_actor() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Agent A acquires the lease.
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:owner")?;
        assert_eq!(execute_claim_list_test(&temp)?.len(), 1);

        // A DIFFERENT actor releases by issue id (no lease UUID).
        let released =
            execute_claim_release_by_issue_test(&temp, &storage, &issue_id, "agent:stranger")?;

        assert_eq!(released.lease_id, lease_id);
        assert_eq!(released.issue_id, issue_id);
        assert_eq!(released.previous_owner, "agent:owner");
        assert_eq!(released.actor, "agent:stranger");

        // Lease is gone.
        assert_eq!(execute_claim_list_test(&temp)?.len(), 0);
        Ok(())
    }

    #[test]
    fn test_release_by_issue_records_actor_in_audit_trail() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:owner")?;
        execute_claim_release_by_issue_test(&temp, &storage, &issue_id, "agent:auditor")?;

        // The eviction is recorded in the append-only claims log with the actor
        // embedded in the reason.
        let claims_log = temp.path().join(".git/jit/claims.jsonl");
        let content = fs::read_to_string(&claims_log)?;
        assert!(
            content.contains("released by agent:auditor via claim release"),
            "claims log should record the acting identity, got: {}",
            content
        );
        assert!(content.contains("force-evict"));
        Ok(())
    }

    #[test]
    fn test_release_by_issue_errors_when_no_active_lease() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Unclaimed Issue")?;

        let result = execute_claim_release_by_issue_test(&temp, &storage, &issue_id, "agent:actor");

        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("no active lease") && msg.contains("not found"),
            "error should clearly state no active lease and contain 'not found', got: {}",
            msg
        );
        Ok(())
    }

    #[test]
    fn test_release_by_issue_resolves_short_id() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:owner")?;

        // Use a short prefix of the issue id.
        let short_id = &issue_id[..8];
        let released =
            execute_claim_release_by_issue_test(&temp, &storage, short_id, "agent:actor")?;

        assert_eq!(released.issue_id, issue_id);
        assert_eq!(execute_claim_list_test(&temp)?.len(), 0);
        Ok(())
    }

    /// Execute claim status with manually constructed paths (for testing)
    fn execute_claim_status_test(
        temp: &TempDir,
        issue_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<Lease>> {
        let paths = create_test_paths(temp);

        // Create coordinator
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths,
            locker,
            "wt:test".to_string(),
            agent_id.unwrap_or("agent:test").to_string(),
        );

        // Initialize coordinator (creates directories)
        coordinator.init()?;

        // Get active leases
        let leases = coordinator.get_active_leases(issue_id, agent_id)?;
        Ok(leases)
    }

    /// Helper to create a test issue
    fn create_test_issue(storage: &JsonFileStorage, title: &str) -> Result<String> {
        let layout = storage.configured_layout()?;
        CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .create_issue(
                title.to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )
            .map(|(id, _)| id)
    }

    /// Seed an exact identifier precondition for short-id collision tests.
    fn create_test_issue_with_id(
        storage: &JsonFileStorage,
        id: &str,
        title: &str,
    ) -> Result<String> {
        let mut issue =
            crate::domain::types::fixture_issue(title.to_string(), "Test description".to_string());
        issue.id = id.to_string();
        let issue_bytes = crate::repository_state::serialize_issue(&issue)?;
        std::fs::write(
            storage.root().join("issues").join(format!("{id}.json")),
            issue_bytes,
        )?;

        let index_path = storage.root().join("index.json");
        let mut index =
            crate::repository_state::RepositoryIndex::parse(&std::fs::read(&index_path)?)?;
        index.all_ids.push(id.to_string());
        index.all_ids.sort();
        index.all_ids.dedup();
        index.deleted_ids.retain(|deleted| deleted != id);
        std::fs::write(index_path, index.to_pretty_bytes()?)?;
        Ok(id.to_string())
    }

    #[test]
    fn test_claim_acquire_fails_when_issue_does_not_exist() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;

        let result = execute_claim_acquire_test(&temp, &storage, "nonexistent", 600, "agent:test");

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found") || err_msg.contains("nonexistent"));
        Ok(())
    }

    #[test]
    fn test_claim_acquire_returns_valid_lease_id() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        let result = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test");

        assert!(result.is_ok());
        let lease_id = result.unwrap();
        assert!(!lease_id.is_empty());
        assert!(lease_id.len() >= 32);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_accepts_different_ttl_values() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;

        let ttls = [60, 600, 3600, 0];
        for (i, ttl) in ttls.iter().enumerate() {
            let issue_id = create_test_issue(&storage, &format!("Issue {}", i))?;
            let result = execute_claim_acquire_test(
                &temp,
                &storage,
                &issue_id,
                *ttl,
                &format!("agent:ttl-{}", i),
            );
            assert!(result.is_ok(), "Should succeed with TTL={}", ttl);
        }
        Ok(())
    }

    #[test]
    fn test_claim_acquire_fails_when_already_claimed() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // First claim succeeds
        let first = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first");
        assert!(first.is_ok());

        // Second claim fails
        let second = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:second");
        assert!(second.is_err());
        let err = second.unwrap_err().to_string();
        assert!(err.contains("already claimed") || err.contains("agent:first"));
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_control_plane_structure() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        assert!(temp.path().join(".git/jit").exists());
        assert!(temp.path().join(".git/jit/locks").exists());
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_claims_log() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        let claims_log = temp.path().join(".git/jit/claims.jsonl");
        assert!(claims_log.exists());

        let content = fs::read_to_string(&claims_log)?;
        assert!(!content.is_empty());
        assert!(content.contains(&issue_id));
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_worktree_identity() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        let worktree_json = temp.path().join(".jit/worktree.json");
        assert!(worktree_json.exists());

        let content = fs::read_to_string(&worktree_json)?;
        assert!(content.contains("worktree_id"));
        assert!(content.contains("wt:"));
        Ok(())
    }

    /// Helper to execute claim renew in tests
    fn execute_claim_renew_test(
        temp: &TempDir,
        lease_id: &str,
        extension_secs: u64,
        agent_id: &str,
    ) -> Result<Lease> {
        let paths = create_test_paths(temp);

        let branch = "test-branch".to_string();
        let identity =
            load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator =
            ClaimCoordinator::new(paths, locker, identity.worktree_id, agent_id.to_string());

        coordinator.renew_lease(lease_id, extension_secs)
    }

    // Tests for claim renew command
    #[test]
    fn test_claim_renew_extends_ttl() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire a claim
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        // Get original lease
        let paths = create_test_paths(&temp);
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths,
            locker,
            "wt:test".to_string(),
            "agent:test".to_string(),
        );
        let original = coordinator
            .get_active_leases(None, None)?
            .into_iter()
            .find(|l| l.lease_id == lease_id)
            .unwrap();

        // Sleep briefly to ensure time passes
        std::thread::sleep(Duration::from_millis(10));

        // Renew with 1200 second extension
        let renewed = execute_claim_renew_test(&temp, &lease_id, 1200, "agent:test")?;

        assert_eq!(renewed.lease_id, lease_id);
        // Note: ttl_secs stays the same (original 600), but expires_at is extended by extension_secs
        assert_eq!(renewed.ttl_secs, 600);
        assert!(renewed.expires_at.unwrap() > original.expires_at.unwrap());

        Ok(())
    }

    #[test]
    fn test_claim_renew_fails_for_nonexistent_lease() -> Result<()> {
        let (temp, _storage) = setup_test_repo()?;

        // Initialize control plane
        let control_plane = temp.path().join(".git/jit");
        fs::create_dir_all(control_plane.join("locks"))?;

        let result = execute_claim_renew_test(&temp, "fake-lease-id", 600, "agent:test");

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("Lease"),
            "Error message should mention lease not found, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_claim_renew_fails_for_wrong_agent() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire with agent1
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first")?;

        // Try to renew with agent2
        let result = execute_claim_renew_test(&temp, &lease_id, 600, "agent:second");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not agent:second"));

        Ok(())
    }

    #[test]
    fn test_claim_renew_indefinite_lease() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire indefinite lease (TTL=0)
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 0, "agent:test")?;

        // Sleep to ensure heartbeat changes
        std::thread::sleep(Duration::from_millis(10));

        // Renew indefinite lease
        let renewed = execute_claim_renew_test(&temp, &lease_id, 0, "agent:test")?;

        assert_eq!(renewed.ttl_secs, 0);
        assert!(renewed.expires_at.is_none());
        // Verify heartbeat was updated (last_beat is DateTime, not 0)

        Ok(())
    }

    // Tests for claim release command
    #[test]
    fn test_claim_release_fails_with_invalid_lease_id() -> Result<()> {
        let (temp, _storage) = setup_test_repo()?;

        let result = execute_claim_release_test(&temp, "invalid-lease-id", "agent:test");

        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn test_claim_release_succeeds_for_valid_lease() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire a claim first
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        // Release it
        let result = execute_claim_release_test(&temp, &lease_id, "agent:test");

        assert!(result.is_ok(), "Should successfully release valid lease");
        Ok(())
    }

    #[test]
    fn test_claim_release_allows_re_acquisition_after_release() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire claim
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first")?;

        // Release it
        execute_claim_release_test(&temp, &lease_id, "agent:first")?;

        // Should be able to acquire again
        let second_lease =
            execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:second");
        assert!(
            second_lease.is_ok(),
            "Should be able to re-acquire after release"
        );

        Ok(())
    }

    #[test]
    fn test_status_empty() -> Result<()> {
        let (temp, _storage) = setup_test_repo()?;

        let leases = execute_claim_status_test(&temp, None, None)?;
        assert!(leases.is_empty());
        Ok(())
    }

    #[test]
    fn test_status_with_active_leases() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire a lease
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        // Query status
        let result = execute_claim_status_test(&temp, None, None);
        assert!(result.is_ok());
        let leases = result.unwrap();
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].lease_id, lease_id);
        assert_eq!(leases[0].issue_id, issue_id);
        Ok(())
    }

    #[test]
    fn test_status_filter_by_issue() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue1 = create_test_issue(&storage, "Issue 1")?;
        let issue2 = create_test_issue(&storage, "Issue 2")?;

        // Acquire leases on both issues
        execute_claim_acquire_test(&temp, &storage, &issue1, 600, "agent:test")?;
        execute_claim_acquire_test(&temp, &storage, &issue2, 600, "agent:test")?;

        // Query status for issue1 only
        let result = execute_claim_status_test(&temp, Some(&issue1), None);
        assert!(result.is_ok());
        let leases = result.unwrap();
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].issue_id, issue1);
        Ok(())
    }

    #[test]
    fn test_status_filter_by_agent() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire lease with specific agent
        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:other-agent")?;

        // Query status for test agent (should be empty)
        let result = execute_claim_status_test(&temp, None, Some("agent:test"));
        assert!(result.is_ok());
        let leases = result.unwrap();
        assert!(leases.is_empty());

        // Query status for other agent (should find it)
        let result = execute_claim_status_test(&temp, None, Some("agent:other-agent"));
        assert!(result.is_ok());
        let leases = result.unwrap();
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].agent_id, "agent:other-agent");
        Ok(())
    }

    // Tests for claim list command
    #[test]
    fn test_list_empty() -> Result<()> {
        let (temp, _storage) = setup_test_repo()?;

        let result = execute_claim_list_test(&temp);
        assert!(result.is_ok());
        let leases = result.unwrap();
        assert!(leases.is_empty());
        Ok(())
    }

    #[test]
    fn test_list_shows_all_leases() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue1 = create_test_issue(&storage, "Issue 1")?;
        let issue2 = create_test_issue(&storage, "Issue 2")?;
        let issue3 = create_test_issue(&storage, "Issue 3")?;

        // Acquire leases with different agents
        execute_claim_acquire_test(&temp, &storage, &issue1, 600, "agent:alice")?;
        execute_claim_acquire_test(&temp, &storage, &issue2, 600, "agent:bob")?;
        execute_claim_acquire_test(&temp, &storage, &issue3, 600, "agent:charlie")?;

        // List should show all leases
        let result = execute_claim_list_test(&temp);
        assert!(result.is_ok());
        let leases = result.unwrap();
        assert_eq!(leases.len(), 3);

        // Verify all agents are present
        let agents: Vec<String> = leases.iter().map(|l| l.agent_id.clone()).collect();
        assert!(agents.contains(&"agent:alice".to_string()));
        assert!(agents.contains(&"agent:bob".to_string()));
        assert!(agents.contains(&"agent:charlie".to_string()));

        Ok(())
    }

    #[test]
    fn test_list_excludes_expired_leases() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue1 = create_test_issue(&storage, "Issue 1")?;
        let issue2 = create_test_issue(&storage, "Issue 2")?;

        // Acquire one lease with very short TTL (will expire)
        execute_claim_acquire_test(&temp, &storage, &issue1, 0, "agent:expired")?;

        // Sleep to ensure first lease expires
        std::thread::sleep(Duration::from_millis(10));

        // Acquire another lease (should still be active)
        execute_claim_acquire_test(&temp, &storage, &issue2, 600, "agent:active")?;

        // List should only show active lease
        let result = execute_claim_list_test(&temp);
        assert!(result.is_ok());
        let leases = result.unwrap();

        // Note: TTL=0 means indefinite, not expired. Both should be in list.
        assert_eq!(leases.len(), 2);

        Ok(())
    }

    /// Helper to execute claim list in tests
    fn execute_claim_list_test(temp: &TempDir) -> Result<Vec<Lease>> {
        let paths = create_test_paths(temp);

        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths,
            locker,
            "wt:test".to_string(),
            "system:list".to_string(),
        );
        coordinator.init()?;

        coordinator.get_active_leases(None, None)
    }

    // Tests for claim force-evict command
    #[test]
    fn test_force_evict_removes_lease() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Acquire a lease
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        // Verify lease exists
        let leases_before = execute_claim_list_test(&temp)?;
        assert_eq!(leases_before.len(), 1);

        // Force evict it
        let result = execute_claim_force_evict_test(&temp, &lease_id, "Test eviction");
        assert!(result.is_ok(), "Force evict should succeed");

        // Verify lease is gone
        let leases_after = execute_claim_list_test(&temp)?;
        assert_eq!(leases_after.len(), 0);

        Ok(())
    }

    #[test]
    fn test_force_evict_fails_for_nonexistent_lease() -> Result<()> {
        let (temp, _storage) = setup_test_repo()?;

        // Initialize control plane
        let control_plane = temp.path().join(".git/jit");
        fs::create_dir_all(control_plane.join("locks"))?;

        let result = execute_claim_force_evict_test(&temp, "fake-lease-id", "Test reason");

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("Lease"),
            "Error should mention lease not found, got: {}",
            err_msg
        );

        Ok(())
    }

    #[test]
    fn test_force_evict_allows_re_acquisition() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // Agent 1 acquires
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first")?;

        // Admin evicts
        execute_claim_force_evict_test(&temp, &lease_id, "Admin intervention")?;

        // Agent 2 should be able to acquire
        let result = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:second");
        assert!(
            result.is_ok(),
            "Should be able to acquire after force eviction"
        );

        Ok(())
    }

    /// Helper to execute force evict in tests
    fn execute_claim_force_evict_test(temp: &TempDir, lease_id: &str, reason: &str) -> Result<()> {
        let paths = create_test_paths(temp);

        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths,
            locker,
            "wt:test".to_string(),
            "system:admin".to_string(),
        );
        coordinator.init()?;

        coordinator.force_evict_lease(lease_id, reason)
    }

    #[test]
    fn test_get_current_branch_errors_when_git_fails() {
        // Create a temp directory that's NOT a git repo
        let temp = TempDir::new().unwrap();
        let original_dir = std::env::current_dir().unwrap();

        // Change to non-git directory
        std::env::set_current_dir(&temp).unwrap();

        // get_current_branch() should return an error, not "main"
        let result = get_current_branch();

        // Restore directory before assertions
        std::env::set_current_dir(original_dir).unwrap();

        // Should fail, not return "main" as fallback
        assert!(
            result.is_err(),
            "get_current_branch() should error in non-git directory, not return fallback"
        );
    }

    // REQ-04's "git repository with no commits" gap is covered at the CLI
    // level in `claim_integration_tests.rs`
    // (`test_claim_acquire_in_git_repo_without_commits_hints_commit_not_git_init`
    // and its `--json` counterpart), not here: those spawn `jit` as a
    // subprocess with its own `current_dir`, so they don't race with this
    // module's other tests over the process-wide working directory the way a
    // unit-level `std::env::set_current_dir` test would.

    // --- REQ-02: coordinator-held acquire synchronization + crash convergence -

    /// Acquire through the production synchronized path with explicit test paths,
    /// exercising the real coordinator-held repository synchronization.
    fn execute_claim_acquire_synchronized_test(
        temp: &TempDir,
        storage: &JsonFileStorage,
        issue_id: &str,
        ttl_secs: u64,
        agent_id: &str,
    ) -> Result<String> {
        let full_id = storage.resolve_issue_id(issue_id)?;
        let paths = create_test_paths(temp);
        let identity = load_or_create_worktree_identity(
            &paths.local_jit,
            &paths.worktree_root,
            "test-branch",
        )?;
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            locker,
            identity.worktree_id.clone(),
            agent_id.to_string(),
        );
        coordinator.init()?;
        let agent: crate::domain::Assignee = agent_id.parse()?;
        let worktree_root = paths.worktree_root.clone();
        let data_root = storage.root().to_path_buf();
        let (lease, ()) = coordinator.acquire_claim_synchronized(
            &full_id,
            ttl_secs,
            None,
            ClaimAcquireLimits::new(100, 100),
            |persisted_id| storage.resolve_issue_id(persisted_id),
            |_lease| {
                super::synchronize_claim_repository_state(
                    storage,
                    &worktree_root,
                    &data_root,
                    &full_id,
                    &agent,
                )
            },
        )?;
        Ok(lease.lease_id)
    }

    fn persist_legacy_claim(temp: &TempDir, issue_id: &str, agent_id: &str) -> Result<Lease> {
        let paths = create_test_paths(temp);
        let identity = load_or_create_worktree_identity(
            &paths.local_jit,
            &paths.worktree_root,
            "test-branch",
        )?;
        let coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(Duration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            identity.worktree_id,
            agent_id.to_string(),
        );
        coordinator.init()?;
        coordinator.acquire_claim(issue_id, 600)
    }

    /// Count claim events for one issue by a given agent in the event log.
    fn claim_events_for(storage: &JsonFileStorage, full_id: &str, agent: &str) -> usize {
        let agent: crate::domain::Assignee = agent.parse().unwrap();
        storage
            .read_events()
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event,
                    crate::domain::Event::IssueClaimed { issue_id, assignee, .. }
                        if issue_id == full_id && assignee == &agent
                )
            })
            .count()
    }

    #[test]
    fn test_claim_acquire_synchronized_publishes_issue_and_event() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Sync Issue")?;

        let lease_id = execute_claim_acquire_synchronized_test(
            &temp,
            &storage,
            &issue_id,
            600,
            "agent:worker",
        )?;
        assert!(!lease_id.is_empty());

        // Issue is assigned with a stamped claimed_at, and exactly one claim event
        // was appended in the same transaction (@/inv/event-log).
        let issue = storage.load_issue(&issue_id)?;
        assert_eq!(
            issue.assignee.as_ref().map(|a| a.to_string()),
            Some("agent:worker".to_string())
        );
        assert!(issue.claimed_at.is_some());
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 1);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_idempotent_over_split_and_full_reflection() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Split Issue")?;

        // First acquire publishes the assigned issue and its claim event in one
        // transaction.
        execute_claim_acquire_synchronized_test(&temp, &storage, &issue_id, 600, "agent:worker")?;
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 1);
        assert!(storage.load_issue(&issue_id)?.assignee.is_some());

        // Simulate a crash that left the issue assigned but lost its claim event
        // (the split state the old save-then-append path could produce).
        std::fs::write(storage.root().join("events.jsonl"), b"")?;
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 0);

        // The retry emits EXACTLY the missing event (the issue is already assigned).
        execute_claim_acquire_synchronized_test(&temp, &storage, &issue_id, 600, "agent:worker")?;
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 1);

        // A further acquire by the same agent/worktree is a complete no-op: the
        // convergent transition never emits a duplicate event.
        execute_claim_acquire_synchronized_test(&temp, &storage, &issue_id, 600, "agent:worker")?;
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 1);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_reuses_uniquely_resolved_legacy_short_lease() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let first_id = create_test_issue_with_id(
            &storage,
            "abcdef12-3000-4000-8000-000000000001",
            "First shared-prefix issue",
        )?;
        create_test_issue_with_id(
            &storage,
            "abcdef12-4000-4000-8000-000000000002",
            "Second shared-prefix issue",
        )?;
        let legacy = persist_legacy_claim(&temp, "abcdef12-3", "agent:worker")?;

        let acquired = execute_claim_acquire_synchronized_test(
            &temp,
            &storage,
            &first_id,
            600,
            "agent:worker",
        )?;

        assert_eq!(acquired, legacy.lease_id);
        assert_eq!(execute_claim_list_test(&temp)?.len(), 1);
        assert_eq!(claim_events_for(&storage, &first_id, "agent:worker"), 1);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_rejects_ambiguous_persisted_short_lease_without_mutation() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let first_id = create_test_issue_with_id(
            &storage,
            "abcdef12-3000-4000-8000-000000000001",
            "First shared-prefix issue",
        )?;
        create_test_issue_with_id(
            &storage,
            "abcdef12-4000-4000-8000-000000000002",
            "Second shared-prefix issue",
        )?;

        persist_legacy_claim(&temp, "abcdef12", "agent:worker")?;

        let paths = create_test_paths(&temp);
        let log_path = paths.shared_jit.join("claims.jsonl");
        let index_path = paths.shared_jit.join("claims.index.json");
        let log_before = fs::read(&log_path)?;
        let index_before = fs::read(&index_path)?;

        let result = execute_claim_acquire_synchronized_test(
            &temp,
            &storage,
            &first_id,
            600,
            "agent:worker",
        );

        let error = result.expect_err("ambiguous persisted short lease must reject acquisition");
        let error_chain = format!("{error:#}");
        assert!(
            error_chain.to_lowercase().contains("ambiguous"),
            "resolution error should be explicit: {error_chain}"
        );
        assert_eq!(
            fs::read(&log_path)?,
            log_before,
            "claim log must not change"
        );
        assert_eq!(
            fs::read(&index_path)?,
            index_before,
            "claims index must not change"
        );
        assert!(storage.load_issue(&first_id)?.assignee.is_none());
        assert_eq!(claim_events_for(&storage, &first_id, "agent:worker"), 0);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_ignores_unrelated_ambiguous_persisted_short_lease() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        create_test_issue_with_id(
            &storage,
            "abcdef12-3000-4000-8000-000000000001",
            "First shared-prefix issue",
        )?;
        create_test_issue_with_id(
            &storage,
            "abcdef12-4000-4000-8000-000000000002",
            "Second shared-prefix issue",
        )?;
        let unrelated_id = create_test_issue_with_id(
            &storage,
            "12345678-9000-4000-8000-000000000003",
            "Unrelated issue",
        )?;
        persist_legacy_claim(&temp, "abcdef12", "agent:worker")?;

        execute_claim_acquire_synchronized_test(
            &temp,
            &storage,
            &unrelated_id,
            600,
            "agent:worker",
        )?;

        let leases = execute_claim_list_test(&temp)?;
        assert_eq!(leases.len(), 2);
        assert!(leases.iter().any(|lease| lease.issue_id == "abcdef12"));
        assert!(leases.iter().any(|lease| lease.issue_id == unrelated_id));
        assert_eq!(claim_events_for(&storage, &unrelated_id, "agent:worker"), 1);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_distinguishes_full_ids_with_shared_prefix() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let first_id = create_test_issue_with_id(
            &storage,
            "abcdef12-3000-4000-8000-000000000001",
            "First shared-prefix issue",
        )?;
        let second_id = create_test_issue_with_id(
            &storage,
            "abcdef12-4000-4000-8000-000000000002",
            "Second shared-prefix issue",
        )?;
        persist_legacy_claim(&temp, &first_id, "agent:worker")?;

        execute_claim_acquire_synchronized_test(&temp, &storage, &second_id, 600, "agent:worker")?;

        let leases = execute_claim_list_test(&temp)?;
        assert_eq!(leases.len(), 2);
        assert!(leases.iter().any(|lease| lease.issue_id == first_id));
        assert!(leases.iter().any(|lease| lease.issue_id == second_id));
        assert_eq!(claim_events_for(&storage, &second_id, "agent:worker"), 1);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_rejects_reverse_guard_order() -> Result<()> {
        use crate::storage::{discover_repository_layout, RepositoryStateStore};

        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Reverse Order")?;
        let full_id = storage.resolve_issue_id(&issue_id)?;

        let paths = create_test_paths(&temp);
        let identity = load_or_create_worktree_identity(
            &paths.local_jit,
            &paths.worktree_root,
            "test-branch",
        )?;
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            locker,
            identity.worktree_id,
            "agent:worker".to_string(),
        );
        coordinator.init()?;

        // Hold a repository mutation session, THEN attempt to coordinate: reverse
        // acquisition is rejected before any coordinator lock is taken.
        let layout = discover_repository_layout(&paths.worktree_root, storage.root()).unwrap();
        let _session = storage.open_mutation_session(layout).unwrap();
        let result = coordinator.acquire_claim_synchronized(
            &full_id,
            600,
            None,
            ClaimAcquireLimits::new(100, 100),
            |persisted_id| storage.resolve_issue_id(persisted_id),
            |_lease| Ok(()),
        );
        assert!(result.is_err(), "reverse guard order must be rejected");
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("coordinator") || message.contains("repository mutation session"),
            "error should name the guard-order violation, got: {message}"
        );
        Ok(())
    }

    #[test]
    fn test_release_renew_heartbeat_never_touch_repository_state() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Coordinator Only")?;
        let full_id = storage.resolve_issue_id(&issue_id)?;

        let paths = create_test_paths(&temp);
        let identity = load_or_create_worktree_identity(
            &paths.local_jit,
            &paths.worktree_root,
            "test-branch",
        )?;
        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            locker,
            identity.worktree_id.clone(),
            "agent:worker".to_string(),
        );
        coordinator.init()?;
        let agent: crate::domain::Assignee = "agent:worker".parse().unwrap();
        let worktree_root = paths.worktree_root.clone();
        let data_root = storage.root().to_path_buf();

        // Acquire an indefinite lease (claims the issue via the synchronized path).
        let (lease, ()) = coordinator.acquire_claim_synchronized(
            &full_id,
            0,
            Some("manual oversight"),
            ClaimAcquireLimits::new(100, 100),
            |persisted_id| storage.resolve_issue_id(persisted_id),
            |_lease| {
                super::synchronize_claim_repository_state(
                    &storage,
                    &worktree_root,
                    &data_root,
                    &full_id,
                    &agent,
                )
            },
        )?;

        // Snapshot repository-owned state after the acquire.
        let issue_before = storage.load_issue(&issue_id)?;
        let events_before = storage.read_events()?;

        // Coordinator-only operations (heartbeat, renew, force-evict/release) never
        // open a repository session, so they cannot mutate the issue or event log.
        coordinator.heartbeat(&lease.lease_id)?;
        coordinator.renew_lease(&lease.lease_id, 0)?;
        coordinator.force_evict_lease(&lease.lease_id, "released")?;

        let issue_after = storage.load_issue(&issue_id)?;
        let events_after = storage.read_events()?;
        assert_eq!(issue_before.assignee, issue_after.assignee);
        assert_eq!(issue_before.claimed_at, issue_after.claimed_at);
        assert_eq!(
            events_before.len(),
            events_after.len(),
            "coordinator-only operations must not append repository events"
        );
        Ok(())
    }
}
