//! Integration tests for `jit gate evaluate-all` (issue df7f934c).
//!
//! Covers: all-pass with per-gate JSON; fail-fast at the first non-passing gate
//! (a later gate's checker never runs); runner error; and, for a mixed
//! auto/manual gate set, fail-fast at an unattested manual gate rather than a
//! silent pass (jit:1d59070d REQ-03).

use super::{create_issue_requiring, define_auto_gate, define_manual_gate};
use jit::test_cli::JitCli;
use std::fs;

/// Define an auto gate `key` whose checker runs `command` and appends a line to
/// `<key>.log` (a side-effect proving the checker ran).
fn define_gate(cli: &JitCli, key: &str, command: &str) {
    define_auto_gate(cli, key, &format!("echo run >> {key}.log; {command}"));
}

/// Add a registry-configured native review placeholder.
fn define_review_placeholder_gate(cli: &JitCli, key: &str) {
    let registry_path = cli.root().join(".jit/gates.toml");
    fs::write(
        registry_path,
        format!(
            r#"
[[gates]]
version = 1
key = "{key}"
title = "Review"
description = "External review placeholder"
stage = "postcheck"
mode = "auto"
priority = 100

[gates.checker]
type = "review_placeholder"
"#
        ),
    )
    .unwrap();
}

/// How many times gate `key`'s checker ran (lines in its side-effect log).
fn run_count(cli: &JitCli, key: &str) -> usize {
    fs::read_to_string(cli.root().join(format!("{key}.log")))
        .map(|s| s.lines().count())
        .unwrap_or(0)
}

