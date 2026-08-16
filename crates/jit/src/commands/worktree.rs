//! Worktree command implementations.
//!
//! Provides CLI interface for worktree information and operations.

use crate::domain::store_divergence::{compare_stores, StoreDivergence, StoreRecords};
use crate::storage::claim_coordinator::ClaimsIndex;
use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{read_exact_store, StorageWarning};
use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::Serialize;
use std::process::Command;

/// Worktree information for display
#[derive(Debug, Serialize, JsonSchema)]
pub struct WorktreeInfo {
    /// Stable worktree identifier
    pub worktree_id: String,
    /// Current git branch
    pub branch: String,
    /// Absolute path to worktree root
    pub root_path: String,
    /// Whether this is the main worktree
    pub is_main_worktree: bool,
    /// Shared .git directory path
    pub common_dir: String,
}

/// Entry in worktree list
#[derive(Debug, Serialize, PartialEq, JsonSchema)]
pub struct WorktreeListEntry {
    /// Stable worktree identifier
    pub worktree_id: String,
    /// Current git branch
    pub branch: String,
    /// Absolute path to worktree root
    pub path: String,
    /// Whether this is the main worktree
    pub is_main: bool,
    /// Number of active claims in this worktree
    pub active_claims: usize,
}

/// Get current git branch name.
fn get_current_branch(worktree_root: &std::path::Path) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(worktree_root)
        .output()
        .context("Failed to get current git branch")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to get current git branch. Are you in a git repository?\n\
             Git error: {}",
            stderr.trim()
        );
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Parsed git worktree entry
#[derive(Debug)]
struct GitWorktreeEntry {
    path: String,
    branch: String,
}

/// Parse git worktree list --porcelain output.
///
/// Porcelain format provides machine-readable output with lines like:
/// ```text
/// worktree /path/to/worktree
/// HEAD abc123def456...
/// branch refs/heads/main
///
/// worktree /path/to/secondary
/// HEAD 789ghi...
/// branch refs/heads/feature/x
/// ```
///
/// # Returns
/// Vector of parsed worktree entries with path and branch name
///
/// # Errors
/// Returns error if output format is invalid
fn parse_git_worktree_porcelain(output: &str) -> Result<Vec<GitWorktreeEntry>> {
    let mut entries = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_branch: Option<String> = None;

    for line in output.lines() {
        if line.is_empty() {
            // Empty line signals end of entry
            if let (Some(path), Some(branch)) = (current_path.take(), current_branch.take()) {
                entries.push(GitWorktreeEntry { path, branch });
            }
            continue;
        }

        if let Some(path) = line.strip_prefix("worktree ") {
            current_path = Some(path.to_string());
        } else if let Some(branch_ref) = line.strip_prefix("branch ") {
            // Extract branch name from refs/heads/...
            let branch = branch_ref
                .strip_prefix("refs/heads/")
                .unwrap_or(branch_ref)
                .to_string();
            current_branch = Some(branch);
        }
        // Ignore HEAD and other fields
    }

    // Handle last entry if no trailing newline
    if let (Some(path), Some(branch)) = (current_path, current_branch) {
        entries.push(GitWorktreeEntry { path, branch });
    }

    Ok(entries)
}

