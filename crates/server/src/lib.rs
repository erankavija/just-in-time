//! JIT REST API Server Library
//!
//! Provides a web API for the Just-In-Time issue tracker, enabling web UI
//! and external integrations to query and visualize issues.

#![deny(unsafe_code)]

#[cfg(feature = "embed-web")]
pub mod embedded;
pub mod routes;
pub mod sse;
pub mod watcher;

// Re-export for convenience
pub use routes::create_routes;

use anyhow::{Context, Result};
use jit::storage::{JsonFileStorage, RecoveryCoordinator, RecoverySession};
use std::path::Path;

/// Recover and validate a mutation-capable server repository before constructing
/// any command executor or configuration cache.
pub fn prepare_server_storage(
    data_dir: impl AsRef<Path>,
) -> Result<(JsonFileStorage, RecoverySession)> {
    let storage = JsonFileStorage::new(data_dir);
    let recovery_session = RecoveryCoordinator::recover_before_services(&storage)
        .context("Failed to recover pending repository transaction state")?;
    storage.validate().map_err(|error| {
        anyhow::anyhow!(
            "Failed to initialize storage: {}\n\n\
             The server requires a JIT repository to be initialized.\n\
             Run 'jit init' in the repository directory, or use --data-dir to point to an existing repository.",
            error
        )
    })?;
    Ok((storage, recovery_session))
}

#[cfg(test)]
mod recovery_startup_tests {
    use super::*;
    use jit::storage::IssueStore;
    use tempfile::TempDir;

    #[test]
    fn test_server_startup_recovers_committed_fresh_root_before_validation() {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        let transaction = temp
            .path()
            .join(".jit-bootstrap/transactions/server-committed");
        std::fs::create_dir_all(transaction.join("stages")).unwrap();
        std::fs::create_dir_all(transaction.join("backups")).unwrap();
        std::fs::write(
            temp.path().join(".jit-bootstrap/transaction-protocol-v1"),
            "1\n",
        )
        .unwrap();
        std::fs::write(
            transaction.join("journal.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "version": 1,
                "transaction_id": "server-committed",
                "plan_hash": "test",
                "fresh_root": true,
                "decision": "committed",
                "actions": []
            }))
            .unwrap(),
        )
        .unwrap();

        let (_storage, session) =
            prepare_server_storage(temp.path().join(".jit")).expect("startup recovery");
        assert_eq!(session.report().recovered_count(), 1);
        assert!(!temp.path().join(".jit-bootstrap").exists());
    }
}
