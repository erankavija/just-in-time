//! Tests for the bracket-aware breakdown path (T10, design doc
//! `dev/active/planning-bracket-design.md`).
//!
//! Given a breakable container `C` already scaffolded to its planning node `P`
//! (`C → P`), the bracket breakdown step creates the breakdown node `B`
//! (`type:breakdown`, `brackets:<C-id>` label, coverage-preview gate), wires
//! `B → P`, drafts the impl children in Backlog, and splices the spine:
//!
//! ```text
//! C ──dep→ {impl subgraph} ──dep→ B ──dep→ P
//! ```
//!
//! Source children (no intra-subgraph predecessor) depend on `B`; sink children
//! (no intra-subgraph successor) are depended-on by `C`. Transitive reduction
//! drops the scaffold's direct `C → P` edge once the spine connects them.
//!
//! These exercise `CommandExecutor` in-process via `TestHarness`
//! (InMemoryStorage); the `[planning]` vocabulary is injected explicitly via a
//! `PlanningConfig` passed to the `*_with_config` core method.

mod harness;

use harness::TestHarness;
use jit::commands::{BracketChild, CommandExecutor};
use jit::config::PlanningConfig;
use jit::domain::{Issue, Priority, State};
use jit::labels::parse_label;
use jit::storage::{InMemoryStorage, IssueStore};

/// SDD-mirroring `[planning]` vocabulary (epic containers, planning/breakdown
/// bracket types, inline plan doc, the two gate presets).
fn sdd_planning_config() -> PlanningConfig {
    PlanningConfig {
        breakable_types: vec!["epic".to_string()],
        planning_type: "planning".to_string(),
        breakdown_type: "breakdown".to_string(),
        plan_doc_location: "inline".to_string(),
        plan_gate_preset: "plan-review".to_string(),
        coverage_gate_preset: "coverage-preview".to_string(),
    }
}

/// The `type:*` value of an issue, if any.
fn type_of(issue: &Issue) -> Option<String> {
    issue.labels.iter().find_map(|l| {
        parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
}

/// Mark the planning node `P`'s plan-review gate PASSED, so a breakdown that
/// requires an approved plan is allowed to proceed. Mirrors the established
/// "pass this gate" pattern used elsewhere in the suite.
fn approve_plan(h: &TestHarness, cfg: &PlanningConfig, planning_id: &str) {
    let mut p = h.get_issue(planning_id);
    let gate = p
        .gates_status
        .get_mut(&cfg.plan_gate_preset)
        .expect("planning node carries the plan gate");
    gate.status = jit::domain::GateStatus::Passed;
    h.storage.save_issue(p).unwrap();
}

/// Scaffold a breakable container `C` bracketed by its planning node `P`
/// (`C → P`) with an APPROVED plan (P's plan-review gate passed), returning
/// `(container_id, planning_id)`.
fn scaffold_container(h: &TestHarness, cfg: &PlanningConfig, title: &str) -> (String, String) {
    let (id, _) = h
        .executor
        .create_issue(
            title.to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();
    let (result, _) = h
        .executor
        .plan_existing_with_config(cfg, &id, false)
        .unwrap();
    approve_plan(h, cfg, &result.planning_id);
    (result.container_id, result.planning_id)
}

/// A simple child with no intra-subgraph dependencies and no labels.
fn child(title: &str) -> BracketChild {
    BracketChild {
        title: title.to_string(),
        description: String::new(),
        priority: Priority::Normal,
        gates: vec![],
        labels: vec![],
        deps: vec![],
    }
}

// ===================== B creation + B → P =====================

#[test]
fn test_bracket_breakdown_creates_breakdown_node_typed_from_config() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login")])
        .unwrap();

    let b = h.get_issue(&result.breakdown_id);
    assert_eq!(
        type_of(&b).as_deref(),
        Some("breakdown"),
        "B must be typed from config.breakdown_type"
    );
}

#[test]
fn test_bracket_breakdown_node_carries_brackets_label_for_container() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login")])
        .unwrap();

    let b = h.get_issue(&result.breakdown_id);
    assert!(
        b.labels.contains(&format!("brackets:{c}")),
        "B must carry brackets:<C-id> naming its container, got {:?}",
        b.labels
    );
}

#[test]
fn test_bracket_breakdown_applies_coverage_preview_gate_to_breakdown_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login")])
        .unwrap();

    let b = h.get_issue(&result.breakdown_id);
    assert!(
        b.gates_required.contains(&"coverage-preview".to_string()),
        "B must carry the coverage-preview gate, got {:?}",
        b.gates_required
    );
    // The gate is registered by the preset-apply path.
    let registry = h.storage.load_gate_registry().unwrap();
    assert!(registry.gates.contains_key("coverage-preview"));
}

