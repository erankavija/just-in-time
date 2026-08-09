//! End-to-end integration test for the bracket coverage-preview gate.
//!
//! The bracket-breakdown helper is a pure bracket-builder: it ATTACHES the
//! coverage-preview gate to the breakdown node `B` (left PENDING) and never runs
//! it. The gate is run separately by the standard gate runner
//! (`jit gate evaluate <B> coverage-preview`) as a breakdown-workflow step. This test
//! proves that the *attached* gate, when run via the real runner against a
//! manually-built bracket fixture, executes the deterministic native
//! `rule_validation` checker and persists a `GateRunResult` reflecting:
//!   - PASS (exit 0) when the drafted children cover every `[hard]` criterion, and
//!   - FAIL (exit 4) when a `[hard]` criterion is left uncovered.
//!
//! It exercises the real subprocess path end-to-end (built `jit` binary against a
//! temp `.jit` repo). The bracket spine `C → child → B` is built by hand so that
//! `B` (which carries the coverage rule's `type:breakdown` selector and the
//! `brackets:<C-short-id>` pointer) is inside `C`'s dependency closure — exactly the shape
//! `bracket_breakdown` produces. The configured rule resolves the selected container from
//! the rule's `container-from-label` setting while keeping `B` as the sole firing issue.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

/// Hierarchy that declares the bracket node types so `epic`/`planning`/`breakdown`
/// are valid in the temp repo. The bracket vocabulary itself (and the
/// `type:breakdown` boundary that `validate --scope` halts on) comes from the
/// `plan` template in `BRACKET_TEMPLATE_TOML`, not from config.
const BRACKET_CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

/// The `plan`-shaped graph template: a planning node `P` (inline plan — no `doc`)
/// and a breakdown node `B`, plus the `B → P` edge, the `C → B` anchor edge, and
/// the `move-upstream-to-role` transform onto `planning`. `validate --scope <C>`
/// reads `B`'s type from this template to bound the coverage walk at `type:breakdown`.
const BRACKET_TEMPLATE_TOML: &str = r#"
[[template]]
name        = "plan"
description = "Plan-before-fan-out bracket."
applies_to  = ["epic"]

  [[template.anchors]]
  name = "container"

  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["plan-review"]
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["coverage-preview"]
  labels      = ["brackets:{container.short_id}"]
  description = "Breakdown node for {container.title}."
  depends_on  = ["planning"]

  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"

  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#;

const COVERAGE_RULES_TOML: &str = r#"
[[rules]]
name = "coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-link = "dependencies", child-type-exclude = ["planning", "breakdown"], container-from-label = "brackets" } }
"#;

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

/// Run a jit command and return parsed JSON stdout (asserting success).
fn jit_json(temp: &TempDir, args: &[&str]) -> serde_json::Value {
    let out = jit(temp)
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).unwrap_or_else(|e| {
        panic!(
            "expected JSON from `jit {}`: {e}\n{}",
            args.join(" "),
            String::from_utf8_lossy(&out)
        )
    })
}

/// Create an issue, returning its full id.
fn create_issue(temp: &TempDir, title: &str, description: &str, labels: &[&str]) -> String {
    let mut args = vec![
        "issue",
        "create",
        "--title",
        title,
        "--description",
        description,
        "--json",
    ];
    for l in labels {
        args.push("--label");
        args.push(l);
    }
    let json = jit_json(temp, &args);
    json["id"].as_str().expect("created issue id").to_string()
}

const COVERAGE_GATE_TOML: &str = r#"
[[gates]]
version = 1
key = "coverage-preview"
title = "Coverage Preview"
description = "Evaluate the configured coverage-preview rule with the gated breakdown as its sole firing issue."
stage = "postcheck"
mode = "auto"
priority = 100
auto = true

[gates.checker]
type = "rule_validation"
rule = "coverage-preview"
"#;

/// Initialize a temp repo with the bracket config, the `plan` template, the
/// coverage rule, and a `coverage-preview` auto gate using the portable native checker.
fn setup_bracket_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();

    let jit_dir = temp.path().join(".jit");
    std::fs::write(jit_dir.join("config.toml"), BRACKET_CONFIG_TOML).unwrap();
    std::fs::write(jit_dir.join("templates.toml"), BRACKET_TEMPLATE_TOML).unwrap();
    std::fs::write(jit_dir.join("rules.toml"), COVERAGE_RULES_TOML).unwrap();
    std::fs::write(jit_dir.join("gates.toml"), COVERAGE_GATE_TOML).unwrap();
    temp
}

