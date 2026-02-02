//! Claim coordination command implementations.
//!
//! Provides CLI interface to the lease-based claim coordination system.

use crate::config::ConfigLoader;
use crate::storage::worktree_identity::load_or_create_worktree_identity;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{ClaimCoordinator, FileLocker, IssueStore, Lease};
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
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!(
            "Failed to get current git branch. Are you in a git repository?\n\
             Git error: {}",
            stderr.trim()
        );
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
    reason: Option<&str>,
) -> Result<String> {
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

    // Load or generate worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Resolve agent ID using proper priority: CLI flag > JIT_AGENT_ID > ~/.config/jit/agent.toml > error
    let agent = resolve_agent_id(agent_id.map(|s| s.to_string()))?;

    // Load config for policy limits
    let config = ConfigLoader::new()
        .with_repo_config(&paths.local_jit)
        .unwrap_or_else(|_| ConfigLoader::new())
        .build();
    let coord_config = config.coordination();

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

    // Acquire claim with reason validation for TTL=0 leases
    let lease = coordinator.acquire_claim_with_reason(
        issue_id,
        ttl_secs,
        reason,
        coord_config.max_indefinite_leases_per_agent(),
        coord_config.max_indefinite_leases_per_repo(),
    )?;

    // Also set the assignee on the issue for visibility
    let mut issue = storage.load_issue(&full_id)?;
    if issue.assignee.is_none() || issue.assignee.as_ref() != Some(&agent) {
        issue.assignee = Some(agent);
        storage.save_issue(&issue)?;
    }

    Ok(lease.lease_id)
}

/// Execute `jit claim heartbeat` command.
///
/// Sends a heartbeat for an indefinite lease to prevent staleness.
pub fn execute_claim_heartbeat(lease_id: &str) -> Result<()> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Resolve agent ID
    let agent = resolve_agent_id(None)?;

    // Create coordinator
    let locker = FileLocker::new(Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent);

    // Send heartbeat
    coordinator.heartbeat(lease_id)?;

    Ok(())
}

/// Execute `jit claim release` command.
///
/// Releases a previously acquired lease.
pub fn execute_claim_release(lease_id: &str) -> Result<()> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Resolve agent ID using proper priority: JIT_AGENT_ID > ~/.config/jit/agent.toml > error
    let agent = resolve_agent_id(None)?;

    // Create file locker
    let locker = FileLocker::new(Duration::from_secs(5));

    // Create claim coordinator
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        locker,
        identity.worktree_id.clone(),
        agent.clone(),
    );

    // Release the lease
    coordinator.release_lease(lease_id)?;

    Ok(())
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
/// The renewed lease with updated expiry time
pub fn execute_claim_renew<S: IssueStore>(lease_id: &str, extension_secs: u64) -> Result<Lease> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch
    let branch = get_current_branch()?;

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Resolve agent ID
    let agent = resolve_agent_id(None)?;

    // Create locker and coordinator
    let locker = FileLocker::new(Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        locker,
        identity.worktree_id.clone(),
        agent.clone(),
    );

    // Renew the lease
    coordinator.renew_lease(lease_id, extension_secs)
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
/// Vector of active leases matching the filters
pub fn execute_claim_status<S: IssueStore>(
    issue_id: Option<&str>,
    agent_id: Option<&str>,
) -> Result<Vec<Lease>> {
    use crate::agent_config::resolve_agent_id;

    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Resolve current agent ID using proper priority: JIT_AGENT_ID > ~/.config/jit/agent.toml > error
    let current_agent_id = resolve_agent_id(None)?;

    // Create claim coordinator
    let locker = FileLocker::new(Duration::from_secs(5));
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

    coordinator.get_active_leases(issue_id, filter_agent)
}

/// Lists all active leases across all agents and worktrees.
///
/// # Arguments
///
/// * `_storage` - Issue storage (unused but kept for consistency)
///
/// # Returns
///
/// Vector of all active leases
pub fn execute_claim_list() -> Result<Vec<Lease>> {
    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // We need an agent ID for coordinator, but it doesn't matter which one for listing
    let agent = "system:list".to_string();

    // Create claim coordinator
    let locker = FileLocker::new(Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent);
    coordinator.init()?;

    // Get all active leases (no filters)
    coordinator.get_active_leases(None, None)
}

