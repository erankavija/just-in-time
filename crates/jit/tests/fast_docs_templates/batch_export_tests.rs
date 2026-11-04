//! Harness tests for scoped batch-shape graph export (issue 3e12ffbd):
//! `jit graph export --format batch [--scope <container>]`, the structural
//! inverse of `jit issue batch-create`.
//!
//! The scenario is a bracketed epic `E` with a plan bracket (`P` planning, `B`
//! breakdown), two child tasks `c1`/`c2`, and a task `ext` that belongs to a
//! second epic `E2` but is also depended on by `c1` — so `c1 -> ext` crosses
//! `E`'s containment boundary. Config/templates/rules are authored on disk so
//! the export reads real membership, bracket-type, and identity-namespace
//! configuration.

use jit::commands::BatchIssueDef;
use jit::declarations::GateMode;
use jit::domain::identity::IssueId;
use jit::domain::{Priority, State};
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;
use jit::test_utils::add_reducing_edge;
use jit::test_utils::toml_string_array;

const CONFIG_TOML: &str = r#"
[version]
schema = 2

[type_hierarchy]
types = { epic = 2, story = 3, planning = 3, breakdown = 3, task = 4 }

[type_hierarchy.label_associations]
epic = "epic"
story = "story"

[namespaces]
type = { description = "Issue type", unique = true }
component = { description = "Component", unique = false }
brackets = { description = "Bracket container", unique = true }
satisfies = { description = "Criterion coverage", unique = false }
"#;

const TEMPLATES_TOML: &str = r#"
[[template]]
name = "plan"
applies_to = ["epic"]

  [[template.nodes]]
  role = "planning"
  type = "planning"

  [[template.nodes]]
  role = "breakdown"
  type = "breakdown"
  depends_on = ["planning"]
"#;

fn rules_toml() -> String {
    format!(
        r#"
[[rules]]
name = "coverage-preview"
origin = "bracket"
description = "coverage preview for tests"
when = {{ type = "breakdown", state = {states} }}
severity = "error"
enforce = true
assert = {{ label-coverage = {{ criteria-section = "success_criteria", marker = "[hard]", id-pattern = "REQ-[0-9]+", satisfies-namespace = "satisfies", container-from-label = "brackets", child-link = "dependencies", child-type-exclude = ["breakdown", "planning"] }} }}
"#,
        states = toml_string_array(&[State::InProgress, State::Gated, State::Done]),
    )
}

/// Write the config/templates/rules files a batch export reads. Must be called
/// before any executor operation, since config is cached on first access.
fn configure(h: &TestHarness) {
    let layout = h.storage.repository_layout();
    let rules = rules_toml();
    for (virtual_path, contents) in [
        (jit::repository_state::VirtualPath::CONFIG, CONFIG_TOML),
        (
            jit::repository_state::VirtualPath::TEMPLATES,
            TEMPLATES_TOML,
        ),
        (jit::repository_state::VirtualPath::RULES, rules.as_str()),
    ] {
        let path = layout.resolve(&virtual_path).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).unwrap();
    }
}

