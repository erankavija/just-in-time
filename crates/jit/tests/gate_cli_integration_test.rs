//! Integration tests for gate CLI commands
//!
//! These tests verify the full CLI → CommandExecutor → Storage flow

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path()).arg("init").assert().success();
    temp
}

/// Define a simple auto gate and create an issue with it.
/// Returns (TempDir, issue_id_short).
fn setup_auto_gate_issue(checker_command: &str) -> (TempDir, String) {
    let temp = setup_repo();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "test-gate",
            "--title",
            "Test Gate",
            "--description",
            "Test",
            "--mode",
            "auto",
            "--checker-command",
            checker_command,
            "--timeout",
            "10",
        ])
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "test-gate",
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
        .unwrap()
        .to_string();
    (temp, issue_id)
}

#[test]
fn test_gate_check_no_prior_runs_shows_not_run_message() {
    let (temp, issue_id) = setup_auto_gate_issue("exit 0");

    // gate check before any pass — should say not run yet, no mutation
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not been run").or(predicate::str::contains("no run")));

    // Confirm non-mutating: gates_status should still be Pending
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate_status = json["gates_status"]["test-gate"]["status"]
        .as_str()
        .unwrap_or("Pending");
    assert!(
        gate_status == "Pending" || gate_status == "pending",
        "Expected Pending, got: {gate_status}"
    );
}

#[test]
fn test_gate_check_shows_last_run_after_pass() {
    let (temp, issue_id) = setup_auto_gate_issue("exit 0");

    // Execute the gate via pass
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "test-gate"])
        .assert()
        .success();

    // gate check now shows last run
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed").or(predicate::str::contains("Passed")))
        .stdout(predicate::str::contains("exit 0").or(predicate::str::contains("exit_code")));
}

#[test]
fn test_gate_check_shows_last_run_after_failure() {
    let (temp, issue_id) = setup_auto_gate_issue("exit 1");

    // Execute (will fail)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "test-gate"])
        .assert(); // don't assert success — gate fails but command itself is ok

    // gate check shows the failure
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("failed").or(predicate::str::contains("Failed")));

    // Non-mutating: gate check itself didn't change status further
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate_status = json["gates_status"]["test-gate"]["status"]
        .as_str()
        .unwrap_or("");
    assert!(
        gate_status.to_lowercase() == "failed",
        "Expected Failed, got: {gate_status}"
    );
}

#[test]
fn test_gate_check_is_non_mutating() {
    let (temp, issue_id) = setup_auto_gate_issue("exit 0");

    // Two gate check calls — neither should mutate
    for _ in 0..2 {
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(temp.path())
            .args(["gate", "check", &issue_id, "test-gate"])
            .assert()
            .success();
    }

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate_status = json["gates_status"]["test-gate"]["status"]
        .as_str()
        .unwrap_or("Pending");
    assert!(
        gate_status == "Pending" || gate_status == "pending",
        "Expected Pending after check-only, got: {gate_status}"
    );
}

#[test]
fn test_gate_check_json_output() {
    let (temp, issue_id) = setup_auto_gate_issue("exit 0");

    // Run the gate first so there's a result to show
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "test-gate"])
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "test-gate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(json["run_id"].is_string(), "Missing run_id");
    assert!(json["gate_key"].is_string(), "Missing gate_key");
    assert!(json["status"].is_string(), "Missing status");
    assert!(!json["exit_code"].is_null(), "Missing exit_code");
    assert!(json["stdout"].is_string(), "Missing stdout");
    assert!(json["stderr"].is_string(), "Missing stderr");
    assert!(json["started_at"].is_string(), "Missing started_at");
}

#[test]
fn test_gate_pass_auto_executes_checker() {
    let (temp, issue_id) = setup_auto_gate_issue("exit 0");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "test-gate"])
        .assert()
        .success();

    // gates_status should be Passed
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate_status = json["gates_status"]["test-gate"]["status"]
        .as_str()
        .unwrap_or("");
    assert!(
        gate_status.to_lowercase() == "passed",
        "Expected Passed, got: {gate_status}"
    );

    // A gate run result file should exist in .jit/gate-runs/
    let gate_runs_dir = temp.path().join(".jit").join("gate-runs");
    assert!(gate_runs_dir.exists(), ".jit/gate-runs/ should exist");
    let entries: Vec<_> = std::fs::read_dir(&gate_runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "Expected at least one gate run result saved"
    );
}

