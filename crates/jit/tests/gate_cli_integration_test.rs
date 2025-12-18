//! Integration tests for gate CLI commands
//!
//! These tests verify the full CLI → CommandExecutor → Storage flow

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path()).arg("init").assert().success();
    temp
}

#[test]
fn test_gate_define_manual_via_cli() {
    let temp = setup_repo();

    // Define a manual gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "code-review",
            "--title",
            "Code Review",
            "--description",
            "Human code review",
            "--stage",
            "postcheck",
            "--mode",
            "manual",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Defined gate 'code-review'"));

    // Verify gate was created by listing
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["gate", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("code-review"))
        .stdout(predicate::str::contains("Code Review"));
}

#[test]
fn test_gate_define_automated_via_cli() {
    let temp = setup_repo();

    // Define an automated gate with checker
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "unit-tests",
            "--title",
            "Unit Tests",
            "--description",
            "Run all unit tests",
            "--stage",
            "postcheck",
            "--mode",
            "auto",
            "--checker-command",
            "cargo test",
            "--timeout",
            "300",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Defined gate 'unit-tests'"));

    // Show gate details
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["gate", "show", "unit-tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unit-tests"))
        .stdout(predicate::str::contains("Unit Tests"))
        .stdout(predicate::str::contains("cargo test"));
}

#[test]
fn test_gate_define_auto_without_checker_fails() {
    let temp = setup_repo();

    // Try to define auto gate without checker - should fail
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "bad-gate",
            "--title",
            "Bad Gate",
            "--description",
            "Missing checker",
            "--mode",
            "auto",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("checker"));
}

#[test]
fn test_gate_list_json() {
    let temp = setup_repo();

    // Define a gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "test-gate",
            "--title",
            "Test",
            "--description",
            "Test gate",
        ])
        .assert()
        .success();

    // List gates in JSON format
    let mut cmd = Command::cargo_bin("jit").unwrap();
    let output = cmd
        .current_dir(temp.path())
        .args(&["gate", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["success"], true);
    assert!(json["data"].is_array());
}

#[test]
fn test_gate_show_json() {
    let temp = setup_repo();

    // Define a gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "test-gate",
            "--title",
            "Test Gate",
            "--description",
            "Test",
        ])
        .assert()
        .success();

    // Show gate in JSON format
    let mut cmd = Command::cargo_bin("jit").unwrap();
    let output = cmd
        .current_dir(temp.path())
        .args(&["gate", "show", "test-gate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["key"], "test-gate");
    assert_eq!(json["data"]["title"], "Test Gate");
}

#[test]
fn test_gate_remove() {
    let temp = setup_repo();

    // Define a gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "test-gate",
            "--title",
            "Test",
            "--description",
            "Test",
        ])
        .assert()
        .success();

    // Remove the gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["gate", "remove", "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed gate 'test-gate'"));

    // Verify it's gone
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["gate", "show", "test-gate"])
        .assert()
        .failure();
}

#[test]
fn test_gate_check_single() {
    let temp = setup_repo();

    // Define an automated gate that succeeds
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&[
            "gate",
            "define",
            "quick-check",
            "--title",
            "Quick Check",
            "--description",
            "Quick check",
            "--mode",
            "auto",
            "--checker-command",
            "exit 0",
            "--timeout",
            "10",
        ])
        .assert()
        .success();

    // Create an issue with the gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    let output = cmd
        .current_dir(temp.path())
        .args(&[
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "quick-check",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output_str = String::from_utf8_lossy(&output);
    let issue_id = output_str
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap();

    // Check the gate
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["gate", "check", issue_id, "quick-check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_gate_check_all() {
    let temp = setup_repo();

    // Define two automated gates
    for gate_name in &["gate-1", "gate-2"] {
        let mut cmd = Command::cargo_bin("jit").unwrap();
        cmd.current_dir(temp.path())
            .args(&[
                "gate",
                "define",
                gate_name,
                "--title",
                gate_name,
                "--description",
                "Test",
                "--mode",
                "auto",
                "--checker-command",
                "exit 0",
            ])
            .assert()
            .success();
    }

    // Create an issue with both gates
    let mut cmd = Command::cargo_bin("jit").unwrap();
    let output = cmd
        .current_dir(temp.path())
        .args(&[
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "gate-1,gate-2",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let output_str = String::from_utf8_lossy(&output);
    let issue_id = output_str
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap();

    // Check all gates
    let mut cmd = Command::cargo_bin("jit").unwrap();
    cmd.current_dir(temp.path())
        .args(&["gate", "check-all", issue_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("gate-1"))
        .stdout(predicate::str::contains("gate-2"));
}
