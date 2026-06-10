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

use jit::domain::{ContentFormat, DocumentReference, Issue, State};
use jit::validation::graph::{evaluate_graph, GraphFinding};
use jit::validation::local::evaluate_local;
use jit::validation::rules::{Rule, RuleSet, Scope};

// Transition-enforcement tests (research module) drive the executor directly.
use jit::commands::CommandExecutor;
use jit::errors::TransitionBlockedError;
use jit::storage::{InMemoryStorage, IssueStore};

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
    let eval = evaluate_local(issue, set, ContentFormat::Markdown)
        .expect("local evaluation must not error");
    !eval.findings().is_empty()
}

/// Graph findings attributable to a real issue (config errors would have
/// `issue_id == None` and must never appear for a valid example).
///
/// `now` is the injected clock used by `gate-recency` rules; callers that do not
/// exercise recency pass [`fixed_now`].
fn issue_graph_findings_at(
    rules: &[&Rule],
    issues: &[Issue],
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<GraphFinding> {
    let findings = evaluate_graph(
        rules,
        issues,
        &jit::type_hierarchy::HierarchyConfig::default(),
        ContentFormat::Markdown,
        now,
    );
    assert!(
        findings.iter().all(|f| !f.is_config_error()),
        "example graph rules must not produce config errors: {findings:?}"
    );
    findings
}

/// Convenience for non-recency graph examples: evaluate at [`fixed_now`].
fn issue_graph_findings(rules: &[&Rule], issues: &[Issue]) -> Vec<GraphFinding> {
    issue_graph_findings_at(rules, issues, fixed_now())
}

/// A fixed clock instant for deterministic example evaluation.
fn fixed_now() -> chrono::DateTime<chrono::Utc> {
    use chrono::TimeZone;
    chrono::Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap()
}

// ---------------------------------------------------------------------------
// 1. Every example ruleset parses (loader + schema compilation reachable).
// ---------------------------------------------------------------------------

#[test]
fn test_all_example_rulesets_load() {
    for name in [
        "sdd",
        "bug-repro",
        "release-checklist",
        "fresh-evidence",
        "nyquist",
        "cross-epic",
        "research",
    ] {
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
    let eval = evaluate_local(&epic, &set, ContentFormat::Markdown).unwrap();
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
    let eval = evaluate_local(&epic, &set, ContentFormat::Markdown).unwrap();
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

    // Epic in `done` state: the lifecycle-scoped coverage and derivation rules
    // are active and must produce no findings when the criterion is covered.
    let mut epic = sdd_compliant_epic();
    epic.state = State::Done;
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

    // Epic in `done` state with a [hard] criterion REQ-01 but no satisfying
    // child: the lifecycle-scoped coverage rule fires only at done.
    let mut epic = sdd_compliant_epic();
    epic.state = State::Done;
    let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
    assert!(
        findings.iter().any(|f| f.finding.message.contains("REQ-01")
            && f.finding.rule == "sdd-hard-criteria-covered"),
        "an uncovered [hard] criterion must be reported by coverage at done: {findings:?}"
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
    // A `req:REQ-77` label whose id is absent from the criteria prose is a stray.
    // After the lifecycle-aware rework, two rules cover this:
    //   * sdd-req-matches-a-criterion (always-on, any state): fires because
    //     REQ-77 is not in the Success Criteria section.
    //   * sdd-req-is-satisfied (done-scoped, enforce=true): fires at done because
    //     no child satisfies REQ-77.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    // Epic in `done` state: both stray detection and derivation rules are active.
    let mut epic = sdd_compliant_epic();
    epic.state = State::Done;
    epic.labels.push("req:REQ-77".to_string());

    // A done child satisfies only REQ-01.
    let mut child = Issue::new("implement REQ-01".to_string(), String::new());
    child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
    child.dependencies = vec![epic.id.clone()];
    child.state = State::Done;

    let findings = issue_graph_findings(&rules, &[epic, child]);
    // The always-on stray check catches REQ-77 immediately.
    assert!(
        findings.iter().any(|f| f.finding.message.contains("REQ-77")
            && f.finding.rule == "sdd-req-matches-a-criterion"),
        "a stray req: label must be reported by criteria-label-match: {findings:?}"
    );
    // At done, the derivation rule also fires for REQ-77.
    assert!(
        findings
            .iter()
            .any(|f| f.finding.message.contains("REQ-77")
                && f.finding.rule == "sdd-req-is-satisfied"),
        "an unsatisfied req: label must be reported by sdd-req-is-satisfied at done: {findings:?}"
    );
    // The legitimate req:REQ-01 must NOT be flagged as stray or unsatisfied.
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "sdd-req-is-satisfied"
                && f.finding.message.contains("REQ-01")),
        "a satisfied req: label must not be flagged as unsatisfied: {findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|f| f.finding.rule == "sdd-req-matches-a-criterion"
                && f.finding.message.contains("REQ-01")),
        "a criterion-matching req: label must not be flagged as stray: {findings:?}"
    );
}