#[test]
fn test_gate_define_manual_via_cli() {
    let temp = setup_repo();

    // Define a manual gate
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("code-review"))
        .stdout(predicate::str::contains("Code Review"));
}

#[test]
fn test_gate_define_automated_via_cli() {
    let temp = setup_repo();

    // Define an automated gate with checker
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "show", "unit-tests"])
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args(["gate", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    // success field removed
    assert!(json["gates"].is_array());
    assert!(json["count"].is_number());
}

#[test]
fn test_gate_show_json() {
    let temp = setup_repo();

    // Define a gate
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args(["gate", "show", "test-gate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    // success field removed
    assert_eq!(json["key"], "test-gate");
    assert_eq!(json["title"], "Test Gate");
}

#[test]
fn test_gate_remove() {
    let temp = setup_repo();

    // Define a gate
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "remove", "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed gate 'test-gate'"));

    // Verify it's gone
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "show", "test-gate"])
        .assert()
        .failure();
}

#[test]
fn test_gate_check_single() {
    let temp = setup_repo();

    // Define an automated gate that succeeds
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args([
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

    // Run the gate first (gate pass executes the checker for auto gates)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "pass", issue_id, "quick-check"])
        .assert()
        .success();

    // Now inspect the last run result (gate check is inspection-only)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "check", issue_id, "quick-check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_gate_check_all() {
    let temp = setup_repo();

    // Define two automated gates
    for gate_name in &["gate-1", "gate-2"] {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
        cmd.current_dir(temp.path())
            .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args([
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
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "check-all", issue_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("gate-1"))
        .stdout(predicate::str::contains("gate-2"));
}

#[test]
fn test_gate_define_with_env_vars() {
    let temp = setup_repo();

    // Define a gate with --env flags
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
            "gate",
            "define",
            "review",
            "--title",
            "AI Review",
            "--description",
            "AI-powered code review",
            "--mode",
            "auto",
            "--checker-command",
            "echo ok",
            "--env",
            "REVIEWER_AGENT=copilot -s",
            "--env",
            "MODEL=claude-haiku-4.5",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Defined gate 'review'"));

    // Verify env vars persisted via --json
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args(["gate", "show", "review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let checker = &json["checker"];
    assert_eq!(checker["env"]["REVIEWER_AGENT"], "copilot -s");
    assert_eq!(checker["env"]["MODEL"], "claude-haiku-4.5");
}

#[test]
fn test_gate_define_env_invalid_format_fails() {
    let temp = setup_repo();

    // --env without = should fail
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
            "gate",
            "define",
            "bad-env",
            "--title",
            "Bad",
            "--description",
            "Bad env",
            "--mode",
            "auto",
            "--checker-command",
            "echo ok",
            "--env",
            "NO_EQUALS_SIGN",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("KEY=VALUE"));
}

#[test]
fn test_gate_env_vars_passed_to_checker() {
    let temp = setup_repo();

    // Define a gate with env vars; checker prints them
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args([
            "gate",
            "define",
            "env-test",
            "--title",
            "Env Test",
            "--description",
            "Test env passing",
            "--mode",
            "auto",
            "--checker-command",
            "echo FOO=$FOO BAZ=$BAZ",
            "--env",
            "FOO=bar",
            "--env",
            "BAZ=qux",
        ])
        .assert()
        .success();

    // Create issue with the gate
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "env-test",
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

    // Run the gate (gate pass executes checker for auto gates)
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path())
        .args(["gate", "pass", issue_id, "env-test"])
        .assert()
        .success();

    // Inspect last run result — checker should have seen the env vars
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    let output = cmd
        .current_dir(temp.path())
        .args(["gate", "check", issue_id, "env-test", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let stdout = json["stdout"].as_str().unwrap();
    assert!(
        stdout.contains("FOO=bar"),
        "Expected FOO=bar in stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("BAZ=qux"),
        "Expected BAZ=qux in stdout: {}",
        stdout
    );
}
