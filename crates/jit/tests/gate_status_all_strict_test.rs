//! Integration tests for the `jit gate status-all` strict readiness exit
//! (issue 949cd9d0) and for the legacy verb aliases resolving to the renamed
//! canonical commands.
//!
//! `status-all` exits 0 only when every required gate — automated AND manual —
//! has passed; a pending (auto never run, manual never attested) or failed gate
//! yields exit 4. The legacy verbs `pass`/`pass-all`/`check`/`check-all` remain
//! silent aliases of `evaluate`/`evaluate-all`/`status`/`status-all`.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn jit() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

fn define_auto_gate(temp: &TempDir, key: &str, checker: &str) {
    jit()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            key,
            "--title",
            key,
            "--description",
            "Test",
            "--mode",
            "auto",
            "--checker-command",
            checker,
            "--timeout",
            "10",
        ])
        .assert()
        .success();
}

fn define_manual_gate(temp: &TempDir, key: &str) {
    jit()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            key,
            "--title",
            key,
            "--description",
            "Test",
            "--mode",
            "manual",
        ])
        .assert()
        .success();
}

/// Create an issue requiring the given gate keys; returns its short id.
fn create_issue(temp: &TempDir, gate_keys: &[&str]) -> String {
    let mut args = vec!["issue", "create", "--title", "Test issue"];
    for key in gate_keys {
        args.push("--gate");
        args.push(key);
    }
    let output = jit()
        .current_dir(temp.path())
        .args(&args)
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

// ===== REQ-02: status-all strict exit =====

#[test]
fn test_status_all_pending_auto_gate_exits_4() {
    let temp = setup_repo();
    define_auto_gate(&temp, "auto-gate", "exit 0");
    let id = create_issue(&temp, &["auto-gate"]);

    // Auto gate never evaluated -> pending -> exit 4, no mutation.
    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id, "--json"])
        .assert()
        .failure()
        .code(4)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["all_passed"], false);
    assert_eq!(json["total"].as_u64(), Some(1));
    assert_eq!(json["passed"].as_u64(), Some(0));
    let entry = &json["gates"][0];
    assert_eq!(entry["key"], "auto-gate");
    assert_eq!(entry["status"], "pending");
}

#[test]
fn test_status_all_pending_manual_gate_exits_4() {
    let temp = setup_repo();
    define_manual_gate(&temp, "manual-gate");
    let id = create_issue(&temp, &["manual-gate"]);

    // A required manual gate that was never attested counts as pending.
    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id, "--json"])
        .assert()
        .failure()
        .code(4)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["all_passed"], false);
    assert_eq!(json["total"].as_u64(), Some(1));
    assert_eq!(json["passed"].as_u64(), Some(0));
    let entry = &json["gates"][0];
    assert_eq!(entry["key"], "manual-gate");
    assert_eq!(entry["status"], "pending");
    // Human output must still explain the pending gate.
    jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id])
        .assert()
        .failure()
        .code(4)
        .stdout(predicate::str::contains("manual-gate"));
}

#[test]
fn test_status_all_failed_gate_exits_4() {
    let temp = setup_repo();
    define_auto_gate(&temp, "fail-gate", "exit 1");
    let id = create_issue(&temp, &["fail-gate"]);

    // Evaluate records a failed verdict for the auto gate.
    jit()
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "fail-gate"])
        .assert()
        .failure();

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id, "--json"])
        .assert()
        .failure()
        .code(4)
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["all_passed"], false);
    assert_eq!(json["passed"].as_u64(), Some(0));
    let entry = &json["gates"][0];
    assert_eq!(entry["key"], "fail-gate");
    // Failed is distinguished from pending in the per-gate JSON.
    assert_eq!(entry["status"], "failed");
    // A failed gate is not reported as "not run".
    assert!(json["not_run"].as_array().unwrap().is_empty());
}

#[test]
fn test_status_all_all_passed_exits_0() {
    let temp = setup_repo();
    define_auto_gate(&temp, "auto-ok", "exit 0");
    define_manual_gate(&temp, "manual-ok");
    let id = create_issue(&temp, &["auto-ok", "manual-ok"]);

    jit()
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "auto-ok"])
        .assert()
        .success();
    jit()
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "manual-ok"])
        .assert()
        .success();

    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["all_passed"], true);
    assert_eq!(json["total"].as_u64(), Some(2));
    assert_eq!(json["passed"].as_u64(), Some(2));
    for entry in json["gates"].as_array().unwrap() {
        assert_eq!(entry["status"], "passed");
    }
}

// ===== REQ-01 / REQ-03: legacy verb aliases resolve to the renamed commands =====

#[test]
fn test_pass_alias_runs_checker_like_evaluate() {
    let temp = setup_repo();
    define_auto_gate(&temp, "gate-a", "exit 0");
    let id = create_issue(&temp, &["gate-a"]);

    // `gate pass` (alias) must execute the checker identically to `gate evaluate`.
    let out = jit()
        .current_dir(temp.path())
        .args(["gate", "pass", &id, "gate-a", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["verdict"], "pass");
    // The recorded gate status confirms the checker actually ran.
    jit()
        .current_dir(temp.path())
        .args(["gate", "status-all", &id])
        .assert()
        .success();
}

#[test]
fn test_eval_alias_runs_checker_like_evaluate() {
    let temp = setup_repo();
    define_auto_gate(&temp, "gate-b", "exit 0");
    let id = create_issue(&temp, &["gate-b"]);

    jit()
        .current_dir(temp.path())
        .args(["gate", "eval", &id, "gate-b", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"verdict\": \"pass\""));
}

#[test]
fn test_check_all_alias_resolves_to_status_all() {
    let temp = setup_repo();
    define_auto_gate(&temp, "gate-c", "exit 0");
    let id = create_issue(&temp, &["gate-c"]);

    // `check-all` (alias) shares the strict exit of `status-all`: pending -> 4.
    jit()
        .current_dir(temp.path())
        .args(["gate", "check-all", &id])
        .assert()
        .failure()
        .code(4);

    jit()
        .current_dir(temp.path())
        .args(["gate", "evaluate", &id, "gate-c"])
        .assert()
        .success();

    // After passing, the alias exits 0 like `status-all`.
    jit()
        .current_dir(temp.path())
        .args(["gate", "check-all", &id])
        .assert()
        .success();
}

#[test]
fn test_check_alias_resolves_to_status_non_strict() {
    let temp = setup_repo();
    define_manual_gate(&temp, "gate-d");
    let id = create_issue(&temp, &["gate-d"]);

    // `check` (alias) resolves to the non-strict singular `status`: inspecting a
    // pending gate exits 0 (pure inspection, no readiness contract).
    jit()
        .current_dir(temp.path())
        .args(["gate", "check", &id, "gate-d"])
        .assert()
        .success();
}
