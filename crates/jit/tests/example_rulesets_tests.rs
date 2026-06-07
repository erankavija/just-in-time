//! Validity tests for the shipped EXAMPLE rulesets under `docs/examples/`.
//!
//! These prove the examples are REAL, not illustrative-only:
//!
//! 1. Every example `rules.toml` parses through the production [`RuleSet::load`]
//!    loader (with the schema root pointed at the example directory, so any
//!    referenced `schemas/*.json` is read and compiled).
//! 2. For each methodology, a sample COMPLIANT issue passes and a sample
//!    NON-COMPLIANT issue fails, evaluated through the real engine
//!    ([`evaluate_local`] for write-path rules, [`evaluate_graph`] for
//!    aggregate rules).
//!
//! The examples live in the repository's `docs/` tree, not in `.jit/`, so they
//! never activate on this repository; these tests load them directly from disk.

use std::path::{Path, PathBuf};

use jit::domain::{DocumentReference, Issue, State};
use jit::validation::graph::{evaluate_graph, GraphFinding};
use jit::validation::local::evaluate_local;
use jit::validation::rules::{Rule, RuleSet, Scope};

/// Absolute path to a `docs/examples/<name>` directory, resolved from the crate
/// manifest dir so the test is independent of the working directory.
fn example_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples")
        .join(name)
}

/// Load an example ruleset, resolving any `schemas/*.json` references relative to
/// the example directory itself.
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

/// Whether a local evaluation surfaced ANY finding (warning or blocking).
fn has_local_finding(issue: &Issue, set: &RuleSet) -> bool {
    let eval = evaluate_local(issue, set).expect("local evaluation must not error");
    !eval.findings().is_empty()
}

/// Graph findings attributable to a real issue (config errors would have
/// `issue_id == None` and must never appear for a valid example).
fn issue_graph_findings(rules: &[&Rule], issues: &[Issue]) -> Vec<GraphFinding> {
    let findings = evaluate_graph(
        rules,
        issues,
        &jit::type_hierarchy::HierarchyConfig::default(),
    );
    assert!(
        findings.iter().all(|f| !f.is_config_error()),
        "example graph rules must not produce config errors: {findings:?}"
    );
    findings
}

// ---------------------------------------------------------------------------
// 1. Every example ruleset parses (loader + schema compilation reachable).
// ---------------------------------------------------------------------------

#[test]
fn test_all_example_rulesets_load() {
    for name in ["sdd", "bug-repro", "release-checklist"] {
        let set = load_example(name);
        assert!(
            !set.rules.is_empty(),
            "example '{name}' must define at least one rule"
        );
    }
}

// ---------------------------------------------------------------------------
// 2a. SDD — local rules: compliant epic passes, non-compliant epic fails.
// ---------------------------------------------------------------------------

/// A well-formed SDD spec body: Requirements, Scenarios, and Success Criteria
/// sections, each shaped as `schemas/spec-body.json` requires.
fn sdd_compliant_body() -> String {
    "## Requirements\n\n\
        - REQ-01: the loader rejects mixed shorthand and raw schema\n\
        - REQ-02: a nicety we would like\n\n\
        ## Scenarios\n\n\
        - Given a rule mixing shorthand and a raw schema When the loader runs Then it errors\n\n\
        ## Success Criteria\n\n\
        - [hard] REQ-01: the loader rejects mixed shorthand and raw schema\n\
        - [aspirational] REQ-02: a nicety we would like\n"
        .to_string()
}

/// A compliant SDD epic: a well-formed spec body (Requirements / Scenarios /
/// Success Criteria with a `[hard]` criterion) and a correctly-formatted `req:`
/// id derived from the criteria.
fn sdd_compliant_epic() -> Issue {
    let mut epic = Issue::new("Validation engine".to_string(), sdd_compliant_body());
    epic.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
    epic
}

