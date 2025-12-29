//! Tests for --quiet flag functionality and broken pipe handling.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

/// Helper to create a test environment with initialized jit repository
fn setup_test_env() -> (TempDir, Command) {
    let temp_dir = TempDir::new().unwrap();
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp_dir.path());

    // Initialize jit repo
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .arg("init")
        .assert()
        .success();

    (temp_dir, cmd)
}

/// Helper to create an issue and return its ID
fn create_test_issue(temp_dir: &TempDir, title: &str) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["issue", "create", "--title", title, "--orphan", "--quiet"])
        .output()
        .unwrap();

    assert!(output.status.success(), "Failed to create test issue");

    let stdout = String::from_utf8(output.stdout).unwrap();
    // In quiet mode, output is just the ID
    stdout.trim().to_string()
}

#[test]
fn test_quiet_flag_exists() {
    let (_temp_dir, mut cmd) = setup_test_env();

    // --quiet should be accepted as a valid flag
    cmd.arg("status").arg("--quiet").assert().success();
}

#[test]
fn test_quiet_short_flag_exists() {
    let (_temp_dir, mut cmd) = setup_test_env();

    // -q should work as short form
    cmd.arg("status").arg("-q").assert().success();
}

#[test]
fn test_quiet_mode_suppresses_success_messages() {
    let (temp_dir, _) = setup_test_env();
    let issue_id = create_test_issue(&temp_dir, "Test Issue");

    // Normal mode should show success message
    let normal_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["issue", "update", &issue_id, "--state", "done"])
        .output()
        .unwrap();

    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();
    assert!(
        normal_stdout.contains("Updated") || normal_stdout.contains("success"),
        "Normal mode should show success message"
    );

    // Quiet mode should suppress success message
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["issue", "update", &issue_id, "--state", "open", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated").not())
        .stdout(predicate::str::contains("success").not());
}

#[test]
fn test_quiet_mode_preserves_essential_output() {
    let (temp_dir, _) = setup_test_env();
    create_test_issue(&temp_dir, "Test Issue 1");
    create_test_issue(&temp_dir, "Test Issue 2");

    // Quiet mode should still output the issue list
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["issue", "list", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Test Issue 1"))
        .stdout(predicate::str::contains("Test Issue 2"));
}

#[test]
fn test_quiet_with_json_outputs_only_json() {
    let (temp_dir, _) = setup_test_env();
    let issue_id = create_test_issue(&temp_dir, "Test Issue");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["issue", "show", &issue_id, "--quiet", "--json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();

    // Should be valid JSON with no extra text
    let json: serde_json::Value =
        serde_json::from_str(&stdout).expect("Output should be pure JSON in quiet+json mode");

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["title"], "Test Issue");
}

#[test]
fn test_quiet_suppresses_informational_output() {
    let (temp_dir, _) = setup_test_env();

    // Status command in normal mode might have headers/formatting
    let normal_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .arg("status")
        .output()
        .unwrap();

    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();

    // Quiet mode should have more concise output
    let quiet_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["status", "--quiet"])
        .output()
        .unwrap();

    let quiet_stdout = String::from_utf8(quiet_output.stdout).unwrap();

    // Both should succeed
    assert!(normal_output.status.success());
    assert!(quiet_output.status.success());

    // Quiet mode should not be empty (still has essential data)
    assert!(!quiet_stdout.trim().is_empty());

    // Normal output should exist too
    assert!(!normal_stdout.trim().is_empty());
}

#[test]
fn test_quiet_with_dep_add() {
    let (temp_dir, _) = setup_test_env();
    let issue1 = create_test_issue(&temp_dir, "Issue 1");
    let issue2 = create_test_issue(&temp_dir, "Issue 2");

    // Normal mode shows confirmation
    let normal_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["dep", "add", &issue1, &issue2])
        .output()
        .unwrap();

    let normal_stdout = String::from_utf8(normal_output.stdout).unwrap();
    assert!(
        normal_stdout.contains("Added") || normal_stdout.contains("dependency"),
        "Normal mode should show confirmation"
    );

    // Quiet mode suppresses confirmation
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["dep", "rm", &issue1, &issue2, "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed").not())
        .stdout(predicate::str::contains("dependency").not());
}

#[test]
fn test_errors_always_shown_even_in_quiet_mode() {
    let (temp_dir, _) = setup_test_env();

    // Try to show non-existent issue in quiet mode
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["issue", "show", "nonexistent", "--quiet"])
        .output()
        .unwrap();

    // Should fail
    assert!(!output.status.success());

    // Error should be visible on stderr even in quiet mode
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Error") || stderr.contains("not found"),
        "Errors must be shown even in quiet mode"
    );
}

#[test]
fn test_issue_create_quiet_outputs_id() {
    let (temp_dir, _) = setup_test_env();

    // In quiet mode, create should output just the ID for scripting
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args([
            "issue",
            "create",
            "--title",
            "New Issue",
            "--quiet",
            "--orphan",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).unwrap();
    let trimmed = stdout.trim();

    // Should have ID in output (at least 8 chars for short hash)
    assert!(trimmed.len() >= 8, "Quiet create should output issue ID");

    // Should not have verbose text
    assert!(
        !stdout.contains("Created issue") && !stdout.contains("Successfully"),
        "Quiet mode should not show verbose messages"
    );
}

#[test]
fn test_quiet_flag_position_independent() {
    let (temp_dir, _) = setup_test_env();

    // --quiet before subcommand
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["--quiet", "status"])
        .assert()
        .success();

    // --quiet after subcommand
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["status", "--quiet"])
        .assert()
        .success();
}

#[test]
fn test_quiet_with_gate_commands() {
    let (temp_dir, _) = setup_test_env();

    // Define a gate first
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args([
            "gate",
            "define",
            "test-gate",
            "--title",
            "Test Gate",
            "--description",
            "A test gate",
        ])
        .assert()
        .success();

    let issue_id = create_test_issue(&temp_dir, "Test Issue");

    // Add gate to issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["gate", "add", &issue_id, "test-gate", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Added").not());

    // Pass gate in quiet mode
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_dir.path())
        .args(["gate", "pass", &issue_id, "test-gate", "--quiet"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Passed").not());
}
