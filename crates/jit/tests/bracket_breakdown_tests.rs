//! Tests for the bracket-aware breakdown path (T10, design doc
//! `dev/active/planning-bracket-design.md`).
//!
//! A breakable container `C` is first scaffolded by the `jit apply plan` engine
//! into the bracket `C → B → P` (planning node `P` carries the plan-review gate;
//! breakdown node `B` carries `type:breakdown`, the `brackets:<C-short-id>` label,
//! and the coverage-preview + breakdown-review gates; `B → P` and `C → B` are
//! wired). After `P`'s plan-review gate is PASSED, the bracket breakdown step
//! **consumes** that pre-created `B` (it does NOT create one), finds `P` through
//! `B`, drafts the impl children in Backlog, and splices the spine:
//!
//! ```text
//! C ──dep→ {impl subgraph} ──dep→ B ──dep→ P
//! ```
//!
//! Source children (no intra-subgraph predecessor) depend on `B`; sink children
//! (no intra-subgraph successor) are depended-on by `C`. Transitive reduction
//! drops the scaffold's direct `C → B` edge once the spine connects them.
//!
//! These exercise `CommandExecutor` in-process via `TestHarness`
//! (InMemoryStorage). The template is authored in-test via
//! `TemplateRegistry::from_toml_str` and passed explicitly to the
//! `*_with_template` core methods (mirroring the apply-engine tests), so no
//! on-disk `templates.toml` is needed.

mod harness;

use harness::TestHarness;
use jit::commands::{BracketChild, CommandExecutor};
use jit::domain::{Issue, Priority, State};
use jit::labels::parse_label;
use jit::storage::{InMemoryStorage, IssueStore};
use jit::templates::{GraphTemplate, TemplateRegistry};
use std::collections::BTreeMap;

const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

/// The repo's `plan`-shaped template: planning node `P` (plan-review gate),
/// breakdown node `B` (`brackets:<short-id>`, coverage-preview + breakdown-review
/// gates, `B → P`), `C → B` anchor edge, and the `move-upstream-to-role`
/// transform onto `P`. Mirrors `.jit/templates.toml` and the apply-engine tests.
fn plan_template() -> GraphTemplate {
    let toml = r#"
[[template]]
name        = "plan"
applies_to  = ["epic"]

  [[template.anchors]]
  name = "container"

  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["plan-review"]
  doc         = "dev/active/{container.id}-plan.md"
  description = "Planning node for {container.title}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
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
    TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("plan")
        .unwrap()
        .clone()
}

/// The `type:*` value of an issue, if any.
fn type_of(issue: &Issue) -> Option<String> {
    issue.labels.iter().find_map(|l| {
        parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
}

fn container_binding(id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("container".to_string(), id.to_string())])
}

/// Mark the planning node `P`'s plan-review gate PASSED, so a breakdown that
/// requires an approved plan is allowed to proceed.
fn approve_plan(h: &TestHarness, planning_id: &str) {
    let mut p = h.get_issue(planning_id);
    let gate = p
        .gates_status
        .get_mut("plan-review")
        .expect("planning node carries the plan gate");
    gate.status = jit::domain::GateStatus::Passed;
    h.storage.save_issue(p).unwrap();
}

/// Scaffold a breakable container `C` via the apply engine into `C → B → P`, with
/// an APPROVED plan (P's plan-review gate passed), returning
/// `(container_id, planning_id)`.
fn scaffold_container(h: &TestHarness, template: &GraphTemplate, title: &str) -> (String, String) {
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
        .apply_template_with(template, &id, &container_binding(&id), false)
        .unwrap();
    let planning_id = result.created_node_ids_by_role["planning"].clone();
    approve_plan(h, &planning_id);
    (id, planning_id)
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

// =============== B is CONSUMED, not re-created; B → P found ===============

#[test]
fn test_bracket_breakdown_consumes_pre_created_breakdown_node() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    // Exactly one breakdown node exists after scaffolding.
    let breakdown_before: Vec<String> = h
        .all_issues()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("breakdown"))
        .map(|i| i.id)
        .collect();
    assert_eq!(breakdown_before.len(), 1, "scaffold creates exactly one B");

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
        .unwrap();

    // Breakdown CONSUMED the pre-created B (same id), did not create a new one.
    assert_eq!(
        result.breakdown_id, breakdown_before[0],
        "breakdown must consume the pre-created B, not create a new one"
    );
    let breakdown_after = h
        .all_issues()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("breakdown"))
        .count();
    assert_eq!(
        breakdown_after, 1,
        "no duplicate breakdown node may be created"
    );
}