#[test]
fn test_sdd_compliant_epic_passes_local() {
    let set = load_example("sdd");
    assert!(
        !has_local_finding(&sdd_compliant_epic(), &set),
        "a well-formed SDD epic must produce no local findings"
    );
}

#[test]
fn test_sdd_missing_criteria_section_fails_local() {
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    epic.description = "## Goals\n\n- ship it\n".to_string();
    let eval = evaluate_local(&epic, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "an epic with no Success Criteria section must be blocked"
    );
}

#[test]
fn test_sdd_malformed_criteria_fail_local() {
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    // No [hard] marker on any item -> violates schemas/spec-body.json.
    epic.description = "## Success Criteria\n\n- just some freeform note\n".to_string();
    let eval = evaluate_local(&epic, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "criteria with no [hard]/[aspirational] marker must be blocked"
    );
}

#[test]
fn test_sdd_bad_req_id_format_fails_local() {
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    epic.labels = vec!["type:epic".to_string(), "req:not-a-req-id".to_string()];
    assert!(
        has_local_finding(&epic, &set),
        "a malformed req: id must produce a finding"
    );
}

// ---------------------------------------------------------------------------
// 2b. SDD — graph rules: coverage and reference-integrity.
// ---------------------------------------------------------------------------

#[test]
fn test_sdd_graph_coverage_and_reference_pass_when_satisfied() {
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let epic = sdd_compliant_epic();
    // A done child that depends on the epic and satisfies the one [hard]
    // criterion REQ-01.
    let mut child = Issue::new("implement REQ-01".to_string(), String::new());
    child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    child.dependencies = vec![epic.id.clone()];
    child.state = State::Done;

    let findings = issue_graph_findings(&rules, &[epic, child]);
    assert!(
        findings.is_empty(),
        "covered + resolvable references must yield no graph findings: {findings:?}"
    );
}

#[test]
fn test_sdd_graph_coverage_fails_when_hard_criterion_uncovered() {
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    // Epic with a [hard] criterion REQ-01 but no satisfying child at all.
    let epic = sdd_compliant_epic();
    let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
    assert!(
        findings.iter().any(|f| f.finding.message.contains("REQ-01")
            && f.finding.rule == "sdd-hard-criteria-covered"),
        "an uncovered [hard] criterion must be reported by coverage: {findings:?}"
    );
}

#[test]
fn test_sdd_graph_reference_warns_on_dangling_satisfies() {
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let epic = sdd_compliant_epic(); // declares req:REQ-01

    // Child satisfies a req id that is declared NOWHERE -> dangling reference.
    let mut child = Issue::new("rogue".to_string(), String::new());
    child.labels = vec!["type:task".to_string(), "satisfies:REQ-99".to_string()];
    child.dependencies = vec![epic.id.clone()];
    child.state = State::Done;

    let findings = issue_graph_findings(&rules, &[epic, child]);
    assert!(
        findings.iter().any(|f| f.finding.message.contains("REQ-99")
            && f.finding.rule == "sdd-satisfies-references-a-req"),
        "a dangling satisfies: reference must be reported: {findings:?}"
    );
}

#[test]
fn test_sdd_graph_stray_req_label_is_reported() {
    // Finding 1: a `req:` label that no child satisfies is stray/invented — it
    // does NOT correspond to any implemented criterion. The `sdd-req-is-satisfied`
    // rule (req -> satisfies integrity) catches it.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    // Epic declares the legitimate req:REQ-01 AND a stray req:REQ-77.
    let mut epic = sdd_compliant_epic();
    epic.labels.push("req:REQ-77".to_string());

    // A done child satisfies only REQ-01.
    let mut child = Issue::new("implement REQ-01".to_string(), String::new());
    child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    child.dependencies = vec![epic.id.clone()];
    child.state = State::Done;

    let findings = issue_graph_findings(&rules, &[epic, child]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.message.contains("REQ-77")
                && f.finding.rule == "sdd-req-is-satisfied"),
        "a stray req: label with no satisfying child must be reported: {findings:?}"
    );
    // The legitimate req:REQ-01 must NOT be reported as stray.
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "sdd-req-is-satisfied"
                && f.finding.message.contains("REQ-01")),
        "a satisfied req: label must not be flagged as stray: {findings:?}"
    );
}

