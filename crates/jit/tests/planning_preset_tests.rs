//! Integration tests for the planning-bracket gate presets (T6).
//!
//! Exercises `apply_gate_preset` end-to-end via the in-process harness:
//! applying `plan-review` attaches the agent gate to a planning node, and
//! applying `coverage-preview` attaches the deterministic scoped-validate gate
//! to a breakdown node.

mod harness;

use harness::TestHarness;
use jit::domain::{GateChecker, GateMode};
use jit::storage::IssueStore;

#[test]
fn test_apply_plan_review_attaches_agent_gate_to_planning_node() {
    let h = TestHarness::new();
    // The planning node P (type label is illustrative; the preset is type-agnostic).
    let planning = h.create_issue("Plan the auth epic");
    h.executor
        .add_label(&planning, "type:planning")
        .expect("label P");

    let (result, _warnings) = h
        .executor
        .apply_gate_preset(&planning, "plan-review", None, false, false, &[])
        .expect("apply plan-review preset");

    assert!(
        result.added.contains(&"plan-review".to_string()),
        "plan-review gate attached to the planning node, got {:?}",
        result.added
    );

    // The issue now requires the plan-review gate, and the registered gate is
    // the agent (command-backed) review.
    let issue = h.get_issue(&planning);
    assert!(issue.gates_required.contains(&"plan-review".to_string()));

    let registry = h.storage.load_gate_registry().expect("load registry");
    let gate = registry.gates.get("plan-review").expect("gate registered");
    assert_eq!(gate.mode, GateMode::Auto);
    match gate.checker.as_ref().expect("agent gate has a checker") {
        GateChecker::Exec { command, .. } => assert_eq!(command, "./scripts/ai-review.sh"),
    }
}

#[test]
fn test_apply_coverage_preview_attaches_scoped_validate_gate_to_breakdown_node() {
    let h = TestHarness::new();
    // The breakdown node B, carrying a brackets: pointer to its container.
    let breakdown = h.create_issue("Breakdown of the auth epic");
    h.executor
        .add_label(&breakdown, "type:breakdown")
        .expect("label B");
    h.executor
        .add_label(&breakdown, "brackets:abc12345")
        .expect("brackets label");

    let (result, _warnings) = h
        .executor
        .apply_gate_preset(&breakdown, "coverage-preview", None, false, false, &[])
        .expect("apply coverage-preview preset");

    assert!(
        result.added.contains(&"coverage-preview".to_string()),
        "coverage-preview gate attached to the breakdown node, got {:?}",
        result.added
    );

    let issue = h.get_issue(&breakdown);
    assert!(issue
        .gates_required
        .contains(&"coverage-preview".to_string()));

    // The registered gate's checker runs the scoped-validate wrapper, which
    // resolves the container from the brackets: label and runs
    // `jit validate --scope <C>`.
    let registry = h.storage.load_gate_registry().expect("load registry");
    let gate = registry
        .gates
        .get("coverage-preview")
        .expect("gate registered");
    assert_eq!(gate.mode, GateMode::Auto);
    match gate.checker.as_ref().expect("coverage gate has a checker") {
        GateChecker::Exec { command, .. } => {
            assert_eq!(command, "./scripts/coverage-preview.sh")
        }
    }
}
