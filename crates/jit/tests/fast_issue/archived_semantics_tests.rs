//! Lifecycle matrix for terminality-preserving `Archived` (jit:45a140ae).
//!
//! `Archived` preserves what was true before it: an issue archived from a
//! terminal state keeps satisfying dependents; one archived from a non-terminal
//! state does not. Reviving restores
//! the recorded pre-archive state exactly, closing the `done → archived → ready`
//! resurrection loophole. These tests exercise the dependency, readiness,
//! transition, rollup, revive, and event-log surfaces.
use jit::commands::StateRollup;
use jit::domain::identity::IssueId;
use jit::domain::{Issue, State};
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;
use jit::test_utils::add_reducing_edge;

/// Drive a dependency-free issue to `Done` (no gates required).
fn drive_to_done(h: &TestHarness, id: &IssueId) {
    jit::test_utils::transition_issue(&h.executor, id, State::InProgress).unwrap();
    jit::test_utils::transition_issue(&h.executor, id, State::Done).unwrap();
    assert_eq!(h.get_issue(id).state, State::Done);
}

/// Park an issue into `Archived` the way `jit issue update --state archived` does.
fn archive(h: &TestHarness, id: &IssueId) {
    jit::test_utils::transition_issue(&h.executor, id, State::Archived).unwrap();
}

fn count_state_changes(h: &TestHarness, id: &IssueId) -> usize {
    h.storage
        .read_events()
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(event.body(), jit::domain::EventBody::IssueStateChanged { issue_id, .. } if issue_id == id)
        })
        .count()
}

#[test]
fn test_archived_from_done_still_satisfies_dependents() {
    let h = TestHarness::new();
    let dependency = h.create_issue("Foundation");
    drive_to_done(&h, &dependency);
    archive(&h, &dependency);
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();
    jit::test_utils::transition_issue(&h.executor, &dependent, State::Backlog).unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Backlog);

    // The archived-from-Done dependency remains met: the dependent reaches Ready.
    jit::test_utils::transition_issue(&h.executor, &dependent, State::Ready).unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Ready);
    assert_eq!(h.get_issue(&dependency).archived_from, Some(State::Done));
}

#[test]
fn test_archived_from_non_terminal_does_not_satisfy_dependents() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Parked mid-flight");
    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();
    jit::test_utils::transition_issue(&h.executor, &dependency, State::InProgress).unwrap();
    archive(&h, &dependency);
    assert_eq!(
        h.get_issue(&dependency).archived_from,
        Some(State::InProgress)
    );

    // Archived from a non-terminal state does not start satisfying dependents.
    let result = jit::test_utils::transition_issue(&h.executor, &dependent, State::Ready);
    assert!(
        result.is_err(),
        "a dependency archived from a non-terminal state must still block"
    );
}

#[test]
fn test_revive_restores_done_exactly_and_clears_origin() {
    let h = TestHarness::new();
    let id = h.create_issue("Completed then archived");
    drive_to_done(&h, &id);
    archive(&h, &id);
    assert_eq!(h.get_issue(&id).archived_from, Some(State::Done));

    // Reviving to the recorded origin succeeds and clears archived_from.
    jit::test_utils::transition_issue(&h.executor, &id, State::Done).unwrap();
    let revived = h.get_issue(&id);
    assert_eq!(revived.state, State::Done);
    assert_eq!(revived.archived_from, None);
}

#[test]
fn test_revive_to_non_origin_is_refused_closing_resurrection_loophole() {
    let h = TestHarness::new();
    let id = h.create_issue("Completed then archived");
    drive_to_done(&h, &id);
    archive(&h, &id);

    // done -> archived -> ready is refused: the archive round-trip cannot
    // resurrect a completed issue into the active lifecycle.
    let error = jit::test_utils::transition_issue(&h.executor, &id, State::Ready)
        .expect_err("archived-from-done issue must not revive to ready");
    let rendered = format!("{error:#}");
    assert!(
        rendered.contains("pre-archive state") && rendered.contains("done"),
        "diagnostic must name the only legal revive target: {rendered}"
    );
    // The refusal persists nothing: the issue stays archived.
    assert_eq!(h.get_issue(&id).state, State::Archived);
    assert!(error.is::<jit::errors::TransitionBlockedError>());
}

