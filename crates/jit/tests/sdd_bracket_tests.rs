//! Plan-before-fan-out **bracket** demonstration for the SDD example ruleset
//! (epic `2fbd2a82`, task T7).
//!
//! These tests live in their own file (NOT `example_rulesets_tests.rs`, which a
//! sibling task also edits) and prove the SDD example wires the bracket:
//!
//!   * the example declares a **preview** coverage rule keyed on the
//!     `type:breakdown` node `B`, which resolves its criteria-bearing container
//!     `C` via the `brackets:<C-short-id>` label (`container-from-label = "brackets"`)
//!     and OMITS `child-state` (any state counts) — so an uncovered `[hard]`
//!     criterion is BLOCKED at the breakdown gate while the drafted children
//!     still sit in Backlog;
//!   * `child-type-exclude = ["planning", "breakdown"]` drops the bracket nodes
//!     from coverage candidates AND halts the transitive walk at them, so the
//!     coverage tally is exactly the impl interior between `C` and `B`;
//!   * the existing **closure** rule (`child-state = "done"`) is untouched.
//!
//! The bracket spine modeled here mirrors the design doc
//! (`dev/active/planning-bracket-design.md`):
//!
//! ```text
//!   C(epic) ──dep→ impl ──dep→ B(type:breakdown) ──dep→ P(type:planning)
//! ```
//!
//! Containment in SDD is `child-link = "dependencies"` (the epic depends on its
//! children), so coverage walks `C`'s dependency closure.

use std::path::{Path, PathBuf};

use jit::domain::{ContentFormat, Issue, State};
use jit::validation::graph::{evaluate_graph, DriftInputs, GraphFinding};
use jit::validation::rules::{Rule, RuleSet, Scope};

/// Absolute path to a `docs/examples/<name>` directory, resolved from the crate
/// manifest dir so the test is independent of the working directory.
fn example_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples")
        .join(name)
}

/// Load an example ruleset, resolving any `schemas/*.json` references relative to
/// the example directory itself. (Minimal local replica of the helper in
/// `example_rulesets_tests.rs`, kept here so this file owns its dependencies.)
fn load_example(name: &str) -> RuleSet {
    let dir = example_dir(name);
    RuleSet::load(&dir)
        .unwrap_or_else(|e| panic!("example '{name}' rules.toml must load cleanly: {e}"))
}

/// The graph-scope rules of a set, as the slice [`evaluate_graph`] expects.
fn graph_rules(set: &RuleSet) -> Vec<&Rule> {
    set.rules
        .iter()
        .filter(|r| r.scope == Scope::Graph)
        .collect()
}

/// A fixed clock instant for deterministic example evaluation.
fn fixed_now() -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(2026, 6, 17, 12, 0, 0).unwrap()
}

/// Evaluate the graph rules over `issues`, asserting no config-error findings
/// (those would carry `issue_id == None` and indicate a malformed example).
fn issue_graph_findings(rules: &[&Rule], issues: &[Issue]) -> Vec<GraphFinding> {
    let findings = evaluate_graph(
        rules,
        issues,
        &jit::type_hierarchy::HierarchyConfig::default(),
        ContentFormat::Markdown,
        fixed_now(),
        &std::collections::HashMap::new(),
        &DriftInputs::none(),
    );
    assert!(
        findings.iter().all(|f| !f.is_config_error()),
        "example graph rules must not produce config errors: {findings:?}"
    );
    findings
}

/// A well-formed SDD spec body with a single `[hard]` criterion REQ-01.
fn sdd_spec_body() -> String {
    "## Requirements\n\n\
        - REQ-01: the loader rejects mixed shorthand and raw schema\n\n\
        ## Scenarios\n\n\
        - Given a rule mixing shorthand and a raw schema When the loader runs Then it errors\n\n\
        ## Success Criteria\n\n\
        - [hard] REQ-01: the loader rejects mixed shorthand and raw schema\n"
        .to_string()
}

/// The breakable container `C` (an SDD epic) declaring `req:REQ-01`.
fn breakable_container() -> Issue {
    let mut epic = Issue::new("Validation engine".to_string(), sdd_spec_body());
    epic.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
    epic
}

// ---------------------------------------------------------------------------
// The example declares the bracket: preview rule, both gates wired in config.
// ---------------------------------------------------------------------------

#[test]
fn test_sdd_example_declares_preview_coverage_rule() {
    // The preview coverage rule must exist, be graph-scoped, key on the
    // breakdown node, and OMIT child-state (the closure rule keeps it). It must
    // resolve its container via `brackets:` and exclude the bracket types.
    let dir = example_dir("sdd");
    let rules_toml =
        std::fs::read_to_string(dir.join("rules.toml")).expect("sdd rules.toml must be readable");

    let set = load_example("sdd");
    let preview = set
        .rules
        .iter()
        .find(|r| r.name == "sdd-coverage-preview")
        .expect("sdd example must define the preview coverage rule sdd-coverage-preview");
    assert_eq!(preview.scope, Scope::Graph);
    assert_eq!(preview.severity, jit::validation::rules::Severity::Error);

    // Structural assertions on the authored TOML: the preview rule is keyed on
    // the breakdown node and resolves its container via the brackets: label,
    // and excludes the bracket types from coverage candidates.
    assert!(
        rules_toml.contains("container-from-label = \"brackets\""),
        "preview rule must resolve its container via the brackets: label"
    );
    assert!(
        rules_toml.contains("child-type-exclude"),
        "the example must use child-type-exclude to drop bracket types"
    );
    assert!(
        rules_toml.contains("\"planning\"") && rules_toml.contains("\"breakdown\""),
        "child-type-exclude must drop both bracket types (planning + breakdown)"
    );
}