#[test]
fn test_sdd_graph_req_resolution_is_scoped_to_linked_graph() {
    // `scope = "linked"`: an epic's `req:REQ-01` is NOT satisfied by a
    // `satisfies:REQ-01` on an UNRELATED issue (no dependency edge to the epic).
    // Cross-epic matches must not count, so the stray-req rule still fires.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    let epic = sdd_compliant_epic(); // declares req:REQ-01

    // An unrelated issue satisfies REQ-01 but is NOT linked to this epic.
    let mut unrelated = Issue::new("unrelated".to_string(), String::new());
    unrelated.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    unrelated.state = State::Done;
    // deliberately NO dependency edge to the epic

    let findings = issue_graph_findings(&rules, &[epic, unrelated]);
    assert!(
        findings
            .iter()
            .any(|f| f.finding.rule == "sdd-req-is-satisfied"
                && f.finding.message.contains("REQ-01")),
        "under linked scope an unrelated satisfies: must NOT resolve the epic's req: {findings:?}"
    );
}

#[test]
fn test_sdd_graph_missing_req_surfaces_via_coverage_chain() {
    // Finding 1 (other direction): a [hard] criterion for which NO `req:` is
    // declared and NO child satisfies it surfaces through the criteria -> satisfies
    // coverage rule. This is the enforceable criteria -> req chain: a hard
    // criterion cannot silently lack a derived label.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    // Epic body has [hard] REQ-01, but the epic declares NO req: label and there
    // is no satisfying child.
    let mut epic = sdd_compliant_epic();
    epic.labels = vec!["type:epic".to_string()];

    let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
    assert!(
        findings.iter().any(|f| f.finding.message.contains("REQ-01")
            && f.finding.rule == "sdd-hard-criteria-covered"),
        "a hard criterion with no derived/satisfying label must be reported: {findings:?}"
    );
}

// ---------------------------------------------------------------------------
// 2b'. SDD — local schema: requirement/scenario STRUCTURE is validated.
// ---------------------------------------------------------------------------

#[test]
fn test_sdd_missing_requirements_section_fails_local() {
    // Finding 2: a spec missing the Requirements section fails schema validation.
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    epic.description = "## Scenarios\n\n\
        - Given x When y Then z\n\n\
        ## Success Criteria\n\n\
        - [hard] REQ-01: do the thing\n"
        .to_string();
    let eval = evaluate_local(&epic, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "a spec with no Requirements section must be blocked"
    );
}

#[test]
fn test_sdd_missing_scenarios_section_fails_local() {
    // Finding 2: a spec missing the Scenarios section fails schema validation.
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    epic.description = "## Requirements\n\n\
        - REQ-01: do the thing\n\n\
        ## Success Criteria\n\n\
        - [hard] REQ-01: do the thing\n"
        .to_string();
    let eval = evaluate_local(&epic, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "a spec with no Scenarios section must be blocked"
    );
}

#[test]
fn test_sdd_malformed_requirement_item_fails_local() {
    // Finding 2: a Requirements item not shaped `REQ-N: ...` fails the schema.
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    epic.description = "## Requirements\n\n\
        - this is not a REQ id at all\n\n\
        ## Scenarios\n\n\
        - Given x When y Then z\n\n\
        ## Success Criteria\n\n\
        - [hard] REQ-01: do the thing\n"
        .to_string();
    let eval = evaluate_local(&epic, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "a malformed Requirements item must be blocked"
    );
}

