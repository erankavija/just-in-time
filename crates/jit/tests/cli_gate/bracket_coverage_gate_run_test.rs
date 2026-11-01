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
//! `brackets:<C-short-id>` pointer) is inside `C`'s dependency closure — exactly the shape of
//! a drafted bracket. The configured rule resolves the selected container from
//! the rule's `container-from-label` setting while keeping `B` as the sole firing issue.

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use jit::test_utils::declaration_dir;

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

/// Create an issue, returning its full id.
fn create_issue(cli: &JitCli, title: &str, description: &str, labels: &[&str]) -> String {
    let labels = labels.iter().flat_map(|label| ["--label", label]);
    cli.create_issue(
        ["--title", title, "--description", description]
            .into_iter()
            .chain(labels),
    )
    .unwrap()
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

[gates.checker]
type = "rule_validation"
rule = "coverage-preview"
"#;

/// Declarations of the bracket config, the `plan` template, the coverage rule,
/// and a `coverage-preview` auto gate using the portable native checker.
const BRACKET_DECLARATIONS: &[(&str, &str)] = &[
    (".jit/config.toml", BRACKET_CONFIG_TOML),
    (".jit/templates.toml", BRACKET_TEMPLATE_TOML),
    (".jit/rules.toml", COVERAGE_RULES_TOML),
    (".jit/gates.toml", COVERAGE_GATE_TOML),
];

/// Build a bracket `C → child → B` by hand (the shape of a drafted bracket)
/// with the coverage-preview gate attached to `B`. When `covered`, the child credits the criterion with `satisfies:<C-short-id>/<req_id>`. Returns
/// `(container_id, breakdown_id)`.
fn build_bracket(
    cli: &JitCli,
    container_title: &str,
    req_id: &str,
    covered: bool,
) -> (String, String) {
    let description = format!("## Success Criteria\n\n- [hard] {req_id}: it works\n");
    let c = create_issue(cli, container_title, &description, &["type:epic"]);

    let credit = format!("satisfies:{}/{req_id}", &c[..8]);
    let mut kid_labels = vec!["type:task"];
    if covered {
        kid_labels.push(&credit);
    }
    let k = create_issue(cli, "Impl child", "", &kid_labels);

    let b = create_issue(
        cli,
        &format!("Breakdown: {container_title}"),
        "",
        &["type:breakdown", &format!("brackets:{}", &c[..8])],
    );

    // Attach the coverage-preview gate to B (PENDING — as the bracket-builder
    // leaves it).
    cli.run(["gate", "add", &b, "coverage-preview"]).unwrap();
    // Spine: source child depends on B; C depends on the sink child. This puts B
    // inside C's dependency closure, where the scoped coverage rule evaluates it.
    cli.run(["dep", "add", &k, &b]).unwrap();
    cli.run(["dep", "add", &c, &k]).unwrap();

    (c, b)
}

/// Assert a persisted gate-run result exists for `B` reflecting `status`.
fn assert_gate_run(cli: &JitCli, breakdown_id: &str, expected_status: &str) {
    // A gate-run result file is persisted on disk.
    let runs_dir = cli.root().join(".jit").join("gate-runs");
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
    let json = cli
        .run_json(["gate", "status", breakdown_id, "coverage-preview"])
        .unwrap();
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
    let cli = JitCli::with_repository(declaration_dir(BRACKET_DECLARATIONS).unwrap());
    cli.run(["init"]).unwrap();
    // Child credits the container's REQ-01 → coverage complete.
    let (_c, b) = build_bracket(&cli, "Auth epic", "REQ-01", true);

    // Run the ATTACHED gate via the standard runner. Covered → exit 0.
    cli.command(["gate", "evaluate", &b, "coverage-preview"])
        .assert()
        .success();

    assert_gate_run(&cli, &b, "passed");

    // The breakdown node's gate status reflects the real run.
    let issue = cli.run_json(["issue", "show", &b]).unwrap();
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
    let cli = JitCli::with_repository(declaration_dir(BRACKET_DECLARATIONS).unwrap());
    cli.run(["init"]).unwrap();
    // Child carries NO satisfies label → REQ-77 left uncovered.
    let (_c, b) = build_bracket(&cli, "Pay epic", "REQ-77", false);

    // Run the ATTACHED gate via the standard runner. Uncovered → failure.
    cli.command(["gate", "evaluate", &b, "coverage-preview"])
        .assert()
        .failure();

    assert_gate_run(&cli, &b, "failed");

    // The persisted run's stdout names the uncovered criterion.
    let json = cli
        .run_json(["gate", "status", &b, "coverage-preview"])
        .unwrap();
    assert!(
        json["stdout"]
            .as_str()
            .unwrap_or_default()
            .contains("REQ-77"),
        "the uncovered criterion REQ-77 must appear in the gate-run output: {json}"
    );

    // The breakdown node's gate status reflects the real failing run.
    let issue = cli.run_json(["issue", "show", &b]).unwrap();
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
    let cli = JitCli::with_repository(declaration_dir(BRACKET_DECLARATIONS).unwrap());
    cli.run(["init"]).unwrap();
    let (_selected_container, selected_breakdown) =
        build_bracket(&cli, "Current epic", "REQ-01", true);
    let (historical_container, _historical_breakdown) =
        build_bracket(&cli, "Historical epic", "REQ-99", false);

    // The historical target is rejected and its breakdown is unrelated to the
    // selected gate application. Both issues remain in the repository image so
    // the checker must prove subject isolation rather than relying on omission.
    cli.command(["issue", "reject", &historical_container])
        .assert()
        .success();

    cli.command(["gate", "evaluate", &selected_breakdown, "coverage-preview"])
        .assert()
        .success();

    assert_gate_run(&cli, &selected_breakdown, "passed");
    let historical = cli
        .run_json(["issue", "show", &historical_container])
        .unwrap();
    assert_eq!(historical["state"].as_str(), Some("rejected"));
}
