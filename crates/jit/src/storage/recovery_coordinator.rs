//! Cache-free transaction recovery before repository service construction.
//!
//! The coordinator reads only the versioned transaction protocol. It does not
//! load `index.json`, configuration, rules, gates, templates, or schemas, so it
//! remains reachable when those files are absent or temporarily inconsistent.

use super::{
    FileTransactionKernel, IssueStore, JsonFileStorage, RepoWriteGuard, TransactionControlLocation,
};
use anyhow::{Context, Result};
use cap_std::{ambient_authority, fs::Dir};

/// Transaction journals recovered before normal repository services start.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryDispatchReport {
    /// External fresh-root journals recovered in deterministic id order.
    pub external_transactions: Vec<String>,
    /// Internal `.jit/tmp` journals recovered in deterministic id order.
    pub internal_transactions: Vec<String>,
}

impl RecoveryDispatchReport {
    /// Total number of transaction journals recovered.
    pub fn recovered_count(&self) -> usize {
        self.external_transactions.len() + self.internal_transactions.len()
    }
}

/// Held startup locks and the recovery work completed under them.
///
/// CLI mutation dispatch retains this value through command execution. Nested
/// storage writes re-enter the same repository lock, preventing a transaction
/// from appearing between startup recovery and the command's first write.
pub struct RecoverySession {
    report: RecoveryDispatchReport,
    // Field order is intentional: repository drops before bootstrap.
    _repository_guard: Option<RepoWriteGuard>,
    _bootstrap_guard: RepoWriteGuard,
}

impl RecoverySession {
    /// Recovery work performed before service construction.
    pub fn report(&self) -> &RecoveryDispatchReport {
        &self.report
    }
}

/// Config-independent recovery entrypoint shared by CLI and HTTP startup.
pub struct RecoveryCoordinator;

impl RecoveryCoordinator {
    /// Recover external journals, then internal journals, before any repository
    /// validation or command-layer cache is constructed.
    ///
    /// Lock order is fixed at bootstrap → repository. The returned session keeps
    /// both locks alive for callers that must close the startup-to-write race.
    pub fn recover_before_services(storage: &JsonFileStorage) -> Result<RecoverySession> {
        let repo_root = storage.root().parent().ok_or_else(|| {
            anyhow::anyhow!(
                "JIT data directory has no repository parent: {}",
                storage.root().display()
            )
        })?;
        let root = Dir::open_ambient_dir(repo_root, ambient_authority())
            .with_context(|| format!("Failed to open repository root {}", repo_root.display()))?;
        let kernel = FileTransactionKernel::new(root)?;

        let bootstrap_guard = storage.acquire_bootstrap_write_lock()?;
        let external_transactions =
            kernel.pending_transaction_ids(TransactionControlLocation::ExternalBootstrap)?;
        for id in &external_transactions {
            kernel.recover(&bootstrap_guard, id)?;
        }
        kernel.cleanup_empty_external_control(&bootstrap_guard)?;

        let repository_guard = if storage.root().exists() {
            let guard = storage.acquire_repo_write_lock_raw()?;
            let internal_transactions =
                kernel.pending_transaction_ids(TransactionControlLocation::InternalRepository)?;
            for id in &internal_transactions {
                kernel.recover(&guard, id)?;
            }
            Some((guard, internal_transactions))
        } else {
            None
        };

        let (repository_guard, internal_transactions) = repository_guard
            .map(|(guard, ids)| (Some(guard), ids))
            .unwrap_or_else(|| (None, Vec::new()));

        Ok(RecoverySession {
            report: RecoveryDispatchReport {
                external_transactions,
                internal_transactions,
            },
            _repository_guard: repository_guard,
            _bootstrap_guard: bootstrap_guard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::{
        FileTransactionPlan, RepoWriteLock, TransactionAction, TransactionFailureInjector,
        TransactionFailurePoint,
    };
    use std::collections::HashSet;
    use std::sync::{mpsc, Arc};
    use std::thread;
    use std::time::Duration;
    use tempfile::TempDir;

    struct SelectedFailures(HashSet<TransactionFailurePoint>);

    impl TransactionFailureInjector for SelectedFailures {
        fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
            if self.0.contains(point) {
                Err(std::io::Error::other(format!("injected {point:?}")))
            } else {
                Ok(())
            }
        }
    }

    fn root(temp: &TempDir) -> Dir {
        Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap()
    }

    fn bootstrap_guard(temp: &TempDir) -> (Arc<RepoWriteLock>, RepoWriteGuard) {
        let lock = RepoWriteLock::for_lock_path(
            temp.path().join(".jit-bootstrap.lock"),
            Duration::from_secs(1),
        );
        let guard = lock.acquire().unwrap();
        (lock, guard)
    }

    #[test]
    fn test_pre_service_recovery_rolls_back_fresh_prepared_transaction_to_no_repository() {
        let temp = TempDir::new().unwrap();
        let failures = Arc::new(SelectedFailures(HashSet::from([
            TransactionFailurePoint::SyncJournal {
                decision: crate::storage::RecoveryState::Prepared,
            },
        ])));
        let kernel = FileTransactionKernel::with_injector(root(&temp), failures).unwrap();
        let (_lock, guard) = bootstrap_guard(&temp);
        kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "fresh-prepared".to_string(),
                    actions: vec![TransactionAction::WriteFile {
                        path: ".jit/index.json".to_string(),
                        contents: b"new".to_vec(),
                        unix_mode: None,
                    }],
                },
            )
            .unwrap_err();
        drop(guard);

        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(
            session.report().external_transactions,
            vec!["fresh-prepared"]
        );
        assert!(
            !temp.path().join(".jit").exists(),
            "prepared fresh-root recovery must restore repository absence"
        );
        assert!(!temp.path().join(".jit-bootstrap").exists());
    }

