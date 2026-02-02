//! Shared test utilities
//!
//! Common helpers used across multiple test modules to reduce duplication.

#![cfg(test)]

use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{IssueStore, JsonFileStorage};
use anyhow::Result;
use std::fs;
use tempfile::TempDir;

/// Standard test repository setup with .jit and .git directories
///
/// Creates a temporary directory with initialized jit storage.
/// Returns both the TempDir (which cleans up on drop) and the storage instance.
///
/// # Example
/// ```no_run
/// use jit::test_utils::setup_test_repo;
///
/// let (temp, storage) = setup_test_repo().unwrap();
/// // Use temp.path() and storage for tests
/// ```
pub fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> {
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

/// Create test WorktreePaths from a TempDir
///
/// Generates a WorktreePaths structure suitable for testing,
/// with standard paths relative to the temp directory.
///
/// # Example
/// ```no_run
/// use jit::test_utils::{setup_test_repo, create_test_paths};
///
/// let (temp, _storage) = setup_test_repo().unwrap();
/// let paths = create_test_paths(&temp);
/// assert_eq!(paths.worktree_root, temp.path());
/// ```
pub fn create_test_paths(temp: &TempDir) -> WorktreePaths {
    WorktreePaths {
        common_dir: temp.path().join(".git"),
        worktree_root: temp.path().to_path_buf(),
        local_jit: temp.path().join(".jit"),
        shared_jit: temp.path().join(".git/jit"),
    }
}
