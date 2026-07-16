//! Tests for the jit recover command

use assert_cmd::Command;
use cap_std::{ambient_authority, fs::Dir};
use jit::storage::{
    FileTransactionKernel, FileTransactionPlan, RecoveryState, RepoWriteLock, TransactionAction,
    TransactionFailureInjector, TransactionFailurePoint,
};
use predicates::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;

fn jit_cmd() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize git repo first (required for recover)
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["init"])
        .output()
        .expect("Failed to init git repo");

    // Configure git user for commits
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.email", "test@test.com"])
        .output()
        .expect("Failed to configure git email");
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.name", "Test"])
        .output()
        .expect("Failed to configure git name");

    // Create initial commit so we have a branch
    std::fs::write(temp.path().join("README.md"), "# Test").unwrap();
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["add", "."])
        .output()
        .expect("Failed to git add");
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["commit", "-m", "Initial commit"])
        .output()
        .expect("Failed to git commit");

    // Initialize jit
    jit_cmd()
        .current_dir(temp.path())
        .args(["init"])
        .assert()
        .success();

    temp
}

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

fn leave_fresh_prepared_journal(temp: &TempDir, transaction_id: &str) {
    let root = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
    let failures = Arc::new(SelectedFailures(HashSet::from([
        TransactionFailurePoint::SyncJournal {
            decision: RecoveryState::Prepared,
        },
    ])));
    let kernel = FileTransactionKernel::with_injector(root, failures).unwrap();
    let lock = RepoWriteLock::for_lock_path(
        temp.path().join(".jit-bootstrap.lock"),
        Duration::from_secs(1),
    );
    let guard = lock.acquire().unwrap();
    kernel
        .execute(
            &guard,
            FileTransactionPlan {
                transaction_id: transaction_id.to_string(),
                actions: vec![TransactionAction::WriteFile {
                    path: ".jit/index.json".to_string(),
                    contents: b"unpublished".to_vec(),
                    unix_mode: None,
                }],
            },
        )
        .unwrap_err();
}

#[test]
fn test_recover_command_exists() {
    let temp = setup_test_repo();

    jit_cmd()
        .current_dir(temp.path())
        .args(["recover"])
        .assert()
        .success();
}

#[test]
fn test_recover_with_json_output() {
    let temp = setup_test_repo();

    jit_cmd()
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\""));
}

#[test]
fn test_recover_reports_actions_taken() {
    let temp = setup_test_repo();

    // Run recover - should succeed even with nothing to recover
    jit_cmd()
        .current_dir(temp.path())
        .args(["recover"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Recovery complete"));
}

#[test]
fn test_recover_help_describes_purpose() {
    jit_cmd()
        .args(["recover", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("recovery"))
        .stdout(predicate::str::contains("stale"));
}

#[test]
fn test_explicit_recover_restores_fresh_prepared_state_before_validation() {
    let temp = TempDir::new().unwrap();
    leave_fresh_prepared_journal(&temp, "explicit-fresh");

    jit_cmd()
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"transactions_recovered\": 1"))
        .stdout(predicate::str::contains("repository absence restored"));

    assert!(!temp.path().join(".jit").exists());
    assert!(!temp.path().join(".jit-bootstrap").exists());
}

#[test]
fn test_init_recovers_fresh_prepared_state_before_scaffolding() {
    let temp = TempDir::new().unwrap();
    leave_fresh_prepared_journal(&temp, "init-fresh");

    jit_cmd()
        .current_dir(temp.path())
        .args(["init"])
        .assert()
        .success();

    assert!(temp.path().join(".jit/index.json").exists());
    assert!(!temp.path().join(".jit-bootstrap").exists());
}