#[test]
fn test_sdd_example_declares_planning_template() {
    // The container/bracket TYPE NAMES and the wired gate presets live in the
    // example's OWN `plan` template (templates.toml), never in engine Rust. This
    // proves the example declares its breakable container, the two bracket node
    // types, and wires both gate presets (plan-review on P, coverage-preview on B).
    let dir = example_dir("sdd");
    let templates_toml = std::fs::read_to_string(dir.join("templates.toml"))
        .expect("sdd example must ship a templates.toml declaring the bracket");
    let reg = jit::templates::TemplateRegistry::from_toml_str(
        &templates_toml,
        &["epic", "planning", "breakdown"],
    )
    .expect("sdd example templates.toml must load and validate");

    let plan = reg
        .get("plan")
        .expect("templates.toml must declare a `plan` template");
    assert_eq!(
        plan.applies_to,
        vec!["epic".to_string()],
        "the SDD example declares epic as its breakable container"
    );
    assert_eq!(
        plan.planning_type(),
        Some("planning"),
        "the planning bracket node type"
    );
    assert_eq!(
        plan.breakdown_type(),
        Some("breakdown"),
        "the breakdown bracket node type"
    );
    assert_eq!(
        plan.planning_node().map(|n| n.gates.as_slice()),
        Some(["plan-review".to_string()].as_slice()),
        "plan-review wired on the planning node",
    );
    assert!(
        plan.breakdown_node()
            .is_some_and(|n| n.gates.iter().any(|g| g == "coverage-preview")),
        "coverage-preview wired on the breakdown node",
    );

    // The config still declares the bracket node types in the type hierarchy so
    // they are valid children of the breakable container.
    let config_toml = std::fs::read_to_string(dir.join("config.toml"))
        .expect("sdd example must ship a config.toml");
    let cfg: toml::Value =
        toml::from_str(&config_toml).expect("sdd example config.toml must parse");
    let types = cfg
        .get("type_hierarchy")
        .and_then(|t| t.get("types"))
        .expect("config.toml must declare [type_hierarchy].types");
    assert!(
        types.get("planning").is_some() && types.get("breakdown").is_some(),
        "the bracket node types must be declared in the type hierarchy: {types:?}"
    );
}

// ---------------------------------------------------------------------------
// Demonstration: an uncovered [hard] criterion is BLOCKED at the breakdown gate.
// ---------------------------------------------------------------------------

#[test]
fn test_sdd_uncovered_hard_criterion_blocked_at_breakdown_gate() {
    // Bracket spine: C(epic, [hard] REQ-01) ──dep→ impl ──dep→ B(type:breakdown).
    // The drafted impl child does NOT carry satisfies:REQ-01, so the preview
    // coverage rule (keyed on B, container resolved via brackets:, child-state
    // omitted) must report REQ-01 uncovered — the breakdown gate BLOCKS.
    //
    // Crucially the impl child sits in Backlog (not done): the preview rule
    // omits child-state, so "mapping exists in ANY state" is what's checked here.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut impl_node = Issue::new("draft impl".to_string(), String::new());
    impl_node.labels = vec!["type:task".to_string()]; // does NOT satisfy REQ-01
    impl_node.state = State::Backlog;
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.short_id()),
    ];
    // Breakdown is underway (the preview rule is state-gated to in_progress/
    // gated/done, so a backlog B that has not started breakdown stays silent).
    breakdown.state = State::InProgress;

    // Containment spine: C depends on impl; impl depends on B.
    container.dependencies = vec![impl_node.id.clone()];
    impl_node.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, impl_node, breakdown]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.rule == "sdd-coverage-preview"
                && f.finding.message.contains("REQ-01")),
        "an uncovered [hard] criterion must be reported by the preview rule at \
         the breakdown gate: {findings:?}"
    );
}

#[test]
fn test_sdd_preview_silent_while_breakdown_node_in_backlog() {
    // A freshly-applied bracket leaves B in Backlog: it depends on the planning
    // node and breakdown has not started, so there is nothing to cover yet. The
    // preview rule is state-gated (in_progress/gated/done), so even an uncovered
    // [hard] criterion produces NO finding while B sits in backlog. This is the
    // lifecycle gate that stops coverage failing before breakdown begins.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut impl_node = Issue::new("draft impl".to_string(), String::new());
    impl_node.labels = vec!["type:task".to_string()]; // does NOT satisfy REQ-01
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.short_id()),
    ];
    breakdown.state = State::Backlog; // breakdown not started

    container.dependencies = vec![impl_node.id.clone()];
    impl_node.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, impl_node, breakdown]);
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "sdd-coverage-preview"),
        "coverage-preview must stay silent while the breakdown node is in backlog \
         (breakdown not started): {findings:?}"
    );
}

