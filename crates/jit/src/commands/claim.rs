//! Claim coordination command implementations.
//!
//! Provides CLI interface to the lease-based claim coordination system.

use super::CommandExecutor;
use crate::config::JitConfig;
use crate::domain::identity::{IssueId, IssueIdPrefix, LeaseId};
use crate::storage::claim_coordinator::{read_leases, LeaseChange};
use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{
    ClaimAcquireLimits, ClaimCoordinator, FileLocker, IssueStore, Lease, StorageWarning,
};
use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::Serialize;
use std::path::Path;
use std::process::Command;

/// True when the selected worktree root is inside a git working tree, regardless
/// of whether any commit exists yet.
///
/// Used by [`get_current_branch`] to tell apart "no git repository" from "git
/// repository with no commits" when `git rev-parse --abbrev-ref HEAD` fails —
/// both fail identically, but only `--is-inside-work-tree` still succeeds in
/// the latter case (it doesn't need a resolvable `HEAD`).
fn is_inside_git_work_tree(worktree_root: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

/// Get current git branch name.
///
/// # Errors
///
/// Returns [`crate::errors::ClaimRequiresGitError`] when the selected worktree
/// is not inside a git repository, when git is not available, or when the
/// selected worktree is a git repository with no commits yet (so `HEAD` doesn't
/// resolve to a branch) — the error's [`GitRequirementGap`](crate::errors::GitRequirementGap)
/// distinguishes the two so the hint matches the actual gap (REQ-04). This
/// typed error allows callers to classify the failure as an external
/// dependency failure (exit code 10) rather than a generic error.
fn get_current_branch(worktree_root: &Path) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to execute git command for branch detection")?;

    if !output.status.success() {
        let gap = if is_inside_git_work_tree(worktree_root) {
            crate::errors::GitRequirementGap::NoCommits
        } else {
            crate::errors::GitRequirementGap::NoRepository
        };
        return Err(crate::errors::ClaimRequiresGitError::new(gap).into());
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Claim and recovery operations of a [`CommandExecutor`].
pub struct ClaimOps<'a, S: IssueStore> {
    exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The claim and recovery family.
    pub fn claims(&self) -> ClaimOps<'_, S> {
        ClaimOps { exec: self }
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> ClaimOps<'_, S> {
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
    /// Returns an error if the issue cannot be loaded, or if the lease
    /// cannot be acquired (e.g. it is already held by another agent).
    pub fn acquire(
        &self,
        issue_id: &IssueId,
        ttl_secs: u64,
        agent_id: Option<&str>,
        reason: Option<&str>,
    ) -> Result<(LeaseId, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        let storage = self.exec.storage();
        use crate::agent_config::resolve_agent_id;

        // Resolve short ID to full ID
        let full_id = issue_id.clone();

        // Validate issue exists
        let _issue = storage
            .load_issue(&full_id)
            .with_context(|| format!("Issue {} not found", full_id))?;

        // Get current branch
        let branch = get_current_branch(&paths.worktree_root)?;

        // Load or generate worktree identity, surfacing relocation as a warning
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        // Resolve agent ID using proper priority: CLI flag > JIT_AGENT_ID > ~/.config/jit/agent.toml > error
        let agent = resolve_agent_id(agent_id.map(|s| s.to_string()))?;

        // Load config for policy limits
        let coord_config = JitConfig::load(&storage.repository_layout()?)?.coordination();

        // Create file locker with the default lock-acquisition timeout
        let locker = FileLocker::with_configured_timeout();

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
        // full id (never the raw user spelling).
        let worktree_root = paths.worktree_root.clone();
        let data_root = storage.root().to_path_buf();
        let (lease, ()) = coordinator.acquire_claim_synchronized(
            &full_id,
            ttl_secs,
            reason,
            ClaimAcquireLimits::new(
                coord_config.max_indefinite_leases_per_agent,
                coord_config.max_indefinite_leases_per_repo,
            ),
            |_lease| {
                synchronize_claim_repository_state(
                    storage,
                    &worktree_root,
                    &data_root,
                    &full_id,
                    &agent,
                )
            },
        )?;

        Ok((lease.lease_id, warnings))
    }
}

/// The claim reads the issue record and appends at the log's end.
fn claim_capture_spec(full_id: &IssueId) -> Result<crate::repository_state::CaptureSpec> {
    use crate::repository_state::{CaptureSpec, VirtualPath};
    let mut spec = CaptureSpec::new(
        [VirtualPath::issue_record(full_id)],
        super::issue_mutation_capture_budget(),
    )?;
    spec.discover_tail(VirtualPath::EVENTS)?;
    Ok(spec)
}

/// Publish the repository-side issue assignment and claim event through one
/// mutation session.
///
/// Invoked under the held coordinator lock, so the guard order is coordinator ->
/// bootstrap -> repository -> events. The captured assignee alone decides: an
/// issue already assigned to `agent` publishes nothing; any other publishes the
/// assignment and its `IssueClaimed` (`@/inv/event-log`).
fn synchronize_claim_repository_state<S>(
    storage: &S,
    worktree_root: &Path,
    data_root: &Path,
    full_id: &IssueId,
    agent: &crate::domain::Assignee,
) -> Result<()>
where
    S: IssueStore + crate::storage::RepositoryStateStore,
{
    use super::{capture_or_retry, with_mutation_session, SessionStep};
    use crate::repository_state::{finalize, MutationIntent};
    use crate::storage::discover_repository_layout;

    let layout = discover_repository_layout(worktree_root, data_root)?;
    let intents = [MutationIntent::ClaimIssue {
        issue_id: full_id.clone(),
        agent: agent.clone(),
    }];
    // Operation-scoped: a capture/apply retry must keep claim audit identity and
    // transition time stable while opening a fresh mutation session.
    let context = super::production_mutation_context();
    with_mutation_session(
        storage,
        &layout,
        "claim repository synchronization",
        |session| {
            let Some(image) = capture_or_retry(session.capture(claim_capture_spec(full_id)?))?
            else {
                return Ok(SessionStep::Retry);
            };
            let plan = finalize(&layout, &image, &context, &intents, None)?;
            Ok(SessionStep::Apply(plan, ()))
        },
    )
}

impl<S: IssueStore> ClaimOps<'_, S> {
    /// Execute `jit claim heartbeat` command.
    ///
    /// Sends a heartbeat for the caller's lease on `issue_id` to prevent
    /// staleness.
    ///
    /// # Returns
    ///
    /// Any non-fatal [`StorageWarning`]s observed while loading the worktree
    /// identity (e.g. a relocation), for the caller's output layer to render.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected worktree's branch cannot be resolved or the
    /// heartbeat cannot be recorded (e.g. the issue has no active lease).
    pub fn heartbeat(&self, issue_id: &IssueId) -> Result<(Lease, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        use crate::agent_config::resolve_agent_id;

        // Get current branch for identity
        let branch = get_current_branch(&paths.worktree_root)?;

        // Load worktree identity, surfacing relocation as a warning
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        // Resolve agent ID
        let agent = resolve_agent_id(None)?;

        // Create coordinator
        let locker = FileLocker::with_configured_timeout();
        let coordinator = ClaimCoordinator::new(paths.clone(), locker, identity.worktree_id, agent);

        let lease = coordinator.heartbeat(issue_id)?;
        Ok((lease, warnings))
    }
}