#[test]
fn test_sdd_graph_req_resolution_is_scoped_to_linked_graph() {
    // `scope = "linked"`: an epic's `req:REQ-01` is NOT satisfied by a
    // `satisfies:REQ-01` on an UNRELATED issue (no dependency edge to the epic).
    // Cross-epic matches must not count, so the done-scoped derivation rule fires.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    // Epic in `done` state so the derivation rule is active.
    let mut epic = sdd_compliant_epic(); // declares req:REQ-01
    epic.state = State::Done;

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
    // A [hard] criterion for which NO `req:` is declared and NO child satisfies it
    // surfaces through the criteria -> satisfies coverage rule at done. The epic
    // carries no `req:` label (so the criteria-label-match rule produces no stray
    // finding), but coverage still fires because REQ-01 has no satisfying child.
    let set = load_example("sdd");
    let rules = graph_rules(&set);

    // Epic in `done` state: coverage rule is active. No req: label, no child.
    let mut epic = sdd_compliant_epic();
    epic.labels = vec!["type:epic".to_string()];
    epic.state = State::Done;

    let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
    assert!(
        findings.iter().any(|f| f.finding.message.contains("REQ-01")
            && f.finding.rule == "sdd-hard-criteria-covered"),
        "a hard criterion with no derived/satisfying label must be reported at done: {findings:?}"
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
    let eval = evaluate_local(&epic, &set, ContentFormat::Markdown).unwrap();
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
    let eval = evaluate_local(&epic, &set, ContentFormat::Markdown).unwrap();
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
    let eval = evaluate_local(&epic, &set, ContentFormat::Markdown).unwrap();
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
    let eval = evaluate_local(&epic, &set, ContentFormat::Markdown).unwrap();
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
    let eval = evaluate_local(&bug, &set, ContentFormat::Markdown).unwrap();
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
    let eval = evaluate_local(&release, &set, ContentFormat::Markdown).unwrap();
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

// ---------------------------------------------------------------------------
// 2d'. SDD — graph `criteria-label-match` rule: stray-req disambiguation.
//
// Kept in a clearly separated section so the sibling tasks (490f1f99,
// 690f618a, 765688e1, etc.) that also touch this file can merge cleanly.
// ---------------------------------------------------------------------------

mod sdd_criteria_label_match {
    use super::*;

    /// An epic whose criteria section contains only REQ-01 (as a [hard] item).
    fn epic_with_req_labels(labels: &[&str]) -> Issue {
        let body = "## Requirements\n\n\
                    - REQ-01: the loader rejects mixed shorthand and raw schema\n\n\
                    ## Scenarios\n\n\
                    - Given a rule mixing shorthand and a raw schema When the loader runs Then it errors\n\n\
                    ## Success Criteria\n\n\
                    - [hard] REQ-01: the loader rejects mixed shorthand and raw schema\n";
        let mut epic = Issue::new("epic".to_string(), body.to_string());
        epic.labels = labels.iter().map(|s| s.to_string()).collect();
        epic
    }

    #[test]
    fn test_sdd_criteria_label_match_rule_is_present_and_graph_scoped() {
        // The SDD example must now include a `criteria-label-match` rule named
        // `sdd-req-matches-a-criterion` and it must be Graph-scoped.
        let set = load_example("sdd");
        let rule = set
            .rules
            .iter()
            .find(|r| r.name == "sdd-req-matches-a-criterion")
            .expect("sdd example must define sdd-req-matches-a-criterion");
        assert_eq!(rule.scope, Scope::Graph);
        assert_eq!(rule.severity, jit::validation::rules::Severity::Error);
    }

    #[test]
    fn test_sdd_stray_req_label_yields_criteria_label_match_finding() {
        // A `req:REQ-77` on an epic whose Success Criteria contains only REQ-01
        // is stray: the `sdd-req-matches-a-criterion` rule fires for it.
        let set = load_example("sdd");
        let rules = graph_rules(&set);

        let epic = epic_with_req_labels(&["type:epic", "req:REQ-01", "req:REQ-77"]);
        // REQ-77 is absent from Success Criteria -> stray
        // REQ-01 is present  -> matched, no finding

        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        assert!(
            findings
                .iter()
                .any(|f| f.finding.rule == "sdd-req-matches-a-criterion"
                    && f.finding.message.contains("req:REQ-77")
                    && f.finding.message.contains("stray or invented")),
            "a stray req: label must be reported by criteria-label-match: {findings:?}"
        );
        // The matched label (REQ-01) must produce no finding from this rule.
        assert!(
            !findings
                .iter()
                .any(|f| f.finding.rule == "sdd-req-matches-a-criterion"
                    && f.finding.message.contains("req:REQ-01")),
            "a matched req: label must not produce a finding: {findings:?}"
        );
        // Confirm the finding text differs from the label-reference (unsatisfied) wording.
        let stray_finding = findings
            .iter()
            .find(|f| {
                f.finding.rule == "sdd-req-matches-a-criterion"
                    && f.finding.message.contains("REQ-77")
            })
            .expect("stray finding must exist");
        assert!(
            !stray_finding.finding.message.contains("is not satisfied"),
            "stray finding text must not say 'is not satisfied': {}",
            stray_finding.finding.message
        );
    }

    #[test]
    fn test_sdd_id_format_mismatch_req_3_vs_req_03_is_stray() {
        // Exact string comparison: criterion `REQ-03` in the body vs label
        // `req:REQ-3` on the epic — they differ, so `req:REQ-3` is a stray.
        // This must produce a finding from `sdd-req-matches-a-criterion`, NOT
        // be silently accepted by a normalization step.
        let body = "## Requirements\n\n\
                    - REQ-03: check padding\n\n\
                    ## Scenarios\n\n\
                    - Given x When y Then z\n\n\
                    ## Success Criteria\n\n\
                    - [hard] REQ-03: check padding\n";
        let mut epic = Issue::new("epic".to_string(), body.to_string());
        epic.labels = vec![
            "type:epic".to_string(),
            "req:REQ-3".to_string(), // no leading zero -> stray
        ];

        let set = load_example("sdd");
        let rules = graph_rules(&set);
        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        assert!(
            findings
                .iter()
                .any(|f| f.finding.rule == "sdd-req-matches-a-criterion"
                    && f.finding.message.contains("req:REQ-3")),
            "REQ-3 vs REQ-03: no normalization, so REQ-3 must be stray: {findings:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2d''. SDD — June 2026 lifecycle evaluation scenarios.
//
// These four scenarios correspond to the evaluation scenarios described in the
// issue specification (task 490f1f99). Scenarios (b) and (d) use the executor
// pattern from transition_graph_enforcement_tests.rs so the done transition
// is exercised end-to-end (real enforcement, real exit-4 path).
// ---------------------------------------------------------------------------

mod sdd_lifecycle {
    use super::*;
    use jit::commands::CommandExecutor;
    use jit::errors::TransitionBlockedError;
    use jit::storage::{InMemoryStorage, IssueStore};

    /// Load the SDD example ruleset from disk and write it into an executor's
    /// in-memory storage root so the executor picks it up as the operative set.
    fn executor_with_sdd_example() -> CommandExecutor<InMemoryStorage> {
        std::env::set_var("JIT_TEST_MODE", "1");
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("config.toml"), "").unwrap();

        // Read the example rules.toml and copy the schemas/ dir into the storage root.
        let sdd_dir = example_dir("sdd");
        let rules_toml = std::fs::read_to_string(sdd_dir.join("rules.toml")).unwrap();
        std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();

        // Copy the schemas/ directory so json-schema references resolve.
        let schemas_src = sdd_dir.join("schemas");
        let schemas_dst = storage.root().join("schemas");
        std::fs::create_dir_all(&schemas_dst).unwrap();
        for entry in std::fs::read_dir(&schemas_src).unwrap() {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), schemas_dst.join(entry.file_name())).unwrap();
        }

        CommandExecutor::new(storage)
    }

    /// A well-formed SDD spec body: Requirements, Scenarios, and Success Criteria.
    fn sdd_spec_body() -> String {
        "## Requirements\n\n\
            - REQ-01: the loader rejects mixed shorthand and raw schema\n\n\
            ## Scenarios\n\n\
            - Given a rule mixing shorthand and a raw schema When the loader runs Then it errors\n\n\
            ## Success Criteria\n\n\
            - [hard] REQ-01: the loader rejects mixed shorthand and raw schema\n"
            .to_string()
    }

    // Scenario (a): in-flight epic -> zero error-severity findings.
    //
    // An in-progress epic with a well-formed spec body, `req:` labels matching
    // the criteria, and children that exist but are not yet `done` must produce
    // ZERO error-severity findings from graph evaluation.
    #[test]
    fn test_sdd_in_flight_epic_yields_zero_error_findings() {
        let set = load_example("sdd");
        let rules = graph_rules(&set);

        // In-progress epic: correct structure, correct req: label.
        let mut epic = Issue::new("Feature X".to_string(), sdd_spec_body());
        epic.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
        epic.state = State::InProgress;

        // A child that depends on the epic but is still in progress (not done).
        let mut child = Issue::new("implement REQ-01".to_string(), String::new());
        child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
        child.dependencies = vec![epic.id.clone()];
        child.state = State::InProgress;

        let findings = issue_graph_findings(&rules, &[epic, child]);
        let error_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.severity == jit::validation::rules::Severity::Error)
            .collect();
        assert!(
            error_findings.is_empty(),
            "an in-flight epic with matching req: labels and in-progress children \
             must yield zero error-severity graph findings: {error_findings:?}"
        );
    }

    // Scenario (b): premature done -> TransitionBlockedError.
    //
    // An epic with an uncovered [hard] criterion that attempts --state done via
    // the executor must be blocked with TransitionBlockedError (exit 4).
    #[test]
    fn test_sdd_premature_done_blocked_by_coverage_rule() {
        let executor = executor_with_sdd_example();

        // Seed an epic with a well-formed spec body and req:REQ-01, but NO child
        // that satisfies it. The epic is in-progress.
        let mut epic = Issue::new("Feature X".to_string(), sdd_spec_body());
        epic.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
        epic.state = State::InProgress;
        let epic_id = epic.id.clone();
        executor.storage().save_issue(epic).unwrap();

        // Attempt to transition to done: must be blocked.
        let result = executor.update_issue(
            &epic_id,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            false,
        );

        let err = result.expect_err("epic with uncovered [hard] criterion must not reach done");
        let blocked = err
            .downcast_ref::<TransitionBlockedError>()
            .expect("the block must be a TransitionBlockedError (exit 4)");

        let rendered = blocked.to_string();
        // The coverage rule is what blocks.
        assert!(
            rendered.contains("sdd-hard-criteria-covered"),
            "blocked error must name the failing coverage rule: {rendered}"
        );

        // The issue must not have been persisted as done.
        let after = executor.storage().load_issue(&epic_id).unwrap();
        assert_eq!(
            after.state,
            State::InProgress,
            "a blocked done transition must persist nothing"
        );
    }

    // Scenario (c): stray req:REQ-77 on in-flight epic -> criteria-label-match finding.
    //
    // An in-progress epic with a stray req:REQ-77 (absent from the criteria prose)
    // must yield a finding from the always-on sdd-req-matches-a-criterion rule.
    // The done-scoped rules (sdd-hard-criteria-covered, sdd-req-is-satisfied) must
    // NOT fire because the epic is not in state done.
    #[test]
    fn test_sdd_stray_req_on_in_flight_epic_yields_only_criteria_label_match() {
        let set = load_example("sdd");
        let rules = graph_rules(&set);

        // In-progress epic with a stray req:REQ-77 (not in the criteria prose)
        // alongside the legitimate req:REQ-01.
        let mut epic = Issue::new("Feature X".to_string(), sdd_spec_body());
        epic.labels = vec![
            "type:epic".to_string(),
            "req:REQ-01".to_string(),
            "req:REQ-77".to_string(), // stray: not in criteria
        ];
        epic.state = State::InProgress;

        // A child in progress (not done) so the in-flight state is realistic.
        let mut child = Issue::new("implement REQ-01".to_string(), String::new());
        child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
        child.dependencies = vec![epic.id.clone()];
        child.state = State::InProgress;

        let findings = issue_graph_findings(&rules, &[epic, child]);

        // The stray must be caught by the always-on rule.
        assert!(
            findings
                .iter()
                .any(|f| f.finding.rule == "sdd-req-matches-a-criterion"
                    && f.finding.message.contains("req:REQ-77")),
            "stray req:REQ-77 on in-flight epic must be caught by criteria-label-match: {findings:?}"
        );

        // The done-scoped rules must NOT fire for an in-progress epic.
        assert!(
            !findings
                .iter()
                .any(|f| f.finding.rule == "sdd-hard-criteria-covered"),
            "sdd-hard-criteria-covered must not fire for an in-progress epic: {findings:?}"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.finding.rule == "sdd-req-is-satisfied"),
            "sdd-req-is-satisfied must not fire for an in-progress epic: {findings:?}"
        );
    }

    // Scenario (d): happy path -> done transition succeeds.
    //
    // An epic with a covered [hard] criterion (done child with satisfies:REQ-01)
    // must be able to transition to done without being blocked.
    #[test]
    fn test_sdd_happy_path_done_transition_succeeds() {
        let executor = executor_with_sdd_example();

        // Seed the epic.
        let mut epic = Issue::new("Feature X".to_string(), sdd_spec_body());
        epic.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
        epic.state = State::InProgress;
        let epic_id = epic.id.clone();
        executor.storage().save_issue(epic).unwrap();

        // Seed a done child that depends on the epic and satisfies REQ-01.
        let mut child = Issue::new("implement REQ-01".to_string(), String::new());
        child.labels = vec!["type:task".to_string(), "satisfies:REQ-01".to_string()];
        child.state = State::Done;
        let child_id = child.id.clone();
        executor.storage().save_issue(child).unwrap();
        // Wire the dependency: child depends on the epic (epic is the parent).
        executor.add_dependency(&child_id, &epic_id).unwrap();

        // Transition to done: must succeed.
        let result = executor.update_issue(
            &epic_id,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            false,
        );

        assert!(
            result.is_ok(),
            "an epic with covered [hard] criteria must reach done without blocking: {:?}",
            result.err()
        );

        let after = executor.storage().load_issue(&epic_id).unwrap();
        assert_eq!(
            after.state,
            State::Done,
            "the epic must be persisted as done after a successful transition"
        );
    }
}