#[test]
fn test_sdd_malformed_scenario_item_fails_local() {
    // Finding 2: a Scenarios item not in Given/When/Then shape fails the schema.
    let set = load_example("sdd");
    let mut epic = sdd_compliant_epic();
    epic.description = "## Requirements\n\n\
        - REQ-01: do the thing\n\n\
        ## Scenarios\n\n\
        - just a freeform note, no given/when/then\n\n\
        ## Success Criteria\n\n\
        - [hard] REQ-01: do the thing\n"
        .to_string();
    let eval = evaluate_local(&epic, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "a malformed Scenarios item must be blocked"
    );
}

#[test]
fn test_sdd_well_formed_structure_passes_local() {
    // Finding 2: a fully well-formed spec (Requirements + Scenarios + Success
    // Criteria, all correctly shaped) passes the structural schema.
    let set = load_example("sdd");
    assert!(
        !has_local_finding(&sdd_compliant_epic(), &set),
        "a well-formed Requirements/Scenarios/Success-Criteria spec must pass"
    );
}

// ---------------------------------------------------------------------------
// 2c. bug-repro (non-SDD) — local rules.
// ---------------------------------------------------------------------------

#[test]
fn test_bug_with_reproduction_steps_passes() {
    let set = load_example("bug-repro");
    let body = "## Reproduction\n\n\
        - run `jit validate`\n\
        - observe the panic\n";
    let mut bug = Issue::new("crash on validate".to_string(), body.to_string());
    bug.labels = vec!["type:bug".to_string()];
    assert!(
        !has_local_finding(&bug, &set),
        "a bug with reproduction steps must pass"
    );
}

#[test]
fn test_bug_without_reproduction_fails() {
    let set = load_example("bug-repro");
    let mut bug = Issue::new("crash".to_string(), "## Notes\n\n- it broke\n".to_string());
    bug.labels = vec!["type:bug".to_string()];
    let eval = evaluate_local(&bug, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "a bug with no Reproduction section must be blocked"
    );
}

// ---------------------------------------------------------------------------
// 2d. release-checklist (non-SDD) — local + graph rules.
// ---------------------------------------------------------------------------

fn release_with_notes_doc() -> Issue {
    let body = "## Checklist\n\n- bump version\n- tag\n";
    let mut release = Issue::new("v1.0.0".to_string(), body.to_string());
    release.labels = vec!["type:release".to_string()];
    release.documents = vec![
        DocumentReference::new("docs/release-notes-1.0.0.md".to_string())
            .with_type("release-notes".to_string()),
    ];
    release
}

#[test]
fn test_release_with_notes_doc_passes_local() {
    let set = load_example("release-checklist");
    assert!(
        !has_local_finding(&release_with_notes_doc(), &set),
        "a release with a checklist + release-notes doc must pass local rules"
    );
}

#[test]
fn test_release_without_notes_doc_fails_local() {
    let set = load_example("release-checklist");
    let mut release = release_with_notes_doc();
    release.documents.clear();
    let eval = evaluate_local(&release, &set).unwrap();
    assert!(
        eval.is_blocking(),
        "a release with no release-notes document must be blocked"
    );
}

#[test]
fn test_release_graph_requires_qa_signoff_dependency() {
    let set = load_example("release-checklist");
    let rules = graph_rules(&set);

    // A release with no qa-signoff dependency -> graph violation.
    let release = release_with_notes_doc();
    let violations = issue_graph_findings(&rules, std::slice::from_ref(&release));
    assert!(
        violations
            .iter()
            .any(|f| f.finding.rule == "release-depends-on-qa-signoff"),
        "a release with no QA sign-off dependency must be reported: {violations:?}"
    );

    // Add the qa-signoff dependency -> the graph rule is satisfied.
    let qa = {
        let mut qa = Issue::new("QA sign-off".to_string(), String::new());
        qa.labels = vec!["type:qa-signoff".to_string()];
        qa
    };
    let mut release = release_with_notes_doc();
    release.dependencies = vec![qa.id.clone()];
    let ok = issue_graph_findings(&rules, &[qa, release]);
    assert!(
        ok.is_empty(),
        "a release that depends on QA sign-off must pass the graph rule: {ok:?}"
    );
}
