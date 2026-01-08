//! Claim coordination command implementations.
//!
//! Provides CLI interface to the lease-based claim coordination system.

use anyhow::{Context, Result};
use crate::storage::{ClaimCoordinator, FileLocker, IssueStore};
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::worktree_identity::load_or_create_worktree_identity;
use std::process::Command;
use std::time::Duration;

/// Get current git branch name.
fn get_current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .context("Failed to get current git branch")?;

    if !output.status.success() {
        return Ok("main".to_string()); // Default fallback
    }

    Ok(String::from_utf8(output.stdout)?
        .trim()
        .to_string())
}

/// Execute `jit claim acquire` command.
///
/// Acquires an exclusive lease on an issue for the specified agent.
pub fn execute_claim_acquire<S: IssueStore>(
    storage: &S,
    issue_id: &str,
    ttl_secs: u64,
    agent_id: Option<&str>,
    _reason: Option<&str>,
) -> Result<String> {
    // Validate issue exists
    let _issue = storage
        .load_issue(issue_id)
        .with_context(|| format!("Issue {} not found", issue_id))?;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch
    let branch = get_current_branch()?;

    // Load or generate worktree identity
    let identity = load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Determine agent ID (use provided or generate from username)
    let agent = agent_id
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            // Use system username as default agent ID
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .map(|u| format!("agent:{}", u))
                .unwrap_or_else(|_| "agent:default".to_string())
        });

    // Create file locker with 5-second timeout
    let locker = FileLocker::new(Duration::from_secs(5));

    // Create claim coordinator
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        locker,
        identity.worktree_id.clone(),
        agent.clone(),
    );

    // Initialize control plane if needed
    coordinator.init()?;

    // Acquire claim - coordinator already has agent_id, worktree_id, branch baked in
    let lease = coordinator.acquire_claim(issue_id, ttl_secs)?;

    Ok(lease.lease_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::JsonFileStorage;
    use crate::domain::Priority;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_claim_acquire_creates_lease() -> Result<()> {
        // Setup
        let temp = TempDir::new()?;
        let repo_root = temp.path();

        // Initialize .jit
        let jit_root = repo_root.join(".jit");
        fs::create_dir_all(&jit_root)?;

        // Initialize .git/jit (control plane)
        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir)?;

        let storage = JsonFileStorage::new(&jit_root)?;

        // Create test issue
        let issue_id = storage.create_issue(
            "Test Issue".to_string(),
            "Description".to_string(),
            Priority::Normal,
        )?;

        // Acquire claim
        let lease_id = execute_claim_acquire(
            &storage,
            &issue_id,
            600,
            Some("agent:test"),
            None,
        )?;

        // Verify lease ID returned
        assert!(!lease_id.is_empty());

        Ok(())
    }

    #[test]
    fn test_claim_acquire_fails_on_nonexistent_issue() -> Result<()> {
        // Setup
        let temp = TempDir::new()?;
        let repo_root = temp.path();

        let jit_root = repo_root.join(".jit");
        fs::create_dir_all(&jit_root)?;

        let git_dir = repo_root.join(".git");
        fs::create_dir_all(&git_dir)?;

        let storage = JsonFileStorage::new(&jit_root)?;

        // Try to acquire claim on non-existent issue
        let result = execute_claim_acquire(
            &storage,
            "nonexistent",
            600,
            Some("agent:test"),
            None,
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));

        Ok(())
    }
}
