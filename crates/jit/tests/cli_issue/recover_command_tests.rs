//! Tests for the jit recover command

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
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

/// REQ-03 at the recover command. The field keeps its transaction spelling;
/// jit:51eb1aa6 renames it.
#[test]
fn test_recover_reports_the_leftover_redo_record_its_session_applied() {
    use jit::storage::{PutFile, RedoRecord};

    let temp = setup_test_repo();
    let published = temp.path().join("applied-by-recovery.md");
    RedoRecord::new(
        Vec::new(),
        vec![PutFile {
            path: jit::repository_state::VirtualPath::worktree("applied-by-recovery.md").unwrap(),
            bytes: b"published by the leftover record\n".to_vec(),
            mode: jit::repository_state::FileMode::Regular,
        }],
        Vec::new(),
        None,
    )
    .unwrap()
    .commit(&temp.path().join(".jit"))
    .unwrap();

    let output = jit_cmd()
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        value["transactions_recovered"], 1,
        "the applied record must reach the recover command's count, got {value}"
    );
    assert!(
        published.exists(),
        "the startup session must have applied the record it counted"
    );
}

/// A leftover record the session cannot read fails `jit recover` as a recovery
/// failure; applying one is the command's own work.
#[test]
fn test_recover_json_classifies_an_unreadable_leftover_record_as_recovery_failed() {
    let temp = setup_test_repo();
    let scratch = temp
        .path()
        .join(".jit")
        .join(jit::storage::RECORD_SCRATCH_DIR);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join(jit::storage::RECORD_FILE_NAME), b"no header").unwrap();

    let output = jit_cmd()
        .current_dir(temp.path())
        .args(["recover", "--json"])
        .assert()
        .code(jit::output::ErrorCode::RecoveryFailed.exit_code().code())
        .get_output()
        .stdout
        .clone();

    let envelope: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(envelope["error"]["code"], "recovery_failed", "{envelope}");
}