#[test]
fn test_bracket_breakdown_node_typed_breakdown() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
        .unwrap();

    let b = h.get_issue(&result.breakdown_id);
    assert_eq!(type_of(&b).as_deref(), Some("breakdown"));
}

#[test]
fn test_bracket_breakdown_node_carries_brackets_short_id_label() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
        .unwrap();

    // The apply engine seeds `brackets:<C-short-id>` (not the full id); breakdown
    // locates B by that label.
    let short_id: String = c.chars().take(8).collect();
    let b = h.get_issue(&result.breakdown_id);
    assert!(
        b.labels.contains(&format!("brackets:{short_id}")),
        "B must carry brackets:<C-short-id> naming its container, got {:?}",
        b.labels
    );
}

#[test]
fn test_bracket_breakdown_finds_plan_through_breakdown_node() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, p) = scaffold_container(&h, &template, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
        .unwrap();

    // P is reported correctly, and B → P is the edge through which it was found.
    assert_eq!(result.planning_id, p, "P must be found through B (B → P)");
    let b = h.get_issue(&result.breakdown_id);
    assert!(
        b.dependencies.contains(&p),
        "B must depend on P (breakdown after plan), got {:?}",
        b.dependencies
    );
}

#[test]
fn test_bracket_breakdown_reports_breakdown_node_gate_presets() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
        .unwrap();

    // The gates were attached by the apply engine; breakdown reports their names.
    assert_eq!(result.coverage_gate_preset, "coverage-preview");
    assert_eq!(result.breakdown_review_gate_preset, "breakdown-review");

    // B still carries both gates, left PENDING (breakdown neither re-attaches nor
    // runs them).
    let b = h.get_issue(&result.breakdown_id);
    assert!(b.gates_required.contains(&"coverage-preview".to_string()));
    assert!(b.gates_required.contains(&"breakdown-review".to_string()));
    assert_eq!(
        b.gates_status.get("breakdown-review").map(|s| s.status),
        Some(jit::domain::GateStatus::Pending),
    );
    assert!(matches!(
        b.gates_status.get("coverage-preview").map(|s| s.status),
        None | Some(jit::domain::GateStatus::Pending)
    ));
}

// ===================== children drafted in Backlog =====================

#[test]
fn test_bracket_breakdown_drafts_children_in_backlog() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_template(
            &template,
            &c,
            vec![child("Build login"), child("Build logout")],
        )
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
fn test_bracket_breakdown_children_carry_membership_not_bracket_types() {
    let h = TestHarness::new();
    let template = plan_template();
    // Container carries a membership label children should inherit.
    let (c, _) = h
        .executor
        .create_issue(
            "Auth epic".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
            None,
            false,
        )
        .unwrap();
    let (apply_result, _) = h
        .executor
        .apply_template_with(&template, &c, &container_binding(&c), false)
        .unwrap();
    approve_plan(&h, &apply_result.created_node_ids_by_role["planning"]);

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
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
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    // Chain: child0 (source) -> child1 -> child2 (sink). Only child0 is a source.
    let mut c1 = child("middle");
    c1.deps = vec![0];
    let mut c2 = child("sink");
    c2.deps = vec![1];
    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("source"), c1, c2])
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
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    // child0 -> child1 (sink). child0 is a source, NOT a sink.
    let mut c1 = child("sink");
    c1.deps = vec![0];
    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("source"), c1])
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
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let mut c1 = child("middle");
    c1.deps = vec![0];
    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("source"), c1])
        .unwrap();

    let middle = h.get_issue(&result.child_ids[1]);
    assert!(
        middle.dependencies.contains(&result.child_ids[0]),
        "internal edge child1 -> child0 must be preserved, got {:?}",
        middle.dependencies
    );
}

// ===================== reduced form: C → B dropped =====================

#[test]
fn test_bracket_breakdown_removes_direct_container_to_breakdown_edge() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, p) = scaffold_container(&h, &template, "Auth epic");

    // Pre-condition: scaffold left a direct C -> B edge (C → B → P).
    let b_before: String = h
        .all_issues()
        .into_iter()
        .find(|i| type_of(i).as_deref() == Some("breakdown"))
        .map(|i| i.id)
        .unwrap();
    assert!(h.get_issue(&c).dependencies.contains(&b_before));

    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("only")])
        .unwrap();

    // After breakdown the spine C -> child -> B makes C -> B redundant;
    // transitive reduction must have dropped the direct edge.
    let container = h.get_issue(&c);
    assert!(
        !container.dependencies.contains(&result.breakdown_id),
        "the direct C -> B edge must be dropped by reduction once the spine \
         connects them, got {:?}",
        container.dependencies
    );
    // C reaches B (and P) transitively: C -> child -> B -> P.
    let only_child = &result.child_ids[0];
    assert!(container.dependencies.contains(only_child));
    let b = h.get_issue(&result.breakdown_id);
    assert!(b.dependencies.contains(&p));
}