/// Details of a lease released via `jit claim release <issue-id>`.
///
/// Returned by [`ClaimOps::release_by_issue`] so the caller can report which
/// lease was released, on which issue, and which identity performed the release
/// (the actor recorded in the audit trail).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ReleasedLeaseInfo {
    /// The lease ID that was released (resolved from the issue, not supplied by the user).
    pub lease_id: LeaseId,
    /// The full issue ID whose lease was released.
    pub issue_id: IssueId,
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
/// [`sanitize_actor`]). The returned assignee is valid and can be used by the
/// `ClaimCoordinator` and recorded in the eviction audit trail.
///
/// # Errors
///
/// Returns an error (see [`crate::errors::no_acting_identity`]) when neither a
/// configured agent id nor a git `user.name` is available. A release must be
/// attributable, so no placeholder identity is invented.
pub(super) fn resolve_release_actor(worktree_root: &Path) -> Result<crate::domain::Assignee> {
    use crate::agent_config::resolve_agent_id;

    if let Ok(agent) = resolve_agent_id(None) {
        return Ok(agent);
    }

    let git_name = Command::new("git")
        .arg("-C")
        .arg(worktree_root)
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());

    sanitize_actor(git_name.as_deref())
        .ok_or_else(|| anyhow::anyhow!("{}", crate::errors::no_acting_identity()))?
        .parse()
        .map_err(Into::into)
}

impl<S: IssueStore> ClaimOps<'_, S> {
    /// Execute `jit claim release <issue-id>` command.
    ///
    /// Releases the issue's lease regardless of its owner, under one held
    /// [`LeaseLock`](crate::storage::claim_coordinator::LeaseLock), recording the
    /// acting identity in the eviction audit trail.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue does not exist, if it has no
    /// active lease (see [`crate::errors::no_active_lease`]), or if the underlying
    /// eviction fails.
    pub fn release_by_issue(
        &self,
        issue_id: &IssueId,
    ) -> Result<(ReleasedLeaseInfo, Vec<StorageWarning>)> {
        self.release_by_issue_with_actor(issue_id, resolve_release_actor)
    }

    /// [`Self::release_by_issue`] with the acting identity resolved by
    /// `resolve_actor`, which receives the worktree root.
    fn release_by_issue_with_actor(
        &self,
        issue_id: &IssueId,
        resolve_actor: impl FnOnce(&Path) -> Result<crate::domain::Assignee>,
    ) -> Result<(ReleasedLeaseInfo, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        let storage = self.exec.storage();
        storage.load_issue(issue_id)?;
        let full_id = issue_id.clone();

        // Get current branch for identity
        let branch = get_current_branch(&paths.worktree_root)?;

        // Load worktree identity, surfacing relocation as a warning
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        // Resolve the acting identity for the audit trail BEFORE evicting anything,
        // so a release never happens without an attributable actor. This is
        // independent of lease ownership: any identified actor may release by issue id.
        let actor = resolve_actor(&paths.worktree_root)
            .context("Cannot release lease without an attributable acting identity")?;

        // Create file locker and coordinator using the actor as the agent id.
        let locker = FileLocker::with_configured_timeout();
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

        let no_active_lease = || crate::storage::claim_coordinator::no_active_lease(&full_id);
        let mut lock = coordinator.lock()?;
        let lease = lock.load(&full_id)?.ok_or_else(no_active_lease)?;
        let reason = format!("released by {} via claim release", actor);
        // An expired lease is evicted as expired, and the release then fails.
        lock.apply(&full_id, |current, _| match current {
            Some(current) if current.lease_id == lease.lease_id => Ok(LeaseChange::Remove(reason)),
            _ => Err(no_active_lease()),
        })?;

        let info = ReleasedLeaseInfo {
            lease_id: lease.lease_id,
            issue_id: lease.issue_id,
            previous_owner: lease.agent_id.to_string(),
            actor: actor.to_string(),
        };
        Ok((info, warnings))
    }
}