#[test]
fn test_bracket_breakdown_wires_breakdown_node_to_plan() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login")])
        .unwrap();

    let b = h.get_issue(&result.breakdown_id);
    assert!(
        b.dependencies.contains(&p),
        "B must depend on P (breakdown after plan), got {:?}",
        b.dependencies
    );
}

// ===================== children drafted in Backlog =====================

#[test]
fn test_bracket_breakdown_drafts_children_in_backlog() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login"), child("Build logout")])
        .unwrap();

    for id in &result.child_ids {
        let issue = h.get_issue(id);
        assert_eq!(
            issue.state,
            State::Backlog,
            "child {id} must be drafted in Backlog (it depends on B, not done), got {:?}",
            issue.state
        );
    }
}

#[test]
fn test_bracket_breakdown_children_carry_breakdown_type_and_membership() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    // Container carries a membership label children should inherit.
    let (c, _) = h
        .executor
        .create_issue(
            "Auth epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
            None,
            false,
        )
        .unwrap();
    let (plan, _) = h
        .executor
        .plan_existing_with_config(&cfg, &c, false)
        .unwrap();
    approve_plan(&h, &cfg, &plan.planning_id);

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login")])
        .unwrap();

    let kid = h.get_issue(&result.child_ids[0]);
    assert!(
        kid.labels.contains(&"epic:auth".to_string()),
        "child must inherit the container's membership label, got {:?}",
        kid.labels
    );
    // Children are impl issues, NOT breakdown/planning-typed.
    assert_ne!(type_of(&kid).as_deref(), Some("breakdown"));
    assert_ne!(type_of(&kid).as_deref(), Some("planning"));
}

// ===================== spine: sources → B, C → sinks =====================

#[test]
fn test_bracket_breakdown_source_children_depend_on_breakdown_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    // Chain: child0 (source) -> child1 -> child2 (sink). Only child0 is a source.
    let mut c1 = child("middle");
    c1.deps = vec![0];
    let mut c2 = child("sink");
    c2.deps = vec![1];
    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("source"), c1, c2])
        .unwrap();

    let source = h.get_issue(&result.child_ids[0]);
    assert!(
        source.dependencies.contains(&result.breakdown_id),
        "source child must depend on B, got {:?}",
        source.dependencies
    );
    // Non-source children do NOT directly depend on B (B reaches them
    // transitively through the source).
    let middle = h.get_issue(&result.child_ids[1]);
    assert!(
        !middle.dependencies.contains(&result.breakdown_id),
        "non-source child must NOT directly depend on B (reduced form), got {:?}",
        middle.dependencies
    );
}

#[test]
fn test_bracket_breakdown_container_depends_on_sinks_only() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    // child0 -> child1 (sink). child0 is a source, NOT a sink.
    let mut c1 = child("sink");
    c1.deps = vec![0];
    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("source"), c1])
        .unwrap();

    let container = h.get_issue(&c);
    let sink = &result.child_ids[1];
    let source = &result.child_ids[0];
    assert!(
        container.dependencies.contains(sink),
        "C must depend on the sink child, got {:?}",
        container.dependencies
    );
    assert!(
        !container.dependencies.contains(source),
        "C must NOT depend on the non-sink (source) child (reduced form), got {:?}",
        container.dependencies
    );
}

#[test]
fn test_bracket_breakdown_internal_edges_preserved() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let mut c1 = child("middle");
    c1.deps = vec![0];
    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("source"), c1])
        .unwrap();

    let middle = h.get_issue(&result.child_ids[1]);
    assert!(
        middle.dependencies.contains(&result.child_ids[0]),
        "internal edge child1 -> child0 must be preserved, got {:?}",
        middle.dependencies
    );
}

