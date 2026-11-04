//! Cross-surface tests for the dependency-met predicate (jit:69ffdcf8).
//!
//! A dependency is met when its target reached a terminal state (`Done` or
//! `Rejected`). Every surface reads that one rule: state transitions, `issue
//! show` rendering, claim-next's candidates, and the Ready-issue demotion that guards a newly added edge.
use jit::domain::identity::IssueId;
use jit::domain::State;
use jit::output::IssueShowResponse;
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;
use jit::test_utils::add_reducing_edge;

/// Reject an issue outright, the way `jit issue reject` does.
fn reject(h: &TestHarness, id: &IssueId) {
    jit::test_utils::transition_issue(&h.executor, id, State::Rejected).unwrap();
}

#[test]
fn test_transition_to_ready_allows_rejected_dependency() {
    let h = TestHarness::new();
    let dependency = h.create_issue("Abandoned upstream");
    reject(&h, &dependency);
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();
    jit::test_utils::transition_issue(&h.executor, &dependent, State::Backlog).unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Backlog);

    jit::test_utils::transition_issue(&h.executor, &dependent, State::Ready).unwrap();

    assert_eq!(h.get_issue(&dependent).state, State::Ready);
}

#[test]
fn test_transition_to_done_allows_rejected_dependency() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Abandoned upstream");
    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();
    reject(&h, &dependency);

    jit::test_utils::transition_issue(&h.executor, &dependent, State::InProgress).unwrap();
    jit::test_utils::transition_issue(&h.executor, &dependent, State::Done).unwrap();

    assert_eq!(h.get_issue(&dependent).state, State::Done);
}

#[test]
fn test_issue_show_omits_rejected_dependency_from_unmet() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let rejected = h.create_issue("Abandoned upstream");
    let pending = h.create_issue("Live upstream");
    add_reducing_edge(&h.executor, &dependent, &rejected).unwrap();
    add_reducing_edge(&h.executor, &dependent, &pending).unwrap();
    reject(&h, &rejected);

    let issue = h.executor.issues().show_issue(&dependent).unwrap();
    let enriched_deps = h.executor.issues().get_dependencies_enriched(&issue);
    let response = IssueShowResponse::from_issue(issue, enriched_deps, &[]);

    let unmet: Vec<&str> = response
        .unmet_dependencies
        .iter()
        .map(|dep| dep.id.as_str())
        .collect();
    assert_eq!(unmet, vec![pending.as_str()]);
}

#[test]
fn test_claim_next_skips_issue_with_unmet_dependency() {
    let h = TestHarness::new();
    let dependent = h.create_ready_issue("Dependent");
    let dependency = h.create_issue("Live upstream");
    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();
    // The upstream leaves the ready set itself, so only `dependent` could match.
    jit::test_utils::transition_issue(&h.executor, &dependency, State::InProgress).unwrap();

    let result = h
        .executor
        .issues()
        .claim_next("agent:worker-1".parse().unwrap(), None);

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

    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();

    assert_eq!(
        h.storage.load_issue(&dependent).unwrap().state,
        State::Ready,
        "a met dependency must not demote a Ready issue"
    );
}
