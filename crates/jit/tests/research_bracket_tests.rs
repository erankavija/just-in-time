//! Plan-before-fan-out **bracket** demonstration for the RESEARCH example ruleset
//! (epic `2fbd2a82`, task T8, design D11).
//!
//! These tests live in their own file (NOT `example_rulesets_tests.rs`, which a
//! sibling task also edits) and prove the bracket works on a NON-SOFTWARE
//! hierarchy: the breakable container here is a research `goal`, NOT an `epic`.
//! There is no software container type anywhere in the research example — only
//! research methodology vocabulary plus the two function-typed bracket nodes
//! (`planning` / `breakdown`). This is the agnosticism proof of D11: the SAME
//! engine drives the SDD bracket (`sdd_bracket_tests.rs`) and this research one,
//! reading the breakable/bracket type names from each example's config.
//!
//! The research example declares:
//!
//!   * a **preview** coverage rule keyed on the `type:breakdown` node `B`, which
//!     resolves its criteria-bearing container goal `C` via the `brackets:<C-id>`
//!     label (`container-from-label = "brackets"`) and OMITS `child-state` (any
//!     state counts) — so an uncovered `[hard]` hypothesis is BLOCKED at the
//!     breakdown gate while the drafted experiments still sit in backlog;
//!   * `child-type-exclude = ["planning", "breakdown"]` drops the bracket nodes
//!     from coverage candidates AND halts the transitive walk at them, so the
//!     coverage tally is exactly the experiment interior between `C` and `B`;
//!   * the existing **closure** rule (`child-state = "done"`) is untouched in
//!     semantics (only the same no-op-in-a-plain-goal `child-type-exclude` added).
//!
//! The bracket spine modeled here mirrors the design doc
//! (`dev/active/planning-bracket-design.md`):
//!
//! ```text
//!   C(goal) ──dep→ experiment ──dep→ B(type:breakdown) ──dep→ P(type:planning)
//! ```
//!
//! Containment in research is `child-link = "dependencies"` (the goal depends on
//! its experiments), so coverage walks `C`'s dependency closure.

use std::path::{Path, PathBuf};

use jit::domain::{ContentFormat, Issue, State};
use jit::validation::graph::{evaluate_graph, GraphFinding};
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
    );
    assert!(
        findings.iter().all(|f| !f.is_config_error()),
        "example graph rules must not produce config errors: {findings:?}"
    );
    findings
}

/// A well-formed goal body with a single `[hard]` hypothesis H-1.
fn goal_body() -> String {
    "## Hypotheses\n\n\
        - [hard] H-1: increasing training data improves accuracy\n\
        - [exploratory] H-2: model size is the primary performance driver\n\n\
        ## Success Criteria\n\n\
        - accuracy exceeds 95% on the held-out test set\n"
        .to_string()
}

/// The breakable container `C` (a research goal) declaring `hyp:H-1`.
fn breakable_container() -> Issue {
    let mut goal = Issue::new("Improve accuracy".to_string(), goal_body());
    goal.labels = vec!["type:goal".to_string(), "hyp:H-1".to_string()];
    goal
}

// ---------------------------------------------------------------------------
// The example declares the bracket: preview rule, both gates wired in config —
// all on a non-software hierarchy (goal, no epic).
// ---------------------------------------------------------------------------

