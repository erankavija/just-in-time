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
