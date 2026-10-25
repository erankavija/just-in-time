//! Integration tests for the `jit gate evaluate` exit-code taxonomy and JSON `verdict`.
//!
//! Matrix:
//!   success             -> exit 0,  verdict "pass"
//!   checker failure     -> exit 4,  verdict "fail"   (checker ran, non-zero exit)
//!   runner error        -> exit 10, verdict "error"  (checker killed, no exit code, exit 75, or a built-in checker fault)
//!   issue not found     -> exit 3,  no verdict
//!   gate not required   -> exit 2,  no verdict (evaluate and fail)
//!   manual gate, no --by -> exit 2, no verdict (jit:1d59070d REQ-03)

use crate::add_auto_gate_issue;
use jit::test_cli::JitCli;

#[test]
fn test_gate_evaluate_success_exit_0_verdict_pass() {
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "true");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
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
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "false");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
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
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "kill -9 $$");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
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
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "jit-no-such-command-xyz");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
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
fn test_gate_evaluate_checker_exit_75_records_error_run() {
    // EX_TEMPFAIL: the checker reports it could not judge.
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "exit 75");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(10));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["details"]["verdict"], "error");

    let status = temp
        .command(["gate", "status", &id, "test-gate", "--json"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&status.stdout);
    assert!(
        text.contains("\"error\""),
        "run not recorded as error: {text}"
    );
}

#[test]
fn test_gate_evaluate_other_nonzero_exit_stays_failed() {
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "exit 74");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["details"]["verdict"], "fail");
}

#[test]
fn test_gate_evaluate_runner_error_non_json_exit_10() {
    // The non-JSON path must agree with the JSON path on exit 10.
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "kill -9 $$");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(10));
}

#[test]
fn test_gate_evaluate_checker_failure_non_json_exit_4() {
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "false");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn test_gate_evaluate_issue_not_found_exit_3() {
    let temp = JitCli::initialised().unwrap();

    let output = temp
        .command(["gate", "evaluate", "nonexistent", "test-gate", "--json"])
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
    let temp = JitCli::initialised().unwrap();

    // Create an issue with NO gates.
    let id = temp.create_issue(["--title", "No gates"]).unwrap();

    let output = temp
        .command(["gate", "evaluate", &id, "not-a-gate", "--json"])
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
    let temp = JitCli::initialised().unwrap();

    let id = temp.create_issue(["--title", "No gates"]).unwrap();

    let output = temp
        .command(["gate", "evaluate", &id, "not-a-gate"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_gate_fail_gate_not_required_exit_2() {
    let temp = JitCli::initialised().unwrap();

    let id = temp.create_issue(["--title", "No gates"]).unwrap();

    let output = temp
        .command(["gate", "fail", &id, "not-a-gate", "--json"])
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
}

#[test]
fn test_gate_evaluate_manual_gate_success_verdict_pass() {
    let temp = JitCli::initialised().unwrap();

    // Define a manual gate.
    temp.command([
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

    let id = temp
        .create_issue(["--title", "Work", "--gate", "review"])
        .unwrap();

    let output = temp
        .command([
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
    let temp = JitCli::initialised().unwrap();

    temp.command([
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

    let id = temp
        .create_issue(["--title", "Work", "--gate", "review"])
        .unwrap();

    let output = temp
        .command(["gate", "evaluate", &id, "review", "--json"])
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

#[test]
fn test_gate_evaluate_builtin_checker_fault_exit_10_verdict_error() {
    // A built-in checker that cannot judge the repository reaches the same
    // runner-error reporting as an exec checker.
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "true");
    let gates_path = temp.root().join(".jit/gates.toml");
    let gates = std::fs::read_to_string(&gates_path).unwrap();
    let (head, checker) = gates.split_once("[gates.checker]").unwrap();
    let tail = checker.split_once("\n\n").map_or("", |(_, rest)| rest);
    std::fs::write(
        &gates_path,
        format!(
            "{head}[gates.checker]\ntype = \"label_target_validation\"\nlabel_namespace = \"absent-target\"\n\n{tail}"
        ),
    )
    .unwrap();

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
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
    assert_eq!(json["error"]["details"]["verdict"], "error");
}
