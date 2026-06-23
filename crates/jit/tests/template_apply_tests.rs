//! Engine tests for the graph-template apply core: `apply_template_with` —
//! validate, snapshot, instantiate (jit:14137e1a), plus edge wiring + the
//! `move-upstream-to-role` transform (jit:73e5e853).
//!
//! These exercise the real `create_issue` + gate-preset + `add_dependency` +
//! storage path through the in-process `TestHarness` (InMemoryStorage), which is
//! fully isolated from the production `.jit/`. The template is authored in-test
//! via `TemplateRegistry::from_toml_str` and passed explicitly to
//! `apply_template_with`, so no on-disk `templates.toml` is needed (mirroring the
//! `*_with_config` split the planning scaffold uses).
//!
//! A fresh apply now also wires the template's internal `depends_on` edges (B→P),
//! its `anchor_edges` (C→B), and runs `move-upstream-to-role` (the container's
//! pre-apply upstream deps move onto P), yielding the acyclic, transitively
//! reduced spine C→B→P→upstream.

mod harness;

use harness::TestHarness;
use jit::domain::Priority;
use jit::labels::parse_label;
use jit::templates::{GraphTemplate, TemplateRegistry};
use std::collections::BTreeMap;

const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

/// The repo's `plan`-shaped template: a planning node `P` and a breakdown node
/// `B` with `brackets:<short-id>`, each with gate presets, doc, and seeded
/// description. A fresh apply wires the `depends_on` edge (B→P), the `anchor_edge`
/// (C→B), and runs the `move-upstream-to-role` transform onto P.
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
  description = "Planning node for {container.title} ({container.short_id}). Doc: {doc}. Cover: {container.hard_criteria}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["coverage-preview", "breakdown-review"]
  labels      = ["brackets:{container.short_id}"]
  description = "Breakdown node bracketing {container.title} ({container.short_id})."
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

fn type_of(issue: &jit::domain::Issue) -> Option<String> {
    issue.labels.iter().find_map(|l| {
        parse_label(l)
            .ok()
            .and_then(|(ns, v)| (ns == "type").then_some(v))
    })
}

fn create_epic(h: &TestHarness, title: &str) -> String {
    let (id, _) = h
        .executor
        .create_issue(
            title.to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n- [soft] nice\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "area:auth".to_string()],
            None,
            false,
        )
        .unwrap();
    id
}

fn container_binding(id: &str) -> BTreeMap<String, String> {
    BTreeMap::from([("container".to_string(), id.to_string())])
}

// === APPA-02 / APPA-04: instantiation ===

#[test]
fn test_apply_instantiates_nodes_with_interpolated_descriptions() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Auth epic");

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    assert_eq!(result.template, "plan");
    let planning_id = &result.created_node_ids_by_role["planning"];
    let breakdown_id = &result.created_node_ids_by_role["breakdown"];

    let planning = h.get_issue(planning_id);
    let breakdown = h.get_issue(breakdown_id);

    // Non-empty, interpolated descriptions (APPA-02).
    assert!(planning.description.contains("Auth epic"));
    assert!(!planning.description.contains("{container."));
    assert!(planning.description.contains("[hard] REQ-01: it works"));
    assert!(breakdown.description.contains("Auth epic"));
    assert!(!breakdown.description.is_empty());

    // Correct types.
    assert_eq!(type_of(&planning).as_deref(), Some("planning"));
    assert_eq!(type_of(&breakdown).as_deref(), Some("breakdown"));

    // Inherited membership label, dropped container `type:` label, interpolated
    // node label.
    assert!(planning.labels.contains(&"area:auth".to_string()));
    assert!(!planning.labels.iter().any(|l| l == "type:epic"));
    let short = h.get_issue(&epic).short_id();
    assert!(breakdown.labels.contains(&format!("brackets:{short}")));
}

#[test]
fn test_apply_resolves_node_doc_location() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Doc epic");

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    let planning = h.get_issue(&result.created_node_ids_by_role["planning"]);
    let full_id = h.get_issue(&epic).id;
    let plan_doc = planning
        .documents
        .iter()
        .find(|d| d.label.as_deref() == Some("plan"))
        .expect("planning node carries a plan-labeled doc reference");
    assert_eq!(plan_doc.path, format!("dev/active/{full_id}-plan.md"));

    // The breakdown node declares no doc, so it has no plan-labeled reference.
    let breakdown = h.get_issue(&result.created_node_ids_by_role["breakdown"]);
    assert!(!breakdown
        .documents
        .iter()
        .any(|d| d.label.as_deref() == Some("plan")));
}

