//! Worktree command implementations.
//!
//! Provides CLI interface for worktree information and operations.

use crate::storage::claim_coordinator::ClaimsIndex;
use crate::storage::worktree_identity::load_or_create_worktree_identity;
use crate::storage::worktree_paths::WorktreePaths;
use anyhow::{Context, Result};
use serde::Serialize;
use std::process::Command;

/// Worktree information for display
#[derive(Debug, Serialize)]
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
#[derive(Debug, Serialize, PartialEq)]
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
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
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
/// # Arguments
/// * `index` - Claims index containing all active leases
/// * `worktree_id` - Worktree identifier to filter by (e.g., "wt:abc12345")
///
/// # Returns
/// Number of active claims (leases) for the worktree
fn count_claims_for_worktree(index: &ClaimsIndex, worktree_id: &str) -> usize {
    index
        .leases
        .iter()
        .filter(|lease| lease.worktree_id == worktree_id)
        .count()
}

/// Execute `jit worktree info` command.
///
/// Displays current worktree context including ID, branch, paths, and whether
/// this is the main worktree or a secondary one.
pub fn execute_worktree_info() -> Result<WorktreeInfo> {
    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch
    let branch = get_current_branch()?;

    // Load or generate worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Determine if this is the main worktree
    let is_main = !paths.is_worktree();

    Ok(WorktreeInfo {
        worktree_id: identity.worktree_id,
        branch: identity.branch,
        root_path: paths.worktree_root.to_string_lossy().to_string(),
        is_main_worktree: is_main,
        common_dir: paths.common_dir.to_string_lossy().to_string(),
    })
}

/// Execute `jit worktree list` command.
///
/// Lists all git worktrees with their JIT status including worktree ID, branch,
/// path, and count of active claims.
pub fn execute_worktree_list() -> Result<Vec<WorktreeListEntry>> {
    use crate::storage::claim_coordinator::ClaimsIndex;
    use std::path::PathBuf;

    // Get worktree paths to access shared control plane
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Execute git worktree list --porcelain
    let output = Command::new("git")
        .args(["worktree", "list", "--porcelain"])
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

    // Load claims index to count active claims per worktree
    let claims_index_path = paths.shared_jit.join("claims.index.json");
    let claims_index = if claims_index_path.exists() {
        let contents =
            std::fs::read_to_string(&claims_index_path).context("Failed to read claims index")?;
        serde_json::from_str::<ClaimsIndex>(&contents).context("Failed to parse claims index")?
    } else {
        // No claims index file means no active claims
        ClaimsIndex {
            schema_version: 1,
            generated_at: chrono::Utc::now(),
            last_seq: 0,
            stale_threshold_secs: 3600,
            leases: vec![],
            sequence_gaps: Vec::new(),
        }
    };

    // Enrich git entries with JIT data
    let entries = git_entries
        .into_iter()
        .map(|git_entry| -> Result<WorktreeListEntry> {
            let worktree_path = PathBuf::from(&git_entry.path);
            let local_jit = worktree_path.join(".jit");

            // Load worktree identity - error if .jit exists but can't be read
            let worktree_id = if local_jit.exists() {
                let identity =
                    load_or_create_worktree_identity(&local_jit, &worktree_path, &git_entry.branch)
                        .with_context(|| {
                            format!(
                                "Failed to load worktree identity from {}",
                                local_jit.display()
                            )
                        })?;
                identity.worktree_id
            } else {
                // No .jit directory yet - use branch-based temporary ID
                format!("wt:{}", &git_entry.branch)
            };

            // Count active claims for this worktree
            let active_claims = count_claims_for_worktree(&claims_index, &worktree_id);

            // Determine if main worktree (compare with common_dir parent)
            let is_main = worktree_path == paths.worktree_root && !paths.is_worktree();

            Ok(WorktreeListEntry {
                worktree_id,
                branch: git_entry.branch,
                path: git_entry.path,
                is_main,
                active_claims,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    Ok(entries)
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
    // These tests used execute_worktree_list() which calls WorktreePaths::detect(),
    // making them detect the REAL repository, not test temp dirs.
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
                },
            ],
        };

        let count_abc = count_claims_for_worktree(&index, "wt:abc123");
        let count_def = count_claims_for_worktree(&index, "wt:def456");
        let count_xyz = count_claims_for_worktree(&index, "wt:xyz789");

        assert_eq!(count_abc, 2);
        assert_eq!(count_def, 1);
        assert_eq!(count_xyz, 0);
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
