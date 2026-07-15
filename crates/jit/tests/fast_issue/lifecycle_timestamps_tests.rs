//! Lifecycle-timestamp write-point tests (jit:0ab468ba).
//!
//! Exercises every path that stamps `first_ready_at`, `claimed_at`, or
//! `done_at` on the issue record, plus the one-time backfill migration derived
//! from the event log. First-occurrence-only semantics are unit-tested at the
//! domain layer (`Issue::mark_*`, `derive_lifecycle_timestamps`); here we verify
//! the command paths wire those helpers in.

use crate::harness::TestHarness;
use chrono::{TimeZone, Utc};
use jit::domain::{Event, State};
use jit::storage::IssueStore;

/// Append a synthetic `issue_state_changed` event with a fixed timestamp.
fn append_state_changed(h: &TestHarness, issue_id: &str, to: State, day: u32) {
    let event = Event::IssueStateChanged {
        id: uuid::Uuid::new_v4().to_string(),
        issue_id: issue_id.to_string(),
        timestamp: Utc.with_ymd_and_hms(2026, 1, day, 0, 0, 0).unwrap(),
        from: State::Backlog,
        to,
    };
    h.storage.append_event(&event).unwrap();
}

/// Strip the lifecycle timestamps off an issue to simulate a record written
/// before the transition-time write points existed.
fn clear_lifecycle_fields(h: &TestHarness, id: &str) {
    let mut issue = h.get_issue(id);
    issue.first_ready_at = None;
    issue.claimed_at = None;
    issue.done_at = None;
    h.storage.save_issue(issue).unwrap();
}

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
    h.storage.save_issue(issue).unwrap();

    h.executor.update_issue_state(&id, State::Ready).unwrap();

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
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();

    let issue = h.get_issue(&id);
    assert!(issue.claimed_at.is_some());
    assert_eq!(issue.state, State::InProgress);
}

/// Assigning an issue (without claiming) stamps `claimed_at` and logs an
/// `issue_claimed` event (@/inv/event-log) so the mutation is auditable and the
/// backfill can fold it.
#[test]
fn test_assign_stamps_claimed_at_and_logs_event() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Assignable");

    h.executor
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

    h.executor.update_issue_state(&id, State::Done).unwrap();

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
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();
    let first = h.get_issue(&id).claimed_at;
    assert!(first.is_some());

    // Re-claiming as the same assignee promotes/no-ops but must not re-stamp.
    h.executor
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();
    assert_eq!(h.get_issue(&id).claimed_at, first);
}

// ========== Backfill migration ==========

/// The backfill derives all three timestamps from the event log for a
/// legacy-shaped issue and reports one update.
#[test]
fn test_backfill_derives_timestamps_from_events() {
    let h = TestHarness::new();
    let id = h.create_issue("Legacy");
    clear_lifecycle_fields(&h, &id);

    append_state_changed(&h, &id, State::Ready, 1);
    let claimed = Utc.with_ymd_and_hms(2026, 1, 2, 0, 0, 0).unwrap();
    h.storage
        .append_event(&Event::IssueClaimed {
            id: uuid::Uuid::new_v4().to_string(),
            issue_id: id.clone(),
            timestamp: claimed,
            assignee: "agent:worker-1".parse().unwrap(),
        })
        .unwrap();
    append_state_changed(&h, &id, State::Done, 3);

    let result = h.executor.backfill_lifecycle_timestamps().unwrap();
    assert_eq!(result.issues_scanned, 1);
    assert_eq!(result.issues_updated, 1);

    let issue = h.get_issue(&id);
    assert_eq!(
        issue.first_ready_at,
        Some(Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap())
    );
    assert_eq!(issue.claimed_at, Some(claimed));
    assert_eq!(
        issue.done_at,
        Some(Utc.with_ymd_and_hms(2026, 1, 3, 0, 0, 0).unwrap())
    );
}

/// Re-running the backfill over an already-migrated repository changes nothing
/// and appends no migration event.
#[test]
fn test_backfill_is_idempotent() {
    let h = TestHarness::new();
    let id = h.create_issue("Legacy");
    clear_lifecycle_fields(&h, &id);
    append_state_changed(&h, &id, State::Ready, 1);
    append_state_changed(&h, &id, State::Done, 3);

    let first = h.executor.backfill_lifecycle_timestamps().unwrap();
    assert_eq!(first.issues_updated, 1);
    let events_after_first = h.storage.read_events().unwrap().len();

    let second = h.executor.backfill_lifecycle_timestamps().unwrap();
    assert_eq!(second.issues_updated, 0);
    // No migration event on the no-op re-run.
    assert_eq!(h.storage.read_events().unwrap().len(), events_after_first);
}

/// An issue with no relevant events keeps its timestamps unset and is not
/// counted as updated.
#[test]
fn test_backfill_leaves_eventless_issue_unset() {
    let h = TestHarness::new();
    let id = h.create_issue("Legacy");
    clear_lifecycle_fields(&h, &id);
    // No lifecycle events appended for this issue.

    let result = h.executor.backfill_lifecycle_timestamps().unwrap();
    assert_eq!(result.issues_updated, 0);

    let issue = h.get_issue(&id);
    assert!(issue.first_ready_at.is_none());
    assert!(issue.claimed_at.is_none());
    assert!(issue.done_at.is_none());
}

/// The backfill never overwrites an already-present timestamp (first-occurrence
/// preserved), even if the event log carries a later signal.
#[test]
fn test_backfill_preserves_existing_timestamp() {
    let h = TestHarness::new();
    let id = h.create_issue("Partly stamped");
    // Keep the birth first_ready_at; clear only done, then log a Done event.
    let existing_ready = h.get_issue(&id).first_ready_at;
    assert!(existing_ready.is_some());
    append_state_changed(&h, &id, State::Ready, 9); // a later, different ready ts
    append_state_changed(&h, &id, State::Done, 10);

    h.executor.backfill_lifecycle_timestamps().unwrap();

    let issue = h.get_issue(&id);
    // first_ready_at is untouched (birth value, not the day-9 event).
    assert_eq!(issue.first_ready_at, existing_ready);
    // done_at was absent, so it is filled from the event.
    assert_eq!(
        issue.done_at,
        Some(Utc.with_ymd_and_hms(2026, 1, 10, 0, 0, 0).unwrap())
    );
}

/// The backfill appends exactly one repo-scoped migration event carrying the
/// updated count.
#[test]
fn test_backfill_appends_migration_event() {
    let h = TestHarness::new();
    let id = h.create_issue("Legacy");
    clear_lifecycle_fields(&h, &id);
    append_state_changed(&h, &id, State::Done, 3);

    h.executor.backfill_lifecycle_timestamps().unwrap();

    let migration_events: Vec<_> = h
        .storage
        .read_events()
        .unwrap()
        .into_iter()
        .filter(|e| matches!(e, Event::LifecycleTimestampsBackfilled { .. }))
        .collect();
    assert_eq!(migration_events.len(), 1);
    match &migration_events[0] {
        Event::LifecycleTimestampsBackfilled { issues_updated, .. } => {
            assert_eq!(*issues_updated, 1);
        }
        _ => unreachable!(),
    }
}