#[test]
fn test_apply_attaches_declared_gate_presets() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Gated epic");

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    let planning = h.get_issue(&result.created_node_ids_by_role["planning"]);
    let breakdown = h.get_issue(&result.created_node_ids_by_role["breakdown"]);

    // APPA-04: each node carries its template-declared gate presets.
    assert!(planning.gates_required.contains(&"plan-review".to_string()));
    assert!(breakdown
        .gates_required
        .contains(&"coverage-preview".to_string()));
    assert!(breakdown
        .gates_required
        .contains(&"breakdown-review".to_string()));
}

#[test]
fn test_apply_snapshots_container_dependencies_before_mutation() {
    let h = TestHarness::new();
    let template = plan_template();
    let upstream = h.create_issue("Upstream");
    let epic = create_epic(&h, "Snapshot epic");
    // Give the container a pre-existing upstream dependency.
    h.executor.add_dependency(&epic, &upstream).unwrap();
    let upstream_full = h.get_issue(&upstream).id;

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    // The pre-apply snapshot carries the container's original upstream dep, the
    // input the edge/transform task consumes.
    assert_eq!(
        result.anchor_dependency_snapshots["container"],
        vec![upstream_full]
    );
}

// === APPA-01: validate before mutating; zero nodes on failure ===

#[test]
fn test_apply_rejects_wrong_container_type_and_creates_nothing() {
    let h = TestHarness::new();
    let template = plan_template();
    let (task, _) = h
        .executor
        .create_issue(
            "A task".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            false,
        )
        .unwrap();

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &task, &container_binding(&task), false)
        .unwrap_err();
    assert!(err.to_string().contains("does not apply"));
    // APPA-01: zero nodes created.
    assert_eq!(h.all_issues().len(), before);
}

#[test]
fn test_apply_rejects_unbound_anchor_and_creates_nothing() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Unbound epic");

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &BTreeMap::new(), false)
        .unwrap_err();
    assert!(err.to_string().contains("not bound"));
    assert_eq!(h.all_issues().len(), before);
}

#[test]
fn test_apply_rejects_anchor_bound_to_missing_issue() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Missing-anchor epic");

    let before = h.all_issues().len();
    let bindings = BTreeMap::from([("container".to_string(), "deadbeefdeadbeef".to_string())]);
    let err = h
        .executor
        .apply_template_with(&template, &epic, &bindings, false)
        .unwrap_err();
    // Validation fails before any node is created.
    assert!(
        err.to_string().to_lowercase().contains("resolve") || err.to_string().contains("exist")
    );
    assert_eq!(h.all_issues().len(), before);
}

// === APPA-03: already-applied rejection + force-refresh in place ===

#[test]
fn test_apply_rejects_already_applied_without_force() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Twice epic");

    // First apply succeeds. Wire C→B so the breakdown node is among the
    // container's deps (this engine does not wire edges, so do it manually to
    // simulate the applied shape the next task produces).
    let (first, _) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let breakdown_id = first.created_node_ids_by_role["breakdown"].clone();
    h.executor.add_dependency(&epic, &breakdown_id).unwrap();

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap_err();
    assert!(err.to_string().contains("already"));
    // No duplicate nodes created on the rejected re-apply.
    assert_eq!(h.all_issues().len(), before);
}

#[test]
fn test_force_refreshes_nodes_in_place_without_duplicating() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Original title");

    let (first, _) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let planning_id = first.created_node_ids_by_role["planning"].clone();
    let breakdown_id = first.created_node_ids_by_role["breakdown"].clone();

    // Wire the applied shape: C→B and B→P, as the edge task will.
    h.executor.add_dependency(&epic, &breakdown_id).unwrap();
    h.executor
        .add_dependency(&breakdown_id, &planning_id)
        .unwrap();

    // Rename the container, then force-refresh.
    h.executor
        .update_issue(
            &epic,
            Some("Renamed title".to_string()),
            None,
            None,
            None,
            vec![],
            vec![],
            None,
            false,
        )
        .unwrap();

    let count_before = h.all_issues().len();
    let (refreshed, _) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), true)
        .unwrap();

    // APPA-03: no duplicate nodes — same count, same ids.
    assert_eq!(h.all_issues().len(), count_before);
    assert_eq!(
        refreshed.created_node_ids_by_role["breakdown"],
        breakdown_id
    );
    assert_eq!(refreshed.created_node_ids_by_role["planning"], planning_id);

    // Descriptions re-interpolated against the new title, in place.
    let planning = h.get_issue(&planning_id);
    let breakdown = h.get_issue(&breakdown_id);
    assert!(planning.description.contains("Renamed title"));
    assert!(breakdown.description.contains("Renamed title"));
    assert!(!planning.description.contains("Original title"));
}

