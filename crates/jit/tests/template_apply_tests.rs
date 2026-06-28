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
use jit::storage::IssueStore;
use jit::templates::{GraphTemplate, TemplateRegistry};
use std::collections::BTreeMap;

const HIERARCHY: [&str; 3] = ["epic", "planning", "breakdown"];

/// The repo's `plan`-shaped template: a planning node `P` and a breakdown node
/// `B` with `brackets:<short-id>`, each with gate presets, doc, and seeded
/// description, plus the `repo-validate` whole-repo integrity gate on the
/// `container` anchor (REQ-13). A fresh apply wires the `depends_on` edge (B→P),
/// the `anchor_edge` (C→B), and runs the `move-upstream-to-role` transform onto P.
fn plan_template() -> GraphTemplate {
    let toml = r#"
[[template]]
name        = "plan"
applies_to  = ["epic"]

  [[template.anchors]]
  name  = "container"
  gates = ["repo-validate"]

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
    // `repo-validate` is a CONFIG-DECLARED gate (it lives in `.jit/gates.json`,
    // not the built-in preset set), so the in-memory registry must declare it for
    // the `plan` template's `container` anchor to resolve it as a registry gate
    // key. Mirrors the shipped whole-repo `jit validate` gate.
    h.add_gate(
        "repo-validate",
        "Repo Validate",
        "Whole-repository validation must pass (jit validate with no issue id)",
        true,
    );
    let (id, _) = h
        .executor
        .create_issue(
            title.to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: it works\n- [soft] nice\n".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "area:auth".to_string()],
            None,
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
    // The `{doc}` token resolves into the planning node's DESCRIPTION as an
    // instruction (where to author and link the plan).
    assert!(
        planning
            .description
            .contains(&format!("dev/active/{full_id}-plan.md")),
        "planning description must carry the resolved plan-doc location: {}",
        planning.description
    );
    // Apply attaches NO document reference: the plan does not exist yet, so the
    // author links it when authored. Neither node carries a plan-labeled ref.
    assert!(
        !planning
            .documents
            .iter()
            .any(|d| d.label.as_deref() == Some("plan")),
        "apply must not create a plan-labeled doc reference (no link to a missing file)"
    );
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

    // First apply succeeds and wires C→B (the fresh-apply path), so the breakdown
    // node is among the container's deps.
    h.executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

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
fn test_apply_rejects_legacy_planning_only_bracket() {
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Legacy P-only epic");

    // Simulate a legacy P-only bracket (from the removed `jit plan`): a planning
    // node wired as the container's dependency, with NO breakdown node. Applying
    // must reject rather than create a duplicate planning node.
    let (planning, _) = h
        .executor
        .create_issue(
            "Plan: Legacy".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:planning".to_string()],
            None,
            None,
            false,
        )
        .unwrap();
    h.executor.add_dependency(&epic, &planning).unwrap();

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("planning node") && msg.contains("legacy"),
        "expected a legacy P-only rejection, got: {msg}"
    );
    // Nothing was created on the rejected apply (precondition phase, no mutation).
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

    // The fresh apply already wired the applied shape: C→B and B→P.

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

#[test]
fn test_force_finds_breakdown_after_spine_reshaping_no_duplicate() {
    // Regression: after a breakdown splices the spine (C → child → … → B), the
    // direct C → B edge is dropped by transitive reduction, so `B` is no longer a
    // DIRECT dependency of `C`. A `--force` refresh must still locate the existing
    // `B` (and `P` through it) by the unique `brackets:<C-short-id>` label — scanning only
    // `C`'s direct deps would miss it and duplicate the whole bracket.
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Spine epic");

    let (first, _) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();
    let planning_id = first.created_node_ids_by_role["planning"].clone();
    let breakdown_id = first.created_node_ids_by_role["breakdown"].clone();

    // Fresh apply wired C → B directly.
    assert!(h.get_issue(&epic).dependencies.contains(&breakdown_id));

    // Simulate the breakdown splice: a child depends on B, and C depends on the
    // child. Transitive reduction then DROPS the direct C → B edge.
    let child = h.create_issue("impl child");
    h.executor.add_dependency(&child, &breakdown_id).unwrap();
    h.executor.add_dependency(&epic, &child).unwrap();
    assert!(
        !h.get_issue(&epic).dependencies.contains(&breakdown_id),
        "reduction must drop the now-redundant direct C → B edge"
    );

    let count_before = h.all_issues().len();
    let (refreshed, _) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), true)
        .unwrap();

    // The existing B + P are refreshed in place — no fresh-apply duplication.
    assert_eq!(
        h.all_issues().len(),
        count_before,
        "force-refresh must NOT create duplicate nodes when B is not a direct dep"
    );
    assert_eq!(
        refreshed.created_node_ids_by_role["breakdown"],
        breakdown_id
    );
    assert_eq!(refreshed.created_node_ids_by_role["planning"], planning_id);

    // Exactly one breakdown node (with brackets:<C-short-id>) and one planning node remain.
    let short_id: String = epic.chars().take(8).collect();
    let bracket_label = format!("brackets:{short_id}");
    let breakdown_count = h
        .all_issues()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("breakdown") && i.labels.contains(&bracket_label))
        .count();
    assert_eq!(breakdown_count, 1, "exactly one breakdown node for C");
    let planning_count = h
        .all_issues()
        .into_iter()
        .filter(|i| type_of(i).as_deref() == Some("planning"))
        .count();
    assert_eq!(planning_count, 1, "exactly one planning node");
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

