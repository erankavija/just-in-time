//! Claim coordination command implementations.
//!
//! Provides CLI interface to the lease-based claim coordination system.

use crate::storage::worktree_identity::load_or_create_worktree_identity;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{ClaimCoordinator, FileLocker, IssueStore};
use anyhow::{Context, Result};
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

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
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
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Determine agent ID (use provided or generate from username)
    let agent = agent_id.map(|s| s.to_string()).unwrap_or_else(|| {
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
    use crate::domain::Issue;
    use crate::storage::worktree_paths::WorktreePaths;
    use crate::storage::{ClaimCoordinator, FileLocker, JsonFileStorage};
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Helper to set up test environment without changing current directory
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

    /// Execute claim acquire with manually constructed paths (bypassing WorktreePaths::detect)
    fn execute_claim_acquire_test(
        temp: &TempDir,
        storage: &JsonFileStorage,
        issue_id: &str,
        ttl_secs: u64,
        agent_id: &str,
    ) -> Result<String> {
        // Validate issue exists
        let _issue = storage.load_issue(issue_id)?;

        // Manually construct WorktreePaths for testing
        let paths = WorktreePaths {
            common_dir: temp.path().join(".git"),
            worktree_root: temp.path().to_path_buf(),
            local_jit: temp.path().join(".jit"),
            shared_jit: temp.path().join(".git/jit"),
        };

        // Get or create worktree identity
        let branch = "test-branch".to_string();
        let identity =
            load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

        // Create coordinator
        let locker = FileLocker::new(Duration::from_secs(5));
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

    /// Helper to create a test issue
    fn create_test_issue(storage: &JsonFileStorage, title: &str) -> Result<String> {
        let issue = Issue::new(title.to_string(), "Test description".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue)?;
        Ok(issue_id)
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

        let ttls = vec![60, 600, 3600, 0];
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
        assert!(temp.path().join(".git/jit/heartbeat").exists());
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
}