// === APPA-01: gate presets validated before any mutation ===

#[test]
fn test_apply_rejects_unknown_gate_preset_and_creates_nothing() {
    let h = TestHarness::new();
    // A single planning node referencing a gate preset that does not exist.
    let toml = r#"
[[template]]
name        = "ghostgate"
applies_to  = ["epic"]
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  gates       = ["does-not-exist"]
  description = "Plan {container.title}."
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("ghostgate")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Ghost-gate epic");

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap_err();
    assert!(err.to_string().contains("does-not-exist"));
    // APPA-01: the missing-preset failure is caught before the first create.
    assert_eq!(h.all_issues().len(), before);
}

#[test]
fn test_apply_rejects_invalid_node_label_before_creating_any_node() {
    let h = TestHarness::new();
    // The FIRST node is valid; the SECOND interpolates to a label that violates
    // the canonical `namespace:value` rule `create_issue` enforces. Without
    // up-front pre-validation the first node would persist before the second
    // failed — so this asserts the whole apply aborts with ZERO new nodes.
    let toml = r#"
[[template]]
name        = "badlabel"
applies_to  = ["epic"]
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  description = "Plan {container.title}."

  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  description = "Break down {container.title}."
  labels      = ["not a valid label"]
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("badlabel")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Bad-label epic");

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap_err();
    // The error names the offending node so the misconfiguration is actionable.
    assert!(err.to_string().contains("breakdown"), "{err}");
    // APPA-01: the later node's invalid label is caught before the first create.
    assert_eq!(h.all_issues().len(), before);
}

// === APPA-02: empty/blank descriptions still produce a non-empty body ===

#[test]
fn test_apply_seeds_non_empty_description_for_blank_template() {
    let h = TestHarness::new();
    // A node whose description template is the empty string must still get a
    // non-empty body from the fallback.
    let toml = r#"
[[template]]
name        = "blankdesc"
applies_to  = ["epic"]
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  description = ""
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("blankdesc")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Blank-desc epic");

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let planning = h.get_issue(&result.created_node_ids_by_role["planning"]);
    assert!(
        !planning.description.trim().is_empty(),
        "blank template must fall back to a non-empty description"
    );
    assert!(planning.description.contains("Blank-desc epic"));
}

// === APPA-03: force-refresh must reach every role or error (no silent partial) ===

#[test]
fn test_force_errors_when_planning_node_unreachable() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Broken bracket");

    let (first, _) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let breakdown_id = first.created_node_ids_by_role["breakdown"].clone();
    let planning_id = first.created_node_ids_by_role["planning"].clone();
    // The fresh apply wired C→B and B→P. Sever B→P so the planning node becomes
    // unreachable through the breakdown node's depends_on: a broken bracket the
    // --force refresh must reject rather than silently refresh only B.
    h.executor
        .remove_dependency(&breakdown_id, &planning_id)
        .unwrap();

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), true)
        .unwrap_err();
    // APPA-03: a broken/incomplete bracket errors rather than silently refreshing
    // only the breakdown node.
    let msg = err.to_string();
    assert!(msg.contains("planning"), "{msg}");
    // No nodes created or duplicated by the failed refresh.
    assert_eq!(h.all_issues().len(), before);
}

// === APPB-01 / APPB-02: edge wiring + move-upstream-to-role transform ===

#[test]
fn test_apply_wires_spine_and_moves_upstream_onto_planning() {
    let h = TestHarness::new();
    let template = plan_template();
    let up1 = h.create_issue("Upstream 1");
    let up2 = h.create_issue("Upstream 2");
    let epic = create_epic(&h, "Spine epic");
    h.executor.add_dependency(&epic, &up1).unwrap();
    h.executor.add_dependency(&epic, &up2).unwrap();
    let up1_full = h.get_issue(&up1).id;
    let up2_full = h.get_issue(&up2).id;

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let planning_id = result.created_node_ids_by_role["planning"].clone();
    let breakdown_id = result.created_node_ids_by_role["breakdown"].clone();

    let container = h.get_issue(&epic);
    let breakdown = h.get_issue(&breakdown_id);
    let planning = h.get_issue(&planning_id);

    // APPB-01: spine C→B→P→{originals}, transitively reduced.
    // C depends only on B now (its originals moved off, so C→up* is gone).
    assert_eq!(container.dependencies, vec![breakdown_id.clone()]);
    assert!(!container.dependencies.contains(&up1_full));
    assert!(!container.dependencies.contains(&up2_full));

    // B depends on P (internal depends_on edge).
    assert_eq!(breakdown.dependencies, vec![planning_id.clone()]);

    // P carries the container's pre-apply upstream deps (move-upstream-to-role).
    assert!(planning.dependencies.contains(&up1_full));
    assert!(planning.dependencies.contains(&up2_full));

    // Acyclic + reduced: no back-edges off the spine. In particular P does not
    // depend on B or C, and B does not depend on C.
    assert!(!planning.dependencies.contains(&breakdown_id));
    assert!(!planning.dependencies.contains(&container.id));
    assert!(!breakdown.dependencies.contains(&container.id));
}