fn new_issue(
    h: &TestHarness,
    title: &str,
    priority: Priority,
    gates: &[jit::domain::identity::GateKey],
    labels: &[&str],
) -> IssueId {
    h.executor
        .issues()
        .create_issue(
            title.to_string(),
            String::new(),
            priority,
            gates.to_vec(),
            labels.iter().map(|s| s.to_string()).collect(),
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id
}

fn short(id: &IssueId) -> String {
    id.short_id().to_string()
}

struct Scenario {
    e: IssueId,
    #[allow(dead_code)]
    e2: IssueId,
    p: IssueId,
    b: IssueId,
    c1: IssueId,
    c2: IssueId,
    ext: IssueId,
}

/// Build the bracketed-epic scenario and return the full ids.
fn build_scenario(h: &TestHarness) -> Scenario {
    configure(h);
    h.add_gate(
        &"code-review".parse().unwrap(),
        "Code review",
        "review",
        GateMode::Manual,
        None,
    );

    let e = new_issue(
        h,
        "Auth epic",
        Priority::High,
        &["code-review".parse().unwrap()],
        &["type:epic"],
    );
    let e2 = new_issue(h, "Billing epic", Priority::Normal, &[], &["type:epic"]);
    let p = new_issue(h, "Plan auth", Priority::Normal, &[], &["type:planning"]);
    let b = new_issue(
        h,
        "Break down auth",
        Priority::Normal,
        &[],
        &["type:breakdown", &format!("brackets:{}", short(&e))],
    );
    let c1 = new_issue(
        h,
        "Login",
        Priority::Normal,
        &["code-review".parse().unwrap()],
        &[
            "type:task",
            &format!("epic:{}", short(&e)),
            "component:core",
            &format!("satisfies:{}/REQ-01", short(&e)),
        ],
    );
    let c2 = new_issue(
        h,
        "Logout",
        Priority::Normal,
        &[],
        &["type:task", &format!("epic:{}", short(&e))],
    );
    let ext = new_issue(h, "Shared lib", Priority::Normal, &[], &["type:task"]);

    // E → {c1, c2, B}; children → B; B → P; c1 → ext (boundary); E2 → ext.
    for dep in [&c1, &c2, &b] {
        add_reducing_edge(&h.executor, &e, dep).unwrap();
    }
    add_reducing_edge(&h.executor, &c1, &b).unwrap();
    add_reducing_edge(&h.executor, &c2, &b).unwrap();
    add_reducing_edge(&h.executor, &b, &p).unwrap();
    add_reducing_edge(&h.executor, &c1, &ext).unwrap();
    add_reducing_edge(&h.executor, &e2, &ext).unwrap();

    Scenario {
        e,
        e2,
        p,
        b,
        c1,
        c2,
        ext,
    }
}

fn def_for<'a>(defs: &'a [BatchIssueDef], id: &IssueId) -> &'a BatchIssueDef {
    defs.iter()
        .find(|d| d.key == short(id))
        .unwrap_or_else(|| panic!("no def for {}", short(id)))
}

