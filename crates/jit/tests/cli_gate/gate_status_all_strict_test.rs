//! Integration tests for the `jit gate status-all` strict readiness exit
//! (issue 949cd9d0) and for the removal of the legacy verb aliases
//! `pass`/`pass-all`/`check`/`check-all` (issue c505031a).
//!
//! `status-all` succeeds only when every required gate — automated AND manual —
//! has passed; a pending (auto never run, manual never attested) or failed gate
//! fails. The retired verbs no longer parse at all: clap rejects them
//! as unrecognized subcommands rather than resolving them as aliases.

use crate::{create_issue_requiring, define_auto_gate, define_manual_gate};
use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;

// ===== REQ-02: status-all strict exit =====

#[test]
fn test_status_all_pending_auto_gate_fails() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "auto-gate", "exit 0");
    let id = create_issue_requiring(&temp, &["auto-gate"]);

    // Auto gate never evaluated -> pending -> failure, no mutation.
    let out = temp
        .command(["gate", "status-all", &id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let envelope: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(envelope["error"]["code"], "GATE_FAILED");
    let json = &envelope["error"]["details"];
    assert_eq!(json["all_passed"], false);
    assert_eq!(json["total"].as_u64(), Some(1));
    assert_eq!(json["passed"].as_u64(), Some(0));
    let entry = &json["gates"][0];
    assert_eq!(entry["key"], "auto-gate");
    assert_eq!(entry["status"], "pending");
}

#[test]
fn test_status_all_pending_manual_gate_fails() {
    let temp = JitCli::initialised().unwrap();
    define_manual_gate(&temp, "manual-gate");
    let id = create_issue_requiring(&temp, &["manual-gate"]);

    // A required manual gate that was never attested counts as pending.
    let out = temp
        .command(["gate", "status-all", &id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let envelope: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(envelope["error"]["code"], "GATE_FAILED");
    let json = &envelope["error"]["details"];
    assert_eq!(json["all_passed"], false);
    assert_eq!(json["total"].as_u64(), Some(1));
    assert_eq!(json["passed"].as_u64(), Some(0));
    let entry = &json["gates"][0];
    assert_eq!(entry["key"], "manual-gate");
    assert_eq!(entry["status"], "pending");
    // Human output must still explain the pending gate.
    temp.command(["gate", "status-all", &id])
        .assert()
        .failure()
        .stdout(predicate::str::contains("manual-gate"));
}

#[test]
fn test_status_all_failed_gate_fails() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "fail-gate", "exit 1");
    let id = create_issue_requiring(&temp, &["fail-gate"]);

    // Evaluate records a failed verdict for the auto gate.
    temp.command(["gate", "evaluate", &id, "fail-gate"])
        .assert()
        .failure();

    let out = temp
        .command(["gate", "status-all", &id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let envelope: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(envelope["error"]["code"], "GATE_FAILED");
    let json = &envelope["error"]["details"];
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
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "auto-ok", "exit 0");
    define_manual_gate(&temp, "manual-ok");
    let id = create_issue_requiring(&temp, &["auto-ok", "manual-ok"]);

    temp.command(["gate", "evaluate", &id, "auto-ok"])
        .assert()
        .success();
    temp.command([
        "gate",
        "evaluate",
        &id,
        "manual-ok",
        "--by",
        "human:reviewer",
    ])
    .assert()
    .success();

    let out = temp
        .command(["gate", "status-all", &id, "--json"])
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

// ===== REQ-01: `eval` remains a working alias of `evaluate` =====

#[test]
fn test_eval_alias_runs_checker_like_evaluate() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "gate-b", "exit 0");
    let id = create_issue_requiring(&temp, &["gate-b"]);

    temp.command(["gate", "eval", &id, "gate-b", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"verdict\": \"pass\""));
}

#[test]
fn test_canonical_verbs_still_work_after_alias_removal() {
    let temp = JitCli::initialised().unwrap();
    define_auto_gate(&temp, "gate-c", "exit 0");
    define_manual_gate(&temp, "gate-d");
    let id = create_issue_requiring(&temp, &["gate-c", "gate-d"]);

    // `evaluate` (formerly aliased as `pass`) still runs the checker.
    let out = temp
        .command(["gate", "evaluate", &id, "gate-c", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["verdict"], "pass");

    // `status` (formerly aliased as `check`) still inspects a single gate,
    // non-strictly: a pending manual gate exits 0.
    temp.command(["gate", "status", &id, "gate-d"])
        .assert()
        .success();

    // `status-all` (formerly aliased as `check-all`) still enforces the
    // strict readiness exit: gate-d is still pending, so this fails.
    temp.command(["gate", "status-all", &id]).assert().failure();

    // `evaluate-all` (formerly aliased as `pass-all`) still evaluates every
    // required gate; once gate-d is attested, status-all exits 0.
    temp.command(["gate", "evaluate-all", &id, "--by", "human:reviewer"])
        .assert()
        .success();
    temp.command(["gate", "status-all", &id]).assert().success();
}