// ===================== reduced form: C → P dropped =====================

#[test]
fn test_bracket_breakdown_removes_direct_container_to_plan_edge() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, p) = scaffold_container(&h, &cfg, "Auth epic");

    // Pre-condition: scaffold left a direct C -> P edge.
    assert!(h.get_issue(&c).dependencies.contains(&p));

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("only")])
        .unwrap();

    // After breakdown the spine C -> child -> B -> P makes C -> P redundant;
    // transitive reduction must have dropped it.
    let container = h.get_issue(&c);
    assert!(
        !container.dependencies.contains(&p),
        "the direct C -> P edge must be dropped by reduction once the spine \
         connects them, got {:?}",
        container.dependencies
    );
    // C reaches P transitively: C -> child -> B -> P.
    let only_child = &result.child_ids[0];
    assert!(container.dependencies.contains(only_child));
    let b = h.get_issue(&result.breakdown_id);
    assert!(b.dependencies.contains(&p));
}

// ============== NO parent-centric wiring (the anti-test) ==============

#[test]
fn test_bracket_breakdown_does_not_use_parent_centric_wiring() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    // Three independent children -> all are both sources AND sinks.
    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("a"), child("b"), child("c")])
        .unwrap();

    let container = h.get_issue(&c);
    // Parent-centric wiring would make C depend on EVERY child directly AND
    // copy C's deps onto every child. Here C depends on every child only
    // because they are ALL sinks (independent). The distinguishing assertion:
    // no child copied C's pre-existing dependency (P). Children depend on B
    // (sources), never on P.
    for id in &result.child_ids {
        let kid = h.get_issue(id);
        assert!(
            !kid.dependencies.contains(&_p),
            "child {id} must NOT copy C's dependency on P (parent-centric \
             wiring is forbidden), got {:?}",
            kid.dependencies
        );
        assert!(
            kid.dependencies.contains(&result.breakdown_id),
            "every source child must depend on B, got {:?}",
            kid.dependencies
        );
    }
    // C depends on B's downstream sinks, never directly on B or P.
    assert!(!container.dependencies.contains(&result.breakdown_id));
    assert!(!container.dependencies.contains(&_p));
}

// ===================== validation / events =====================

#[test]
fn test_bracket_breakdown_logs_breakdown_node_creation_event() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("only")])
        .unwrap();

    let events = h.storage.read_events().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.get_type() == "issue_created" && e.get_issue_id() == result.breakdown_id),
        "creating B must log an issue_created event"
    );
}

#[test]
fn test_bracket_breakdown_rejects_non_breakable_container() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (task, _) = h
        .executor
        .create_issue(
            "Just a task".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            false,
        )
        .unwrap();

    let err = h
        .executor
        .bracket_breakdown_with_config(&cfg, &task, vec![child("x")])
        .unwrap_err();
    assert!(
        err.to_string().contains("breakable") || err.to_string().contains("task"),
        "must reject non-breakable container, got: {err}"
    );
}

#[test]
fn test_bracket_breakdown_requires_planning_node() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    // Breakable container that was NEVER scaffolded (no P).
    let (c, _) = h
        .executor
        .create_issue(
            "Unplanned epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();

    let err = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("x")])
        .unwrap_err();
    assert!(
        err.to_string().contains("planning") || err.to_string().contains("plan"),
        "must require a scaffolded planning node, got: {err}"
    );
}

#[test]
fn test_bracket_breakdown_rejects_empty_children() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let err = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![])
        .unwrap_err();
    assert!(
        err.to_string().contains("child") || err.to_string().contains("empty"),
        "must reject an empty child set, got: {err}"
    );
}

// ============= config-reading public wrapper (reads [planning]) =============

const PLANNING_CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }

[planning]
breakable_types = ["epic"]
planning_type = "planning"
breakdown_type = "breakdown"
plan_doc_location = "inline"
plan_gate_preset = "plan-review"
coverage_gate_preset = "coverage-preview"
"#;

