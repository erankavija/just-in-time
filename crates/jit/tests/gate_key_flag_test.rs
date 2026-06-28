//! REQ-03: gate pass and gate check accept --gate <key> in addition to the positional key.
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
// gate pass — positional (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_gate_pass_positional_key_still_works() {
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "code-review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Passed gate 'code-review'"));
}

// ---------------------------------------------------------------------------
// gate pass — flag form
// ---------------------------------------------------------------------------

#[test]
fn test_gate_pass_flag_key_accepted() {
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "--gate", "code-review"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Passed gate 'code-review'"));
}

#[test]
fn test_gate_pass_flag_and_positional_produce_identical_outcome() {
    // Positional form
    let (temp_pos, id_pos) = setup_manual_gate_issue("code-review");
    let out_pos = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_pos.path())
        .args(["gate", "pass", &id_pos, "code-review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Flag form
    let (temp_flag, id_flag) = setup_manual_gate_issue("code-review");
    let out_flag = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_flag.path())
        .args(["gate", "pass", &id_flag, "--gate", "code-review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json_pos: serde_json::Value = serde_json::from_slice(&out_pos).unwrap();
    let json_flag: serde_json::Value = serde_json::from_slice(&out_flag).unwrap();

    // gate_key and status must match between the two forms
    assert_eq!(json_pos["gate_key"], json_flag["gate_key"]);
    assert_eq!(json_pos["status"], json_flag["status"]);
    assert_eq!(json_pos["verdict"], json_flag["verdict"]);
}

// ---------------------------------------------------------------------------
// gate pass — both positional and --gate: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_pass_both_positional_and_flag_errors() {
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "pass",
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
// gate pass — neither: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_pass_neither_positional_nor_flag_errors() {
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").or(predicate::str::contains("gate key")));
}

// ---------------------------------------------------------------------------
// gate check — positional (regression guard)
// ---------------------------------------------------------------------------

#[test]
fn test_gate_check_positional_key_still_works() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");

    // Run gate first so check has something to show
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "tests"])
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed").or(predicate::str::contains("Passed")));
}

// ---------------------------------------------------------------------------
// gate check — flag form
// ---------------------------------------------------------------------------

#[test]
fn test_gate_check_flag_key_accepted() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "pass", &issue_id, "tests"])
        .assert()
        .success();

    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "--gate", "tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed").or(predicate::str::contains("Passed")));
}

#[test]
fn test_gate_check_flag_and_positional_produce_identical_outcome() {
    // Positional form
    let (temp_pos, id_pos) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_pos.path())
        .args(["gate", "pass", &id_pos, "tests"])
        .assert()
        .success();
    let out_pos = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_pos.path())
        .args(["gate", "check", &id_pos, "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    // Flag form
    let (temp_flag, id_flag) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_flag.path())
        .args(["gate", "pass", &id_flag, "tests"])
        .assert()
        .success();
    let out_flag = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_flag.path())
        .args(["gate", "check", &id_flag, "--gate", "tests", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json_pos: serde_json::Value = serde_json::from_slice(&out_pos).unwrap();
    let json_flag: serde_json::Value = serde_json::from_slice(&out_flag).unwrap();

    assert_eq!(json_pos["gate_key"], json_flag["gate_key"]);
    assert_eq!(json_pos["status"], json_flag["status"]);
}

// ---------------------------------------------------------------------------
// gate check — both positional and --gate: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_check_both_positional_and_flag_errors() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "tests", "--gate", "tests"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").and(predicate::str::contains("positional")));
}

// ---------------------------------------------------------------------------
// gate check — neither: actionable error
// ---------------------------------------------------------------------------

#[test]
fn test_gate_check_neither_positional_nor_flag_errors() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id])
        .assert()
        .failure()
        .stderr(predicate::str::contains("--gate").or(predicate::str::contains("gate key")));
}

// ---------------------------------------------------------------------------
// --json contract: the both/neither error is machine-readable, not plain text
// (REQ-03 must not violate the every-command-supports-`--json` contract).
// ---------------------------------------------------------------------------

#[test]
fn test_gate_pass_both_with_json_emits_machine_readable_error() {
    let (temp, issue_id) = setup_manual_gate_issue("code-review");
    let out = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "gate",
            "pass",
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
        .expect("`gate pass --json` argument error must be valid JSON on stdout");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_gate_check_neither_with_json_emits_machine_readable_error() {
    let (temp, issue_id) = setup_auto_gate_issue("tests");
    let out = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["gate", "check", &issue_id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out)
        .expect("`gate check --json` argument error must be valid JSON on stdout");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}
