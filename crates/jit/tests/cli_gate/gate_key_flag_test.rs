//! REQ-03: gate evaluate and gate status accept --gate <key> in addition to the positional key.
//!
//! Tests confirm:
//!   1. Positional form still works (regression guard).
//!   2. Flag form (`--gate <key>`) works identically.
//!   3. Supplying BOTH positional and --gate errors with an actionable message.
//!   4. Supplying NEITHER errors with an actionable message.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

/// Define a manual gate and create an issue that requires it.
/// Returns (TempDir, short_issue_id).
fn setup_manual_gate_issue(gate_key: &str) -> (TempDir, String) {
    let temp = setup_repo();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            gate_key,
            "--title",
            "Test Gate",
            "--description",
            "Test gate for REQ-03",
            "--mode",
            "manual",
        ])
        .assert()
        .success();

    let out = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
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

    let out_str = String::from_utf8_lossy(&out);
    let issue_id = out_str
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();
    (temp, issue_id)
}

/// Define an auto gate (exit 0) and create an issue with it.
fn setup_auto_gate_issue(gate_key: &str) -> (TempDir, String) {
    let temp = setup_repo();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            gate_key,
            "--title",
            "Auto Gate",
            "--description",
            "Auto gate for REQ-03",
            "--mode",
            "auto",
            "--checker-command",
            "exit 0",
            "--timeout",
            "10",
        ])
        .assert()
        .success();

    let out = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
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

    let out_str = String::from_utf8_lossy(&out);
    let issue_id = out_str
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();
    (temp, issue_id)
}

// ---------------------------------------------------------------------------
// gate evaluate — positional (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_gate_evaluate_positional_key_still_works() {
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
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
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
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
    let (temp_pos, id_pos) = setup_manual_gate_issue("code-review");
    let out_pos = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_pos.path())
        .args([
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
    let (temp_flag, id_flag) = setup_manual_gate_issue("code-review");
    let out_flag = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_flag.path())
        .args([
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
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
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
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").or(predicate::str::contains("gate key")));
}

// ---------------------------------------------------------------------------
// gate status — positional (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_positional_key_still_works() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");

    // Run gate first so status has something to show
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &issue_id, "tests"])
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "status", &issue_id, "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed").or(predicate::str::contains("Passed")));
}

// ---------------------------------------------------------------------------
// gate status — flag form
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_flag_key_accepted() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "evaluate", &issue_id, "tests"])
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "status", &issue_id, "--gate", "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed").or(predicate::str::contains("Passed")));
}

#[test]
fn test_gate_status_flag_and_positional_produce_identical_outcome() {
    // Positional form
    let (temp_pos, id_pos) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_pos.path())
        .args(["gate", "evaluate", &id_pos, "tests"])
        .assert()
        .success();
    let out_pos = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_pos.path())
        .args(["gate", "status", &id_pos, "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Flag form
    let (temp_flag, id_flag) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_flag.path())
        .args(["gate", "evaluate", &id_flag, "tests"])
        .assert()
        .success();
    let out_flag = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_flag.path())
        .args(["gate", "status", &id_flag, "--gate", "tests", "--json"])
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
    let (temp, issue_id) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "status", &issue_id, "tests", "--gate", "tests"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").and(predicate::str::contains("positional")));
}

// ---------------------------------------------------------------------------
// gate status — neither: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_status_neither_positional_nor_flag_errors() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "status", &issue_id])
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
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    let out = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
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
    let (temp, issue_id) = setup_auto_gate_issue("tests");
    let out = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "status", &issue_id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .expect("`gate status --json` argument error must be valid JSON on stdout");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}