/// Build a bracket `C → child → B` by hand (the shape `bracket_breakdown`
/// produces) with the coverage-preview gate attached to `B`. `child_labels`
/// carries the child's coverage credits (`satisfies:<id>`), if any. Returns
/// `(container_id, breakdown_id)`.
fn build_bracket(
    temp: &TempDir,
    container_title: &str,
    req_id: &str,
    child_labels: &[&str],
) -> (String, String) {
    let description = format!("## Success Criteria\n\n- [hard] {req_id}: it works\n");
    let c = create_issue(temp, container_title, &description, &["type:epic"]);

    let mut kid_labels = vec!["type:task"];
    kid_labels.extend_from_slice(child_labels);
    let k = create_issue(temp, "Impl child", "", &kid_labels);

    let b = create_issue(
        temp,
        &format!("Breakdown: {container_title}"),
        "",
        &["type:breakdown", &format!("brackets:{}", &c[..8])],
    );

    // Attach the coverage-preview gate to B (PENDING — as the bracket-builder
    // leaves it).
    jit(temp)
        .args(["gate", "add", &b, "coverage-preview"])
        .assert()
        .success();
    // Spine: source child depends on B; C depends on the sink child. This puts B
    // inside C's dependency closure, where the scoped coverage rule evaluates it.
    jit(temp).args(["dep", "add", &k, &b]).assert().success();
    jit(temp).args(["dep", "add", &c, &k]).assert().success();

    (c, b)
}

/// Assert a persisted gate-run result exists for `B` reflecting `status`.
fn assert_gate_run(temp: &TempDir, breakdown_id: &str, expected_status: &str) {
    // A gate-run result file is persisted on disk.
    let runs_dir = temp.path().join(".jit").join("gate-runs");
    assert!(
        runs_dir.exists(),
        ".jit/gate-runs/ must exist after a real gate run"
    );
    let any_run = std::fs::read_dir(&runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .next()
        .is_some();
    assert!(any_run, "a GateRunResult must be persisted for the run");

    // `jit gate status` reflects the recorded run (status + exit code).
    let json = jit_json(
        temp,
        &["gate", "status", breakdown_id, "coverage-preview", "--json"],
    );
    assert_eq!(
        json["status"].as_str(),
        Some(expected_status),
        "recorded gate-run status mismatch: {json}"
    );
    assert!(
        json["run_id"].is_string(),
        "persisted run must carry a run_id: {json}"
    );
}

#[test]
fn test_attached_coverage_gate_runs_and_passes_when_hard_criterion_covered() {
    let temp = setup_bracket_repo();
    // Child credits REQ-01 via satisfies:REQ-01 → coverage complete.
    let (_c, b) = build_bracket(&temp, "Auth epic", "REQ-01", &["satisfies:REQ-01"]);

    // Run the ATTACHED gate via the standard runner. Covered → exit 0.
    jit(&temp)
        .args(["gate", "evaluate", &b, "coverage-preview"])
        .assert()
        .success();

    assert_gate_run(&temp, &b, "passed");

    // The breakdown node's gate status reflects the real run.
    let issue = jit_json(&temp, &["issue", "show", &b, "--json"]);
    let gate = issue["gates"]
        .as_array()
        .and_then(|gs| gs.iter().find(|g| g["key"] == "coverage-preview"))
        .expect("coverage-preview in gates array");
    assert_eq!(
        gate["status"].as_str(),
        Some("passed"),
        "covered run must record B's coverage-preview gate Passed: {issue}"
    );
}

#[test]
fn test_attached_coverage_gate_runs_and_fails_when_hard_criterion_uncovered() {
    let temp = setup_bracket_repo();
    // Child carries NO satisfies label → REQ-77 left uncovered.
    let (_c, b) = build_bracket(&temp, "Pay epic", "REQ-77", &[]);

    // Run the ATTACHED gate via the standard runner. Uncovered → exit 4.
    jit(&temp)
        .args(["gate", "evaluate", &b, "coverage-preview"])
        .assert()
        .failure()
        .code(4);

    assert_gate_run(&temp, &b, "failed");

    // The persisted run's stdout names the uncovered criterion.
    let json = jit_json(&temp, &["gate", "status", &b, "coverage-preview", "--json"]);
    assert!(
        json["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("REQ-77"),
        "the uncovered criterion REQ-77 must appear in the gate-run output: {json}"
    );

    // The breakdown node's gate status reflects the real failing run.
    let issue = jit_json(&temp, &["issue", "show", &b, "--json"]);
    let gate = issue["gates"]
        .as_array()
        .and_then(|gs| gs.iter().find(|g| g["key"] == "coverage-preview"))
        .expect("coverage-preview in gates array");
    assert_eq!(
        gate["status"].as_str(),
        Some("failed"),
        "uncovered run must record B's coverage-preview gate Failed: {issue}"
    );
}

#[test]
fn test_selected_rule_ignores_historical_rejected_bracket_target() {
    let temp = setup_bracket_repo();
    let (_selected_container, selected_breakdown) =
        build_bracket(&temp, "Current epic", "REQ-01", &["satisfies:REQ-01"]);
    let (historical_container, _historical_breakdown) =
        build_bracket(&temp, "Historical epic", "REQ-99", &[]);

    // The historical target is rejected and its breakdown is unrelated to the
    // selected gate application. Both issues remain in the repository image so
    // the checker must prove subject isolation rather than relying on omission.
    jit(&temp)
        .args([
            "issue",
            "update",
            &historical_container,
            "--state",
            "rejected",
        ])
        .assert()
        .success();

    jit(&temp)
        .args(["gate", "evaluate", &selected_breakdown, "coverage-preview"])
        .assert()
        .success();

    assert_gate_run(&temp, &selected_breakdown, "passed");
    let historical = jit_json(&temp, &["issue", "show", &historical_container, "--json"]);
    assert_eq!(historical["state"].as_str(), Some("rejected"));
}