#[test]
fn test_evaluate_all_all_gates_pass_reports_each_gate() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    define_gate(&cli, "g1", "true");
    define_gate(&cli, "g2", "true");
    let id = create_issue_requiring(&cli, &["g1", "g2"]);

    let out = cli
        .command(["gate", "evaluate-all", &id, "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
    let gates = json["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 2);
    assert_eq!(gates[0]["key"], "g1");
    assert_eq!(gates[1]["key"], "g2");
    assert_eq!(run_count(&cli, "g1"), 1);
    assert_eq!(run_count(&cli, "g2"), 1);
}

#[test]
fn test_evaluate_all_json_includes_native_placeholder_warning_per_gate() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    define_review_placeholder_gate(&cli, "review-any-key");
    let id = create_issue_requiring(&cli, &["review-any-key"]);

    let out = cli
        .command(["gate", "evaluate-all", &id, "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "JSON mode must not leak warnings to stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let gates = json["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 1);
    assert_eq!(gates[0]["key"], "review-any-key");
    assert_eq!(
        json["warnings"],
        serde_json::json!([format!(
            "(review-any-key): {}",
            jit::declarations::REVIEW_PLACEHOLDER_WARNING
        )])
    );
}

#[test]
fn test_evaluate_all_fail_fast_stops_at_first_failure() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    define_gate(&cli, "g1", "false"); // first gate fails (checker exit 1)
    define_gate(&cli, "g2", "true"); // must never run
    let id = create_issue_requiring(&cli, &["g1", "g2"]);

    let out = cli
        .command(["gate", "evaluate-all", &id, "--json"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    assert_eq!(json["error"]["details"]["verdict"], "fail");
    assert_eq!(json["error"]["details"]["key"], "g1");

    // Fail-fast: the first gate ran, the later gate's checker never did.
    assert_eq!(run_count(&cli, "g1"), 1);
    assert_eq!(
        run_count(&cli, "g2"),
        0,
        "later gate must NOT run after a fail-fast"
    );
}

#[test]
fn test_evaluate_all_runner_error_reports_error_verdict() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    // First gate's checker kills itself -> no exit code -> runner error.
    define_gate(&cli, "g1", "kill -9 $$");
    define_gate(&cli, "g2", "true");
    let id = create_issue_requiring(&cli, &["g1", "g2"]);

    let out = cli
        .command(["gate", "evaluate-all", &id, "--json"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "IO_ERROR");
    assert_eq!(json["error"]["details"]["verdict"], "error");
    assert_eq!(run_count(&cli, "g2"), 0, "later gate must not run");
}

#[test]
fn test_evaluate_all_no_required_gates_succeeds() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    let id = create_issue_requiring(&cli, &[]);

    let out = cli
        .command(["gate", "evaluate-all", &id, "--json"])
        .output()
        .unwrap();

    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gates"].as_array().unwrap().len(), 0);
}

#[test]
fn test_evaluate_all_issue_not_found_reports_error_code() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();

    let out = cli
        .command(["gate", "evaluate-all", "nonexistent", "--json"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
}

/// REQ-03 (jit:1d59070d): `evaluate-all` over a mixed gate set must not
/// silently pass a manual gate. Each required gate is passed through the
/// same `pass_gate` a bare `evaluate` uses, so evaluate-all inherits its
/// fail-fast, --by-required behavior for free: the auto gate ahead of the
/// manual one in priority order still runs and records its verdict, then the
/// manual gate fails with the same usage error and hint — evaluate-all never
/// reports an all-green result that includes an unattested manual gate.
#[test]
fn test_evaluate_all_mixed_gates_without_by_fails_fast_at_manual_gate() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    define_gate(&cli, "auto-gate", "true"); // runs first (declared first)
    define_manual_gate(&cli, "manual-gate");
    let id = create_issue_requiring(&cli, &["auto-gate", "manual-gate"]);

    let out = cli
        .command(["gate", "evaluate-all", &id, "--json"])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert!(json.get("verdict").is_none());
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("manual-gate"));
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("--by <attestor>"));

    // The auto gate ahead of it still ran and recorded a pass (fail-fast is
    // per-gate, not atomic across the whole set — same as a checker failure).
    assert_eq!(run_count(&cli, "auto-gate"), 1);
}

/// The same mixed set with `--by` supplied: every gate passes, auto via its
/// checker and manual via attestation.
#[test]
fn test_evaluate_all_mixed_gates_with_by_all_pass() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    define_gate(&cli, "auto-gate", "true");
    define_manual_gate(&cli, "manual-gate");
    let id = create_issue_requiring(&cli, &["auto-gate", "manual-gate"]);

    let out = cli
        .command([
            "gate",
            "evaluate-all",
            &id,
            "--by",
            "human:reviewer",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["verdict"], "pass");
    let gates = json["gates"].as_array().unwrap();
    assert_eq!(gates.len(), 2);
    assert_eq!(gates[0]["key"], "auto-gate");
    assert_eq!(gates[1]["key"], "manual-gate");
    assert_eq!(run_count(&cli, "auto-gate"), 1);
}

#[test]
fn test_evaluate_many_reports_each_issue_and_continues_after_failure() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("init").unwrap();
    // The first invocation fails and the second passes. The shared log also
    // proves the checker calls were serialized in the supplied issue order.
    define_gate(
        &cli,
        "wave-gate",
        "count=$(wc -l < wave-gate-sequence.log 2>/dev/null || echo 0); echo run-$count >> wave-gate-sequence.log; test \"$count\" -gt 0",
    );
    let first = create_issue_requiring(&cli, &["wave-gate"]);
    let second = create_issue_requiring(&cli, &["wave-gate"]);

    let out = cli
        .command([
            "gate",
            "evaluate-many",
            "wave-gate",
            &first,
            &second,
            "--json",
        ])
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    let results = json["error"]["details"]["results"].as_array().unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["issue_id"], first);
    assert_eq!(results[0]["status"], "failed");
    assert_eq!(results[0]["verdict"], "fail");
    assert_eq!(results[1]["issue_id"], second);
    assert_eq!(results[1]["status"], "passed");
    assert_eq!(results[1]["verdict"], "pass");
    assert_eq!(run_count(&cli, "wave-gate"), 2);
    assert_eq!(
        fs::read_to_string(cli.root().join("wave-gate-sequence.log")).unwrap(),
        "run-0\nrun-1\n"
    );

    for issue_id in [&first, &second] {
        let status = cli
            .command(["gate", "status", issue_id, "wave-gate", "--json"])
            .output()
            .unwrap();
        assert!(status.status.success());
        let status_json: serde_json::Value = serde_json::from_slice(&status.stdout).unwrap();
        assert!(
            status_json["run_id"].is_string(),
            "each issue must have its own recorded run: {status_json}"
        );
    }
}