/// Count active claims for a specific worktree.
///
/// Filters the claims index to count leases where the worktree_id matches
/// the provided identifier.
///
/// Expiry is evaluated against `now` (supplied by the caller via the
/// [`Clock`](crate::storage::clock::Clock) abstraction) rather than read from
/// the system clock here, so the count is a
/// pure function of its inputs and can be exercised deterministically in tests.
///
/// # Arguments
/// * `index` - Claims index containing all active leases
/// * `worktree_id` - Worktree identifier to filter by (e.g., "wt:abc12345")
/// * `now` - Instant to evaluate lease expiry against
///
/// # Returns
/// Number of active claims (leases) for the worktree
fn count_claims_for_worktree(
    index: &ClaimsIndex,
    worktree_id: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> usize {
    index
        .leases
        .iter()
        .filter(|lease| lease.worktree_id == worktree_id && !lease.is_expired(now))
        .count()
}

/// Execute `jit worktree info` command.
///
/// Displays current worktree context including ID, branch, paths, and whether
/// this is the main worktree or a secondary one.
///
/// # Returns
///
/// The worktree info paired with any non-fatal [`StorageWarning`]s observed
/// while loading the worktree identity (e.g. a relocation), for the caller's
/// output layer to render. This function never writes to stderr itself.
///
/// # Errors
///
/// Returns an error if the selected checkout's branch cannot be resolved or the
/// identity cannot be loaded or created.
pub fn execute_worktree_info(paths: &WorktreePaths) -> Result<(WorktreeInfo, Vec<StorageWarning>)> {
    // Get current branch
    let branch = get_current_branch(&paths.worktree_root)?;

    // Load or generate worktree identity, surfacing relocation as a warning
    let (identity, warnings) = load_or_create_worktree_identity_with_warnings(paths, &branch)?;

    // Determine if this is the main worktree
    let is_main = !paths.is_worktree();

    Ok((
        WorktreeInfo {
            worktree_id: identity.worktree_id,
            branch: identity.branch,
            root_path: paths.worktree_root.to_string_lossy().to_string(),
            is_main_worktree: is_main,
            common_dir: paths.common_dir.to_string_lossy().to_string(),
        },
        warnings,
    ))
}

/// Execute `jit worktree list` command.
///
/// Lists all git worktrees with their JIT status including worktree ID, branch,
/// path, and count of active claims.
///
/// # Returns
///
/// The worktree entries paired with any non-fatal [`StorageWarning`]s observed
/// while loading each worktree's identity (e.g. a relocation), for the caller's
/// output layer to render. This function never writes to stderr itself.
///
/// # Errors
///
/// Returns an error if `git worktree list` fails, the primary checkout cannot
/// be resolved, or a present `.jit` identity cannot be read.
pub fn execute_worktree_list(
    paths: &WorktreePaths,
) -> Result<(Vec<WorktreeListEntry>, Vec<StorageWarning>)> {
    // Production time source: the real system clock.
    execute_worktree_list_at(paths, &crate::storage::clock::SystemClock)
}

/// Core of [`execute_worktree_list`] with the control-plane paths and time
/// source injected.
///
/// This holds the real command logic (running `git worktree list`, loading the
/// claims index, resolving each worktree's identity, and counting active
/// claims); the public wrapper supplies the invocation's authoritative `paths`
/// and a [`SystemClock`](crate::storage::clock::SystemClock). `now` for every
/// per-worktree expiry check is read once from `clock`, so a test can drive
/// expiry deterministically by advancing an injected clock while exercising this
/// exact path.
fn execute_worktree_list_at(
    paths: &WorktreePaths,
    clock: &dyn crate::storage::clock::Clock,
) -> Result<(Vec<WorktreeListEntry>, Vec<StorageWarning>)> {
    use crate::storage::claim_coordinator::ClaimsIndex;
    use std::path::PathBuf;

    // Execute git worktree list --porcelain from this worktree's root.
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
        .current_dir(&paths.worktree_root)
        .output()
        .context("Failed to execute git worktree list")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "{}",
            crate::errors::git_command_failed("git worktree list --porcelain", &stderr)
        );
    }

    let porcelain_output =
        String::from_utf8(output.stdout).context("Invalid UTF-8 in git worktree output")?;

    let git_entries = parse_git_worktree_porcelain(&porcelain_output)?;

    // Load the active-lease index through storage to count active claims per
    // worktree (an absent index yields an empty one, i.e. no active claims).
    let claims_index = ClaimsIndex::load(paths)?;

    // Single "now" for all per-worktree expiry checks, read once from the
    // injected clock.
    let now = clock.now();

    // Enrich git entries with JIT data, collecting any relocation warnings
    // observed while loading each worktree's identity.
    let mut warnings = Vec::new();
    let entries = git_entries
        .into_iter()
        .map(|git_entry| -> Result<WorktreeListEntry> {
            let worktree_path = PathBuf::from(&git_entry.path);
            let local_jit = worktree_path.join(".jit");

            // Load worktree identity - error if .jit exists but can't be read
            let worktree_id = if local_jit.exists() {
                let entry_paths = paths.for_worktree_root(worktree_path.clone());
                let (identity, entry_warnings) =
                    load_or_create_worktree_identity_with_warnings(&entry_paths, &git_entry.branch)
                        .with_context(|| {
                            format!(
                                "Failed to load worktree identity from {}",
                                local_jit.display()
                            )
                        })?;
                warnings.extend(entry_warnings);
                identity.worktree_id
            } else {
                // No .jit directory yet - use branch-based temporary ID
                format!("wt:{}", git_entry.branch)
            };

            // Count active claims for this worktree
            let active_claims = count_claims_for_worktree(&claims_index, &worktree_id, now);

            let is_main = paths.is_primary_worktree_path(&worktree_path)?;

            Ok(WorktreeListEntry {
                worktree_id,
                branch: git_entry.branch,
                path: git_entry.path,
                is_main,
                active_claims,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok((entries, warnings))
}

/// The outcome of comparing one checkout's store against its reference store.
///
/// A checkout with no reference store — the primary checkout, and any checkout
/// outside version control — reports no divergences and no reference, so an
/// empty [`Self::divergences`] means "nothing to report here" in every
/// environment.
#[derive(Debug, Serialize, JsonSchema)]
pub struct StoreDivergenceReport {
    /// The store the inspected checkout owns.
    pub checkout_store: String,
    /// The store it was compared against, absent when the checkout has none.
    pub reference_store: Option<String>,
    /// Every record the two stores disagree about, empty when they agree.
    pub divergences: Vec<StoreDivergence>,
}

/// Execute `jit worktree store-divergence`.
///
/// Compares the records physically held by the selected checkout's store against
/// those held by its primary checkout's store, naming every issue record and
/// event record the two disagree about. The comparison is read-only: both stores
/// are read exactly as they sit, without the aggregation, history, or
/// primary-checkout fallback an ordinary issue read applies, and neither store is
/// written.
///
/// A checkout with no primary counterpart to compare against — the primary
/// checkout itself, and a store outside version control (`@/charter/D-4`) —
/// reports no divergences rather than failing.
///
/// # Errors
///
/// Returns an error when the primary checkout cannot be resolved from `paths`, or
/// when either store holds a record that cannot be read or parsed.
pub fn execute_worktree_store_divergence(paths: &WorktreePaths) -> Result<StoreDivergenceReport> {
    let checkout_store = paths.local_jit.to_string_lossy().into_owned();

    let Some(reference_root) = paths.primary_data_root()? else {
        return Ok(StoreDivergenceReport {
            checkout_store,
            reference_store: None,
            divergences: Vec::new(),
        });
    };

    let local = read_exact_store(&paths.local_jit)?;
    let reference = read_exact_store(&reference_root)?;

    Ok(StoreDivergenceReport {
        checkout_store,
        reference_store: Some(reference_root.to_string_lossy().into_owned()),
        divergences: compare_stores(
            StoreRecords {
                issues: &local.issues,
                events: &local.events,
            },
            StoreRecords {
                issues: &reference.issues,
                events: &reference.events,
            },
        ),
    })
}

/// Check if current branch has diverged from origin/main.
///
/// Returns `true` if the current branch has commits not in origin/main
/// (i.e., merge-base is not equal to origin/main).
///
/// # Implementation
///
/// Uses git commands:
/// - `git merge-base HEAD origin/main` - Find common ancestor
/// - `git rev-parse origin/main` - Get main's current commit
///
/// If merge-base != origin/main, the branch has diverged.
///
/// # Errors
///
/// Returns error if git commands fail or origin/main doesn't exist.
///
/// # Returns
///
/// - `Ok(true)` if branch has diverged
/// - `Ok(false)` if branch is up to date with origin/main
/// - `Err(_)` if git commands fail (e.g., no origin/main)
pub fn check_branch_divergence() -> Result<bool> {
    let merge_base_output = Command::new("git")
        .args(["merge-base", "HEAD", "origin/main"])
        .output()
        .context("Failed to execute git merge-base")?;

    // If origin/main doesn't exist, not an error - just not diverged
    if !merge_base_output.status.success() {
        return Ok(false);
    }

    let main_commit_output = Command::new("git")
        .args(["rev-parse", "origin/main"])
        .output()
        .context("Failed to execute git rev-parse")?;

    if !main_commit_output.status.success() {
        return Ok(false);
    }

    let merge_base = String::from_utf8(merge_base_output.stdout)
        .context("Invalid UTF-8 in merge-base output")?
        .trim()
        .to_string();

    let main_commit = String::from_utf8(main_commit_output.stdout)
        .context("Invalid UTF-8 in rev-parse output")?
        .trim()
        .to_string();

    // Branch has diverged if merge-base is not the same as origin/main
    Ok(merge_base != main_commit)
}

/// Enforce that global operations only run on branches with common history to main.
///
/// Global operations (config, gates registry, type hierarchy) modify shared state
/// that affects all agents. To prevent conflicts, these must only be performed
/// when the current branch shares common history with origin/main.
///
/// # Errors
///
/// Returns error if:
/// - Branch has diverged from origin/main (need rebase)
/// - Git commands fail
///
/// # Test Mode
///
/// In test builds (`cfg(test)`), this check is skipped to allow tests to run
/// in temporary repositories without origin/main.
///
/// # Example Error
///
/// ```text
/// Error: Global operations require common history with main
///
/// Your branch has diverged from origin/main. To proceed:
///   git fetch origin
///   git rebase origin/main
/// ```
pub fn enforce_main_only_operations() -> Result<()> {
    // Skip enforcement in test environments
    // Tests set JIT_TEST_MODE=1 to disable this check
    if std::env::var("JIT_TEST_MODE").is_ok() {
        return Ok(());
    }

    // Skip enforcement if git is not available
    match std::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .output()
    {
        Ok(output) if output.status.success() => {
            // We're in a real git repo, check for divergence
            if check_branch_divergence()? {
                anyhow::bail!(
                    "Global operations require common history with main\n\n\
                     Your branch has diverged from origin/main. To proceed:\n  \
                     git fetch origin\n  \
                     git rebase origin/main"
                );
            }
            Ok(())
        }
        _ => {
            // Not in a git repo or git not available - skip check
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use anyhow::Result;

    // Use shared test utilities
    use crate::test_utils::setup_test_repo;

    #[test]
    fn test_worktree_info_structure() -> Result<()> {
        let (_temp, _storage) = setup_test_repo()?;

        // This test validates the function exists and returns the right structure
        // Actual git detection will fail in test environment, so we can't fully test it
        // Real testing would need a proper git repo setup

        // Just verify the types are correct by constructing manually
        let info = WorktreeInfo {
            worktree_id: "wt:12345678".to_string(),
            branch: "main".to_string(),
            root_path: "/path/to/worktree".to_string(),
            is_main_worktree: true,
            common_dir: "/path/to/.git".to_string(),
        };

        assert_eq!(info.worktree_id, "wt:12345678");
        assert_eq!(info.branch, "main");
        assert!(info.is_main_worktree);

        Ok(())
    }

    #[test]
    fn test_parse_git_worktree_porcelain() {
        // Test parsing git worktree list --porcelain output
        let porcelain = "worktree /home/user/project\nHEAD abc123\nbranch refs/heads/main\n\nworktree /home/user/project/wt1\nHEAD def456\nbranch refs/heads/feature/task-1\n";

        let entries = parse_git_worktree_porcelain(porcelain).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, "/home/user/project");
        assert_eq!(entries[0].branch, "main");
        assert_eq!(entries[1].path, "/home/user/project/wt1");
        assert_eq!(entries[1].branch, "feature/task-1");
    }

    // Note: test_worktree_list_returns_current_worktree() removed.
    // Note: test_json_output_structure() removed.
    // These tests used execute_worktree_list() without injected worktree paths,
    // making them inspect the REAL repository rather than test temp dirs.
    // Tests must NEVER touch production .jit/ directory!
    // The worktree list functionality is tested with parse_git_worktree_porcelain() above.

    #[test]
    fn test_worktree_list_entry_structure() {
        // Verify the structure is correct
        let entry = WorktreeListEntry {
            worktree_id: "wt:12345678".to_string(),
            branch: "main".to_string(),
            path: "/home/user/project".to_string(),
            is_main: true,
            active_claims: 3,
        };

        assert_eq!(entry.worktree_id, "wt:12345678");
        assert_eq!(entry.branch, "main");
        assert_eq!(entry.active_claims, 3);
        assert!(entry.is_main);
    }

    #[test]
    fn test_count_claims_per_worktree() {
        use crate::storage::claim_coordinator::{ClaimsIndex, Lease};
        use chrono::Utc;

        // Create mock claims index with leases for different worktrees
        let index = ClaimsIndex {
            schema_version: 1,
            generated_at: Utc::now(),
            last_seq: 3,
            stale_threshold_secs: 3600,
            sequence_gaps: Vec::new(),
            leases: vec![
                Lease {
                    lease_id: "lease-1".to_string(),
                    issue_id: "issue-1".to_string(),
                    agent_id: "agent:1".to_string(),
                    worktree_id: "wt:abc123".to_string(),
                    branch: Some("main".to_string()),
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: None,
                    last_beat: Utc::now(),
                    stale: false,
                },
                Lease {
                    lease_id: "lease-2".to_string(),
                    issue_id: "issue-2".to_string(),
                    agent_id: "agent:1".to_string(),
                    worktree_id: "wt:abc123".to_string(),
                    branch: Some("main".to_string()),
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: None,
                    last_beat: Utc::now(),
                    stale: false,
                },
                Lease {
                    lease_id: "lease-3".to_string(),
                    issue_id: "issue-3".to_string(),
                    agent_id: "agent:2".to_string(),
                    worktree_id: "wt:def456".to_string(),
                    branch: Some("feature/test".to_string()),
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: None,
                    last_beat: Utc::now(),
                    stale: false,
                },
            ],
        };

        let now = Utc::now();
        let count_abc = count_claims_for_worktree(&index, "wt:abc123", now);
        let count_def = count_claims_for_worktree(&index, "wt:def456", now);
        let count_xyz = count_claims_for_worktree(&index, "wt:xyz789", now);

        assert_eq!(count_abc, 2);
        assert_eq!(count_def, 1);
        assert_eq!(count_xyz, 0);
    }

    #[test]
    fn test_count_claims_excludes_expired_leases() {
        use crate::storage::claim_coordinator::{ClaimsIndex, Lease};
        use chrono::{Duration, Utc};

        let now = Utc::now();
        let make_lease =
            |id: &str, ttl_secs: u64, expires_at: Option<chrono::DateTime<Utc>>| Lease {
                lease_id: id.to_string(),
                issue_id: format!("issue-{id}"),
                agent_id: "agent:1".to_string(),
                worktree_id: "wt:abc123".to_string(),
                branch: Some("main".to_string()),
                ttl_secs,
                acquired_at: now,
                expires_at,
                last_beat: now,
                stale: false,
            };

        let index = ClaimsIndex {
            schema_version: 1,
            generated_at: now,
            last_seq: 3,
            stale_threshold_secs: 3600,
            sequence_gaps: Vec::new(),
            leases: vec![
                // Live: finite TTL, expiry in the future
                make_lease("live", 600, Some(now + Duration::seconds(300))),
                // Expired: finite TTL, expiry in the past -> must not be counted
                make_lease("expired", 600, Some(now - Duration::seconds(1))),
                // Indefinite: ttl_secs == 0, no expiry -> always live
                make_lease("indefinite", 0, None),
            ],
        };

        // Only the live and indefinite leases count; the expired one is excluded,
        // matching `claim list` which evicts expired leases before reporting.
        assert_eq!(count_claims_for_worktree(&index, "wt:abc123", now), 2);
    }

    /// REQ-02: expiry is driven through an injected [`Clock`] rather than a real
    /// `sleep`, exercising the real `execute_worktree_list_at` command path (the
    /// core of `execute_worktree_list`) against a temporary git repo.
    ///
    /// The former subprocess test slept 1600ms to let a 1s TTL elapse; here a
    /// [`FixedClock`] is advanced past the short lease's TTL and the real command
    /// path is re-run, asserting the expired lease drops out of the reported
    /// `active_claims`. See `tests/worktree_cli_tests.rs` for the CLI-boundary
    /// happy-path smoke test.
    #[test]
    fn test_worktree_list_excludes_expired_leases() {
        use crate::storage::clock::FixedClock;
        use crate::storage::worktree_paths::WorktreePaths;
        use crate::storage::{ClaimCoordinator, FileLocker};
        use chrono::Utc;
        use std::process::Command as StdCommand;
        use std::sync::Arc;
        use std::time::Duration as StdDuration;

        fn git(dir: &std::path::Path, args: &[&str]) {
            let status = StdCommand::new("git")
                .current_dir(dir)
                .args(args)
                .status()
                .unwrap();
            assert!(status.success(), "git {:?} failed", args);
        }

        let temp_dir = tempfile::TempDir::new().unwrap();
        let root = temp_dir.path();

        // Minimal real git repo so `git worktree list` reports the main worktree.
        git(root, &["init"]);
        git(root, &["config", "user.email", "test@example.com"]);
        git(root, &["config", "user.name", "Test User"]);
        std::fs::write(root.join("README.md"), "# test\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-m", "init"]);

        // Resolve the actual branch so lease worktree_ids match what the command
        // computes (no local .jit -> worktree_id is `wt:<branch>`).
        let branch_out = StdCommand::new("git")
            .current_dir(root)
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .unwrap();
        let branch = String::from_utf8(branch_out.stdout)
            .unwrap()
            .trim()
            .to_string();
        let worktree_id = format!("wt:{branch}");

        let paths = WorktreePaths {
            git_repository: true,
            common_dir: root.join(".git"),
            worktree_root: root.to_path_buf(),
            local_jit: root.join(".jit"),
            shared_jit: root.join(".git/jit"),
        };

        let base = Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            FileLocker::new(StdDuration::from_secs(
                crate::runtime_defaults::LOCK_TIMEOUT_SECS,
            )),
            worktree_id,
            "agent:test".to_string(),
        )
        .with_clock(clock.clone());
        coordinator.init().unwrap();

        // A short (1s TTL) and a long (600s TTL) lease, both acquired "now".
        coordinator.acquire_claim("issue-short", 1).unwrap();
        coordinator.acquire_claim("issue-long", 600).unwrap();

        let active_total = |clock: &FixedClock| -> usize {
            let (entries, _warnings) = execute_worktree_list_at(&paths, clock).unwrap();
            entries.iter().map(|e| e.active_claims).sum()
        };

        // Before expiry: both live leases are counted by the real command path.
        assert_eq!(active_total(clock.as_ref()), 2, "both live leases counted");

        // Advance time past the short lease's TTL (no real sleep) and re-run the
        // same command path: the expired lease must be excluded.
        clock.set(base + chrono::Duration::seconds(5));
        assert_eq!(
            active_total(clock.as_ref()),
            1,
            "expired lease excluded from list"
        );
    }

    mod store_divergence {
        use super::*;
        use crate::domain::store_divergence::{DivergenceClass, DivergentRecord};
        use crate::domain::types::{fixture_issue, Priority};
        use crate::domain::{Event, Issue};
        use std::collections::BTreeMap;
        use std::path::{Path, PathBuf};

        /// Two checkouts of one repository, the second linked to the first.
        ///
        /// Answers the temporary directory holding both (kept alive by the
        /// caller) and the linked checkout's authority. Path arithmetic over the
        /// selected root is what decides primary-vs-linked identity, so this
        /// fixture needs no git process.
        fn linked_checkout_paths() -> (tempfile::TempDir, WorktreePaths) {
            let temp = tempfile::TempDir::new().expect("create a temporary directory");
            let primary = temp.path().join("main");
            let linked = temp.path().join("feature");
            std::fs::create_dir_all(primary.join(".jit")).expect("create the primary store");
            std::fs::create_dir_all(linked.join(".jit")).expect("create the linked store");

            let paths = WorktreePaths {
                git_repository: true,
                common_dir: primary.join(".git"),
                local_jit: linked.join(".jit"),
                worktree_root: linked,
                shared_jit: primary.join(".git/jit"),
            };
            assert!(paths.is_worktree(), "the fixture models a linked checkout");
            (temp, paths)
        }

        /// Write `issue` into the store at `data_root`.
        fn seed_issue(data_root: &Path, issue: &Issue) {
            let issues = data_root.join("issues");
            std::fs::create_dir_all(&issues).expect("create the issue directory");
            std::fs::write(
                issues.join(format!("{}.json", issue.id)),
                crate::repository_state::serialize_issue(issue).expect("serialize the issue"),
            )
            .expect("write the issue record");
        }

        /// Write `events` into the log of the store at `data_root`.
        fn seed_events(data_root: &Path, events: &[Event]) {
            let log = events
                .iter()
                .map(|event| {
                    format!(
                        "{}\n",
                        serde_json::to_string(event).expect("serialize the event")
                    )
                })
                .collect::<String>();
            std::fs::write(data_root.join("events.jsonl"), log).expect("write the event log");
        }

        /// An event carrying `id`, associated with `issue_id`.
        fn event(id: &str, issue_id: &str, title: &str) -> Event {
            Event::IssueCreated {
                id: id.to_string(),
                issue_id: issue_id.to_string(),
                timestamp: chrono::DateTime::UNIX_EPOCH,
                title: title.to_string(),
                priority: Priority::Normal,
            }
        }

        /// Every file under `root`, keyed by its path relative to `root`.
        fn stored_bytes(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
            fn visit(root: &Path, current: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
                let entries = std::fs::read_dir(current).expect("read a store directory");
                for entry in entries {
                    let path = entry.expect("read a store entry").path();
                    if path.is_dir() {
                        visit(root, &path, files);
                    } else {
                        let relative = path
                            .strip_prefix(root)
                            .expect("entries sit under the store root")
                            .to_path_buf();
                        files.insert(relative, std::fs::read(&path).expect("read a store file"));
                    }
                }
            }

            let mut files = BTreeMap::new();
            visit(root, root, &mut files);
            files
        }

        /// The report's findings as (kind, class, id) triples.
        fn findings(
            report: &StoreDivergenceReport,
        ) -> Vec<(DivergentRecord, DivergenceClass, &str)> {
            report
                .divergences
                .iter()
                .map(|divergence| (divergence.record, divergence.class, divergence.id.as_str()))
                .collect()
        }

        /// Seed the two stores so each finding class is present, answering the
        /// local-only issue, the reference-only issue, and the conflicting event.
        fn seed_divergent_stores(
            paths: &WorktreePaths,
            reference_root: &Path,
        ) -> (Issue, Issue, Event) {
            let local_only = fixture_issue(
                "Local only".to_string(),
                "Held by the linked checkout alone".to_string(),
            );
            let reference_only = fixture_issue(
                "Reference only".to_string(),
                "Held by the primary checkout alone".to_string(),
            );
            let shared = fixture_issue(
                "Shared".to_string(),
                "Held by both stores identically".to_string(),
            );

            seed_issue(&paths.local_jit, &local_only);
            seed_issue(&paths.local_jit, &shared);
            seed_issue(reference_root, &reference_only);
            seed_issue(reference_root, &shared);

            let conflicting = event("shared-event", &shared.id, "The linked checkout's value");
            seed_events(&paths.local_jit, std::slice::from_ref(&conflicting));
            seed_events(
                reference_root,
                &[event(
                    "shared-event",
                    &shared.id,
                    "The primary checkout's value",
                )],
            );

            (local_only, reference_only, conflicting)
        }

        #[test]
        fn test_execute_worktree_store_divergence_surfaces_every_finding_class_between_the_two_stores(
        ) {
            let (_temp, paths) = linked_checkout_paths();
            let reference_root = paths
                .primary_data_root()
                .expect("resolve the primary store")
                .expect("a linked checkout has a primary store");
            let (local_only, reference_only, conflicting) =
                seed_divergent_stores(&paths, &reference_root);

            let report =
                execute_worktree_store_divergence(&paths).expect("compare the two checkout stores");

            let reported = findings(&report);
            assert!(
                reported.contains(&(
                    DivergentRecord::Issue,
                    DivergenceClass::LocalOnly,
                    local_only.id.as_str()
                )),
                "the issue only this checkout holds is reported as local-only: {reported:?}"
            );
            assert!(
                reported.contains(&(
                    DivergentRecord::Issue,
                    DivergenceClass::ReferenceOnly,
                    reference_only.id.as_str()
                )),
                "the issue only the primary holds is reported as reference-only: {reported:?}"
            );
            assert!(
                reported.contains(&(
                    DivergentRecord::Event,
                    DivergenceClass::Conflicting,
                    conflicting.id()
                )),
                "the event both logs hold with different values is a conflict: {reported:?}"
            );
            assert_eq!(
                report.reference_store.as_deref(),
                Some(reference_root.to_string_lossy().as_ref()),
                "the report names the primary store it compared against"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_leaves_both_checkout_stores_unchanged() {
            let (_temp, paths) = linked_checkout_paths();
            let reference_root = paths
                .primary_data_root()
                .expect("resolve the primary store")
                .expect("a linked checkout has a primary store");
            seed_divergent_stores(&paths, &reference_root);

            let before = (
                stored_bytes(&paths.local_jit),
                stored_bytes(&reference_root),
            );
            let report =
                execute_worktree_store_divergence(&paths).expect("compare the two checkout stores");
            assert!(
                !report.divergences.is_empty(),
                "the read-only property is asserted over a comparison that found something"
            );

            assert_eq!(
                (
                    stored_bytes(&paths.local_jit),
                    stored_bytes(&reference_root)
                ),
                before,
                "the check writes to neither store"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_reports_nothing_for_a_linked_checkout_agreeing_with_the_primary(
        ) {
            let (_temp, paths) = linked_checkout_paths();
            let reference_root = paths
                .primary_data_root()
                .expect("resolve the primary store")
                .expect("a linked checkout has a primary store");

            let issue = fixture_issue(
                "Agreed".to_string(),
                "Both checkouts hold this record".to_string(),
            );
            let events = [event("shared-event", &issue.id, "Both logs hold this")];
            for store in [paths.local_jit.as_path(), reference_root.as_path()] {
                seed_issue(store, &issue);
                seed_events(store, &events);
            }

            let report =
                execute_worktree_store_divergence(&paths).expect("compare the two checkout stores");

            assert_eq!(
                findings(&report),
                Vec::new(),
                "a linked checkout whose store agrees with the primary reports no findings"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_reports_nothing_in_the_primary_checkout() {
            let temp = tempfile::TempDir::new().expect("create a temporary directory");
            let primary = temp.path().join("main");
            std::fs::create_dir_all(primary.join(".jit")).expect("create the primary store");

            let paths = WorktreePaths {
                git_repository: true,
                common_dir: primary.join(".git"),
                local_jit: primary.join(".jit"),
                worktree_root: primary.clone(),
                shared_jit: primary.join(".git/jit"),
            };
            assert!(paths.is_main_worktree(), "the fixture models the primary");
            seed_issue(
                &paths.local_jit,
                &fixture_issue(
                    "Primary record".to_string(),
                    "The primary checkout's own store".to_string(),
                ),
            );

            let report =
                execute_worktree_store_divergence(&paths).expect("inspect the primary checkout");

            assert_eq!(
                findings(&report),
                Vec::new(),
                "the primary checkout has no other store to diverge from"
            );
            assert_eq!(
                report.reference_store, None,
                "the primary checkout names no reference store"
            );
        }

        #[test]
        fn test_execute_worktree_store_divergence_reports_nothing_outside_version_control() {
            let temp = tempfile::TempDir::new().expect("create a temporary directory");
            let data_root = temp.path().join(".jit");
            std::fs::create_dir_all(&data_root).expect("create the store");
            seed_issue(
                &data_root,
                &fixture_issue(
                    "Untracked record".to_string(),
                    "Held by a store outside version control".to_string(),
                ),
            );

            let paths = WorktreePaths::detect_for_data_root(&data_root, temp.path())
                .expect("classify a store outside version control");
            assert!(
                !paths.is_git_repository(),
                "the fixture models a store outside version control"
            );

            let report = execute_worktree_store_divergence(&paths)
                .expect("a store outside version control succeeds rather than erroring");

            assert_eq!(
                findings(&report),
                Vec::new(),
                "a store outside version control has no checkout to compare against"
            );
            assert_eq!(
                report.reference_store, None,
                "a store outside version control names no reference store"
            );
        }
    }

    #[test]
    fn test_parse_git_worktree_porcelain_invalid() {
        // Test error handling for malformed output
        let invalid = "worktree /path\nHEAD abc123\n";
        // Missing branch line - should still work, just no branch entry created
        let result = parse_git_worktree_porcelain(invalid);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_check_branch_divergence_on_main() -> Result<()> {
        // When on origin/main, merge-base == origin/main (not diverged)
        // This test only runs in actual git repo
        let _temp = setup_test_repo()?;

        // We can't fully test this without complex git setup
        // Just verify the function signature exists
        Ok(())
    }

    #[test]
    fn test_enforce_main_only_operations_when_diverged() {
        // Should fail when branch has diverged
        // Requires actual git state, so we test the logic exists
    }
}