// ============== NO parent-centric wiring (the anti-test) ==============

#[test]
fn test_bracket_breakdown_does_not_use_parent_centric_wiring() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    // Three independent children -> all are both sources AND sinks.
    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("a"), child("b"), child("c")])
        .unwrap();

    let container = h.get_issue(&c);
    // Parent-centric wiring would copy C's deps onto every child. The
    // distinguishing assertion: no child copied C's pre-existing dependency.
    // Children depend on B (sources), never on P.
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
fn test_bracket_breakdown_logs_no_breakdown_node_creation_event() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    // Capture the event count before breakdown: B was created during scaffolding,
    // so breakdown must NOT emit a second issue_created for the same B.
    let result = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("only")])
        .unwrap();

    let events = h.storage.read_events().unwrap();
    let b_created = events
        .iter()
        .filter(|e| e.get_type() == "issue_created" && e.get_issue_id() == result.breakdown_id)
        .count();
    assert_eq!(
        b_created, 1,
        "B is created exactly once (by the scaffold), never re-created by breakdown"
    );
}

#[test]
fn test_bracket_breakdown_rejects_non_breakable_container() {
    let h = TestHarness::new();
    let template = plan_template();
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
        .bracket_breakdown_with_template(&template, &task, vec![child("x")])
        .unwrap_err();
    assert!(
        err.to_string().contains("breakable") || err.to_string().contains("task"),
        "must reject non-breakable container, got: {err}"
    );
}

#[test]
fn test_bracket_breakdown_errors_when_bracket_absent_points_at_apply_plan() {
    let h = TestHarness::new();
    let template = plan_template();
    // Breakable container that was NEVER scaffolded (no B, no P).
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

    let issues_before = h.storage.list_issues().unwrap().len();

    let err = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("x")])
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("breakdown bracket") || msg.contains("no 'breakdown'"),
        "must report the missing breakdown bracket, got: {msg}"
    );
    assert!(
        msg.contains("jit apply plan"),
        "must point the user at `jit apply plan`, got: {msg}"
    );

    // Nothing was created.
    assert_eq!(
        h.storage.list_issues().unwrap().len(),
        issues_before,
        "a rejected breakdown must create NOTHING"
    );
}

#[test]
fn test_bracket_breakdown_rejects_empty_children() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let err = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![])
        .unwrap_err();
    assert!(
        err.to_string().contains("child") || err.to_string().contains("empty"),
        "must reject an empty child set, got: {err}"
    );
}

// ============= config-reading public wrapper (reads templates.toml) =============

const TEMPLATES_TOML: &str = r#"
[[template]]
name        = "plan"
applies_to  = ["epic"]

  [[template.anchors]]
  name = "container"

  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["plan-review"]

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
  labels      = ["brackets:{container.short_id}"]
  depends_on  = ["planning"]

  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"

  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#;

const CONFIG_TOML: &str = r#"
[type_hierarchy]
types = { epic = 1, planning = 2, breakdown = 2, task = 3 }
"#;

fn executor_with_templates() -> CommandExecutor<InMemoryStorage> {
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), CONFIG_TOML).unwrap();
    std::fs::write(storage.root().join("templates.toml"), TEMPLATES_TOML).unwrap();
    CommandExecutor::new(storage)
}

#[test]
fn test_bracket_breakdown_reads_template_from_disk() {
    let executor = executor_with_templates();
    let (c, _) = executor
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
    // Scaffold via the public apply command (resolves the template from disk).
    let (apply_result, _) = executor
        .apply_template("plan", &c, &container_binding(&c), false)
        .unwrap();
    let planning_id = apply_result.created_node_ids_by_role["planning"].clone();

    // Approve P's plan-review gate: breakdown consumes an approved plan.
    let mut p = executor.storage().load_issue(&planning_id).unwrap();
    p.gates_status.get_mut("plan-review").unwrap().status = jit::domain::GateStatus::Passed;
    executor.storage().save_issue(p).unwrap();

    // The public wrapper resolves the template from templates.toml; none injected.
    let result = executor.bracket_breakdown(&c, vec![child("only")]).unwrap();
    let b = executor.storage().load_issue(&result.breakdown_id).unwrap();
    assert_eq!(type_of(&b).as_deref(), Some("breakdown"));
    let short_id: String = c.chars().take(8).collect();
    assert!(b.labels.contains(&format!("brackets:{short_id}")));
}