#[test]
fn test_batch_export_scope_projects_membership_and_lifts_type() {
    // REQ-01/REQ-02: --scope restricts to the container's containment closure;
    // each node carries key/title/type/priority/gates/depends_on.
    let h = TestHarness::new();
    let s = build_scenario(&h);

    let export = h.executor.graph().export_graph_batch(Some(&s.e)).unwrap();
    let keys: Vec<String> = export.defs.iter().map(|d| d.key.clone()).collect();

    // Only the non-bracket in-scope nodes: E, c1, c2 (ext is out of scope),
    // ordered by ascending short id.
    let mut expected = vec![short(&s.e), short(&s.c1), short(&s.c2)];
    expected.sort();
    assert_eq!(keys, expected);

    let e_def = def_for(&export.defs, &s.e);
    assert_eq!(e_def.title, "Auth epic");
    assert_eq!(e_def.r#type.as_deref(), Some("epic"));
    assert_eq!(e_def.priority, Some(Priority::High));
    assert_eq!(
        e_def.gates,
        vec!["code-review"
            .parse::<jit::domain::identity::GateKey>()
            .unwrap()]
    );
    // E depends on c1 and c2 (E → B dropped as a bracket edge). depends_on is
    // sorted by short id, so compare against the sorted expectation.
    let mut expected_deps = vec![short(&s.c1), short(&s.c2)];
    expected_deps.sort();
    assert_eq!(e_def.depends_on, expected_deps);
}

#[test]
fn test_batch_export_strips_identity_bound_labels_keeps_generic() {
    // REQ-04: type:* lifted to `type`; membership (epic:), satisfies:, and
    // brackets: stripped; generic labels (component:) survive.
    let h = TestHarness::new();
    let s = build_scenario(&h);

    let export = h.executor.graph().export_graph_batch(Some(&s.e)).unwrap();
    let c1_def = def_for(&export.defs, &s.c1);

    assert_eq!(c1_def.r#type.as_deref(), Some("task"));
    assert_eq!(c1_def.labels, vec!["component:core".to_string()]);
}

#[test]
fn test_batch_export_excludes_bracket_nodes_and_their_edges() {
    // REQ-05: planning/breakdown nodes never appear, and edges touching them
    // (c1 → B, E → B, B → P) are dropped, not reported as boundary crossings.
    let h = TestHarness::new();
    let s = build_scenario(&h);

    let export = h.executor.graph().export_graph_batch(Some(&s.e)).unwrap();
    let keys: Vec<String> = export.defs.iter().map(|d| d.key.clone()).collect();

    assert!(!keys.contains(&short(&s.b)), "breakdown node exported");
    assert!(!keys.contains(&short(&s.p)), "planning node exported");
    // No def references B or P through depends_on.
    for def in &export.defs {
        assert!(!def.depends_on.contains(&short(&s.b)));
        assert!(!def.depends_on.contains(&short(&s.p)));
    }
    // The bracket edges are not boundary edges.
    for edge in &export.boundary_edges {
        assert_ne!(edge.to, short(&s.b));
        assert_ne!(edge.to, short(&s.p));
    }
}

#[test]
fn test_batch_export_reports_boundary_edges() {
    // REQ-06: c1 → ext crosses E's membership boundary; excluded from
    // depends_on and reported (count + node pair).
    let h = TestHarness::new();
    let s = build_scenario(&h);

    let export = h.executor.graph().export_graph_batch(Some(&s.e)).unwrap();

    assert_eq!(export.boundary_edges.len(), 1);
    let edge = &export.boundary_edges[0];
    assert_eq!(edge.from, short(&s.c1));
    assert_eq!(edge.to, short(&s.ext));

    // The crossing edge is not silently present in depends_on.
    let c1_def = def_for(&export.defs, &s.c1);
    assert!(!c1_def.depends_on.contains(&short(&s.ext)));
    assert!(c1_def.depends_on.is_empty());
}

#[test]
fn test_batch_export_omits_lifecycle_and_includes_all_states() {
    // REQ-03: no lifecycle fields; every in-scope node exports regardless of
    // state (here c2 is rejected but still present).
    let h = TestHarness::new();
    let s = build_scenario(&h);

    h.executor
        .issues()
        .update_issue(
            &s.c2,
            None,
            None,
            None,
            Some(State::Rejected),
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();

    let export = h.executor.graph().export_graph_batch(Some(&s.e)).unwrap();
    let keys: Vec<String> = export.defs.iter().map(|d| d.key.clone()).collect();
    assert!(keys.contains(&short(&s.c2)), "rejected node dropped");

    let json = serde_json::to_string(&export.defs).unwrap();
    for lifecycle in ["\"state\"", "\"assignee\"", "\"created_at\"", "\"done_at\""] {
        assert!(
            !json.contains(lifecycle),
            "batch output leaked lifecycle field {lifecycle}: {json}"
        );
    }
}

#[test]
fn test_scoped_graph_exports_carry_no_out_of_scope_edges() {
    // REQ-01 (jit:3e12ffbd code-review F1 regression): a scoped export in every
    // graph-rendering format lists only in-scope nodes AND only edges between
    // them — the boundary-crossing dependency c1 → ext must not leak into
    // json/dot/mermaid output as a dangling edge to an unlisted node.
    use jit::commands::GraphExportFormat;

    let h = TestHarness::new();
    let s = build_scenario(&h);

    for format in [
        GraphExportFormat::Json,
        GraphExportFormat::Dot,
        GraphExportFormat::Mermaid,
    ] {
        let output = h
            .executor
            .graph()
            .export_graph(format, false, Some(&s.e))
            .unwrap()
            .document_text()
            .unwrap();
        assert!(
            output.contains(s.c1.as_str()),
            "{format:?}: in-scope node c1 must be exported"
        );
        assert!(
            !output.contains(s.ext.as_str()),
            "{format:?}: out-of-scope node ext must not appear as node or edge:\n{output}"
        );
    }

    // The full-record JSON shape shares the same edges list.
    let full = h
        .executor
        .graph()
        .export_graph(GraphExportFormat::Json, true, Some(&s.e))
        .unwrap()
        .document_text()
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&full).unwrap();
    let edges = parsed["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .all(|e| e["to"].as_str() != Some(s.ext.as_str())),
        "full JSON edges must not reference the out-of-scope node"
    );
}

#[test]
fn test_batch_export_whole_graph_without_scope() {
    // REQ-08: no --scope exports the whole graph in batch shape (still minus
    // bracket nodes). Every non-bracket issue, including E2 and ext, appears.
    let h = TestHarness::new();
    let s = build_scenario(&h);

    let export = h.executor.graph().export_graph_batch(None).unwrap();
    let keys: std::collections::HashSet<String> =
        export.defs.iter().map(|d| d.key.clone()).collect();

    for id in [&s.e, &s.e2, &s.c1, &s.c2, &s.ext] {
        assert!(
            keys.contains(&short(id)),
            "whole-graph missing {}",
            short(id)
        );
    }
    assert!(!keys.contains(&short(&s.b)));
    assert!(!keys.contains(&short(&s.p)));
    // Whole-graph export has no out-of-scope target, so no boundary edges.
    assert!(export.boundary_edges.is_empty());
}

#[test]
fn test_batch_export_roundtrips_into_fresh_repo() {
    // REQ-07: export E's subtree, feed it to batch-create in a fresh repo with
    // compatible config; the recreated subgraph is isomorphic (titles, types,
    // priorities, gates, in-scope edges) and passes repository validation.
    let source = TestHarness::new();
    let s = build_scenario(&source);
    let export = source
        .executor
        .graph()
        .export_graph_batch(Some(&s.e))
        .unwrap();

    let fresh = TestHarness::new();
    configure(&fresh);
    fresh.add_gate(
        &"code-review".parse().unwrap(),
        "Code review",
        "review",
        GateMode::Manual,
        None,
    );

    let outcome = fresh
        .executor
        .issues()
        .batch_create_from_json(export.defs.clone())
        .unwrap();
    let map = outcome.as_map();
    assert_eq!(map.len(), 3);

    // Titles / types / priorities / gates round-trip.
    let e_new = fresh.storage.load_issue(&map[&short(&s.e)]).unwrap();
    assert_eq!(e_new.title, "Auth epic");
    assert_eq!(jit::labels::type_label_value(&e_new.labels), Some("epic"));
    assert_eq!(e_new.priority, Priority::High);
    assert_eq!(
        e_new.gates_required,
        vec!["code-review"
            .parse::<jit::domain::identity::GateKey>()
            .unwrap()]
    );

    let c1_new = fresh.storage.load_issue(&map[&short(&s.c1)]).unwrap();
    assert_eq!(c1_new.title, "Login");
    assert_eq!(jit::labels::type_label_value(&c1_new.labels), Some("task"));
    assert!(c1_new.labels.contains(&"component:core".to_string()));
    assert_eq!(
        c1_new.gates_required,
        vec!["code-review"
            .parse::<jit::domain::identity::GateKey>()
            .unwrap()]
    );

    // In-scope edges round-trip: E depends on both children; the boundary edge
    // (c1 → ext) and bracket edges did not travel.
    assert!(e_new.dependencies.contains(&map[&short(&s.c1)]));
    assert!(e_new.dependencies.contains(&map[&short(&s.c2)]));
    assert_eq!(c1_new.dependencies.len(), 0);

    // The recreated repository passes whole-repo validation.
    let report = fresh
        .executor
        .validation()
        .validate_repository_report()
        .unwrap()
        .rule_report();
    assert!(
        !report.has_errors(),
        "fresh repo failed validation: {:?}",
        report.findings
    );
}