#[test]
fn test_sdd_preview_passes_when_backlog_child_carries_mapping() {
    // Same spine, but the (still-Backlog) impl child carries satisfies:REQ-01.
    // The preview rule OMITS child-state, so a mapping in ANY state (Backlog
    // here) satisfies it -> the breakdown gate passes. This is the preview vs
    // closure distinction: closure would still require child-state = "done".
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut impl_node = Issue::new("draft impl".to_string(), String::new());
    impl_node.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    impl_node.state = State::Backlog; // NOT done
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.short_id()),
    ];
    // Breakdown is underway (the preview rule is state-gated to in_progress/
    // gated/done, so a backlog B that has not started breakdown stays silent).
    breakdown.state = State::InProgress;
    container.dependencies = vec![impl_node.id.clone()];
    impl_node.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, impl_node, breakdown]);
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "sdd-coverage-preview"),
        "a Backlog child carrying satisfies: must satisfy the preview rule \
         (child-state omitted): {findings:?}"
    );
}

#[test]
fn test_sdd_preview_excludes_bracket_types_and_halts_walk() {
    // child-type-exclude = ["planning", "breakdown"]: the walk from C must halt
    // at B and never credit a satisfies: label sitting on the planning node P
    // BEYOND the breakdown boundary. So even though P carries satisfies:REQ-01,
    // the criterion reads uncovered at the breakdown gate.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut impl_node = Issue::new("draft impl".to_string(), String::new());
    impl_node.labels = vec!["type:task".to_string()]; // no satisfies here
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.short_id()),
    ];
    // Breakdown is underway (the preview rule is state-gated to in_progress/
    // gated/done, so a backlog B that has not started breakdown stays silent).
    breakdown.state = State::InProgress;
    // P, beyond the boundary, is the only carrier of satisfies:REQ-01.
    let mut plan = Issue::new("plan".to_string(), String::new());
    plan.labels = vec!["type:planning".to_string(), "satisfies:REQ-01".to_string()];

    container.dependencies = vec![impl_node.id.clone()];
    impl_node.dependencies = vec![breakdown.id.clone()];
    breakdown.dependencies = vec![plan.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, impl_node, breakdown, plan]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.rule == "sdd-coverage-preview"
                && f.finding.message.contains("REQ-01")),
        "the walk must halt at the breakdown boundary, leaving REQ-01 uncovered \
         despite P satisfying it beyond the boundary: {findings:?}"
    );
}

#[test]
fn test_sdd_preview_credits_non_sink_impl_interior() {
    // Coverage traversal is transitive: a satisfies: on a NON-SINK impl issue
    // (deep in the interior, not directly adjacent to C) is still credited.
    // Spine: C ──dep→ impl_a ──dep→ impl_b(satisfies:REQ-01) ──dep→ B.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut impl_a = Issue::new("impl a (sink)".to_string(), String::new());
    impl_a.labels = vec!["type:task".to_string()];
    let mut impl_b = Issue::new("impl b (interior)".to_string(), String::new());
    impl_b.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.short_id()),
    ];
    // Breakdown is underway (the preview rule is state-gated to in_progress/
    // gated/done, so a backlog B that has not started breakdown stays silent).
    breakdown.state = State::InProgress;

    container.dependencies = vec![impl_a.id.clone()];
    impl_a.dependencies = vec![impl_b.id.clone()];
    impl_b.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, impl_a, impl_b, breakdown]);
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "sdd-coverage-preview"),
        "a non-sink interior impl issue satisfying REQ-01 must be credited \
         (transitive walk): {findings:?}"
    );
}

// ---------------------------------------------------------------------------
// The bracket additions are ADDITIVE: the closure rule still bites at done.
// ---------------------------------------------------------------------------

#[test]
fn test_sdd_closure_rule_still_requires_done_child() {
    // The existing closure rule (sdd-hard-criteria-covered, child-state = "done")
    // must be unchanged: at done, a Backlog child carrying satisfies:REQ-01 does
    // NOT satisfy it (closure needs mapping DONE). This is the preview/closure
    // split — preview accepts any state, closure requires done.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let mut epic = breakable_container();
    epic.state = State::Done;
    // A child satisfying REQ-01 but still in Backlog (not done).
    let mut child = Issue::new("impl".to_string(), String::new());
    child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    child.state = State::Backlog;
    epic.dependencies.push(child.id.clone());

    let findings = issue_graph_findings(&rules, &[epic, child]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.rule == "sdd-hard-criteria-covered"
                && f.finding.message.contains("REQ-01")),
        "the closure rule must still require a DONE child at the done transition: {findings:?}"
    );
}
