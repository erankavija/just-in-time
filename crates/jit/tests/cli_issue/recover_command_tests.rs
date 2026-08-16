//! Tests for the jit recover command

use assert_cmd::Command;
use jit::commands::production_mutation_context;
use jit::config::ProjectName;
use jit::repository_state::{
    derive_materialization, CaptureBudget, CaptureSpec, InitializationScaffold,
    MaterializationRequest,
};
use jit::storage::{
    discover_repository_layout, JsonFileStorage, RepositoryStateStore, TransactionFailureInjector,
    TransactionFailurePoint,
};
use predicates::prelude::*;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;
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

fn leave_fresh_prepared_journal(temp: &TempDir) {
    let data = temp.path().join(".jit");
    let failures = Arc::new(SelectedFailures(HashSet::from([
        TransactionFailurePoint::RepositoryBeforeDataRootPublication,
    ])));
    let storage = JsonFileStorage::with_repository_state_failures(&data, failures);
    let layout = discover_repository_layout(temp.path(), &data).unwrap();
    let scaffold = InitializationScaffold::render(
        "",
        "recovery-fixture".parse::<ProjectName>().unwrap(),
        None,
    )
    .unwrap();
    let spec = CaptureSpec::phase_one(
        scaffold.delta_paths().unwrap(),
        CaptureBudget {
            max_listings: 0,
            max_bytes: 16 * 1024 * 1024,
            max_depth: 16,
        },
    )
    .unwrap();
    let mut session = storage.open_mutation_session(layout).unwrap();
    let image = session.capture(spec).unwrap();
    let context = production_mutation_context();
    let plan = derive_materialization(
        &image,
        MaterializationRequest::Initialize {
            scaffold: &scaffold,
            context: &context,
        },
    )
    .unwrap();
    session.apply(&plan).unwrap_err();
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
fn test_recover_json_failure_writes_registered_pretty_envelope_to_stdout() {
    let temp = TempDir::new().unwrap();

    jit_cmd()
        .current_dir(temp.path())
        .args(["init"])
        .assert()
        .success();
    std::process::Command::new("git")
        .current_dir(temp.path())
        .args(["init"])
        .output()
        .expect("Failed to initialize headless git repository");

    let assert = jit_cmd()
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .assert()
        .code(jit::output::ErrorCode::RecoveryFailed.exit_code().code());
    let output = assert.get_output();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    let envelope: Value = serde_json::from_str(stdout).expect("stdout must be one JSON document");

    assert!(
        stdout.contains("\n  \"error\":"),
        "stdout must be pretty JSON: {stdout}"
    );
    assert!(
        stdout.ends_with('\n'),
        "stdout must end with one newline: {stdout}"
    );
    assert_eq!(envelope["error"]["code"], "recovery_failed");
    assert!(
        !stderr.contains('{'),
        "stderr must not contain JSON: {stderr}"
    );
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
    leave_fresh_prepared_journal(&temp);

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
    leave_fresh_prepared_journal(&temp);

    jit_cmd()
        .current_dir(temp.path())
        .args(["init"])
        .assert()
        .success();

    assert!(temp.path().join(".jit/index.json").exists());
    assert!(!temp.path().join(".jit-bootstrap").exists());
}