impl<S: IssueStore> ClaimOps<'_, S> {
    /// Renews the caller's lease on `issue_id`, extending its expiry time by
    /// `extension_secs`.
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
    /// Returns an error if the selected worktree's branch cannot be resolved or the
    /// lease cannot be renewed (e.g. the issue has no active lease).
    pub fn renew(
        &self,
        issue_id: &IssueId,
        extension_secs: u64,
    ) -> Result<(Lease, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        use crate::agent_config::resolve_agent_id;

        // Get current branch
        let branch = get_current_branch(&paths.worktree_root)?;

        // Load worktree identity, surfacing relocation as a warning
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        // Resolve agent ID
        let agent = resolve_agent_id(None)?;

        // Create locker and coordinator
        let locker = FileLocker::with_configured_timeout();
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            locker,
            identity.worktree_id.clone(),
            agent.clone(),
        );

        let lease = coordinator.renew_lease(issue_id, extension_secs)?;
        Ok((lease, warnings))
    }
}

impl<S: IssueStore> ClaimOps<'_, S> {
    /// Shows unexpired leases matching `issue` and `agent_id`, stale ones
    /// marked; with neither filter, the current agent's. Takes no lock and
    /// writes nothing.
    ///
    /// # Errors
    ///
    /// The selected worktree is not a git checkout with a commit, or the leases
    /// cannot be read.
    pub fn status(
        &self,
        issue: Option<&IssueIdPrefix>,
        agent_id: Option<&str>,
    ) -> Result<Vec<ListedLease>> {
        let paths = self.exec.require_worktree_paths()?;
        use crate::agent_config::resolve_agent_id;

        // Claims require git (`@/charter/D-4`).
        get_current_branch(&paths.worktree_root)?;

        // Default to the current agent when no filter is given.
        let filter_agent = match (issue, agent_id) {
            (None, None) => Some(resolve_agent_id(None)?.to_string()),
            (_, agent_id) => agent_id.map(str::to_string),
        };

        unexpired_leases(paths, issue, filter_agent.as_deref())
    }
}

/// A lease as `claim list` and `claim status` report it.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct ListedLease {
    #[serde(flatten)]
    pub lease: Lease,
    /// Whether the lease was stale ([`Lease::is_stale`]) when it was read.
    pub stale: bool,
}

/// Unexpired leases matching the filters, most recently acquired first, read
/// without the claims lock. A stale lease is kept and marked stale as of now.
fn unexpired_leases(
    paths: &WorktreePaths,
    issue: Option<&IssueIdPrefix>,
    agent_id: Option<&str>,
) -> Result<Vec<ListedLease>> {
    let now = chrono::Utc::now();
    let mut leases = read_leases(paths)?
        .into_iter()
        .filter(|lease| !lease.is_expired(now))
        .filter(|lease| issue.is_none_or(|issue| issue.matches(&lease.issue_id)))
        .filter(|lease| agent_id.is_none_or(|agent| lease.agent_id.to_string() == agent))
        .map(|lease| ListedLease {
            stale: lease.is_stale(now),
            lease,
        })
        .collect::<Vec<_>>();
    leases.sort_by_key(|listed| std::cmp::Reverse(listed.lease.acquired_at));
    Ok(leases)
}

impl<S: IssueStore> ClaimOps<'_, S> {
    /// Lists unexpired leases across all agents and worktrees, stale ones
    /// marked. Takes no lock and writes nothing.
    ///
    /// # Errors
    ///
    /// The selected worktree is not a git checkout with a commit, or the leases
    /// cannot be read.
    pub fn list(&self) -> Result<Vec<ListedLease>> {
        let paths = self.exec.require_worktree_paths()?;
        // Claims require git (`@/charter/D-4`).
        get_current_branch(&paths.worktree_root)?;
        unexpired_leases(paths, None, None)
    }
}

impl<S: IssueStore> ClaimOps<'_, S> {
    /// Force-evicts the lease on the issue among the stored lease records that
    /// `issue` names (admin operation), recording `reason` in the audit trail.
    /// The issue store is not consulted, so a lease on a deleted issue stays
    /// evictable.
    ///
    /// # Returns
    ///
    /// The evicted lease and any non-fatal [`StorageWarning`]s observed while loading the worktree
    /// identity (e.g. a relocation), for the caller's output layer to render. This
    /// function never writes to stderr itself.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected worktree's branch cannot be resolved or the
    /// lease cannot be evicted.
    pub fn force_evict(
        &self,
        issue: &IssueIdPrefix,
        reason: &str,
    ) -> Result<(Lease, Vec<StorageWarning>)> {
        let paths = self.exec.require_worktree_paths()?;
        // Get current branch for identity
        let branch = get_current_branch(&paths.worktree_root)?;

        // Load worktree identity, surfacing relocation as a warning
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

        // For force-evict, we use a system agent (admin operation)
        let agent = crate::domain::Assignee::SYSTEM_ADMIN;

        // Create claim coordinator
        let locker = FileLocker::with_configured_timeout();
        let coordinator = ClaimCoordinator::new(paths.clone(), locker, identity.worktree_id, agent);
        coordinator.init()?;

        let lease = coordinator.force_evict_lease(issue, reason)?;
        Ok((lease, warnings))
    }
}