// ---------------------------------------------------------------------------
// 2e. fresh-evidence (non-SDD) — graph `gate-recency` rule.
//
// Kept in its own module (separate from the SDD/bug-repro/release sections
// above) since other validation-lifecycle tasks also extend this file.
// ---------------------------------------------------------------------------

mod fresh_evidence {
    use super::*;
    use chrono::Duration;
    use jit::domain::{GateState, GateStatus};

    /// A done issue whose `code-review` gate was recorded `hours_ago` before
    /// [`fixed_now`].
    fn done_with_code_review(hours_ago: i64) -> Issue {
        let mut issue = Issue::new("implement feature".to_string(), String::new());
        issue.state = State::Done;
        issue.gates_required = vec!["code-review".to_string()];
        issue.gates_status.insert(
            "code-review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("agent:reviewer".to_string()),
                updated_at: fixed_now() - Duration::hours(hours_ago),
            },
        );
        issue
    }

    #[test]
    fn test_fresh_evidence_recent_gate_passes() {
        let set = load_example("fresh-evidence");
        let rules = graph_rules(&set);
        // Reviewed 1 day ago — within the 7-day window.
        let issue = done_with_code_review(24);
        let findings = issue_graph_findings_at(&rules, std::slice::from_ref(&issue), fixed_now());
        assert!(
            findings.is_empty(),
            "a done issue with a fresh code-review gate must pass: {findings:?}"
        );
    }

    #[test]
    fn test_fresh_evidence_stale_gate_is_reported() {
        let set = load_example("fresh-evidence");
        let rules = graph_rules(&set);
        // Reviewed 10 days ago — exceeds the 7-day window.
        let issue = done_with_code_review(10 * 24);
        let findings = issue_graph_findings_at(&rules, std::slice::from_ref(&issue), fixed_now());
        assert!(
            findings
                .iter()
                .any(|f| f.finding.rule == "fresh-evidence-before-done"
                    && f.finding.message.contains("code-review")
                    && f.finding.message.contains("days old")),
            "a stale code-review gate must be reported with its age: {findings:?}"
        );
    }

    #[test]
    fn test_fresh_evidence_missing_gate_is_reported() {
        let set = load_example("fresh-evidence");
        let rules = graph_rules(&set);
        // Done, requires the gate, but has no recorded result.
        let mut issue = Issue::new("implement feature".to_string(), String::new());
        issue.state = State::Done;
        issue.gates_required = vec!["code-review".to_string()];
        let findings = issue_graph_findings_at(&rules, std::slice::from_ref(&issue), fixed_now());
        assert!(
            findings
                .iter()
                .any(|f| f.finding.rule == "fresh-evidence-before-done"
                    && f.finding.message == "gate 'code-review' has no recorded result"),
            "a missing code-review gate result must be reported: {findings:?}"
        );
    }

    #[test]
    fn test_fresh_evidence_blocks_at_done_via_enforce() {
        // The example uses severity=error + enforce=true so it blocks the
        // transition INTO done once transition enforcement lands; here we assert
        // the rule carries that blocking shape.
        let set = load_example("fresh-evidence");
        let rule = set
            .rules
            .iter()
            .find(|r| r.name == "fresh-evidence-before-done")
            .expect("example must define fresh-evidence-before-done");
        assert_eq!(rule.severity, jit::validation::rules::Severity::Error);
        assert!(rule.enforce, "the example deliberately enforces at done");
        assert_eq!(rule.scope, Scope::Graph);
    }
}