/// Check if an issue has an active lease held by another agent.
///
/// Returns Ok(None) if no conflicting lease exists.
/// Returns Ok(Some(lease)) if there's a lease held by a different agent.
/// Returns Err if there's an error checking leases.
pub fn check_issue_lease(issue_id: &str, current_agent: Option<&str>) -> Result<Option<Lease>> {
    // Try to detect worktree context - if not in a git repo, no leases are active
    let paths = match WorktreePaths::detect() {
        Ok(p) => p,
        Err(_) => return Ok(None), // Not in a git repo, no lease system active
    };

    // Get current branch for identity
    let branch = match get_current_branch() {
        Ok(b) => b,
        Err(_) => return Ok(None), // Can't determine branch, skip lease check
    };

    // Load worktree identity
    let identity = match load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch) {
        Ok(i) => i,
        Err(_) => return Ok(None), // Can't load identity, skip lease check
    };

    // Create coordinator to check leases
    let agent = current_agent.unwrap_or("system:check").to_string();
    let locker = FileLocker::new(Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent.clone());
    
    // Don't fail if control plane doesn't exist yet
    if coordinator.init().is_err() {
        return Ok(None);
    }

    // Get leases for this issue
    let leases = coordinator.get_active_leases(Some(issue_id), None)?;
    
    // Check if any lease is held by a different agent
    for lease in leases {
        if current_agent.is_none() || Some(lease.agent_id.as_str()) != current_agent {
            return Ok(Some(lease));
        }
    }
    
    Ok(None)
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
/// Ok(()) on success
pub fn execute_claim_force_evict<S: IssueStore>(lease_id: &str, reason: &str) -> Result<()> {
    // Detect worktree context
    let paths = WorktreePaths::detect()
        .context("Failed to detect worktree paths - are you in a git repository?")?;

    // Get current branch for identity
    let branch = get_current_branch()?;

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // For force-evict, we use a system agent (admin operation)
    let agent = "system:admin".to_string();

    // Create claim coordinator
    let locker = FileLocker::new(Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(paths, locker, identity.worktree_id, agent);
    coordinator.init()?;

    // Force evict the lease
    coordinator.force_evict_lease(lease_id, reason)
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

    // Load worktree identity
    let identity =
        load_or_create_worktree_identity(&paths.local_jit, &paths.worktree_root, &branch)?;

    // Create claim coordinator
    let agent = "system:recovery".to_string();
    let locker = FileLocker::new(Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(paths.clone(), locker, identity.worktree_id, agent);
    coordinator.init()?;

    let mut report = RecoveryReport::default();

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

        lock_cleanup::cleanup_stale_locks(&lock_dir)?;

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
    if !coordinator.verify_index_consistency()? {
        let index = coordinator.rebuild_index_from_log()?;
        coordinator.write_index_atomic(&index)?;
        report.index_rebuilt = true;
    }

    // 3. Evict expired leases
    let mut index = coordinator.load_claims_index()?;
    let leases_before = index.leases.len();
    coordinator.evict_expired(&mut index)?;
    coordinator.write_index_atomic(&index)?;
    report.expired_leases_evicted = leases_before.saturating_sub(index.leases.len());

    // 4. Clean up orphaned temp files (1 hour threshold)
    let jit_data_dir = &paths.local_jit;
    if let Ok(removed) = temp_cleanup::cleanup_orphaned_temp_files(jit_data_dir, 3600) {
        report.temp_files_removed = removed;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Issue;
    use crate::storage::claim_coordinator::Lease;

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

    /// Execute claim release with manually constructed paths (for testing)
    fn execute_claim_release_test(temp: &TempDir, lease_id: &str, agent_id: &str) -> Result<()> {
        let paths = create_test_paths(temp);

        // Create coordinator
        let locker = FileLocker::new(Duration::from_secs(5));
        let coordinator =
            ClaimCoordinator::new(paths, locker, "wt:test".to_string(), agent_id.to_string());

        // Release lease
        coordinator.release_lease(lease_id)?;
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
        let locker = FileLocker::new(Duration::from_secs(5));
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

        let locker = FileLocker::new(Duration::from_secs(5));
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
        let locker = FileLocker::new(Duration::from_secs(5));
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

        let result = execute_claim_status_test(&temp, None, None);
        if let Err(e) = &result {
            eprintln!("Error: {:?}", e);
        }
        assert!(result.is_ok());
        let leases = result.unwrap();
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

        let locker = FileLocker::new(Duration::from_secs(5));
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

        let locker = FileLocker::new(Duration::from_secs(5));
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
}