// === REQ-13: anchor-level gate presets (jit:2614ecf2) ===

#[test]
fn test_apply_plan_template_attaches_repo_validate_to_container() {
    // REQ-13 (issue 552ff75c): applying the `plan` template attaches the
    // whole-repo integrity gate `repo-validate` to the bound container anchor —
    // now via the registry-gate-key path (`repo-validate` is config-declared in
    // `.jit/gates.json`, registered here by `create_epic`, NOT a built-in preset)
    // — so the container cannot reach Done until whole-repo validation passes.
    let h = TestHarness::new();
    let template = plan_template();
    let epic = create_epic(&h, "Repo-validate epic");

    h.executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    let container = h.get_issue(&epic);
    assert!(
        container
            .gates_required
            .contains(&"repo-validate".to_string()),
        "bound container missing the plan template's repo-validate anchor gate: {:?}",
        container.gates_required
    );
}

#[test]
fn test_apply_attaches_registry_gate_key_not_preset() {
    // REQ-13 (issue 552ff75c): a template gate that names a REGISTRY GATE KEY
    // (declared in `.jit/gates.json`, NOT a built-in preset) is attached to the
    // bound issue at apply time — the same effect as `jit gate add <issue> <key>`.
    // Here the anchor and a node each name `deploy-check`, a registry-only gate.
    let h = TestHarness::new();
    h.add_gate(
        "deploy-check",
        "Deploy Check",
        "A config-declared gate, not a preset",
        true,
    );
    // Guard: the name must NOT also be a preset, so the test exercises the
    // registry-key branch and not the preset branch.
    assert!(
        h.storage.get_gate_preset("deploy-check").is_err(),
        "deploy-check must be a registry-only gate, not a preset"
    );

    let toml = r#"
[[template]]
name        = "registrygate"
applies_to  = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["deploy-check"]
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  gates       = ["deploy-check"]
  description = "Break down {container.title}."
  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("registrygate")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Registry-gate epic");

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    // The bound anchor (container) carries the registry gate...
    let container = h.get_issue(&epic);
    assert!(
        container
            .gates_required
            .contains(&"deploy-check".to_string()),
        "bound anchor missing its registry-key gate: {:?}",
        container.gates_required
    );
    // ...and so does the created node.
    let breakdown = h.get_issue(&result.created_node_ids_by_role["breakdown"]);
    assert!(
        breakdown
            .gates_required
            .contains(&"deploy-check".to_string()),
        "node missing its registry-key gate: {:?}",
        breakdown.gates_required
    );
}

#[test]
fn test_apply_attaches_anchor_gate_presets_to_bound_anchor() {
    let h = TestHarness::new();
    // A template whose `container` anchor declares a completion gate preset.
    let toml = r#"
[[template]]
name        = "anchored"
applies_to  = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["plan-review"]
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  description = "Break down {container.title}."
  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("anchored")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Anchor-gated epic");

    h.executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    // REQ-13: the bound anchor (the container epic) carries the anchor's preset,
    // attached through the same `apply_gate_preset` path node gates use.
    let container = h.get_issue(&epic);
    assert!(
        container
            .gates_required
            .contains(&"plan-review".to_string()),
        "bound anchor missing its declared gate preset: {:?}",
        container.gates_required
    );
}