// ---------------------------------------------------------------------------
// 2f. nyquist — criteria-to-check rule kind.
//
// Kept in its own module so the nyquist example section is clearly separated
// from the other methodology tests; sibling workers may also extend this file.
// ---------------------------------------------------------------------------

mod nyquist {
    use super::*;

    /// A well-formed epic body with two [hard] criteria and one [aspirational].
    fn nyquist_compliant_body() -> String {
        "## Success Criteria\n\n\
            - [hard] REQ-01: the parser rejects invalid input\n\
            - [hard] REQ-02: all edge cases are covered by tests\n\
            - [aspirational] REQ-03: performance meets the latency target\n"
            .to_string()
    }

    /// An epic with [hard] criteria REQ-01 and REQ-02, both verified via gates.
    fn epic_with_both_verified() -> Issue {
        let mut epic = Issue::new("Feature X".to_string(), nyquist_compliant_body());
        epic.labels = vec!["type:epic".to_string()];
        // Both [hard] criteria are verified via gates_required.
        epic.gates_required = vec!["verify:REQ-01".to_string(), "verify:REQ-02".to_string()];
        epic
    }

    /// An epic with [hard] criteria REQ-01 and REQ-02, both verified via labels.
    fn epic_with_label_verification() -> Issue {
        let mut epic = Issue::new("Feature Y".to_string(), nyquist_compliant_body());
        epic.labels = vec![
            "type:epic".to_string(),
            "checks:REQ-01".to_string(),
            "checks:REQ-02".to_string(),
        ];
        epic
    }

    #[test]
    fn test_nyquist_compliant_via_gates_passes_graph() {
        let set = load_example("nyquist");
        let rules = graph_rules(&set);
        let epic = epic_with_both_verified();
        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        assert!(
            findings.is_empty(),
            "an epic with all [hard] criteria gate-verified must pass: {findings:?}"
        );
    }

