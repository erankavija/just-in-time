//! Lifecycle matrix for terminality-preserving `Archived` (jit:45a140ae).
//!
//! `Archived` preserves what was true before it: an issue archived from a
//! terminal state keeps satisfying dependents; one archived from a non-terminal
//! state (or a legacy record with no recorded origin) does not. Reviving restores
//! the recorded pre-archive state exactly, closing the `done → archived → ready`
//! resurrection loophole. These tests exercise the dependency, readiness,
//! transition, rollup, revive, event-log, and compatibility surfaces.
use crate::harness::TestHarness;
use jit::domain::{Issue, State};
use jit::output::StateRollup;
use jit::storage::IssueStore;

/// Drive a dependency-free issue to `Done` (no gates required).
fn drive_to_done(h: &TestHarness, id: &str) {
    h.executor
        .update_issue_state(id, State::InProgress)
        .unwrap();
    h.executor.update_issue_state(id, State::Done).unwrap();
    assert_eq!(h.get_issue(id).state, State::Done);
}

/// Park an issue into `Archived` the way `jit issue update --state archived` does.
fn archive(h: &TestHarness, id: &str) {
    h.executor.update_issue_state(id, State::Archived).unwrap();
}

/// Persist a legacy `Archived` record: state `Archived` with no recorded origin,
/// as an issue file written before `archived_from` existed would deserialize.
fn save_legacy_archived(h: &TestHarness, id: &str) {
    let mut issue = h.get_issue(id);
    issue.state = State::Archived;
    issue.archived_from = None;
    crate::harness::seed_memory_issue(&h.storage, &issue);
}

fn count_state_changes(h: &TestHarness, id: &str) -> usize {
    h.storage
        .read_events()
        .unwrap()
        .iter()
        .filter(|event| {
            matches!(event, jit::domain::Event::IssueStateChanged { issue_id, .. } if issue_id == id)
        })
        .count()
}

#[test]
fn test_archived_from_done_still_satisfies_dependents() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Foundation");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    drive_to_done(&h, &dependency);
    archive(&h, &dependency);

    // The archived-from-Done dependency remains met: the dependent reaches Ready.
    h.executor
        .update_issue_state(&dependent, State::Ready)
        .unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Ready);
    assert_eq!(h.get_issue(&dependency).archived_from, Some(State::Done));
}

#[test]
fn test_archived_from_non_terminal_does_not_satisfy_dependents() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Parked mid-flight");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    h.executor
        .update_issue_state(&dependency, State::InProgress)
        .unwrap();
    archive(&h, &dependency);
    assert_eq!(
        h.get_issue(&dependency).archived_from,
        Some(State::InProgress)
    );

    // Archived from a non-terminal state does not start satisfying dependents.
    let result = h.executor.update_issue_state(&dependent, State::Ready);
    assert!(
        result.is_err(),
        "a dependency archived from a non-terminal state must still block"
    );
}

#[test]
fn test_legacy_archived_without_origin_does_not_satisfy_dependents() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Legacy parked");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    save_legacy_archived(&h, &dependency);

    // Conservative compatibility default: a legacy Archived record (no recorded
    // origin) is not effectively terminal, matching the pre-change behavior.
    let result = h.executor.update_issue_state(&dependent, State::Ready);
    assert!(
        result.is_err(),
        "a legacy archived dependency (no recorded origin) must not unblock dependents"
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
    h.executor.update_issue_state(&id, State::Done).unwrap();
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
    let error = h
        .executor
        .update_issue_state(&id, State::Ready)
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
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Abandoned then archived");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    h.executor
        .update_issue_state(&dependency, State::Rejected)
        .unwrap();
    archive(&h, &dependency);
    assert_eq!(
        h.get_issue(&dependency).archived_from,
        Some(State::Rejected)
    );

    // Effectively terminal: the dependent is unblocked.
    h.executor
        .update_issue_state(&dependent, State::Ready)
        .unwrap();
    assert_eq!(h.get_issue(&dependent).state, State::Ready);

    // Reviving to a non-origin (done) is refused; reviving to rejected succeeds.
    assert!(h
        .executor
        .update_issue_state(&dependency, State::Done)
        .is_err());
    h.executor
        .update_issue_state(&dependency, State::Rejected)
        .unwrap();
    assert_eq!(h.get_issue(&dependency).state, State::Rejected);
}