#[test]
fn test_archived_from_rejected_revives_only_to_rejected_and_stays_terminal() {
    let h = TestHarness::new();
    let dependency = h.create_issue("Abandoned then archived");
    jit::test_utils::transition_issue(&h.executor, &dependency, State::Rejected).unwrap();
    archive(&h, &dependency);
    assert_eq!(
        h.get_issue(&dependency).archived_from,
        Some(State::Rejected)
    );
    let dependent = h.create_issue("Dependent");
    add_reducing_edge(&h.executor, &dependent, &dependency).unwrap();
    jit::test_utils::transition_issue(&h.executor, &dependent, State::Backlog).unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Backlog);

    // Effectively terminal: the dependent is unblocked.
    jit::test_utils::transition_issue(&h.executor, &dependent, State::Ready).unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Ready);

    // Reviving to a non-origin (done) is refused; reviving to rejected succeeds.
    assert!(jit::test_utils::transition_issue(&h.executor, &dependency, State::Done).is_err());
    jit::test_utils::transition_issue(&h.executor, &dependency, State::Rejected).unwrap();
    assert_eq!(h.get_issue(&dependency).state, State::Rejected);
}

#[test]
fn test_archive_from_ready_records_origin_and_revives_to_ready() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Parked from ready");
    archive(&h, &id);
    assert_eq!(h.get_issue(&id).archived_from, Some(State::Ready));

    // Reviving to done (not the origin) is refused; reviving to ready succeeds.
    assert!(jit::test_utils::transition_issue(&h.executor, &id, State::Done).is_err());
    jit::test_utils::transition_issue(&h.executor, &id, State::Ready).unwrap();
    assert_eq!(h.get_issue(&id).state, State::Ready);
    assert_eq!(h.get_issue(&id).archived_from, None);
}

#[test]
fn test_rollup_folds_archived_by_terminal_origin() {
    let h = TestHarness::new();
    let delivered = h.create_issue("Delivered then archived");
    drive_to_done(&h, &delivered);
    archive(&h, &delivered);
    let abandoned = h.create_issue("Rejected then archived");
    jit::test_utils::transition_issue(&h.executor, &abandoned, State::Rejected).unwrap();
    archive(&h, &abandoned);
    let parked = h.create_issue("Parked mid-flight");
    jit::test_utils::transition_issue(&h.executor, &parked, State::InProgress).unwrap();
    archive(&h, &parked);

    let rollup = StateRollup::from_issues(&h.all_issues());
    assert_eq!(rollup.extras.total, 3);
    // Archived-from-Done counts as delivered, archived-from-Rejected as rejected,
    // archived-from-non-terminal as open.
    assert_eq!(rollup.extras.done, 1, "archived-from-done is delivered");
    assert_eq!(
        rollup.extras.rejected, 1,
        "archived-from-rejected is rejected"
    );
    assert_eq!(rollup.extras.open, 1, "archived-from-non-terminal is open");
    assert_eq!(rollup.extras.percent, 33);
}

#[test]
fn test_entering_and_leaving_archived_each_log_state_change() {
    let h = TestHarness::new();
    let id = h.create_issue("Completed then archived");
    drive_to_done(&h, &id); // backlog->? plus in_progress + done state changes
    let before = count_state_changes(&h, &id);
    archive(&h, &id); // one state change into Archived
    jit::test_utils::transition_issue(&h.executor, &id, State::Done).unwrap(); // one revive change
    let after = count_state_changes(&h, &id);
    assert_eq!(
        after - before,
        2,
        "entering Archived and reviving each append an issue_state_changed event"
    );
}

#[test]
fn test_archived_to_archived_is_noop() {
    let h = TestHarness::new();
    let id = h.create_issue("Completed then archived");
    drive_to_done(&h, &id);
    archive(&h, &id);
    let before = count_state_changes(&h, &id);
    // Re-archiving is a no-op: no event, origin unchanged.
    jit::test_utils::transition_issue(&h.executor, &id, State::Archived).unwrap();
    assert_eq!(count_state_changes(&h, &id), before);
    assert_eq!(h.get_issue(&id).archived_from, Some(State::Done));
}

#[test]
fn test_archived_from_field_absent_on_active_issue_roundtrips() {
    // An issue that never entered Archived carries no archived_from, and the
    // field is skipped on serialize so existing issue files round-trip.
    let issue = crate::fixture_issue("Active".to_string(), String::new());
    assert_eq!(issue.archived_from, None);
    let json = serde_json::to_string(&issue).unwrap();
    assert!(
        !json.contains("archived_from"),
        "archived_from is skipped when absent: {json}"
    );
    let restored: Issue = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.archived_from, None);
}