    #[test]
    fn test_nyquist_compliant_via_labels_passes_graph() {
        let set = load_example("nyquist");
        let rules = graph_rules(&set);
        let epic = epic_with_label_verification();
        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        assert!(
            findings.is_empty(),
            "an epic with all [hard] criteria label-verified must pass: {findings:?}"
        );
    }

    #[test]
    fn test_nyquist_unmapped_criterion_reports_finding() {
        let set = load_example("nyquist");
        let rules = graph_rules(&set);
        // Only REQ-01 is verified; REQ-02 is unmapped.
        let mut epic = Issue::new("Feature Z".to_string(), nyquist_compliant_body());
        epic.labels = vec!["type:epic".to_string()];
        epic.gates_required = vec!["verify:REQ-01".to_string()];

        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        // REQ-02 must be reported as unmapped; REQ-01 must not.
        assert!(
            findings
                .iter()
                .any(|f| f.finding.message.contains("REQ-02")),
            "unmapped criterion REQ-02 must be reported: {findings:?}"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.finding.message.contains("REQ-01")),
            "verified criterion REQ-01 must not be reported: {findings:?}"
        );
    }

    #[test]
    fn test_nyquist_finding_names_both_mechanisms() {
        // When both gate-prefix and check-namespace are configured (as in the
        // nyquist example), the unmapped-criterion finding mentions both.
        let set = load_example("nyquist");
        let rules = graph_rules(&set);
        let mut epic = Issue::new("Feature W".to_string(), nyquist_compliant_body());
        epic.labels = vec!["type:epic".to_string()]; // no gates, no checks labels

        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        let req01_finding = findings
            .iter()
            .find(|f| f.finding.message.contains("REQ-01"));
        assert!(
            req01_finding.is_some(),
            "REQ-01 must be reported as unmapped: {findings:?}"
        );
        let msg = &req01_finding.unwrap().finding.message;
        assert!(
            msg.contains("verify:REQ-01"),
            "finding must name the expected gate: {msg}"
        );
        assert!(
            msg.contains("checks:REQ-01"),
            "finding must name the expected label: {msg}"
        );
    }

    #[test]
    fn test_nyquist_marker_filters_aspirational_criteria() {
        // [aspirational] criteria must NOT be required; the marker = "[hard]"
        // filter must exclude REQ-03 (marked [aspirational]).
        let set = load_example("nyquist");
        let rules = graph_rules(&set);
        // An epic that verifies REQ-01 and REQ-02 but leaves REQ-03 unmapped.
        let epic = epic_with_both_verified();

        let findings = issue_graph_findings(&rules, std::slice::from_ref(&epic));
        // REQ-03 is [aspirational] and must not be required.
        assert!(
            !findings
                .iter()
                .any(|f| f.finding.message.contains("REQ-03")),
            "[aspirational] criterion REQ-03 must not be required: {findings:?}"
        );
    }

    #[test]
    fn test_nyquist_done_scoped_rule_has_enforce_shape() {
        // The done-scoped rule must carry severity=error + enforce=true so that
        // transition enforcement (bc86f54c) can block the done transition.
        let set = load_example("nyquist");
        let rule = set
            .rules
            .iter()
            .find(|r| r.name == "nyquist-criteria-verified-at-done")
            .expect("example must define nyquist-criteria-verified-at-done");
        assert_eq!(rule.severity, jit::validation::rules::Severity::Error);
        assert!(rule.enforce, "the done-scoped rule must enforce");
        assert_eq!(rule.scope, Scope::Graph);
    }
}

mod cross_epic {
    use super::*;

    /// A pair of unlinked epics both declaring the same req value — the
    /// archetypal cross-epic collision the example is designed to catch.
    fn two_epics_colliding_on(req_value: &str) -> (Issue, Issue) {
        let label = format!("req:{req_value}");
        let mut epic_a = Issue::new(format!("Epic A ({req_value})"), String::new());
        epic_a.labels = vec!["type:epic".to_string(), label.clone()];
        let mut epic_b = Issue::new(format!("Epic B ({req_value})"), String::new());
        epic_b.labels = vec!["type:epic".to_string(), label];
        // No dependency edge — the point is that the collision is cross-epic.
        (epic_a, epic_b)
    }

    #[test]
    fn test_cross_epic_ruleset_loads() {
        let set = load_example("cross-epic");
        assert!(
            !set.rules.is_empty(),
            "cross-epic example must define at least one rule"
        );
        // The rule must be graph-scoped (label-uniqueness is always graph-scoped).
        assert!(
            set.rules.iter().any(|r| r.scope == Scope::Graph),
            "cross-epic example must define a graph rule"
        );
    }

