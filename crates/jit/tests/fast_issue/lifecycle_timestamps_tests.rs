//! Lifecycle-timestamp write-point tests (jit:0ab468ba).
//!
//! Exercises every path that stamps `first_ready_at`, `claimed_at`, or
//! `done_at` on the issue record. First-occurrence-only semantics are
//! unit-tested at the domain layer (`Issue::mark_*`); here we verify the command
//! paths wire those helpers in.

use jit::domain::event_catalog::EventTag;
use jit::domain::identity::{IssueId, IssueIdPrefix};
use jit::domain::{EventBody, State};
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;

/// A dependency-free issue is born Ready (auto-promotion at creation), so it
/// gets `first_ready_at` even though that write bypasses the transition
/// chokepoint.
#[test]
fn test_create_dependency_free_issue_stamps_first_ready() {
    let h = TestHarness::new();
    let id = h.create_issue("Standalone");

    let issue = h.get_issue(&id);
    assert_eq!(issue.state, State::Ready);
    assert!(issue.first_ready_at.is_some());
    assert!(issue.claimed_at.is_none());
    assert!(issue.done_at.is_none());
}

/// A precise chokepoint check: an issue that has NEVER been Ready (forced into
/// Backlog with the stamp cleared) gets `first_ready_at` set the first time
/// a state update lands it in Ready.
#[test]
fn test_backlog_to_ready_transition_stamps_first_ready() {
    let h = TestHarness::new();
    let id = h.create_issue("Fresh");

    // Force a pristine Backlog issue with no prior Ready stamp, bypassing the
    // birth auto-promotion so the chokepoint's Ready arm is what stamps.
    let mut issue = h.get_issue(&id);
    issue.state = State::Backlog;
    issue.first_ready_at = None;
    jit::test_harness::seed_memory_issue(&h.storage, &issue);

    jit::test_utils::transition_issue(&h.executor, &id, State::Ready).unwrap();

    let after = h.get_issue(&id);
    assert_eq!(after.state, State::Ready);
    assert!(after.first_ready_at.is_some());
}

/// Claiming an issue stamps `claimed_at`.
#[test]
fn test_claim_stamps_claimed_at() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Claimable");
    assert!(h.get_issue(&id).claimed_at.is_none());

    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(&id), "agent:worker-1".parse().unwrap())
        .unwrap();

    let issue = h.get_issue(&id);
    assert!(issue.claimed_at.is_some());
    assert_eq!(issue.state, State::InProgress);
}

/// Assigning an issue (without claiming) stamps `claimed_at` and logs an
/// `issue_claimed` event (@/inv/event-log) so the mutation is auditable.
#[test]
fn test_assign_stamps_claimed_at_and_logs_event() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Assignable");

    h.executor
        .issues()
        .assign_issue(&IssueIdPrefix::from(&id), "human:alice".parse().unwrap())
        .unwrap();

    assert!(h.get_issue(&id).claimed_at.is_some());

    let claimed = h
        .storage
        .read_events()
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e.body(), EventBody::IssueClaimed { .. }))
        .count();
    assert_eq!(claimed, 1);
}

/// The events a claim of `id` by `assignee` appends, by tag.
fn tags_appended_by_claim(h: &TestHarness, id: &IssueId, assignee: &str) -> Vec<EventTag> {
    let before = h.storage.read_events().unwrap().len();
    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(id), assignee.parse().unwrap())
        .unwrap();
    h.storage.read_events().unwrap()[before..]
        .iter()
        .map(|event| event.body().tag())
        .collect()
}

#[test]
fn test_claim_issue_of_unassigned_issue_appends_a_claim_event() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Unassigned");

    let tags = tags_appended_by_claim(&h, &id, "agent:worker-1");

    assert_eq!(
        tags.iter()
            .filter(|tag| **tag == EventTag::IssueClaimed)
            .count(),
        1
    );
}

#[test]
fn test_claim_issue_promoting_same_assignee_appends_only_the_state_change() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Assigned");
    h.executor
        .issues()
        .assign_issue(&IssueIdPrefix::from(&id), "agent:worker-1".parse().unwrap())
        .unwrap();

    let tags = tags_appended_by_claim(&h, &id, "agent:worker-1");

    assert_eq!(tags, [EventTag::IssueStateChanged]);
    assert_eq!(h.get_issue(&id).state, State::InProgress);
}

/// A claim that neither reassigns nor promotes changes nothing.
#[test]
fn test_claim_issue_same_assignee_without_promotion_appends_nothing() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Finished");
    h.executor
        .issues()
        .assign_issue(&IssueIdPrefix::from(&id), "agent:worker-1".parse().unwrap())
        .unwrap();
    jit::test_utils::transition_issue(&h.executor, &id, State::Done).unwrap();

    assert!(tags_appended_by_claim(&h, &id, "agent:worker-1").is_empty());
}

/// Transitioning an issue to Done stamps `done_at`.
#[test]
fn test_done_transition_stamps_done_at() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Finishable");
    assert!(h.get_issue(&id).done_at.is_none());

    jit::test_utils::transition_issue(&h.executor, &id, State::Done).unwrap();

    let issue = h.get_issue(&id);
    assert_eq!(issue.state, State::Done);
    assert!(issue.done_at.is_some());
}

/// A re-assignment does not move `claimed_at` off its first value (command-level
/// first-occurrence check; the assignee is the same so the write is idempotent).
#[test]
fn test_reclaim_keeps_first_claimed_at() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Claimable");
    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(&id), "agent:worker-1".parse().unwrap())
        .unwrap();
    let first = h.get_issue(&id).claimed_at;
    assert!(first.is_some());

    // Re-claiming as the same assignee promotes/no-ops but must not re-stamp.
    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(&id), "agent:worker-1".parse().unwrap())
        .unwrap();
    assert_eq!(h.get_issue(&id).claimed_at, first);
}
