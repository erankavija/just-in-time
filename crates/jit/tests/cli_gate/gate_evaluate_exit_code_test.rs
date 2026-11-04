//! Integration tests for the JSON `verdict` of `jit gate evaluate`.
//!
//! Matrix:
//!   success             -> verdict "pass"
//!   checker failure     -> verdict "fail"   (checker ran, non-zero exit)
//!   runner error        -> verdict "error"  (checker killed, no exit code, exit 75, or a built-in checker fault)
//!   issue not found     -> no verdict
//!   gate not required   -> no verdict
//!   manual gate, no --by -> no verdict (jit:1d59070d REQ-03)

use super::add_auto_gate_issue;
use jit::test_cli::JitCli;

#[test]
fn test_gate_evaluate_success_verdict_pass() {
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "true");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["status"], "passed");
    assert_eq!(json["verdict"], "pass");
}

#[test]
fn test_gate_evaluate_checker_failure_verdict_fail() {
    // `false` exits non-zero -> GateRunStatus::Failed -> checker failure.
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "false");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    assert_eq!(json["error"]["details"]["verdict"], "fail");
}

#[test]
fn test_gate_evaluate_runner_error_verdict_error() {
    // A checker that kills itself with SIGKILL leaves no exit code, which the
    // runner classifies as GateRunStatus::Error (runner/infra failure).
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "kill -9 $$");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "IO_ERROR");
    assert_eq!(json["error"]["details"]["verdict"], "error");
}

#[test]
fn test_gate_evaluate_command_not_found_verdict_error() {
    // A nonexistent executable makes `sh -c` exit 127 (command not found),
    // which the runner classifies as GateRunStatus::Error (runner/infra error),
    // not a checker failure.
    let temp = JitCli::initialised().unwrap();
    let id = add_auto_gate_issue(&temp, "test-gate", "jit-no-such-command-xyz");

    let output = temp
        .command(["gate", "evaluate", &id, "test-gate", "--json"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
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

    assert!(!output.status.success());
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

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["details"]["verdict"], "fail");
}

#[test]
fn test_gate_evaluate_issue_not_found_reports_no_verdict() {
    let temp = JitCli::initialised().unwrap();

    let output = temp
        .command(["gate", "evaluate", "nonexistent", "test-gate", "--json"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
    // Pre-verdict lookup error: no verdict field.
    assert!(json["error"]["details"]["verdict"].is_null());
}

#[test]
fn test_gate_evaluate_invalid_id_prefix_reports_its_classified_code_in_json_and_human_modes() {
    // Formerly reported as the GATE_ERROR fallback under --json only.
    let temp = JitCli::initialised().unwrap();
    let code = jit::output::ErrorCode::InvalidIdPrefix;

    for json in [true, false] {
        let mut args = vec!["gate", "evaluate", "!", "test-gate"];
        args.extend(json.then_some("--json"));
        let output = temp.command(args).output().unwrap();

        assert_eq!(
            output.status.code(),
            Some(code.exit_code().code()),
            "{json}"
        );
        if json {
            let printed: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(printed["error"]["code"], code.as_str());
        }
    }
}

#[test]
fn test_gate_evaluate_gate_not_required_reports_no_verdict() {
    let temp = JitCli::initialised().unwrap();

    // Create an issue with NO gates.
    let id = temp.create_issue(["--title", "No gates"]).unwrap();

    let output = temp
        .command(["gate", "evaluate", &id, "not-a-gate", "--json"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
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

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
}

/// REQ-03 (jit:1d59070d): a manual gate has no checker to run, so a bare
/// evaluate would silently record an unattributed pass. It must instead fail
/// as a usage error (no verdict) with a hint naming the attested
/// form. Auto gates are unaffected by --by.
#[test]
fn test_gate_evaluate_manual_gate_without_by_reports_no_verdict() {
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

    assert!(!output.status.success());
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
fn test_gate_evaluate_builtin_checker_fault_verdict_error() {
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

    assert!(
        !output.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["details"]["verdict"], "error");
}