#[test]
fn test_archive_from_ready_records_origin_and_revives_to_ready() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Parked from ready");
    archive(&h, &id);
    assert_eq!(h.get_issue(&id).archived_from, Some(State::Ready));

    // Reviving to done (not the origin) is refused; reviving to ready succeeds.
    assert!(h.executor.update_issue_state(&id, State::Done).is_err());
    h.executor.update_issue_state(&id, State::Ready).unwrap();
    assert_eq!(h.get_issue(&id).state, State::Ready);
    assert_eq!(h.get_issue(&id).archived_from, None);
}

#[test]
fn test_legacy_archived_revive_is_unconstrained_but_warns() {
    let h = TestHarness::new();
    let id = h.create_issue("Legacy parked");
    save_legacy_archived(&h, &id);

    // A legacy record with no recorded origin keeps the prior unconstrained
    // revive, with an advisory warning naming the missing origin.
    let warnings = h.executor.update_issue_state(&id, State::Ready).unwrap();
    assert_eq!(h.get_issue(&id).state, State::Ready);
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("pre-archive state was recorded")),
        "legacy revive should warn about the unrecorded origin: {warnings:?}"
    );
}

#[test]
fn test_archived_is_not_ready_but_terminal_origin_unblocks_dependents() {
    let h = TestHarness::new();
    let dependent = h.create_issue("Dependent");
    let dependency = h.create_issue("Foundation");
    h.executor.add_dependency(&dependent, &dependency).unwrap();
    drive_to_done(&h, &dependency);
    archive(&h, &dependency);

    // The archived issue is never itself in the ready set.
    let ready = jit::domain::queries::query_ready(&h.all_issues());
    assert!(ready.iter().all(|issue| issue.id != dependency));

    // But its dependent, now unblocked, is claimable.
    let dependent_issue = h.get_issue(&dependent);
    assert!(!dependent_issue.is_blocked(&jit::domain::queries::build_issue_map(&h.all_issues())));
}

#[test]
fn test_rollup_folds_archived_by_terminal_origin() {
    let h = TestHarness::new();
    let delivered = h.create_issue("Delivered then archived");
    drive_to_done(&h, &delivered);
    archive(&h, &delivered);
    let abandoned = h.create_issue("Rejected then archived");
    h.executor
        .update_issue_state(&abandoned, State::Rejected)
        .unwrap();
    archive(&h, &abandoned);
    let parked = h.create_issue("Parked mid-flight");
    h.executor
        .update_issue_state(&parked, State::InProgress)
        .unwrap();
    archive(&h, &parked);

    let rollup = StateRollup::from_issues(&h.all_issues());
    assert_eq!(rollup.total, 3);
    // Archived-from-Done counts as delivered, archived-from-Rejected as rejected,
    // archived-from-non-terminal as open.
    assert_eq!(rollup.done, 1, "archived-from-done is delivered");
    assert_eq!(rollup.rejected, 1, "archived-from-rejected is rejected");
    assert_eq!(rollup.open, 1, "archived-from-non-terminal is open");
    assert_eq!(rollup.percent, 33);
}

#[test]
fn test_query_closed_uses_effective_terminality() {
    let h = TestHarness::new();
    let delivered = h.create_issue("Delivered then archived");
    drive_to_done(&h, &delivered);
    archive(&h, &delivered);
    let parked = h.create_issue("Parked mid-flight");
    h.executor
        .update_issue_state(&parked, State::InProgress)
        .unwrap();
    archive(&h, &parked);

    let closed: Vec<String> = jit::domain::queries::query_closed(&h.all_issues())
        .into_iter()
        .map(|issue| issue.id)
        .collect();
    assert!(closed.contains(&delivered), "archived-from-done is closed");
    assert!(
        !closed.contains(&parked),
        "archived-from-non-terminal is not closed"
    );
}

#[test]
fn test_entering_and_leaving_archived_each_log_state_change() {
    let h = TestHarness::new();
    let id = h.create_issue("Completed then archived");
    drive_to_done(&h, &id); // backlog->? plus in_progress + done state changes
    let before = count_state_changes(&h, &id);
    archive(&h, &id); // one state change into Archived
    h.executor.update_issue_state(&id, State::Done).unwrap(); // one revive change
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
    h.executor.update_issue_state(&id, State::Archived).unwrap();
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
