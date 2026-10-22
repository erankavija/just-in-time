//! Lifecycle-timestamp write-point tests (jit:0ab468ba).
//!
//! Exercises every path that stamps `first_ready_at`, `claimed_at`, or
//! `done_at` on the issue record. First-occurrence-only semantics are
//! unit-tested at the domain layer (`Issue::mark_*`); here we verify the command
//! paths wire those helpers in.

use jit::domain::{Event, State};
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

/// Completing a dependency auto-transitions the dependent to Ready THROUGH the
/// chokepoint, leaving it Ready with `first_ready_at` set.
///
/// (A harness-created issue is dependency-free and therefore born Ready — hence
/// already stamped — before `add_dependency` reverts it to Backlog; the
/// first-occurrence chokepoint stamp is exercised precisely by
/// `test_backlog_to_ready_transition_stamps_first_ready` below.)
#[test]
fn test_auto_ready_on_dependency_completion_leaves_ready_stamp() {
    let h = TestHarness::new();
    let dep = h.create_issue("Dependency");
    let blocked = h.create_issue("Blocked");
    h.executor.add_dependency(&blocked, &dep).unwrap();

    // Adding the dependency reverted the dependent to Backlog.
    assert_eq!(h.get_issue(&blocked).state, State::Backlog);

    // Complete the dependency through `update_issue` (which cascades ready
    // auto-transitions to dependents); the dependent returns to Ready.
    h.executor
        .issues()
        .update_issue(
            &dep,
            None,
            None,
            None,
            Some(State::Done),
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();

    let after = h.get_issue(&blocked);
    assert_eq!(after.state, State::Ready);
    assert!(after.first_ready_at.is_some());
}

/// A precise chokepoint check: an issue that has NEVER been Ready (forced into
/// Backlog with the stamp cleared) gets `first_ready_at` set the first time
/// `update_issue_state` lands it in Ready.
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

    h.executor
        .issues()
        .update_issue_state(&id, State::Ready)
        .unwrap();

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
        .claim_issue(&id, "agent:worker-1".to_string())
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
        .assign_issue(&id, "human:alice".to_string())
        .unwrap();

    assert!(h.get_issue(&id).claimed_at.is_some());

    let claimed = h
        .storage
        .read_events()
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e, Event::IssueClaimed { .. }))
        .count();
    assert_eq!(claimed, 1);
}

/// Transitioning an issue to Done stamps `done_at`.
#[test]
fn test_done_transition_stamps_done_at() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Finishable");
    assert!(h.get_issue(&id).done_at.is_none());

    h.executor
        .issues()
        .update_issue_state(&id, State::Done)
        .unwrap();

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
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();
    let first = h.get_issue(&id).claimed_at;
    assert!(first.is_some());

    // Re-claiming as the same assignee promotes/no-ops but must not re-stamp.
    h.executor
        .issues()
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();
    assert_eq!(h.get_issue(&id).claimed_at, first);
}
