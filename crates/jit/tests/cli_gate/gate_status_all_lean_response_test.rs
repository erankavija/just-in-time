//! Verify `jit gate status-all --json` omits stdout/stderr for passing runs
//! by default, retains them for failing runs, and restores them under --full.

use crate::add_auto_gate_issue;
use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

#[test]
fn test_status_all_omits_stdout_for_passing_runs_by_default() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "noisy-pass", "printf SECRET_OUTPUT && exit 0");

    temp.command(["gate", "evaluate", &issue_id, "noisy-pass"])
        .assert()
        .success();

    let output = temp
        .command(["gate", "status-all", &issue_id, "--json"])
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
fn test_status_all_includes_stdout_for_failing_runs_even_without_full() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "noisy-fail", "printf FAILURE_DETAIL && exit 1");

    // Run the gate so a failed result is recorded. `gate evaluate` for an auto
    // gate executes the checker and records the run regardless of status.
    let _ = temp
        .command(["gate", "evaluate", &issue_id, "noisy-fail"])
        .output()
        .unwrap();

    // A failed required gate makes `status-all` fail.
    let output = temp
        .command(["gate", "status-all", &issue_id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let result = &json["error"]["details"]["results"][0];
    assert_ne!(result["status"].as_str(), Some("passed"));
    assert_eq!(result["status"].as_str(), Some("failed"));
    let serialized = serde_json::to_string(&json).unwrap();
    assert!(
        serialized.contains("FAILURE_DETAIL"),
        "failing run must retain stdout so agents can diagnose; got: {}",
        serialized
    );
}

#[test]
fn test_status_all_full_flag_restores_stdout_for_passing_runs() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "noisy-pass2", "printf RESTORED && exit 0");

    temp.command(["gate", "evaluate", &issue_id, "noisy-pass2"])
        .assert()
        .success();

    let output = temp
        .command(["gate", "status-all", &issue_id, "--json", "--full"])
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
