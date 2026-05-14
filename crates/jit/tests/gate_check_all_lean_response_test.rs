//! Verify `jit gate check-all --json` omits stdout/stderr for passing runs
//! by default, retains them for failing runs, and restores them under --full.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

fn create_issue_with_gate(temp: &TempDir, gate_key: &str, checker_command: &str) -> String {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            gate_key,
            "--title",
            gate_key,
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
            gate_key,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output_str = String::from_utf8_lossy(&output);
    output_str
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

#[test]
fn test_check_all_omits_stdout_for_passing_runs_by_default() {
    let temp = setup_repo();
    let issue_id = create_issue_with_gate(&temp, "noisy-pass", "printf SECRET_OUTPUT && exit 0");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "noisy-pass"])
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check-all", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let result = &json["results"][0];
    assert_eq!(result["status"].as_str(), Some("passed"));
    assert!(
        result.get("stdout").is_none() || result["stdout"].is_null(),
        "passed run should omit stdout by default, got: {}",
        result
    );
    assert!(
        result.get("stderr").is_none() || result["stderr"].is_null(),
        "passed run should omit stderr by default, got: {}",
        result
    );
}

#[test]
fn test_check_all_includes_stdout_for_failing_runs_even_without_full() {
    let temp = setup_repo();
    let issue_id = create_issue_with_gate(&temp, "noisy-fail", "printf FAILURE_DETAIL && exit 1");

    // Run the gate so a failed result is recorded. `gate pass` for an auto
    // gate executes the checker and records the run regardless of status.
    let _ = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "noisy-fail"])
        .output()
        .unwrap();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check-all", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let result = &json["results"][0];
    assert_ne!(result["status"].as_str(), Some("passed"));
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(
        serialized.contains("FAILURE_DETAIL"),
        "failing run must retain stdout so agents can diagnose; got: {}",
        serialized
    );
}

#[test]
fn test_check_all_full_flag_restores_stdout_for_passing_runs() {
    let temp = setup_repo();
    let issue_id = create_issue_with_gate(&temp, "noisy-pass2", "printf RESTORED && exit 0");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "noisy-pass2"])
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check-all", &issue_id, "--json", "--full"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let serialized = String::from_utf8_lossy(&output);
    assert!(
        serialized.contains("RESTORED"),
        "--full should restore stdout for passing runs; got: {}",
        serialized
    );
}