fn executor_with_planning_config() -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), PLANNING_CONFIG_TOML).unwrap();
    CommandExecutor::new(storage)
}

#[test]
fn test_bracket_breakdown_reads_planning_config_from_disk() {
    let executor = executor_with_planning_config();
    let (c, _) = executor
        .create_issue(
            "Auth epic".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();
    let (plan, _) = executor.plan_existing(&c, false).unwrap();

    // Approve P's plan-review gate: breakdown consumes an approved plan.
    let mut p = executor.storage().load_issue(&plan.planning_id).unwrap();
    p.gates_status.get_mut("plan-review").unwrap().status = jit::domain::GateStatus::Passed;
    executor.storage().save_issue(p).unwrap();

    // The public wrapper reads [planning] from config.toml; no config injected.
    let result = executor.bracket_breakdown(&c, vec![child("only")]).unwrap();
    let b = executor.storage().load_issue(&result.breakdown_id).unwrap();
    assert_eq!(type_of(&b).as_deref(), Some("breakdown"));
    assert!(b.labels.contains(&format!("brackets:{c}")));
}

// ===================== Finding 1: approved plan required =====================

#[test]
fn test_bracket_breakdown_rejected_when_plan_gate_not_passed() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    // Scaffold a container WITHOUT approving its plan (do not use the helper,
    // which auto-approves).
    let (c, _) = h
        .executor
        .create_issue(
            "Auth epic".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            false,
        )
        .unwrap();
    let (plan, _) = h
        .executor
        .plan_existing_with_config(&cfg, &c, false)
        .unwrap();
    // P's plan-review gate is PENDING (never passed).

    let issues_before = h.storage.list_issues().unwrap().len();

    let err = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build login")])
        .unwrap_err();
    assert!(
        err.to_string().contains("approved plan") && err.to_string().contains("plan-review"),
        "must reject breakdown when the plan gate is not passed, got: {err}"
    );

    // Nothing was created: no B, no children. (Only C and P exist.)
    let issues_after = h.storage.list_issues().unwrap();
    assert_eq!(
        issues_after.len(),
        issues_before,
        "a rejected breakdown must create NOTHING (no B/children)"
    );
    assert!(
        !issues_after
            .iter()
            .any(|i| type_of(i).as_deref() == Some("breakdown")),
        "no breakdown node may exist after a rejected breakdown"
    );
    // P's gate is untouched.
    let p = h.get_issue(&plan.planning_id);
    assert_eq!(
        p.gates_status.get("plan-review").map(|g| g.status),
        Some(jit::domain::GateStatus::Pending),
    );
}

// ===================== Finding 3: cycle detection ===========================

#[test]
fn test_bracket_breakdown_rejects_cyclic_child_plan() {
    let h = TestHarness::new();
    let cfg = sdd_planning_config();
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    // A cyclic child plan: 0 -> 1 -> 2 -> 0.
    let mut a = child("a");
    a.deps = vec![2];
    let mut b = child("b");
    b.deps = vec![0];
    let mut d = child("c");
    d.deps = vec![1];

    let issues_before = h.storage.list_issues().unwrap().len();

    let err = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![a, b, d])
        .unwrap_err();
    assert!(
        err.to_string().contains("cycle"),
        "must reject a cyclic child plan, got: {err}"
    );

    // No B/children created: the cycle is caught BEFORE any mutation.
    let issues_after = h.storage.list_issues().unwrap();
    assert_eq!(
        issues_after.len(),
        issues_before,
        "a cyclic child plan must create NOTHING (pre-mutation rejection)"
    );
    assert!(
        !issues_after
            .iter()
            .any(|i| type_of(i).as_deref() == Some("breakdown")),
        "no breakdown node may exist after a rejected cyclic plan"
    );
}

// ===================== Finding 2: coverage-preview run =======================