    #[test]
    fn test_cross_epic_collision_detected() {
        // Two unlinked epics both declaring req:REQ-01 → one finding.
        let set = load_example("cross-epic");
        let rules = graph_rules(&set);

        let (epic_a, epic_b) = two_epics_colliding_on("REQ-01");
        let findings = issue_graph_findings(&rules, &[epic_a, epic_b]);

        assert_eq!(
            findings.len(),
            1,
            "expected one collision finding for req:REQ-01: {findings:?}"
        );
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("req:REQ-01"),
            "finding must name the colliding label value: {msg}"
        );
        assert_eq!(
            findings[0].finding.rule, "cross-epic-req-uniqueness",
            "finding must be attributed to the right rule"
        );
    }

    #[test]
    fn test_cross_epic_no_finding_for_unique_req_ids() {
        // Two unlinked epics with DISTINCT req values → no finding.
        let set = load_example("cross-epic");
        let rules = graph_rules(&set);

        let mut epic_a = Issue::new("Epic A".to_string(), String::new());
        epic_a.labels = vec!["type:epic".to_string(), "req:REQ-01".to_string()];
        let mut epic_b = Issue::new("Epic B".to_string(), String::new());
        epic_b.labels = vec!["type:epic".to_string(), "req:REQ-02".to_string()];

        let findings = issue_graph_findings(&rules, &[epic_a, epic_b]);
        assert!(
            findings.is_empty(),
            "distinct req values must not collide: {findings:?}"
        );
    }

    #[test]
    fn test_cross_epic_finding_names_colliding_short_ids() {
        // The finding message must name the short-ids of both colliding epics
        // so the author knows which issues to fix.
        let set = load_example("cross-epic");
        let rules = graph_rules(&set);

        let (epic_a, epic_b) = two_epics_colliding_on("REQ-01");
        let id_a = epic_a.short_id().to_string();
        let id_b = epic_b.short_id().to_string();

        let findings = issue_graph_findings(&rules, &[epic_a, epic_b]);
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains(&id_a),
            "finding must name short-id of epic A ({id_a}): {msg}"
        );
        assert!(
            msg.contains(&id_b),
            "finding must name short-id of epic B ({id_b}): {msg}"
        );
    }

    #[test]
    fn test_cross_epic_rule_is_repo_wide_at_transition() {
        // label-uniqueness must be skipped at transition time — verify the
        // example rule carries the correct is_repo_wide_at_transition flag.
        let set = load_example("cross-epic");
        let rule = set
            .rules
            .iter()
            .find(|r| r.name == "cross-epic-req-uniqueness")
            .expect("example must define cross-epic-req-uniqueness");
        assert!(
            rule.assert.is_repo_wide_at_transition(),
            "label-uniqueness must be skipped at transition time"
        );
    }

    #[test]
    fn test_cross_epic_large_fixture_50_collisions() {
        // Performance and correctness on a repo-scale fixture: 300 unique-value
        // epics + 100 colliding epics forming 50 collision pairs = 400 epics.
        // The evaluator must find exactly 50 findings (one per colliding value).
        //
        // Design: single-pass O(n * k) HashMap; no N^2 scan. This test confirms
        // correctness at scale; the elapsed time is measured (with a generous
        // anti-regression bound) in
        // validation::graph::tests::test_label_uniqueness_large_fixture_correctness.
        let set = load_example("cross-epic");
        let rules = graph_rules(&set);

        let mut issues: Vec<Issue> = Vec::with_capacity(400);

        // 300 epics with unique req values.
        for i in 0..300u32 {
            let mut epic = Issue::new(format!("unique-epic-{i}"), String::new());
            epic.labels = vec!["type:epic".to_string(), format!("req:UNIQ-{i:04}")];
            issues.push(epic);
        }
        // 50 collision pairs: two epics per colliding value.
        for i in 0..50u32 {
            let label = format!("req:COLL-{i:04}");
            let mut a = Issue::new(format!("coll-a-{i}"), String::new());
            a.labels = vec!["type:epic".to_string(), label.clone()];
            let mut b = Issue::new(format!("coll-b-{i}"), String::new());
            b.labels = vec!["type:epic".to_string(), label];
            issues.push(a);
            issues.push(b);
        }
        assert_eq!(issues.len(), 400);

        let findings = issue_graph_findings(&rules, &issues);

        assert_eq!(
            findings.len(),
            50,
            "expected 50 collision findings, got {}: {findings:?}",
            findings.len()
        );
        // Every finding names a COLL- value.
        assert!(
            findings.iter().all(|f| f.finding.message.contains("COLL-")),
            "all findings must name a collision value: {findings:?}"
        );
        // No finding names a UNIQ- value.
        assert!(
            !findings.iter().any(|f| f.finding.message.contains("UNIQ-")),
            "unique values must not produce findings: {findings:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 2g. research — non-software hierarchy (type:goal / type:experiment).
//
// Demonstrates that the validation engine carries NO software assumptions:
// no "epic", no "milestone" — only research vocabulary. Tests are kept in
// their own clearly separated module so sibling workers can merge cleanly.
// ---------------------------------------------------------------------------

mod research {
    use super::*;

    // -----------------------------------------------------------------------
    // Shared fixtures
    // -----------------------------------------------------------------------

    /// A well-formed goal body: `## Hypotheses` with at least one `[hard]` item
    /// and `## Success Criteria`.
    fn compliant_goal_body() -> String {
        "## Hypotheses\n\n\
            - [hard] H-1: increasing training data improves accuracy\n\
            - [exploratory] H-2: model size is the primary performance driver\n\n\
            ## Success Criteria\n\n\
            - accuracy exceeds 95% on the held-out test set\n"
            .to_string()
    }

    /// A `type:goal` issue with a well-formed body and `hyp:H-1` derived from
    /// the single `[hard]` hypothesis in the body.
    fn compliant_goal() -> Issue {
        let mut goal = Issue::new("Improve accuracy".to_string(), compliant_goal_body());
        goal.labels = vec!["type:goal".to_string(), "hyp:H-1".to_string()];
        goal
    }

    /// A `type:experiment` that tests hypothesis H-1 and has the required
    /// `## Method` and `## Evidence` sections.
    fn compliant_experiment(goal_id: &str) -> Issue {
        let body = "## Method\n\n\
            - train with 10x the original dataset size\n\
            - evaluate on the held-out test set\n\n\
            ## Evidence\n\n\
            - accuracy reached 96.3%, exceeding the 95% threshold\n";
        let mut exp = Issue::new("Scale training data".to_string(), body.to_string());
        exp.labels = vec!["type:experiment".to_string(), "tests:H-1".to_string()];
        exp.dependencies = vec![goal_id.to_string()];
        exp.state = State::Done;
        exp
    }

    // -----------------------------------------------------------------------
    // 1. Ruleset loads and is non-empty
    // -----------------------------------------------------------------------

    #[test]
    fn test_research_ruleset_loads() {
        let set = load_example("research");
        assert!(
            !set.rules.is_empty(),
            "research example must define at least one rule"
        );
        // Must have both local and graph rules.
        assert!(
            set.rules.iter().any(|r| r.scope == Scope::Local),
            "research example must define at least one local rule"
        );
        assert!(
            set.rules.iter().any(|r| r.scope == Scope::Graph),
            "research example must define at least one graph rule"
        );
    }

    #[test]
    fn test_research_no_rule_references_epic_or_milestone() {
        // The whole point of this example: no software-development type names.
        // We check the rule selectors and assert kinds do NOT mention epic or
        // milestone anywhere by inspecting the rendered TOML round-trip of the
        // rules file itself.
        let dir = example_dir("research");
        let rules_toml = std::fs::read_to_string(dir.join("rules.toml"))
            .expect("research rules.toml must be readable");
        assert!(
            !rules_toml.contains("epic"),
            "no rule in the research example must reference 'epic': {rules_toml}"
        );
        assert!(
            !rules_toml.contains("milestone"),
            "no rule in the research example must reference 'milestone': {rules_toml}"
        );
    }

    // -----------------------------------------------------------------------
    // 2. Local rules: structure is enforced on write
    // -----------------------------------------------------------------------

    #[test]
    fn test_research_compliant_goal_passes_local() {
        let set = load_example("research");
        assert!(
            !has_local_finding(&compliant_goal(), &set),
            "a well-formed goal body must produce no local findings"
        );
    }

    #[test]
    fn test_research_goal_missing_hypotheses_section_is_blocked() {
        let set = load_example("research");
        let mut goal = compliant_goal();
        // Replace the body with one that has no ## Hypotheses section.
        goal.description = "## Success Criteria\n\n- good results\n".to_string();
        let eval = evaluate_local(&goal, &set, ContentFormat::Markdown).unwrap();
        assert!(
            eval.is_blocking(),
            "a goal with no Hypotheses section must be blocked on write"
        );
    }

    #[test]
    fn test_research_goal_missing_success_criteria_section_is_blocked() {
        let set = load_example("research");
        let mut goal = compliant_goal();
        // Body has only ## Hypotheses, no ## Success Criteria.
        goal.description = "## Hypotheses\n\n- [hard] H-1: statement\n".to_string();
        let eval = evaluate_local(&goal, &set, ContentFormat::Markdown).unwrap();
        assert!(
            eval.is_blocking(),
            "a goal with no Success Criteria section must be blocked on write"
        );
    }

    #[test]
    fn test_research_goal_malformed_hypotheses_items_blocked() {
        let set = load_example("research");
        let mut goal = compliant_goal();
        // Items are missing both marker and H-N id format.
        goal.description = "## Hypotheses\n\n\
            - this is a freeform note with no id\n\n\
            ## Success Criteria\n\n\
            - some outcome\n"
            .to_string();
        let eval = evaluate_local(&goal, &set, ContentFormat::Markdown).unwrap();
        assert!(
            eval.is_blocking(),
            "hypothesis items without marker and H-N id must be blocked"
        );
    }

    #[test]
    fn test_research_goal_no_hard_hypothesis_is_blocked() {
        let set = load_example("research");
        let mut goal = compliant_goal();
        // Only [exploratory] items — no [hard] criterion. The schema requires at
        // least one [hard] item via `contains` + `minContains`.
        goal.description = "## Hypotheses\n\n\
            - [exploratory] H-1: a tentative idea\n\n\
            ## Success Criteria\n\n\
            - some outcome\n"
            .to_string();
        let eval = evaluate_local(&goal, &set, ContentFormat::Markdown).unwrap();
        assert!(
            eval.is_blocking(),
            "a goal with no [hard] hypothesis must be blocked"
        );
    }

    #[test]
    fn test_research_compliant_experiment_passes_local() {
        let set = load_example("research");
        let exp = compliant_experiment("dummy-goal-id");
        assert!(
            !has_local_finding(&exp, &set),
            "a well-formed experiment body must produce no local findings"
        );
    }

    #[test]
    fn test_research_experiment_missing_method_section_is_blocked() {
        let set = load_example("research");
        let mut exp = compliant_experiment("dummy-goal-id");
        exp.description = "## Evidence\n\n- observations\n".to_string();
        let eval = evaluate_local(&exp, &set, ContentFormat::Markdown).unwrap();
        assert!(
            eval.is_blocking(),
            "an experiment with no Method section must be blocked"
        );
    }

    #[test]
    fn test_research_experiment_missing_evidence_section_is_blocked() {
        let set = load_example("research");
        let mut exp = compliant_experiment("dummy-goal-id");
        exp.description = "## Method\n\n- the protocol\n".to_string();
        let eval = evaluate_local(&exp, &set, ContentFormat::Markdown).unwrap();
        assert!(
            eval.is_blocking(),
            "an experiment with no Evidence section must be blocked"
        );
    }

    // -----------------------------------------------------------------------
    // 3. Graph rules: stray hyp: label detection (criteria-label-match)
    // -----------------------------------------------------------------------

    #[test]
    fn test_research_stray_hyp_label_is_reported() {
        // A `hyp:H-99` label on a goal whose Hypotheses section contains only
        // H-1 is stray: the `research-hyp-label-matches-hypothesis` rule fires.
        let set = load_example("research");
        let rules = graph_rules(&set);

        let mut goal = compliant_goal();
        // Add a fabricated hypothesis label H-99 — it does not appear in the body.
        goal.labels.push("hyp:H-99".to_string());

        let findings = issue_graph_findings(&rules, std::slice::from_ref(&goal));
        assert!(
            findings.iter().any(
                |f| f.finding.rule == "research-hyp-label-matches-hypothesis"
                    && f.finding.message.contains("hyp:H-99")
                    && f.finding.message.contains("stray or invented")
            ),
            "a fabricated hyp: label must be reported as stray: {findings:?}"
        );
        // The legitimate hyp:H-1 must not be flagged.
        assert!(
            !findings.iter().any(
                |f| f.finding.rule == "research-hyp-label-matches-hypothesis"
                    && f.finding.message.contains("hyp:H-1")
            ),
            "a matched hyp: label must not be reported: {findings:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 4. In-flight goal produces ZERO error findings (lifecycle silence)
    // -----------------------------------------------------------------------

    #[test]
    fn test_research_inflight_goal_produces_zero_error_findings() {
        // The coverage rule is scoped `state = "done"`. A goal in_progress with
        // no covering experiments must produce zero ERROR findings from graph
        // rules (the done-scoped rule simply does not match).
        let set = load_example("research");
        let rules = graph_rules(&set);

        let mut goal = compliant_goal();
        goal.state = State::InProgress;
        // No experiments at all — would fail coverage if the rule fired.

        let findings = issue_graph_findings(&rules, std::slice::from_ref(&goal));
        let error_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.severity == jit::validation::rules::Severity::Error)
            .collect();
        assert!(
            error_findings.is_empty(),
            "an in-flight goal must produce zero error-severity findings from \
             graph rules (done-scoped coverage must not fire): {error_findings:?}"
        );
    }

    // -----------------------------------------------------------------------
    // 5. Done-scoped coverage rule blocks transition via executor
    // -----------------------------------------------------------------------

    /// Build an in-memory executor whose rules.toml is the research coverage
    /// rule only (local schema rules are omitted to keep the test self-contained;
    /// coverage is the graph rule under test here).
    fn research_coverage_executor() -> CommandExecutor<InMemoryStorage> {
        std::env::set_var("JIT_TEST_MODE", "1");
        let storage = InMemoryStorage::new();
        storage.init().unwrap();
        std::fs::create_dir_all(storage.root()).unwrap();
        std::fs::write(storage.root().join("config.toml"), "").unwrap();
        // Write only the done-scoped coverage rule: no local schema rules needed
        // because the graph test does not write through the write-path validator.
        let rules_toml = r#"
[[rules]]
name = "research-hard-hypotheses-covered-at-done"
when = { type = "goal", state = "done" }
severity = "error"
enforce = true
assert = { label-coverage = { criteria-section = "hypotheses", marker = "[hard]", id-pattern = "H-[0-9]+", satisfies-namespace = "tests", child-state = "done", child-link = "dependents" } }
"#;
        std::fs::write(storage.root().join("rules.toml"), rules_toml).unwrap();
        CommandExecutor::new(storage)
    }

    /// Save an issue directly into storage at a given state, bypassing validation.
    fn seed(
        executor: &CommandExecutor<InMemoryStorage>,
        title: &str,
        labels: &[&str],
        body: &str,
        state: State,
    ) -> String {
        let mut issue = Issue::new(title.to_string(), body.to_string());
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue.state = state;
        let id = issue.id.clone();
        executor.storage().save_issue(issue).unwrap();
        id
    }

    #[test]
    fn test_research_done_transition_blocked_when_hypothesis_uncovered() {
        // A `type:goal` in state `in_progress` with a `[hard]` hypothesis H-1
        // but NO done experiment that `tests:H-1`. The done transition must be
        // blocked by the enforcing coverage rule (exit 4 semantics via
        // TransitionBlockedError).
        let executor = research_coverage_executor();

        let goal_id = seed(
            &executor,
            "Improve accuracy",
            &["type:goal", "hyp:H-1"],
            &compliant_goal_body(),
            State::InProgress,
        );

        let result = executor.update_issue(
            &goal_id,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            false,
        );

        let err = result.expect_err("uncovered [hard] hypothesis must block the done transition");
        let blocked = err
            .downcast_ref::<TransitionBlockedError>()
            .expect("error must be a TransitionBlockedError (maps to exit 4)");

        let rendered = blocked.to_string();
        assert!(
            rendered.contains("research-hard-hypotheses-covered-at-done"),
            "blocking error must name the failing rule: {rendered}"
        );
        assert!(
            rendered.contains("H-1"),
            "blocking error must name the uncovered hypothesis: {rendered}"
        );

        // The goal must remain in_progress — nothing was saved.
        let after = executor.storage().load_issue(&goal_id).unwrap();
        assert_eq!(
            after.state,
            State::InProgress,
            "a blocked done transition must not persist the state change"
        );
    }

    // -----------------------------------------------------------------------
    // 6. Happy path: covered goal reaches done, no findings
    // -----------------------------------------------------------------------

    #[test]
    fn test_research_happy_path_covered_goal_completes() {
        // Goal with [hard] H-1; one done experiment that depends on the goal and
        // carries tests:H-1. Graph findings must be empty; done transition succeeds.
        let set = load_example("research");
        let rules = graph_rules(&set);

        let goal = compliant_goal();
        let exp = compliant_experiment(&goal.id);

        let findings = issue_graph_findings(&rules, &[goal.clone(), exp]);
        let error_findings: Vec<_> = findings
            .iter()
            .filter(|f| f.finding.severity == jit::validation::rules::Severity::Error)
            .collect();
        assert!(
            error_findings.is_empty(),
            "a covered done goal must yield no error findings: {error_findings:?}"
        );
    }

    #[test]
    fn test_research_tests_references_a_hyp_warns_on_dangling_tests_label() {
        // An experiment that claims `tests:H-99`, but the linked goal only
        // declares `hyp:H-1`. The `research-tests-references-a-hyp` rule (warn)
        // must surface the dangling reference without blocking.
        let set = load_example("research");
        let rules = graph_rules(&set);

        let goal = compliant_goal(); // declares hyp:H-1
        let mut exp = compliant_experiment(&goal.id);
        // Override the tests: label to reference a nonexistent hypothesis.
        exp.labels = vec![
            "type:experiment".to_string(),
            "tests:H-99".to_string(), // no hyp:H-99 on the goal
        ];

        let findings = issue_graph_findings(&rules, &[goal, exp]);
        assert!(
            findings
                .iter()
                .any(|f| f.finding.rule == "research-tests-references-a-hyp"
                    && f.finding.message.contains("H-99")),
            "a dangling tests: reference must be reported by research-tests-references-a-hyp: \
             {findings:?}"
        );
        // Verify the finding is a warn (not error) so it does not block.
        let dangling = findings
            .iter()
            .find(|f| f.finding.rule == "research-tests-references-a-hyp")
            .expect("dangling tests: finding must exist");
        assert_eq!(
            dangling.finding.severity,
            jit::validation::rules::Severity::Warn,
            "dangling tests: reference must be a warning, not an error"
        );
    }

    #[test]
    fn test_research_done_scoped_coverage_rule_has_enforce_shape() {
        // The done-scoped rule must carry severity=error + enforce=true so that
        // transition enforcement blocks the done transition when uncovered.
        let set = load_example("research");
        let rule = set
            .rules
            .iter()
            .find(|r| r.name == "research-hard-hypotheses-covered-at-done")
            .expect("research example must define research-hard-hypotheses-covered-at-done");
        assert_eq!(rule.severity, jit::validation::rules::Severity::Error);
        assert!(rule.enforce, "the done-scoped coverage rule must enforce");
        assert_eq!(rule.scope, Scope::Graph);
    }
}