#[test]
fn test_research_example_declares_preview_coverage_rule() {
    // The preview coverage rule must exist, be graph-scoped, key on the
    // breakdown node, and OMIT child-state (the closure rule keeps it). It must
    // resolve its container via `brackets:` and exclude the bracket types.
    let dir = example_dir("research");
    let rules_toml = std::fs::read_to_string(dir.join("rules.toml"))
        .expect("research rules.toml must be readable");

    let set = load_example("research");
    let preview = set
        .rules
        .iter()
        .find(|r| r.name == "research-hypotheses-covered-preview")
        .expect(
            "research example must define the preview rule research-hypotheses-covered-preview",
        );
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
fn test_research_example_declares_planning_config_no_epic() {
    // The container/bracket TYPE NAMES and the wired gate presets live in the
    // example's OWN config (config.toml), never in engine Rust. This proves the
    // example declares its breakable container, the two bracket node types, and
    // wires both gate presets (plan-review on P, coverage-preview on B) — and
    // that the breakable container is a research `goal`, NOT a software `epic`.
    let dir = example_dir("research");
    let config_toml = std::fs::read_to_string(dir.join("config.toml"))
        .expect("research example must ship a config.toml declaring the bracket");
    let cfg: toml::Value =
        toml::from_str(&config_toml).expect("research example config.toml must parse");

    let planning = cfg
        .get("planning")
        .expect("config.toml must declare a [planning] block");
    assert_eq!(
        planning.get("breakable_types").and_then(|v| v.as_array()),
        Some(&vec![toml::Value::String("goal".to_string())]),
        "the research example declares goal (NOT epic) as its breakable container"
    );
    assert_eq!(
        planning.get("planning_type").and_then(|v| v.as_str()),
        Some("planning"),
        "the planning bracket node type"
    );
    assert_eq!(
        planning.get("breakdown_type").and_then(|v| v.as_str()),
        Some("breakdown"),
        "the breakdown bracket node type"
    );
    assert_eq!(
        planning.get("plan_gate_preset").and_then(|v| v.as_str()),
        Some("plan-review"),
        "plan-review wired on the planning node",
    );
    assert_eq!(
        planning
            .get("coverage_gate_preset")
            .and_then(|v| v.as_str()),
        Some("coverage-preview"),
        "coverage-preview wired on the breakdown node",
    );

    // The bracket node types must also be declared in the type hierarchy so they
    // are valid children of the breakable container.
    let types = cfg
        .get("type_hierarchy")
        .and_then(|t| t.get("types"))
        .expect("config.toml must declare [type_hierarchy].types");
    assert!(
        types.get("planning").is_some() && types.get("breakdown").is_some(),
        "the bracket node types must be declared in the type hierarchy: {types:?}"
    );
    // Agnosticism proof: NO software container types in the declared hierarchy.
    assert!(
        types.get("epic").is_none() && types.get("milestone").is_none(),
        "the research example must declare NO epic/milestone type: {types:?}"
    );
}

// ---------------------------------------------------------------------------
// Demonstration: an uncovered [hard] hypothesis is BLOCKED at the breakdown gate.
// ---------------------------------------------------------------------------

#[test]
fn test_research_uncovered_hard_hypothesis_blocked_at_breakdown_gate() {
    // Bracket spine: C(goal, [hard] H-1) ──dep→ experiment ──dep→ B(type:breakdown).
    // The drafted experiment does NOT carry tests:H-1, so the preview coverage
    // rule (keyed on B, container resolved via brackets:, child-state omitted)
    // must report H-1 uncovered — the breakdown gate BLOCKS.
    //
    // Crucially the experiment sits in backlog (not done): the preview rule omits
    // child-state, so "mapping exists in ANY state" is what's checked here.
    let set = load_example("research");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut experiment = Issue::new("draft experiment".to_string(), String::new());
    experiment.labels = vec!["type:experiment".to_string()]; // does NOT test H-1
    experiment.state = State::Backlog;
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.id),
    ];

    // Containment spine: C depends on experiment; experiment depends on B.
    container.dependencies = vec![experiment.id.clone()];
    experiment.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, experiment, breakdown]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.rule == "research-hypotheses-covered-preview"
                && f.finding.message.contains("H-1")),
        "an uncovered [hard] hypothesis must be reported by the preview rule at \
         the breakdown gate: {findings:?}"
    );
}

#[test]
fn test_research_preview_passes_when_backlog_experiment_carries_mapping() {
    // Same spine, but the (still-backlog) experiment carries tests:H-1. The
    // preview rule OMITS child-state, so a mapping in ANY state (backlog here)
    // satisfies it -> the breakdown gate passes. This is the preview vs closure
    // distinction: closure would still require child-state = "done".
    let set = load_example("research");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut experiment = Issue::new("draft experiment".to_string(), String::new());
    experiment.labels = vec!["type:experiment".to_string(), "tests:H-1".to_string()];
    experiment.state = State::Backlog; // NOT done
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.id),
    ];
    container.dependencies = vec![experiment.id.clone()];
    experiment.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, experiment, breakdown]);
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "research-hypotheses-covered-preview"),
        "a backlog experiment carrying tests: must satisfy the preview rule \
         (child-state omitted): {findings:?}"
    );
}

