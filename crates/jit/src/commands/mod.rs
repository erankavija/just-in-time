//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `issue`: Issue CRUD operations and lifecycle management
//! - `dependency`: Dependency graph operations  
//! - `breakdown`: Issue breakdown operations
//! - `gate`: Quality gate operations
//! - `graph`: Graph visualization and traversal
//! - `query`: Issue query operations
//! - `validate`: Validation and status operations
//! - `labels`: Label operations
//! - `document`: Document reference operations
//! - `events`: Event log operations
//! - `search`: Issue search operations

mod breakdown;
pub mod bulk_update;
pub mod claim;
mod dependency;
mod document;
mod events;
mod gate;
mod gate_check;
mod gate_cli_tests;
pub mod graph;
pub mod hooks;
mod issue;
mod labels;
mod query;
mod search;
pub mod snapshot;
mod validate;
pub mod worktree;

#[cfg(test)]
pub mod test_helpers;

pub use bulk_update::{BulkUpdatePreview, BulkUpdateResult, UpdateOperations};

// Common imports used across modules
use crate::config_manager::ConfigManager;
use crate::domain::{Event, Gate, GateState, GateStatus, Issue, Priority, State};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use serde::Serialize;
use std::collections::HashMap;

/// Information about a git commit
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize, // Backlog count (kept as 'open' for compatibility)
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize, // New: count of rejected issues
    pub blocked: usize,
    pub total: usize,
}

/// Result of adding a dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAddResult {
    /// Dependency was added
    Added,
    /// Dependency was skipped because it's transitive (redundant)
    Skipped { reason: String },
    /// Dependency already existed
    AlreadyExists,
}

/// Executes CLI commands with business logic and validation.
///
/// Generic over storage backend to support different implementations
/// (JSON files, SQLite, in-memory, etc.).
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
    pub config_manager: ConfigManager,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        let config_manager = ConfigManager::new(storage.root());
        Self {
            storage,
            config_manager,
        }
    }

    /// Get reference to the storage backend
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Initialize a new jit repository in the current directory
    pub fn init(&self) -> Result<()> {
        self.storage.init()?;
        println!("Initialized jit repository");
        Ok(())
    }

    /// Check if an active lease exists for the given issue by the current agent.
    ///
    /// Returns true if the current agent has an active lease (not expired or stale).
    /// Returns false if no lease exists, lease is stale, or belongs to another agent.
    ///
    /// In tests: If JIT_AGENT_ID is not set, any valid lease counts (single-user mode).
    fn check_active_lease(&self, issue_id: &str) -> Result<bool> {
        use crate::agent_config::resolve_agent_id;
        use crate::storage::claim_coordinator::ClaimsIndex;
        use crate::storage::worktree_paths::WorktreePaths;

        // Get worktree paths to access shared control plane
        let paths = match WorktreePaths::detect() {
            Ok(p) => p,
            Err(_) => {
                // Not in a git repository - no claims possible
                return Ok(false);
            }
        };

        // Load claims index
        let claims_index_path = paths.shared_jit.join("claims.index.json");
        if !claims_index_path.exists() {
            // No claims index - no active leases
            return Ok(false);
        }

        let contents =
            std::fs::read_to_string(&claims_index_path).context("Failed to read claims index")?;
        let claims_index: ClaimsIndex =
            serde_json::from_str(&contents).context("Failed to parse claims index")?;

        // Resolve current agent identity (or None for single-user mode)
        let current_agent = resolve_agent_id(None).ok();

        // Check if active lease exists for this issue
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let now = chrono::Utc::now();

        let has_active_lease = claims_index.leases.iter().any(|lease| {
            // Must match issue ID
            if lease.issue_id != full_id {
                return false;
            }

            // Must not be expired
            if let Some(expires) = lease.expires_at {
                if expires <= now {
                    return false;
                }
            }

            // Must not be stale
            if claims_index.is_stale(lease) {
                return false;
            }

            // Agent verification:
            // - If current_agent is Some, lease must belong to this agent
            // - If current_agent is None (single-user mode), any valid lease counts
            match &current_agent {
                Some(agent_id) => lease.agent_id == *agent_id,
                None => true, // Single-user mode: any valid lease
            }
        });

        Ok(has_active_lease)
    }

    /// Require an active lease for the given issue, respecting enforcement mode.
    ///
    /// Checks the configured enforcement mode and either blocks, warns, or bypasses
    /// the lease requirement. Used before structural operations that modify issues.
    ///
    /// # Errors
    ///
    /// Returns an error in `strict` mode when no active lease exists.
    /// In `warn` mode, prints a warning but returns Ok.
    /// In `off` mode, always returns Ok.
    pub fn require_active_lease(&self, issue_id: &str) -> Result<()> {
        use crate::config::EnforcementMode;

        let mode = self.config_manager.get_enforcement_mode()?;

        match mode {
            EnforcementMode::Off => Ok(()),
            EnforcementMode::Warn | EnforcementMode::Strict => {
                let has_lease = self.check_active_lease(issue_id)?;

                if !has_lease {
                    let msg = format!(
                        "No active lease for issue {}.\nAcquire lease with: jit claim acquire {}",
                        issue_id, issue_id
                    );

                    match mode {
                        EnforcementMode::Warn => {
                            eprintln!("⚠️  Warning: {}", msg);
                            Ok(())
                        }
                        EnforcementMode::Strict => {
                            anyhow::bail!("{}", msg)
                        }
                        _ => unreachable!(),
                    }
                } else {
                    Ok(())
                }
            }
        }
    }
}

// Helper functions for parsing command-line arguments
#[cfg(test)]
mod tests {
    use super::*;

    // Enforcement tests
    #[test]
    fn test_require_active_lease_off_mode() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();

        // Create the root directory and config with enforcement off
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should always succeed in off mode, even without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_check_active_lease_no_claims_index() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();

        let executor = CommandExecutor::new(storage);

        // No claims index - should return false
        let result = executor.check_active_lease(&issue_id);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_require_active_lease_strict_mode_no_lease() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();

        // Create the root directory and config with enforcement strict
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should fail in strict mode without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No active lease"));
        assert!(err_msg.contains("jit claim acquire"));
    }

    #[test]
    fn test_require_active_lease_off_mode_default() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        storage.init().unwrap();

        // Create a test issue
        let issue = Issue::new("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        storage.save_issue(&issue).unwrap();

        // No config file - should default to off mode (single-agent development)
        let executor = CommandExecutor::new(storage);

        // Should succeed in off mode (default) without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
    }

    // Agent identity verification tests
    #[test]
    fn test_check_active_lease_verifies_agent_identity() {
        // This test documents the agent identity verification behavior.
        // Since check_active_lease() now uses resolve_agent_id(),
        // it verifies agent ownership in multi-agent scenarios:
        //
        // 1. If JIT_AGENT_ID is set (or --agent-id / ~/.config/jit/agent.toml),
        //    only leases belonging to that agent count as active.
        // 2. If not set (single-user mode), any valid lease counts.
        //
        // This prevents Agent A from modifying issues claimed by Agent B.
        //
        // Full workflow testing requires integration tests with git repos
        // and actual claims.index.json files.
    }
}
