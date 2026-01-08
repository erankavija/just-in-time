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
    use crate::storage::JsonFileStorage;
    use std::env;
    use std::fs;
    use tempfile::TempDir;

    /// Helper to set up test environment
    /// Must change to temp directory for WorktreePaths::detect() to work
    fn setup_test_repo() -> Result<(TempDir, JsonFileStorage, std::path::PathBuf)> {
        let temp = TempDir::new()?;
        let original_dir = env::current_dir()?;

        // Create .jit directory
        let jit_root = temp.path().join(".jit");
        fs::create_dir_all(&jit_root)?;

        // Create .git directory
        let git_dir = temp.path().join(".git");
        fs::create_dir_all(&git_dir)?;

        // Change to temp directory so WorktreePaths::detect() works
        env::set_current_dir(temp.path())?;

        // Initialize storage
        let storage = JsonFileStorage::new(&jit_root);
        storage.init()?;

        Ok((temp, storage, original_dir))
    }

    /// Restore original directory after test
    fn teardown(original_dir: std::path::PathBuf) -> Result<()> {
        env::set_current_dir(original_dir)?;
        Ok(())
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
        let (_temp, storage, original_dir) = setup_test_repo()?;

        let result = execute_claim_acquire(
            &storage,
            "nonexistent-issue-id",
            600,
            Some("agent:test"),
            None,
        );

        assert!(result.is_err(), "Should fail for non-existent issue");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("nonexistent"),
            "Error should mention issue not found, got: {}",
            err_msg
        );

        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_returns_valid_lease_id() -> Result<()> {
        let (_temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        let result =
            execute_claim_acquire(&storage, &issue_id, 600, Some("agent:test-acquire"), None);

        assert!(
            result.is_ok(),
            "Claim acquisition failed: {:?}",
            result.err()
        );
        let lease_id = result.unwrap();

        // Lease ID should be a valid UUID
        assert!(!lease_id.is_empty(), "Lease ID should not be empty");
        assert!(
            lease_id.len() >= 32,
            "Lease ID should be UUID-like, got: {}",
            lease_id
        );

        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_uses_provided_agent_id() -> Result<()> {
        let (_temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        let custom_agent = "agent:custom-bot-123";
        let result = execute_claim_acquire(&storage, &issue_id, 600, Some(custom_agent), None);

        assert!(result.is_ok(), "Claim acquisition should succeed");
        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_uses_default_agent_when_not_provided() -> Result<()> {
        let (_temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        let result = execute_claim_acquire(&storage, &issue_id, 600, None, None);

        assert!(result.is_ok(), "Should succeed with default agent");
        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_accepts_different_ttl_values() -> Result<()> {
        let (_temp, storage, original_dir) = setup_test_repo()?;

        // Test various TTL values
        let ttls = vec![60, 600, 3600, 0];

        for (i, ttl) in ttls.iter().enumerate() {
            let issue_id = create_test_issue(&storage, &format!("Issue {}", i))?;

            let result = execute_claim_acquire(
                &storage,
                &issue_id,
                *ttl,
                Some(&format!("agent:ttl-test-{}", i)),
                None,
            );

            assert!(
                result.is_ok(),
                "Should succeed with TTL={}, got error: {:?}",
                ttl,
                result.err()
            );
        }

        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_fails_when_already_claimed() -> Result<()> {
        let (_temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        // First claim should succeed
        let first_claim =
            execute_claim_acquire(&storage, &issue_id, 600, Some("agent:first"), None);
        assert!(first_claim.is_ok(), "First claim should succeed");

        // Second claim should fail
        let second_claim =
            execute_claim_acquire(&storage, &issue_id, 600, Some("agent:second"), None);

        assert!(second_claim.is_err(), "Second claim should fail");
        let err_msg = second_claim.unwrap_err().to_string();
        assert!(
            err_msg.contains("already claimed") || err_msg.contains("agent:first"),
            "Error should mention already claimed, got: {}",
            err_msg
        );

        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_control_plane_structure() -> Result<()> {
        let (temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        let result =
            execute_claim_acquire(&storage, &issue_id, 600, Some("agent:structure-test"), None);

        assert!(result.is_ok(), "Claim should succeed");

        // Verify control plane structure was created
        let control_plane = temp.path().join(".git/jit");
        assert!(control_plane.exists(), ".git/jit should exist");

        let locks_dir = control_plane.join("locks");
        assert!(locks_dir.exists(), ".git/jit/locks should exist");

        let heartbeat_dir = control_plane.join("heartbeat");
        assert!(heartbeat_dir.exists(), ".git/jit/heartbeat should exist");

        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_claims_log() -> Result<()> {
        let (temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire(&storage, &issue_id, 600, Some("agent:log-test"), None)?;

        // Verify claims.jsonl was created
        let claims_log = temp.path().join(".git/jit/claims.jsonl");
        assert!(claims_log.exists(), "claims.jsonl should be created");

        // Verify it has content
        let content = fs::read_to_string(&claims_log)?;
        assert!(!content.is_empty(), "claims.jsonl should not be empty");
        assert!(content.contains(&issue_id), "Log should contain issue ID");

        teardown(original_dir)?;
        Ok(())
    }

    #[test]
    fn test_claim_acquire_creates_worktree_identity() -> Result<()> {
        let (temp, storage, original_dir) = setup_test_repo()?;
        let issue_id = create_test_issue(&storage, "Test Issue")?;

        execute_claim_acquire(&storage, &issue_id, 600, Some("agent:identity-test"), None)?;

        // Verify worktree.json was created in .jit
        let worktree_json = temp.path().join(".jit/worktree.json");
        assert!(worktree_json.exists(), "worktree.json should be created");

        // Verify it has valid content
        let content = fs::read_to_string(&worktree_json)?;
        assert!(content.contains("worktree_id"), "Should have worktree_id");
        assert!(content.contains("wt:"), "Worktree ID should start with wt:");

        teardown(original_dir)?;
        Ok(())
    }
}
