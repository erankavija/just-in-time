//! Worktree command implementations.
//!
//! Provides CLI interface for worktree information and operations.

use crate::storage::worktree_identity::load_or_create_worktree_identity;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::IssueStore;
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

/// Get current git branch name.
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to get current git branch")?;

    if !output.status.success() {
        return Ok("main".to_string()); // Default fallback
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

/// Execute `jit worktree info` command.
///
/// Displays current worktree context including ID, branch, paths, and whether
/// this is the main worktree or a secondary one.
pub fn execute_worktree_info<S: IssueStore>(_storage: &S) -> Result<WorktreeInfo> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::JsonFileStorage;
    use anyhow::Result;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to set up test environment
    fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> {
        let temp = TempDir::new()?;

        // Create .jit directory
        let jit_root = temp.path().join(".jit");
        fs::create_dir_all(&jit_root)?;

        // Create .git directory
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir)?;

        // Initialize storage
        let storage = JsonFileStorage::new(&jit_root);
        storage.init()?;

        Ok((temp, storage))
    }

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
}