#[test]
fn test_research_preview_excludes_bracket_types_and_halts_walk() {
    // child-type-exclude = ["planning", "breakdown"]: the walk from C must halt
    // at B and never credit a tests: label sitting on the planning node P BEYOND
    // the breakdown boundary. So even though P carries tests:H-1, the hypothesis
    // reads uncovered at the breakdown gate.
    let set = load_example("research");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut experiment = Issue::new("draft experiment".to_string(), String::new());
    experiment.labels = vec!["type:experiment".to_string()]; // no tests: here
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.id),
    ];
    // P, beyond the boundary, is the only carrier of tests:H-1.
    let mut plan = Issue::new("plan".to_string(), String::new());
    plan.labels = vec!["type:planning".to_string(), "tests:H-1".to_string()];

    container.dependencies = vec![experiment.id.clone()];
    experiment.dependencies = vec![breakdown.id.clone()];
    breakdown.dependencies = vec![plan.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, experiment, breakdown, plan]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.rule == "research-hypotheses-covered-preview"
                && f.finding.message.contains("H-1")),
        "the walk must halt at the breakdown boundary, leaving H-1 uncovered \
         despite P testing it beyond the boundary: {findings:?}"
    );
}

#[test]
fn test_research_preview_credits_non_sink_experiment_interior() {
    // Coverage traversal is transitive: a tests: on a NON-SINK experiment (deep
    // in the interior, not directly adjacent to C) is still credited.
    // Spine: C ──dep→ exp_a ──dep→ exp_b(tests:H-1) ──dep→ B.
    let set = load_example("research");
    let rules = graph_rules(&set);

    let mut container = breakable_container();
    let mut exp_a = Issue::new("exp a (sink)".to_string(), String::new());
    exp_a.labels = vec!["type:experiment".to_string()];
    let mut exp_b = Issue::new("exp b (interior)".to_string(), String::new());
    exp_b.labels = vec!["type:experiment".to_string(), "tests:H-1".to_string()];
    let mut breakdown = Issue::new("breakdown".to_string(), String::new());
    breakdown.labels = vec![
        "type:breakdown".to_string(),
        format!("brackets:{}", container.id),
    ];

    container.dependencies = vec![exp_a.id.clone()];
    exp_a.dependencies = vec![exp_b.id.clone()];
    exp_b.dependencies = vec![breakdown.id.clone()];

    let findings = issue_graph_findings(&rules, &[container, exp_a, exp_b, breakdown]);
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "research-hypotheses-covered-preview"),
        "a non-sink interior experiment testing H-1 must be credited \
         (transitive walk): {findings:?}"
    );
}

// ---------------------------------------------------------------------------
// The bracket additions are ADDITIVE: the closure rule still bites at done.
// ---------------------------------------------------------------------------

#[test]
fn test_research_closure_rule_still_requires_done_experiment() {
    // The existing closure rule (research-hard-hypotheses-covered-at-done,
    // child-state = "done") must be unchanged in semantics: at done, a backlog
    // experiment carrying tests:H-1 does NOT satisfy it (closure needs mapping
    // DONE). This is the preview/closure split — preview accepts any state,
    // closure requires done.
    let set = load_example("research");
    let rules = graph_rules(&set);

    let mut goal = breakable_container();
    goal.state = State::Done;
    // An experiment testing H-1 but still in backlog (not done).
    let mut experiment = Issue::new("experiment".to_string(), String::new());
    experiment.labels = vec!["type:experiment".to_string(), "tests:H-1".to_string()];
    experiment.state = State::Backlog;
    goal.dependencies.push(experiment.id.clone());

    let findings = issue_graph_findings(&rules, &[goal, experiment]);
    assert!(
        findings.iter().any(
            |f| f.finding.rule == "research-hard-hypotheses-covered-at-done"
                && f.finding.message.contains("H-1")
        ),
        "the closure rule must still require a DONE experiment at the done \
         transition: {findings:?}"
    );
}