// ===================== Finding 1: approved plan required =====================

#[test]
fn test_bracket_breakdown_rejected_when_plan_gate_not_passed() {
    let h = TestHarness::new();
    let template = plan_template();
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
    let (apply_result, _) = h
        .executor
        .apply_template_with(&template, &c, &container_binding(&c), false)
        .unwrap();
    let planning_id = apply_result.created_node_ids_by_role["planning"].clone();
    // P's plan-review gate is PENDING (never passed).

    let issues_before = h.storage.list_issues().unwrap().len();

    let err = h
        .executor
        .bracket_breakdown_with_template(&template, &c, vec![child("Build login")])
        .unwrap_err();
    assert!(
        err.to_string().contains("approved plan") && err.to_string().contains("plan-review"),
        "must reject breakdown when the plan gate is not passed, got: {err}"
    );

    // Nothing was created: no children. (C, B, P from the scaffold still exist.)
    let issues_after = h.storage.list_issues().unwrap();
    assert_eq!(
        issues_after.len(),
        issues_before,
        "a rejected breakdown must create NOTHING (no children)"
    );
    // P's gate is untouched.
    let p = h.get_issue(&planning_id);
    assert_eq!(
        p.gates_status.get("plan-review").map(|g| g.status),
        Some(jit::domain::GateStatus::Pending),
    );
}

// ===================== Finding 3: cycle detection ===========================

#[test]
fn test_bracket_breakdown_rejects_cyclic_child_plan() {
    let h = TestHarness::new();
    let template = plan_template();
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

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
        .bracket_breakdown_with_template(&template, &c, vec![a, b, d])
        .unwrap_err();
    assert!(
        err.to_string().contains("cycle"),
        "must reject a cyclic child plan, got: {err}"
    );

    // No children created: the cycle is caught BEFORE any mutation.
    let issues_after = h.storage.list_issues().unwrap();
    assert_eq!(
        issues_after.len(),
        issues_before,
        "a cyclic child plan must create NOTHING (pre-mutation rejection)"
    );
}

// ============ coverage gate is ATTACHED (by the scaffold), not run =======
//
// The breakdown step is a spine-splicer: B's coverage gate was attached by
// `jit apply plan` and is never run/stamped/fabricated by breakdown. The gate is
// run later by the standard gate runner (`jit gate pass <B> coverage-preview`).

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
fn test_bracket_breakdown_leaves_coverage_gate_pending_and_forwards_credit() {
    let h = TestHarness::new();
    let template = plan_template();
    // Container declares [hard] REQ-01 (scaffold_container's body), covered by
    // the child's satisfies:REQ-01 label.
    let (c, _p) = scaffold_container(&h, &template, "Auth epic");

    let result = h
        .executor
        .bracket_breakdown_with_template(
            &template,
            &c,
            vec![child_satisfying("Build login", "REQ-01")],
        )
        .unwrap();

    assert_eq!(result.coverage_gate_preset, "coverage-preview");

    // The child carries its satisfies:<id> coverage credit (forwarded by the
    // splicer to create_issue's labels) — what the coverage gate later reads.
    let kid = h.get_issue(&result.child_ids[0]);
    assert!(
        kid.labels.contains(&"satisfies:REQ-01".to_string()),
        "child must carry its satisfies:REQ-01 coverage credit, got {:?}",
        kid.labels
    );

    // The coverage gate stays PENDING (breakdown never runs it), and no gate
    // verdict event is emitted for B by breakdown.
    let b = h.get_issue(&result.breakdown_id);
    let coverage_status = b.gates_status.get("coverage-preview").map(|g| g.status);
    assert!(
        matches!(
            coverage_status,
            None | Some(jit::domain::GateStatus::Pending)
        ),
        "the splicer must leave the coverage gate PENDING/unset, got {coverage_status:?}"
    );
    let events = h.storage.read_events().unwrap();
    assert!(
        !events.iter().any(|e| {
            (e.get_type() == "gate_passed" || e.get_type() == "gate_failed")
                && e.get_issue_id() == result.breakdown_id
        }),
        "breakdown must NOT emit a gate verdict event for B (no faked gate run)"
    );
}
