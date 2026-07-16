//! Integration tests for the `jit gate evaluate` exit-code taxonomy and JSON `verdict`.
//!
//! Matrix:
//!   success             -> exit 0,  verdict "pass"
//!   checker failure     -> exit 4,  verdict "fail"   (checker ran, non-zero exit)
//!   runner error        -> exit 10, verdict "error"  (checker killed, no exit code)
//!   issue not found     -> exit 3,  no verdict
//!   gate not required   -> exit 2,  no verdict
//!   manual gate, no --by -> exit 2, no verdict (jit:1d59070d REQ-03)

use std::process::Command;
use tempfile::TempDir;

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .output()
        .unwrap();
    temp
}

/// Define an auto gate with the given checker command and create an issue that
/// requires it. Returns (TempDir, issue_id).
fn setup_auto_gate_issue(checker_command: &str) -> (TempDir, String) {
    let temp = setup_repo();
    let define = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
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
        .output()
        .unwrap();
    assert!(
        define.status.success(),
        "gate define failed: {}",
        String::from_utf8_lossy(&define.stderr)
    );

    let create = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Test issue",
            "--gate",
            "test-gate",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(create.status.success());
    let json: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    let id = json["id"].as_str().unwrap().to_string();
    (temp, id)
}

#[test]
fn test_gate_evaluate_success_exit_0_verdict_pass() {
    let (temp, id) = setup_auto_gate_issue("true");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "passed");
    assert_eq!(json["verdict"], "pass");
}

#[test]
fn test_gate_evaluate_checker_failure_exit_4_verdict_fail() {
    // `false` exits non-zero -> GateRunStatus::Failed -> checker failure.
    let (temp, id) = setup_auto_gate_issue("false");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(4),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    assert_eq!(json["error"]["details"]["verdict"], "fail");
}

#[test]
fn test_gate_evaluate_runner_error_exit_10_verdict_error() {
    // A checker that kills itself with SIGKILL leaves no exit code, which the
    // runner classifies as GateRunStatus::Error (runner/infra failure).
    let (temp, id) = setup_auto_gate_issue("kill -9 $$");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(10),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "IO_ERROR");
    assert_eq!(json["error"]["details"]["verdict"], "error");
}

#[test]
fn test_gate_evaluate_command_not_found_exit_10_verdict_error() {
    // A nonexistent executable makes `sh -c` exit 127 (command not found),
    // which the runner classifies as GateRunStatus::Error (runner/infra error),
    // not a checker failure.
    let (temp, id) = setup_auto_gate_issue("jit-no-such-command-xyz");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(10),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "IO_ERROR");
    assert_eq!(json["error"]["details"]["verdict"], "error");
}

#[test]
fn test_gate_evaluate_runner_error_non_json_exit_10() {
    // The non-JSON path must agree with the JSON path on exit 10.
    let (temp, id) = setup_auto_gate_issue("kill -9 $$");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "test-gate"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(10));
}

#[test]
fn test_gate_evaluate_checker_failure_non_json_exit_4() {
    let (temp, id) = setup_auto_gate_issue("false");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "test-gate"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn test_gate_evaluate_issue_not_found_exit_3() {
    let temp = setup_repo();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", "nonexistent", "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(3));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
    // Pre-verdict lookup error: no verdict field.
    assert!(json["error"]["details"]["verdict"].is_null());
}

#[test]
fn test_gate_evaluate_gate_not_required_exit_2() {
    let temp = setup_repo();

    // Create an issue with NO gates.
    let create = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "No gates", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    let id = json["id"].as_str().unwrap().to_string();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "not-a-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    // Pre-verdict argument error: no verdict field.
    assert!(json["error"]["details"]["verdict"].is_null());
}

#[test]
fn test_gate_evaluate_gate_not_required_non_json_exit_2() {
    let temp = setup_repo();

    let create = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "create", "--title", "No gates", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    let id = json["id"].as_str().unwrap().to_string();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "not-a-gate"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_gate_evaluate_manual_gate_success_verdict_pass() {
    let temp = setup_repo();

    // Define a manual gate.
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "review",
            "--title",
            "Review",
            "--description",
            "Manual review",
            "--mode",
            "manual",
        ])
        .output()
        .unwrap();

    let create = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue", "create", "--title", "Work", "--gate", "review", "--json",
        ])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    let id = json["id"].as_str().unwrap().to_string();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "evaluate",
            &id,
            "review",
            "--by",
            "human:reviewer",
            "--json",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
}

/// REQ-03 (jit:1d59070d): a manual gate has no checker to run, so a bare
/// evaluate would silently record an unattributed pass. It must instead fail
/// as a usage error (exit 2, no verdict) with a hint naming the attested
/// form. Auto gates are unaffected by --by.
#[test]
fn test_gate_evaluate_manual_gate_without_by_exit_2_no_verdict() {
    let temp = setup_repo();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "review",
            "--title",
            "Review",
            "--description",
            "Manual review",
            "--mode",
            "manual",
        ])
        .output()
        .unwrap();

    let create = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue", "create", "--title", "Work", "--gate", "review", "--json",
        ])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&create.stdout).unwrap();
    let id = json["id"].as_str().unwrap().to_string();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "review", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json.get("verdict").is_none());
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--by <attestor>"));
    assert!(json["error"]["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .any(|s| s
            .as_str()
            .unwrap()
            .contains(&format!("jit gate evaluate {} review --by <attestor>", id))));
}