#[test]
fn test_apply_rejects_unknown_anchor_gate_preset_and_creates_nothing() {
    let h = TestHarness::new();
    // The `container` anchor references a gate name that is NEITHER a preset NOR
    // a registry gate key; this must be rejected up front, exactly like an
    // unknown node gate.
    let toml = r#"
[[template]]
name        = "ghostanchorgate"
applies_to  = ["epic"]
  [[template.anchors]]
  name  = "container"
  gates = ["does-not-exist"]
  [[template.nodes]]
  role        = "breakdown"
  type        = "breakdown"
  description = "Break down {container.title}."
  [[template.anchor_edges]]
  from = "container"
  to   = "breakdown"
"#;
    let template = TemplateRegistry::from_toml_str(toml, &HIERARCHY)
        .unwrap()
        .get("ghostanchorgate")
        .unwrap()
        .clone();
    let epic = create_epic(&h, "Ghost-anchor-gate epic");

    let before = h.all_issues().len();
    let err = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap_err();
    assert!(err.to_string().contains("does-not-exist"), "{err}");
    // The unknown-gate failure is caught before the first create (APPA-01).
    assert_eq!(h.all_issues().len(), before);
}

#[test]
fn test_repo_gates_json_declares_repo_validate_whole_repo_checker() {
    // REQ-13 (issue 552ff75c): `repo-validate` is CONFIG-DECLARED in the repo's
    // own `.jit/gates.json` (not a built-in preset), with a whole-repo checker —
    // `jit validate` with NO issue id — distinct from the per-issue `jit-validate`
    // gate (`jit validate "$JIT_ISSUE_ID"`).
    let gates_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.jit/gates.json");
    let raw = std::fs::read_to_string(&gates_path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", gates_path.display()));
    let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

    let gate = &json["gates"]["repo-validate"];
    assert!(
        gate.is_object(),
        ".jit/gates.json must declare the `repo-validate` gate"
    );
    assert_eq!(gate["stage"], "postcheck");
    assert_eq!(gate["mode"], "auto");

    let command = gate["checker"]["command"]
        .as_str()
        .expect("repo-validate checker must have a string command");
    assert_eq!(
        command, "jit validate",
        "repo-validate must run whole-repo validation (no issue id)"
    );
    assert!(
        !command.contains("JIT_ISSUE_ID"),
        "repo-validate must NOT scope to a single issue like the jit-validate gate"
    );

    // Sanity: the per-issue jit-validate gate is the distinct, id-scoped one.
    assert_eq!(
        json["gates"]["jit-validate"]["checker"]["command"],
        "jit validate \"$JIT_ISSUE_ID\""
    );
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

// === TSTA-02: the JSON apply-result output ===

#[test]
fn test_apply_result_serializes_to_expected_json_shape() {
    // The `TemplateApplyResult` is what `jit apply --json` serializes. Pin its
    // serialized contract directly (the CLI subprocess test in apply_cli_tests.rs
    // covers the wired path; this pins the struct's own `Serialize` shape in an
    // isolated in-process repo): the template name, the resolved anchor bindings,
    // the role→id map, and the pre-apply snapshot must all round-trip with their
    // exact field names and values.
    let h = TestHarness::new();
    let template = plan_template();
    let upstream = h.create_issue("Upstream");
    let epic = create_epic(&h, "JSON epic");
    h.executor.add_dependency(&epic, &upstream).unwrap();
    let upstream_full = h.get_issue(&upstream).id;
    let epic_full = h.get_issue(&epic).id;

    let (result, _w) = h
        .executor
        .apply_template_with(&template, &epic, &container_binding(&epic), false)
        .unwrap();

    let json = serde_json::to_value(&result).unwrap();

    // Top-level fields are present under their documented names.
    assert_eq!(json["template"], "plan");
    assert_eq!(json["anchor_bindings"]["container"], epic_full);

    // The role→id map names both roles with the ids the result reports.
    let planning_id = &result.created_node_ids_by_role["planning"];
    let breakdown_id = &result.created_node_ids_by_role["breakdown"];
    assert_eq!(json["created_node_ids_by_role"]["planning"], *planning_id);
    assert_eq!(json["created_node_ids_by_role"]["breakdown"], *breakdown_id);

    // The pre-apply snapshot is serialized as the container's ORIGINAL upstream
    // deps (the input the move-upstream transform consumed), not its post-apply
    // deps.
    assert_eq!(
        json["anchor_dependency_snapshots"]["container"],
        serde_json::json!([upstream_full])
    );

    // The serialized shape has exactly the four documented top-level keys — no
    // stray or missing field leaks into the machine-readable contract.
    let obj = json
        .as_object()
        .expect("result serializes to a JSON object");
    let mut keys: Vec<&String> = obj.keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        vec![
            "anchor_bindings",
            "anchor_dependency_snapshots",
            "created_node_ids_by_role",
            "template",
        ]
    );
}

// === TSTA-02: property-based engine invariants (proptest) ===
//
// These complement the example-based tests above by exercising the apply engine
// over a RANDOMIZED input space: a fixed, valid `plan` template applied to a
// container carrying an arbitrary set of pre-existing upstream dependencies (the
// move-upstream transform's input). Each case runs in its own isolated
// `TestHarness` (InMemoryStorage), so the production `.jit/` is never touched.
// They pin invariants the single-shape examples cannot: acyclicity and
// transitive-reduction must hold for ANY upstream arrangement, and a `--force`
// refresh must be idempotent on the resulting node/edge set regardless of it.

use proptest::prelude::*;

/// Each apply case builds a fresh in-memory repo and does several real (in-memory)
/// writes, so cap the case count well under the per-test time budget while still
/// covering a wide range of upstream arrangements.
fn engine_config() -> ProptestConfig {
    ProptestConfig::with_cases(48)
}

/// Build an isolated harness with an `epic` container that has `upstream_count`
/// pre-existing upstream dependencies. Returns the harness, the container's short
/// id, and the full ids of its upstream deps (the move-upstream transform's
/// snapshot input).
fn harness_with_upstreams(
    title: &str,
    upstream_count: usize,
) -> (TestHarness, String, Vec<String>) {
    let h = TestHarness::new();
    let epic = create_epic(&h, title);
    let upstreams: Vec<String> = (0..upstream_count)
        .map(|i| {
            let up = h.create_issue(&format!("Upstream {i}"));
            h.executor.add_dependency(&epic, &up).unwrap();
            h.get_issue(&up).id
        })
        .collect();
    (h, epic, upstreams)
}

/// True iff the whole store is acyclic, checked through the same `DependencyGraph`
/// the engine uses (`Issue` implements `GraphNode`).
fn store_is_acyclic(h: &TestHarness) -> bool {
    let issues = h.all_issues();
    let refs: Vec<&jit::domain::Issue> = issues.iter().collect();
    jit::graph::DependencyGraph::new(&refs)
        .validate_dag()
        .is_ok()
}

/// True iff the store's dependency graph carries no transitively-redundant DIRECT
/// edge: for every edge `from -> to`, `to` must NOT be reachable from `from`
/// through any OTHER direct dependency. `add_dependency`'s eager transitive
/// reduction must leave the result in this reduced form.
fn store_is_transitively_reduced(h: &TestHarness) -> bool {
    let issues = h.all_issues();
    let deps_by_id: BTreeMap<String, Vec<String>> = issues
        .iter()
        .map(|i| (i.id.clone(), i.dependencies.clone()))
        .collect();

    // Reachability from `start` to `target` over the dependency edges, optionally
    // ignoring a single direct edge `(start -> skip)` so we can ask "is `target`
    // reachable WITHOUT the direct edge under test".
    fn reachable(
        deps_by_id: &BTreeMap<String, Vec<String>>,
        start: &str,
        target: &str,
        skip: (&str, &str),
    ) -> bool {
        let mut stack = vec![start.to_string()];
        let mut seen = std::collections::HashSet::new();
        while let Some(node) = stack.pop() {
            if !seen.insert(node.clone()) {
                continue;
            }
            if let Some(deps) = deps_by_id.get(&node) {
                for dep in deps {
                    if node == skip.0 && dep == skip.1 {
                        continue; // ignore the direct edge under test
                    }
                    if dep == target {
                        return true;
                    }
                    stack.push(dep.clone());
                }
            }
        }
        false
    }

    deps_by_id.iter().all(|(from, deps)| {
        deps.iter().all(|to| {
            // The direct edge `from -> to` is redundant iff `to` is still reachable
            // from `from` after removing that edge. A reduced graph has none.
            !reachable(&deps_by_id, from, to, (from.as_str(), to.as_str()))
        })
    })
}

/// The set of `(issue_id, sorted dependency_ids)` pairs across the whole store —
/// the canonical "node + edge set" fingerprint used to compare two graph states.
fn edge_fingerprint(h: &TestHarness) -> BTreeMap<String, Vec<String>> {
    h.all_issues()
        .into_iter()
        .map(|i| {
            let mut deps = i.dependencies.clone();
            deps.sort();
            (i.id, deps)
        })
        .collect()
}

proptest! {
    #![proptest_config(engine_config())]

    /// Invariant: applying the template ALWAYS yields an acyclic DAG, for any
    /// number of pre-existing upstream deps on the container. The spine wiring
    /// (`C → B → P`) plus the move-upstream transform (`P → upstream*`) must never
    /// introduce a cycle.
    #[test]
    fn prop_apply_always_yields_acyclic_dag(upstream_count in 0usize..6usize) {
        let template = plan_template();
        let (h, epic, _ups) = harness_with_upstreams("Acyclic prop", upstream_count);

        h.executor
            .apply_template_with(&template, &epic, &container_binding(&epic), false)
            .unwrap();

        prop_assert!(
            store_is_acyclic(&h),
            "apply with {upstream_count} upstream dep(s) must leave the store acyclic"
        );
    }

    /// Invariant: the post-apply graph is transitively reduced — no direct edge is
    /// redundant — regardless of the upstream arrangement. The example tests pin
    /// this for one shape (the dropped direct `C → B` after a splice); this asserts
    /// it as a global property over every edge in the store.
    #[test]
    fn prop_apply_yields_transitively_reduced_graph(upstream_count in 0usize..6usize) {
        let template = plan_template();
        let (h, epic, _ups) = harness_with_upstreams("Reduced prop", upstream_count);

        h.executor
            .apply_template_with(&template, &epic, &container_binding(&epic), false)
            .unwrap();

        prop_assert!(
            store_is_transitively_reduced(&h),
            "apply with {upstream_count} upstream dep(s) must leave the store transitively reduced"
        );
    }

    /// Invariant: apply, then apply `--force`, is idempotent on the graph — the same
    /// node set and the same edge set, with NO duplicate nodes — for any upstream
    /// arrangement. A `--force` refresh only re-seeds prose; it must not create,
    /// drop, or rewire any node or edge.
    #[test]
    fn prop_force_refresh_is_idempotent_on_node_and_edge_set(upstream_count in 0usize..6usize) {
        let template = plan_template();
        let (h, epic, _ups) = harness_with_upstreams("Idempotent prop", upstream_count);

        let (first, _) = h
            .executor
            .apply_template_with(&template, &epic, &container_binding(&epic), false)
            .unwrap();
        let fingerprint_before = edge_fingerprint(&h);
        let count_before = h.all_issues().len();

        let (refreshed, _) = h
            .executor
            .apply_template_with(&template, &epic, &container_binding(&epic), true)
            .unwrap();

        // No duplicate nodes, and the role→id map is stable across the refresh.
        prop_assert_eq!(
            h.all_issues().len(),
            count_before,
            "force-refresh must not create or drop any node"
        );
        prop_assert_eq!(
            &refreshed.created_node_ids_by_role,
            &first.created_node_ids_by_role,
            "force-refresh must map each role to the same id"
        );

        // The full node + edge set is byte-for-byte stable across the refresh.
        prop_assert_eq!(
            edge_fingerprint(&h),
            fingerprint_before,
            "force-refresh must leave the node/edge set unchanged"
        );

        // And the refreshed graph is still an acyclic DAG.
        prop_assert!(store_is_acyclic(&h), "force-refresh must keep the store acyclic");
    }
}
