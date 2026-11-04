//! REQ-03: gate evaluate and gate status accept --gate <key> in addition to the positional key.
//!
//! Tests confirm:
//!   1. Positional form still works (regression guard).
//!   2. Flag form (`--gate <key>`) works identically.
//!   3. Supplying BOTH positional and --gate errors with an actionable message.
//!   4. Supplying NEITHER errors with an actionable message.

use super::{add_auto_gate_issue, add_manual_gate_issue};
use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// gate evaluate — positional (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_positional_key_still_works() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_manual_gate_issue(&temp, "code-review");
    temp.command([
        "gate",
        "evaluate",
        &issue_id,
        "code-review",
        "--by",
        "human:reviewer",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Passed gate 'code-review'"));
}

// ---------------------------------------------------------------------------
// gate evaluate — flag form
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_flag_key_accepted() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_manual_gate_issue(&temp, "code-review");
    temp.command([
        "gate",
        "evaluate",
        &issue_id,
        "--gate",
        "code-review",
        "--by",
        "human:reviewer",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Passed gate 'code-review'"));
}

#[test]
fn test_gate_evaluate_flag_and_positional_produce_identical_outcome() {
    // Positional form
    let temp_pos = JitCli::initialised().unwrap();
    let id_pos = add_manual_gate_issue(&temp_pos, "code-review");
    let out_pos = temp_pos
        .command([
            "gate",
            "evaluate",
            &id_pos,
            "code-review",
            "--by",
            "human:reviewer",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Flag form
    let temp_flag = JitCli::initialised().unwrap();
    let id_flag = add_manual_gate_issue(&temp_flag, "code-review");
    let out_flag = temp_flag
        .command([
            "gate",
            "evaluate",
            &id_flag,
            "--gate",
            "code-review",
            "--by",
            "human:reviewer",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json_pos: serde_json::Value = serde_json::from_slice(&out_pos).unwrap();
    let json_flag: serde_json::Value = serde_json::from_slice(&out_flag).unwrap();

    // key and status must match between the two forms
    assert_eq!(json_pos["key"], json_flag["key"]);
    assert_eq!(json_pos["status"], json_flag["status"]);
    assert_eq!(json_pos["verdict"], json_flag["verdict"]);
}

// ---------------------------------------------------------------------------
// gate evaluate — both positional and --gate: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_both_positional_and_flag_errors() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_manual_gate_issue(&temp, "code-review");
    temp.command([
        "gate",
        "evaluate",
        &issue_id,
        "code-review",
        "--gate",
        "code-review",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("--gate").and(predicate::str::contains("positional")));
}

// ---------------------------------------------------------------------------
// gate evaluate — neither: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_neither_positional_nor_flag_errors() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_manual_gate_issue(&temp, "code-review");
    temp.command(["gate", "evaluate", &issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").or(predicate::str::contains("gate key")));
}

// ---------------------------------------------------------------------------
// gate status — positional (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_positional_key_still_works() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "tests", "exit 0");

    // Run gate first so status has something to show
    temp.command(["gate", "evaluate", &issue_id, "tests"])
        .assert()
        .success();

    temp.command(["gate", "status", &issue_id, "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

// ---------------------------------------------------------------------------
// gate status — flag form
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_flag_key_accepted() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "tests", "exit 0");

    temp.command(["gate", "evaluate", &issue_id, "tests"])
        .assert()
        .success();

    temp.command(["gate", "status", &issue_id, "--gate", "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_gate_status_flag_and_positional_produce_identical_outcome() {
    // Positional form
    let temp_pos = JitCli::initialised().unwrap();
    let id_pos = add_auto_gate_issue(&temp_pos, "tests", "exit 0");
    temp_pos
        .command(["gate", "evaluate", &id_pos, "tests"])
        .assert()
        .success();
    let out_pos = temp_pos
        .command(["gate", "status", &id_pos, "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Flag form
    let temp_flag = JitCli::initialised().unwrap();
    let id_flag = add_auto_gate_issue(&temp_flag, "tests", "exit 0");
    temp_flag
        .command(["gate", "evaluate", &id_flag, "tests"])
        .assert()
        .success();
    let out_flag = temp_flag
        .command(["gate", "status", &id_flag, "--gate", "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json_pos: serde_json::Value = serde_json::from_slice(&out_pos).unwrap();
    let json_flag: serde_json::Value = serde_json::from_slice(&out_flag).unwrap();

    assert_eq!(json_pos["key"], json_flag["key"]);
    assert_eq!(json_pos["status"], json_flag["status"]);
}

// ---------------------------------------------------------------------------
// gate status — both positional and --gate: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_both_positional_and_flag_errors() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "tests", "exit 0");
    temp.command(["gate", "status", &issue_id, "tests", "--gate", "tests"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").and(predicate::str::contains("positional")));
}

// ---------------------------------------------------------------------------
// gate status — neither: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_neither_positional_nor_flag_errors() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "tests", "exit 0");
    temp.command(["gate", "status", &issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").or(predicate::str::contains("gate key")));
}

// ---------------------------------------------------------------------------
// --json contract: the both/neither error is machine-readable, not plain text
// (REQ-03 must not violate the every-command-supports-`--json` contract).
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_both_with_json_emits_machine_readable_error() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_manual_gate_issue(&temp, "code-review");
    let out = temp
        .command([
            "gate",
            "evaluate",
            &issue_id,
            "code-review",
            "--gate",
            "code-review",
            "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .expect("`gate evaluate --json` argument error must be valid JSON on stdout");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_gate_status_neither_with_json_emits_machine_readable_error() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "tests", "exit 0");
    let out = temp
        .command(["gate", "status", &issue_id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .expect("`gate status --json` argument error must be valid JSON on stdout");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

// ---------------------------------------------------------------------------
// Passed line is identical across evaluate, evaluate-all and evaluate-many
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_commands_report_passed_gate_with_one_suffixless_line() {
    let passed_stdout = |subcommand_args: &[&str]| {
        let temp = JitCli::initialised().unwrap();
        let issue_id = add_manual_gate_issue(&temp, "code-review");
        let mut args = vec!["gate"];
        args.extend_from_slice(subcommand_args);
        let args: Vec<String> = args
            .into_iter()
            .map(|a| a.replace("{id}", &issue_id))
            .collect();
        let out = temp
            .command(&args)
            .args(["--by", "human:reviewer"])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        let line = String::from_utf8_lossy(&out)
            .lines()
            .find(|l| l.contains("Passed gate"))
            .expect("a passed line is printed")
            .replace(&issue_id, "{id}");
        line
    };

    let evaluate = passed_stdout(&["evaluate", "{id}", "code-review"]);
    let evaluate_all = passed_stdout(&["evaluate-all", "{id}"]);
    let evaluate_many = passed_stdout(&["evaluate-many", "code-review", "{id}"]);

    assert_eq!(evaluate, "Passed gate 'code-review' for issue {id}");
    assert_eq!(evaluate_all, evaluate);
    assert_eq!(evaluate_many, evaluate);
}