/// Report of recovery actions taken.
#[derive(Debug, Clone, Default)]
pub struct RecoveryReport {
    /// Number of stale locks cleaned up
    pub stale_locks_cleaned: usize,
    /// Number of expired leases evicted
    pub expired_leases_evicted: usize,
    /// Number of indefinite leases reclaimed for a stale heartbeat
    pub stale_leases_reclaimed: usize,
    /// Number of orphaned temp files removed, lease stage files included
    pub temp_files_removed: usize,
    /// Number of derived-state targets repaired during recovery.
    pub derived_state_repaired: usize,
    /// Whether claims and lease recovery ran or was skipped because it
    /// requires git.
    pub claims_lease_recovery: RecoveryPhase,
    /// Non-fatal warnings observed during recovery, for the output layer to
    /// render. Storage returns these instead of writing to stderr directly.
    pub warnings: Vec<StorageWarning>,
}

/// Outcome of a recovery phase whose execution depends on an external
/// capability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema, Default)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecoveryPhase {
    /// The phase ran to completion.
    #[default]
    Completed,
    /// The phase was skipped, with the capability it requires recorded.
    Skipped { reason: RecoverySkipReason },
}

/// Why a recovery phase was skipped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoverySkipReason {
    /// The phase requires a git repository.
    RequiresGit,
}

