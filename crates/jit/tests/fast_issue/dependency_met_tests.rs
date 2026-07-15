//! Cross-surface tests for the dependency-met predicate (jit:69ffdcf8).
//!
//! A dependency is met when its target reached a terminal state (`Done` or
//! `Rejected`). Every surface reads that one rule: state transitions, the
//! blocked query's reasons, `issue show` rendering, and the Ready-issue
//! demotion that guards a newly added edge.
use crate::harness::TestHarness;
use jit::domain::queries::{query_blocked, BlockingReason};
use jit::domain::State;
use jit::output::IssueShowResponse;
use jit::storage::IssueStore;

/// Reject an issue outright, the way `jit issue update --state rejected` does.
fn reject(h: &TestHarness, id: &str) {
    h.executor.update_issue_state(id, State::Rejected).unwrap();
}

#[test]
fn test_transition_to_ready_allows_rejected_dependency() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Abandoned upstream");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    reject(&h, &dependency);

    h.executor
        .update_issue_state(&dependent, State::Ready)
        .unwrap();

    assert_eq!(h.get_issue(&dependent).state, State::Ready);
}

#[test]
fn test_transition_to_done_allows_rejected_dependency() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Abandoned upstream");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    reject(&h, &dependency);

    h.executor
        .update_issue_state(&dependent, State::InProgress)
        .unwrap();
    h.executor
        .update_issue_state(&dependent, State::Done)
        .unwrap();

    assert_eq!(h.get_issue(&dependent).state, State::Done);
}

#[test]
fn test_query_blocked_reports_only_the_unmet_dependency() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let rejected = h.create_issue("Abandoned upstream");
    let pending = h.create_issue("Live upstream");
    h.executor.add_dependency(&dependent, &rejected).unwrap();
    h.executor.add_dependency(&dependent, &pending).unwrap();
    reject(&h, &rejected);

    let blocked = query_blocked(&h.all_issues());
    let reasons = &blocked
        .iter()
        .find(|(issue, _)| issue.id == dependent)
        .expect("dependent is still blocked by the live upstream")
        .1;

    assert_eq!(
        reasons,
        &vec![BlockingReason::Dependency {
            id: pending.clone(),
            title: "Live upstream".to_string(),
            state: State::Ready,
        }]
    );
}

#[test]
fn test_issue_show_omits_rejected_dependency_from_unmet() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let rejected = h.create_issue("Abandoned upstream");
    let pending = h.create_issue("Live upstream");
    h.executor.add_dependency(&dependent, &rejected).unwrap();
    h.executor.add_dependency(&dependent, &pending).unwrap();
    reject(&h, &rejected);

    let issue = h.executor.show_issue(&dependent).unwrap();
    let enriched_deps = h.executor.get_dependencies_enriched(&issue);
    let response = IssueShowResponse::from_issue(issue, enriched_deps, &[]);

    let unmet: Vec<&str> = response
        .unmet_dependencies
        .iter()
        .map(|dep| dep.id.as_str())
        .collect();
    assert_eq!(unmet, vec![pending.as_str()]);
}

#[test]
fn test_claim_next_selects_issue_whose_dependency_is_rejected() {
    let h = TestHarness::new();
    let dependent = h.create_ready_issue("Dependent");
    let dependency = h.create_issue("Abandoned upstream");
    reject(&h, &dependency);
    h.executor.add_dependency(&dependent, &dependency).unwrap();

    let (claimed, _) = h
        .executor
        .claim_next("agent:worker-1".to_string(), None)
        .unwrap();

    assert_eq!(claimed, dependent);
}

#[test]
fn test_claim_next_skips_issue_with_unmet_dependency() {
    let h = TestHarness::new();
    let dependent = h.create_ready_issue("Dependent");
    let dependency = h.create_issue("Live upstream");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    // The upstream leaves the ready set itself, so only `dependent` could match.
    h.executor
        .update_issue_state(&dependency, State::InProgress)
        .unwrap();

    let result = h.executor.claim_next("agent:worker-1".to_string(), None);

    assert!(
        result.is_err(),
        "an unmet dependency keeps the issue out of the ready set"
    );
}

#[test]
fn test_adding_rejected_dependency_keeps_issue_ready() {
    let h = TestHarness::new();
    let dependent = h.create_ready_issue("Dependent");
    let dependency = h.create_issue("Abandoned upstream");
    reject(&h, &dependency);

    h.executor.add_dependency(&dependent, &dependency).unwrap();

    assert_eq!(
        h.storage.load_issue(&dependent).unwrap().state,
        State::Ready,
        "a met dependency must not demote a Ready issue"
    );
}