    #[test]
    fn test_pre_service_recovery_rolls_forward_committed_fresh_transaction() {
        let temp = TempDir::new().unwrap();
        let failures = Arc::new(SelectedFailures(HashSet::from([
            TransactionFailurePoint::CleanupTerminalResidue,
        ])));
        let kernel = FileTransactionKernel::with_injector(root(&temp), failures).unwrap();
        let (_lock, guard) = bootstrap_guard(&temp);
        kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "fresh-committed".to_string(),
                    actions: vec![TransactionAction::WriteFile {
                        path: ".jit/index.json".to_string(),
                        contents: b"final".to_vec(),
                        unix_mode: None,
                    }],
                },
            )
            .unwrap_err();
        drop(guard);

        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(
            session.report().external_transactions,
            vec!["fresh-committed"]
        );
        assert_eq!(
            std::fs::read(temp.path().join(".jit/index.json")).unwrap(),
            b"final"
        );
        assert!(!temp.path().join(".jit-bootstrap").exists());
    }

    #[test]
    fn test_pre_service_recovery_ignores_invalid_repository_caches() {
        let temp = TempDir::new().unwrap();
        let jit = temp.path().join(".jit");
        std::fs::create_dir_all(&jit).unwrap();
        std::fs::write(jit.join("index.json"), b"{not-json").unwrap();
        std::fs::write(jit.join("config.toml"), b"[broken").unwrap();
        std::fs::write(jit.join("rules.toml"), b"[[bad").unwrap();

        let storage = JsonFileStorage::new(&jit);
        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(session.report().recovered_count(), 0);
        assert!(jit.join("index.json").exists());
    }

    #[test]
    fn test_pre_service_recovery_cleans_internal_prepared_journal_before_cache_reads() {
        let temp = TempDir::new().unwrap();
        let jit = temp.path().join(".jit");
        std::fs::create_dir_all(&jit).unwrap();
        std::fs::write(jit.join("index.json"), b"{}").unwrap();
        std::fs::write(jit.join("config.toml"), b"original").unwrap();

        let failures = Arc::new(SelectedFailures(HashSet::from([
            TransactionFailurePoint::SyncJournal {
                decision: crate::storage::RecoveryState::Prepared,
            },
        ])));
        let kernel = FileTransactionKernel::with_injector(root(&temp), failures).unwrap();
        let storage = JsonFileStorage::new(&jit);
        let guard = storage.acquire_repo_write_lock_raw().unwrap();
        kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "internal-prepared".to_string(),
                    actions: vec![TransactionAction::WriteFile {
                        path: ".jit/config.toml".to_string(),
                        contents: b"replacement".to_vec(),
                        unix_mode: None,
                    }],
                },
            )
            .unwrap_err();
        drop(guard);

        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        assert_eq!(
            session.report().internal_transactions,
            vec!["internal-prepared"]
        );
        assert_eq!(std::fs::read(jit.join("config.toml")).unwrap(), b"original");
        assert!(!jit.join("tmp/transactions/internal-prepared").exists());
    }

    #[test]
    fn test_recovery_session_excludes_writer_until_dispatch_finishes() {
        let temp = TempDir::new().unwrap();
        let jit = temp.path().join(".jit");
        std::fs::create_dir_all(&jit).unwrap();
        std::fs::write(jit.join("index.json"), b"{}").unwrap();

        let storage = JsonFileStorage::new(&jit);
        let session = RecoveryCoordinator::recover_before_services(&storage).unwrap();
        let concurrent_storage = JsonFileStorage::new(&jit);
        let (started_tx, started_rx) = mpsc::channel();
        let (acquired_tx, acquired_rx) = mpsc::channel();
        let writer = thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _guard = concurrent_storage.acquire_repo_write_lock_raw().unwrap();
            acquired_tx.send(()).unwrap();
        });

        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            acquired_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "a writer must not enter between recovery and mutation dispatch"
        );

        drop(session);
        acquired_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        writer.join().unwrap();
    }
}