#[test]
fn test_transform_does_not_move_freshly_created_breakdown_onto_planning() {
    // APPB-02: the transform must read the PRE-APPLY snapshot, not live deps. If
    // it read live deps it would see the freshly-wired C→B edge and move B onto P
    // (creating P→B, which conflicts with B→P). Assert no such edge exists.
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "No-upstream epic"); // C has NO upstream deps

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let planning_id = result.created_node_ids_by_role["planning"].clone();
    let breakdown_id = result.created_node_ids_by_role["breakdown"].clone();

    let planning = h.get_issue(&planning_id);
    // P must have NO dependency on the freshly-created breakdown node.
    assert!(
        !planning.dependencies.contains(&breakdown_id),
        "transform moved the scaffold breakdown node onto planning (read live deps, not snapshot)"
    );
    // With no upstream, P has no dependencies at all.
    assert!(planning.dependencies.is_empty());
    // The spine B→P is intact and not inverted.
    let breakdown = h.get_issue(&breakdown_id);
    assert_eq!(breakdown.dependencies, vec![planning_id]);
}

#[test]
fn test_apply_rejects_unknown_transform_kind_and_creates_nothing() {
    let h = TestHarness::new();
    // A template declaring an unsupported transform kind. The loader validates
    // the transform's ROLE but not its KIND, so the engine must reject it — and,
    // per validate-before-mutate, BEFORE creating any node.
    let toml = r#"
[[template]]
name        = "weird"
applies_to  = ["epic"]
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  description = "Plan {container.title}."
  [[template.transforms]]
  kind = "teleport"
  role = "planning"
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("weird")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Weird epic");

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap_err();
    assert!(err.to_string().contains("teleport"), "{err}");
    // The unknown kind is caught in the precondition phase, before any mutation:
    // ZERO new issues (and thus zero edges) were created.
    assert_eq!(h.all_issues().len(), before);
}

// === APPLY-04: a prospective cycle is caught before any mutation ===

#[test]
fn test_apply_rejects_prospective_cycle_and_creates_nothing() {
    let h = TestHarness::new();
    // A multi-anchor template that, given C→U pre-existing, forms a cycle once
    // applied: anchor edge U→B (U depends on the breakdown node), B→P (internal),
    // and move-upstream-to-role planning moves C's snapshot dep U onto P (P→U).
    // The resulting U→B→P→U is a cycle. It must be caught BEFORE any node is
    // created (validate-before-mutate / "simulate edges for acyclicity").
    let toml = r#"
[[template]]
name        = "cyclic"
applies_to  = ["epic"]
  [[template.anchors]]
  name = "container"
  [[template.anchors]]
  name = "upstream"
  [[template.nodes]]
  role        = "planning"
  type        = "planning"
  description = "Plan {container.title}."
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  description = "Break down {container.title}."
  labels      = ["brackets:{container.short_id}"]
  depends_on  = ["planning"]
  [[template.anchor_edges]]
  from = "upstream"
  to   = "breakdown"
  [[template.transforms]]
  kind = "move-upstream-to-role"
  role = "planning"
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("cyclic")
        .unwrap()
        .clone();

    let upstream = h.create_issue("Upstream U");
    let epic = create_epic(&h, "Cyclic epic");
    // C → U pre-existing.
    h.executor.add_dependency(&epic, &upstream).unwrap();

    let bindings = BTreeMap::from([
        ("container".to_string(), epic.clone()),
        ("upstream".to_string(), upstream.clone()),
    ]);

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &bindings, false)
        .unwrap_err();
    assert!(err.to_string().contains("cycle"), "{err}");
    // APPLY-04: the prospective cycle is caught before mutation — ZERO new issues.
    assert_eq!(h.all_issues().len(), before);
}