/// Run recovery for claims, worktree identity, locks, and leases.
///
/// This phase intentionally requires git. The caller must keep it separate
/// from git-independent recovery so derived-state repair remains available in
/// repositories that do not use git.
fn execute_git_recovery(paths: &WorktreePaths, report: &mut RecoveryReport) -> Result<()> {
    use crate::storage::lock_cleanup;

    // Get current branch for identity.
    let branch = get_current_branch(&paths.worktree_root)?;

    // Load worktree identity (surfacing any relocation as a typed warning).
    let (identity, identity_warnings) =
        load_or_create_worktree_identity_with_warnings(paths, &branch)?;
    report.warnings.extend(identity_warnings);

    // Create claim coordinator.
    let agent = crate::domain::Assignee::SYSTEM_RECOVERY;
    let locker = FileLocker::with_configured_timeout();
    let coordinator = ClaimCoordinator::new(paths.clone(), locker, identity.worktree_id, agent);
    coordinator.init()?;

    // 1. Clean up stale locks.
    let lock_dir = paths.shared_jit.join("locks");
    if lock_dir.exists() {
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

    // 2. Evict expired leases and reclaim stale indefinite ones.
    let lapsed = coordinator.evict_lapsed()?;
    report.expired_leases_evicted = lapsed.expired;
    report.stale_leases_reclaimed = lapsed.reclaimed;

    // 3. Remove interrupted lease operations' stage files; lease records are
    // not `*.tmp` files.
    sweep_temp_files(
        &crate::storage::WorktreePaths::leases_dir(&paths.shared_jit),
        report,
    );

    Ok(())
}

/// Remove orphaned `*.tmp` files under `root` past the cleanup threshold,
/// counting them in `report`. Best-effort: a failure is surfaced as a warning
/// rather than aborting recovery.
fn sweep_temp_files(root: &std::path::Path, report: &mut RecoveryReport) {
    match crate::storage::temp_cleanup::cleanup_orphaned_temp_files(
        root,
        crate::runtime_defaults::TEMP_CLEANUP_THRESHOLD_SECS,
    ) {
        Ok(removed) => report.temp_files_removed += removed,
        Err(e) => report.warnings.push(StorageWarning::TempCleanupFailed {
            reason: e.to_string(),
        }),
    }
}

impl<S: IssueStore + crate::storage::RepositoryStateStore> ClaimOps<'_, S> {
    /// Run the repair routines of `jit recover`.
    ///
    /// Like every mutating command, `jit recover` applies a leftover redo record
    /// when its session opens, before this runs. The routines, in sequence:
    /// - Repairs declaration-derived state through the `jit validate --fix` path
    /// - With claims coordination: cleans up stale claim locks from crashed processes
    ///   (PID check), evicts expired leases, and reclaims indefinite leases whose
    ///   heartbeat is stale, and removes interrupted lease operations' stage
    ///   files; without Git, skips these with a warning
    /// - Removes orphaned `*.tmp` files under `.jit/` (older than 1 hour)
    ///
    /// Safe to run at any time: it removes only provably stale data and never
    /// the pending redo record.
    pub fn recover(&self) -> Result<RecoveryReport> {
        let paths = self.exec.require_worktree_paths()?;
        let storage = self.exec.storage();

        let mut report = RecoveryReport::default();

        // Derived-state drift is repaired before anything that depends on git.
        // The git-dependent phase below aborts in a repository whose HEAD does not
        // resolve, and repairing afterwards would leave that repository's drift
        // untouched — a core capability made conditional on git history, against
        // `@/charter/D-4`. Repairing first keeps the repair unconditional while
        // preserving the git phase's own failure.
        let layout =
            crate::storage::discover_repository_layout(&paths.worktree_root, &paths.local_jit)?;
        if layout
            .resolve(&crate::repository_state::VirtualPath::CONFIG)?
            .is_file()
        {
            let executor =
                crate::commands::CommandExecutor::new(storage.clone()).with_layout(layout);
            report.derived_state_repaired = executor.validation().repair_derived_state(false)?.0;
        }

        if paths.has_claims_coordination() {
            execute_git_recovery(paths, &mut report)?;
        } else {
            report.claims_lease_recovery = RecoveryPhase::Skipped {
                reason: RecoverySkipReason::RequiresGit,
            };
            report
                .warnings
                .push(StorageWarning::ClaimsLeaseRecoverySkipped);
        }

        sweep_temp_files(&paths.local_jit, &mut report);

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::claim_coordinator::Lease;
    // Test helpers below load identity with explicit paths and don't surface
    // warnings; the production paths use the `_with_warnings` variant instead.
    use crate::storage::worktree_identity::load_or_create_worktree_identity;

    use crate::storage::{ClaimCoordinator, FileLocker, JsonFileStorage};
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    // Use shared test utilities
    use crate::test_utils::{
        create_test_paths, grow_event_log_beyond, setup_test_repo, setup_test_repo_with_config,
    };

    /// Production claim entry points resolve the branch through git, so the
    /// fixture's synthetic `.git` directory becomes a real checkout once.
    fn ensure_git_checkout(temp: &TempDir) -> Result<()> {
        if temp.path().join(".git/HEAD").exists() {
            return Ok(());
        }
        crate::test_cli::init_git_repository_with_commit(temp.path())
    }

    fn execute_claim_acquire_test(
        temp: &TempDir,
        storage: &JsonFileStorage,
        issue_id: &IssueId,
        ttl_secs: u64,
        agent_id: &str,
    ) -> Result<LeaseId> {
        ensure_git_checkout(temp)?;
        CommandExecutor::new(storage.clone())
            .with_worktree_paths(create_test_paths(temp))
            .claims()
            .acquire(issue_id, ttl_secs, Some(agent_id), Some("test lease"))
            .map(|(lease_id, _)| lease_id)
    }

    fn execute_claim_release_by_issue_test(
        temp: &TempDir,
        storage: &JsonFileStorage,
        issue_id: &IssueId,
        actor: &str,
    ) -> Result<ReleasedLeaseInfo> {
        ensure_git_checkout(temp)?;
        let actor = actor.parse()?;
        CommandExecutor::new(storage.clone())
            .with_worktree_paths(create_test_paths(temp))
            .claims()
            .release_by_issue_with_actor(issue_id, |_| Ok(actor))
            .map(|(info, _)| info)
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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Unclaimed Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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

    /// The leases `jit claim status` reports for explicit filters.
    fn execute_claim_status_test(
        temp: &TempDir,
        issue_id: Option<&crate::domain::identity::IssueId>,
        agent_id: Option<&str>,
    ) -> Result<Vec<Lease>> {
        Ok(unexpired_leases(
            &create_test_paths(temp),
            issue_id.map(IssueIdPrefix::from).as_ref(),
            agent_id,
        )?
        .into_iter()
        .map(|listed| listed.lease)
        .collect())
    }

    /// Seed an exact identifier precondition for short-id collision tests.
    fn create_test_issue_with_id(
        storage: &JsonFileStorage,
        id: &IssueId,
        title: &str,
    ) -> Result<IssueId> {
        let mut issue =
            crate::test_utils::fixture_issue(title.to_string(), "Test description".to_string());
        issue.id = id.clone();
        let issue_bytes = crate::repository_state::serialize_issue(&issue)?;
        let issue_path = storage
            .repository_layout()?
            .resolve(&crate::repository_state::VirtualPath::issue_record(id))?;
        std::fs::write(issue_path, issue_bytes)?;
        Ok(id.clone())
    }

    #[test]
    fn test_claim_acquire_fails_when_issue_does_not_exist() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;

        let result = execute_claim_acquire_test(
            &temp,
            &storage,
            &crate::test_utils::issue_id(0x96bf_59db_932e_7f49_ee5c_c6bb_5a1f_6645),
            600,
            "agent:test",
        );

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not found") || err_msg.contains("nonexistent"));
        Ok(())
    }

    #[test]
    fn test_claim_acquire_returns_valid_lease_id() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        let result = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test");

        assert!(result.is_ok());
        let lease_id = result.unwrap();
        assert!(!lease_id.as_str().is_empty());
        assert!(lease_id.as_str().len() >= 32);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_accepts_different_ttl_values() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();

        let ttls = [60, 600, 3600, 0];
        for (i, ttl) in ttls.iter().enumerate() {
            let issue_id = executor
                .issues()
                .create_issue(
                    format!("Issue {}", i),
                    "Test description".to_string(),
                    crate::domain::Priority::Normal,
                    Vec::new(),
                    Vec::new(),
                    None,
                    None,
                    false,
                )?
                .issue
                .id;
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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        assert!(temp.path().join(".git/jit").exists());
        assert!(temp.path().join(".git/jit/locks").exists());
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_claims_log() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        let claims_log = temp.path().join(".git/jit/claims.jsonl");
        assert!(claims_log.exists());

        let content = fs::read_to_string(&claims_log)?;
        assert!(!content.is_empty());
        assert!(content.contains(issue_id.as_str()));
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_worktree_identity() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        issue_id: &IssueId,
        extension_secs: u64,
        agent_id: &str,
    ) -> Result<Lease> {
        let paths = create_test_paths(temp);

        let branch = "test-branch".to_string();
        let identity = load_or_create_worktree_identity(&paths, &branch)?;

        let locker = FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        ));
        let coordinator =
            ClaimCoordinator::new(paths, locker, identity.worktree_id, agent_id.parse()?);

        coordinator.renew_lease(issue_id, extension_secs)
    }

    // Tests for claim renew command
    #[test]
    fn test_claim_renew_extends_ttl() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        // Acquire a claim
        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        let original = execute_claim_list_test(&temp)?
            .into_iter()
            .find(|l| l.lease_id == lease_id)
            .unwrap();

        // Sleep briefly to ensure time passes
        std::thread::sleep(Duration::from_millis(10));

        // Renew with 1200 second extension
        let renewed = execute_claim_renew_test(&temp, &issue_id, 1200, "agent:test")?;

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

        let result =
            execute_claim_renew_test(&temp, &crate::test_utils::issue_id(7), 600, "agent:test");

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        // Acquire with agent1
        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first")?;

        // Try to renew with agent2
        let result = execute_claim_renew_test(&temp, &issue_id, 600, "agent:second");

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not agent:second"));

        Ok(())
    }

    #[test]
    fn test_claim_renew_indefinite_lease() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        // Acquire indefinite lease (TTL=0)
        execute_claim_acquire_test(&temp, &storage, &issue_id, 0, "agent:test")?;

        // Sleep to ensure heartbeat changes
        std::thread::sleep(Duration::from_millis(10));

        // Renew indefinite lease
        let renewed = execute_claim_renew_test(&temp, &issue_id, 0, "agent:test")?;

        assert_eq!(renewed.ttl_secs, 0);
        assert!(renewed.expires_at.is_none());
        // Verify heartbeat was updated (last_beat is DateTime, not 0)

        Ok(())
    }

    // Tests for claim release command
    #[test]
    fn test_claim_release_allows_re_acquisition_after_release() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        // Acquire claim
        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first")?;

        // Release it
        execute_claim_release_by_issue_test(&temp, &storage, &issue_id, "agent:first")?;

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue1 = executor
            .issues()
            .create_issue(
                "Issue 1".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        let issue2 = executor
            .issues()
            .create_issue(
                "Issue 2".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        assert_eq!(leases[0].agent_id.to_string(), "agent:other-agent");
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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue1 = executor
            .issues()
            .create_issue(
                "Issue 1".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        let issue2 = executor
            .issues()
            .create_issue(
                "Issue 2".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        let issue3 = executor
            .issues()
            .create_issue(
                "Issue 3".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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
        let agents: Vec<String> = leases.iter().map(|l| l.agent_id.to_string()).collect();
        assert!(agents.contains(&"agent:alice".to_string()));
        assert!(agents.contains(&"agent:bob".to_string()));
        assert!(agents.contains(&"agent:charlie".to_string()));

        Ok(())
    }

    #[test]
    fn test_list_excludes_expired_leases() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue1 = executor
            .issues()
            .create_issue(
                "Issue 1".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        let issue2 = executor
            .issues()
            .create_issue(
                "Issue 2".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

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

    /// The leases `jit claim list` reports.
    fn execute_claim_list_test(temp: &TempDir) -> Result<Vec<Lease>> {
        Ok(unexpired_leases(&create_test_paths(temp), None, None)?
            .into_iter()
            .map(|listed| listed.lease)
            .collect())
    }

    #[test]
    fn test_list_and_status_omit_expired_and_mark_stale_leases_without_a_lock_or_a_write(
    ) -> Result<()> {
        use crate::storage::clock::FixedClock;
        use crate::test_lease_store::history_path;

        let (temp, executor) = setup_test_repo_with_config("")?;
        ensure_git_checkout(&temp)?;
        let paths = create_test_paths(&temp);
        let expired_id = crate::test_utils::issue_id(0x5000_4000_8000_0000_0000_0001_abcd_ef77);
        let stale_id = crate::test_utils::issue_id(0x5000_4000_8000_0000_0000_0001_abcd_ef78);
        let threshold = crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS as i64;
        let past = ClaimCoordinator::new(
            paths.clone(),
            FileLocker::new(Duration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            "wt:test".parse()?,
            "agent:test".parse()?,
        )
        .with_clock(std::sync::Arc::new(FixedClock::new(
            chrono::Utc::now() - chrono::Duration::seconds(threshold + 60),
        )));
        let acquire = |issue_id, ttl_secs| {
            past.acquire_claim_synchronized(
                issue_id,
                ttl_secs,
                Some("test lease"),
                ClaimAcquireLimits::new(1, 1),
                |_| Ok(()),
            )
            .map(|(lease, ())| lease)
        };
        acquire(&expired_id, 60)?;
        let stale = acquire(&stale_id, 0)?;
        let store = || -> Result<(Vec<Lease>, Vec<u8>)> {
            Ok((
                crate::storage::claim_coordinator::read_leases(&paths)?,
                fs::read(history_path(&paths.shared_jit))?,
            ))
        };
        let before = store()?;
        let _held = FileLocker::new(Duration::from_millis(1))
            .try_lock_exclusive(&paths.shared_jit.join("locks/claims.lock"))?
            .expect("no claim operation is running");
        let executor = executor.with_worktree_paths(paths.clone());
        let status = |issue_id| {
            executor
                .claims()
                .status(Some(&IssueIdPrefix::from(issue_id)), None)
        };
        let marked = ListedLease {
            lease: stale,
            stale: true,
        };

        let listed = executor.claims().list()?;

        assert_eq!(listed, vec![marked.clone()]);
        assert_eq!(status(&stale_id)?, vec![marked]);
        assert!(status(&expired_id)?.is_empty());
        assert_eq!(store()?, before);
        assert_eq!(
            crate::storage::worktree_identity::read_worktree_id(&paths)?,
            None,
            "reads record no worktree identity"
        );
        Ok(())
    }

    // Tests for claim force-evict command
    #[test]
    fn test_force_evict_removes_lease() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        // Acquire a lease
        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:test")?;

        // Verify lease exists
        let leases_before = execute_claim_list_test(&temp)?;
        assert_eq!(leases_before.len(), 1);

        // Force evict it
        let result = execute_claim_force_evict_test(&temp, &issue_id, "Test eviction");
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

        let result =
            execute_claim_force_evict_test(&temp, &crate::test_utils::issue_id(7), "Test reason");

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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Test Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        // Agent 1 acquires
        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:first")?;

        // Admin evicts
        execute_claim_force_evict_test(&temp, &issue_id, "Admin intervention")?;

        // Agent 2 should be able to acquire
        let result = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:second");
        assert!(
            result.is_ok(),
            "Should be able to acquire after force eviction"
        );

        Ok(())
    }

    fn execute_claim_force_evict_test(
        temp: &TempDir,
        issue_id: &IssueId,
        reason: &str,
    ) -> Result<()> {
        ensure_git_checkout(temp)?;
        CommandExecutor::new(JsonFileStorage::new(temp.path().join(".jit")))
            .with_worktree_paths(create_test_paths(temp))
            .claims()
            .force_evict(&IssueIdPrefix::from(issue_id), reason)
            .map(drop)
    }

    #[test]
    fn test_get_current_branch_errors_when_git_fails() {
        // Create a temp directory that's NOT a git repo
        let temp = TempDir::new().unwrap();

        // get_current_branch() should return an error, not "main"
        let result = get_current_branch(temp.path());

        // Should fail, not return "main" as fallback
        assert!(
            result.is_err(),
            "get_current_branch() should error in non-git directory, not return fallback"
        );
    }

    // REQ-04's "git repository with no commits" gap is covered at the CLI
    // level in `claim_integration_tests.rs`
    // (`test_claim_acquire_in_git_repo_without_commits_hints_commit_not_git_init`
    // and its `--json` counterpart), not here: the gap is the CLI's hint.

    // --- REQ-02: coordinator-held acquire synchronization + crash convergence -

    /// Count claim events for one issue by a given agent in the event log.
    fn claim_events_for(storage: &JsonFileStorage, full_id: &IssueId, agent: &str) -> usize {
        let agent: crate::domain::Assignee = agent.parse().unwrap();
        storage
            .read_events()
            .unwrap()
            .into_iter()
            .filter(|event| {
                matches!(
                    event.body(),
                    crate::domain::EventBody::IssueClaimed { issue_id, assignee, .. }
                        if issue_id == full_id && assignee == &agent
                )
            })
            .count()
    }

    #[test]
    fn test_claim_acquire_synchronized_publishes_issue_and_event() -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Sync Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        let lease_id = execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:worker")?;
        assert!(!lease_id.as_str().is_empty());

        // Issue is assigned with a stamped claimed_at, and exactly one claim event
        // was appended in the same publication (@/inv/event-log).
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
    fn test_claim_acquire_appends_one_event_per_assignee_change() -> Result<()> {
        let (temp, executor) =
            setup_test_repo_with_config("[worktree]\nenforce_leases = \"off\"\n")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Reclaimed Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        let acquire =
            || execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:worker");

        acquire()?;
        acquire()?;
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 1);

        executor
            .issues()
            .unassign_issue(&IssueIdPrefix::from(&issue_id))?;
        acquire()?;
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 2);
        Ok(())
    }

    #[test]
    fn test_claim_capture_spec_requests_only_the_issue_record_and_the_log_tail() -> Result<()> {
        use crate::repository_state::{ReadMode, VirtualPath};

        let issue_id = crate::test_utils::issue_id(0x5000_4000_8000_0000_0000_0001_abcd_ef12);
        let record = VirtualPath::issue_record(&issue_id);
        let spec = super::claim_capture_spec(&issue_id)?;

        assert_eq!(spec.paths().count(), 2);
        assert_eq!(spec.read_mode(&record), Some(ReadMode::Whole));
        assert_eq!(spec.read_mode(&VirtualPath::EVENTS), Some(ReadMode::Tail));
        Ok(())
    }

    /// A repository's accumulated history does not decide whether an issue can
    /// be claimed: the claim capture charges no ceiling the log can outgrow, so
    /// a log past the 16 MiB one it once held still claims and still appends
    /// exactly one event (@/inv/event-log).
    #[test]
    fn test_claim_acquire_synchronized_succeeds_over_an_event_log_past_the_former_ceiling(
    ) -> Result<()> {
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Long History Issue".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        grow_event_log_beyond(&storage, 16 * 1024 * 1024)?;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 600, "agent:worker")?;

        assert_eq!(
            storage
                .load_issue(&issue_id)?
                .assignee
                .as_ref()
                .map(ToString::to_string),
            Some("agent:worker".to_string())
        );
        assert_eq!(claim_events_for(&storage, &issue_id, "agent:worker"), 1);
        Ok(())
    }

    #[test]
    fn test_claim_acquire_distinguishes_full_ids_with_shared_prefix() -> Result<()> {
        let (temp, storage) = setup_test_repo()?;
        let first_id = create_test_issue_with_id(
            &storage,
            &crate::test_utils::issue_id(0x3000_4000_8000_0000_0000_0001_abcd_ef12),
            "First shared-prefix issue",
        )?;
        let second_id = create_test_issue_with_id(
            &storage,
            &crate::test_utils::issue_id(0x4000_4000_8000_0000_0000_0002_abcd_ef12),
            "Second shared-prefix issue",
        )?;
        execute_claim_acquire_test(&temp, &storage, &first_id, 600, "agent:worker")?;

        execute_claim_acquire_test(&temp, &storage, &second_id, 600, "agent:worker")?;

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

        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Reverse Order".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;
        let full_id = issue_id.clone();

        ensure_git_checkout(&temp)?;
        let paths = create_test_paths(&temp);

        // Hold a repository mutation session, THEN attempt to coordinate: reverse
        // acquisition is rejected before any coordinator lock is taken.
        let layout = discover_repository_layout(&paths.worktree_root, storage.root()).unwrap();
        let _session = storage.open_mutation_session(layout).unwrap();
        let result = executor.with_worktree_paths(paths).claims().acquire(
            &full_id,
            600,
            Some("agent:worker"),
            None,
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
        let (temp, executor) = setup_test_repo_with_config("")?;
        let storage = executor.storage().clone();
        let issue_id = executor
            .issues()
            .create_issue(
                "Coordinator Only".to_string(),
                "Test description".to_string(),
                crate::domain::Priority::Normal,
                Vec::new(),
                Vec::new(),
                None,
                None,
                false,
            )?
            .issue
            .id;

        execute_claim_acquire_test(&temp, &storage, &issue_id, 0, "agent:worker")?;

        // Snapshot repository-owned state after the acquire.
        let issue_before = storage.load_issue(&issue_id)?;
        let events_before = storage.read_events()?;

        // Coordinator-only operations (heartbeat, renew, force-evict/release) never
        // open a repository session, so they cannot mutate the issue or event log.
        let paths = create_test_paths(&temp);
        let identity = load_or_create_worktree_identity(&paths, "test-branch")?;
        let coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(Duration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            identity.worktree_id,
            "agent:worker".parse()?,
        );
        coordinator.heartbeat(&issue_id)?;
        coordinator.renew_lease(&issue_id, 0)?;
        execute_claim_force_evict_test(&temp, &issue_id, "released")?;

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

    #[test]
    fn test_recover_removes_stale_lease_stage_files_and_keeps_lease_records() -> Result<()> {
        use crate::storage::WorktreePaths;
        let (temp, storage) = setup_test_repo()?;
        ensure_git_checkout(&temp)?;
        let leased = crate::test_utils::issue_id(1);
        let shared_jit = create_test_paths(&temp).shared_jit;
        let now = chrono::Utc::now();
        crate::test_lease_store::plant_lease(
            &shared_jit,
            &Lease {
                lease_id: "00000000-0000-4000-8000-000000000001".parse()?,
                issue_id: leased.clone(),
                agent_id: "agent:holder".parse()?,
                worktree_id: "wt:holder".parse()?,
                branch: None,
                ttl_secs: 600,
                acquired_at: now,
                expires_at: Some(now + chrono::Duration::seconds(600)),
                last_beat: now,
            },
        )?;
        let record = WorktreePaths::lease_path(&shared_jit, &leased);
        let stale_stage = WorktreePaths::lease_stage_path(&shared_jit, &leased);
        let fresh_stage =
            WorktreePaths::lease_stage_path(&shared_jit, &crate::test_utils::issue_id(2));
        fs::write(&stale_stage, "interrupted")?;
        fs::write(&fresh_stage, "in flight")?;
        let past_threshold = std::time::SystemTime::now()
            - Duration::from_secs(crate::runtime_defaults::TEMP_CLEANUP_THRESHOLD_SECS + 60);
        for aged in [&record, &stale_stage] {
            fs::File::options()
                .write(true)
                .open(aged)?
                .set_modified(past_threshold)?;
        }

        let report = CommandExecutor::new(storage.clone())
            .with_worktree_paths(create_test_paths(&temp))
            .claims()
            .recover()?;

        assert!(!stale_stage.exists(), "{report:?}");
        assert!(fresh_stage.exists() && record.exists(), "{report:?}");
        Ok(())
    }
}