/// The SDD-style preview coverage rule keyed on `type:breakdown`, resolving its
/// container via `brackets:` and checking `[hard]` REQ coverage via `satisfies:`.
const COVERAGE_RULES_TOML: &str = r#"
[[rules]]
name = "coverage-preview"
when = { type = "breakdown" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", child-link = "dependencies", child-type-exclude = ["planning", "breakdown"], container-from-label = "brackets" } }
"#;

/// A harness whose storage root carries the `[planning]` config and the preview
/// coverage rule, so `validate_scope` (run in-process by the breakdown helper)
/// evaluates `[hard]` criteria coverage.
fn coverage_harness() -> TestHarness {
    let h = TestHarness::new();
    std::fs::create_dir_all(h.storage.root()).unwrap();
    std::fs::write(h.storage.root().join("config.toml"), PLANNING_CONFIG_TOML).unwrap();
    std::fs::write(h.storage.root().join("rules.toml"), COVERAGE_RULES_TOML).unwrap();
    h
}

/// A child that credits the given `[hard]` criterion id via `satisfies:<id>`.
fn child_satisfying(title: &str, req_id: &str) -> BracketChild {
    BracketChild {
        title: title.to_string(),
        description: String::new(),
        priority: Priority::Normal,
        gates: vec![],
        labels: vec![format!("satisfies:{req_id}")],
        deps: vec![],
    }
}

#[test]
fn test_bracket_breakdown_coverage_passes_when_hard_criterion_covered() {
    let h = coverage_harness();
    let cfg = sdd_planning_config();
    // Container declares [hard] REQ-01 (scaffold_container's body).
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child_satisfying("Build login", "REQ-01")])
        .unwrap();

    assert!(
        result.coverage_passed,
        "a covered [hard] criterion must yield coverage_passed=true; report: {:?}",
        result.coverage_report
    );
    assert!(
        !result.coverage_report.has_errors(),
        "covered plan must have no error-severity coverage findings, got {:?}",
        result.coverage_report
    );

    // The verdict is RECORDED on B's coverage gate (not just returned): B's
    // coverage-preview gate status is Passed.
    let b = h.get_issue(&result.breakdown_id);
    assert_eq!(
        b.gates_status.get("coverage-preview").map(|g| g.status),
        Some(jit::domain::GateStatus::Passed),
        "covered plan must record B's coverage-preview gate as Passed, got {:?}",
        b.gates_status
    );
    // ...and a gate_passed event exists for B against that gate.
    let events = h.storage.read_events().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.get_type() == "gate_passed" && e.get_issue_id() == result.breakdown_id),
        "a covered plan must log a gate_passed event for B"
    );
}

#[test]
fn test_bracket_breakdown_coverage_fails_when_hard_criterion_uncovered() {
    let h = coverage_harness();
    let cfg = sdd_planning_config();
    // Container declares [hard] REQ-01, but the child satisfies nothing.
    let (c, _p) = scaffold_container(&h, &cfg, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_config(&cfg, &c, vec![child("Build something unrelated")])
        .unwrap();

    assert!(
        !result.coverage_passed,
        "an uncovered [hard] criterion must yield coverage_passed=false"
    );
    assert!(
        result.coverage_report.has_errors(),
        "uncovered plan must carry an error-severity finding, got {:?}",
        result.coverage_report
    );
    // The uncovered criterion is named in the findings.
    assert!(
        result
            .coverage_report
            .findings
            .iter()
            .any(|f| f.message.contains("REQ-01")),
        "the uncovered criterion REQ-01 must appear in the coverage findings, got {:?}",
        result.coverage_report.findings
    );

    // B and the child ARE still created — the preview REPORTS the gap, it does
    // not block creation.
    assert!(
        h.storage.load_issue(&result.breakdown_id).is_ok(),
        "B must still be created (the preview reports, not blocks)"
    );
    assert_eq!(result.child_ids.len(), 1);

    // The failing verdict is RECORDED on B's coverage gate: status is Failed.
    let b = h.get_issue(&result.breakdown_id);
    assert_eq!(
        b.gates_status.get("coverage-preview").map(|g| g.status),
        Some(jit::domain::GateStatus::Failed),
        "uncovered plan must record B's coverage-preview gate as Failed, got {:?}",
        b.gates_status
    );
    // ...and a gate_failed event exists for B against that gate.
    let events = h.storage.read_events().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.get_type() == "gate_failed" && e.get_issue_id() == result.breakdown_id),
        "an uncovered plan must log a gate_failed event for B"
    );
}
