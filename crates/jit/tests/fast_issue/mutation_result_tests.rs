use jit::commands::IssueLocalUpdate;
use jit::domain::identity::IssueIdPrefix;
use jit::domain::{Assignee, EventBody, Issue};
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;

fn assert_persisted(h: &TestHarness, issue: Issue) {
    assert_eq!(issue, h.get_issue(&issue.id));
}

#[test]
fn test_assign_issue_returns_updated_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    let prefix = id.as_str()[..8].parse::<IssueIdPrefix>().unwrap();
    let issue = h
        .executor
        .issues()
        .assign_issue(&prefix, "agent:test".parse().unwrap())
        .unwrap()
        .issue;
    assert_persisted(&h, issue);
}

#[test]
fn test_unassign_issue_returns_updated_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(&id), "agent:test".parse().unwrap())
        .unwrap();
    let prefix = id.as_str()[..8].parse::<IssueIdPrefix>().unwrap();
    let issue = h.executor.issues().unassign_issue(&prefix).unwrap().issue;
    assert_persisted(&h, issue);
}

#[test]
fn test_claim_issue_returns_updated_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    let prefix = id.as_str()[..8].parse::<IssueIdPrefix>().unwrap();
    let issue = h
        .executor
        .issues()
        .claim_issue(&prefix, "agent:test".parse().unwrap())
        .unwrap()
        .issue;
    assert_persisted(&h, issue);
}

#[test]
fn test_release_issue_returns_updated_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(&id), "agent:test".parse().unwrap())
        .unwrap();
    let prefix = id.as_str()[..8].parse::<IssueIdPrefix>().unwrap();
    let issue = h
        .executor
        .issues()
        .release_issue_as(
            &prefix,
            "finished attempt",
            "human:reviewer".parse().unwrap(),
        )
        .unwrap();
    assert_persisted(&h, issue);
}

#[test]
fn test_release_issue_as_records_actor_independently_of_previous_assignee() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    let prefix = IssueIdPrefix::from(&id);
    let assignee: Assignee = "agent:worker".parse().unwrap();
    let actor: Assignee = "human:reviewer".parse().unwrap();
    h.executor
        .issues()
        .claim_issue(&prefix, assignee.clone())
        .unwrap();

    h.executor
        .issues()
        .release_issue_as(&prefix, "handoff", actor.clone())
        .unwrap();

    let releases: Vec<_> = h
        .storage
        .read_events()
        .unwrap()
        .into_iter()
        .filter_map(|event| match event.body().clone() {
            EventBody::IssueReleased {
                issue_id,
                assignee,
                reason,
                released_by,
            } => Some((issue_id, assignee, reason, released_by)),
            _ => None,
        })
        .collect();
    assert_eq!(
        releases,
        vec![(id, assignee, "handoff".into(), Some(actor))]
    );
}

#[test]
fn test_release_issue_resolves_missing_issue_before_acting_identity() {
    let h = TestHarness::new();
    let prefix: IssueIdPrefix = "00000000".parse().unwrap();
    let lookup_error = h.storage.resolve_issue_id(&prefix).unwrap_err();
    let events_before = h.storage.read_events().unwrap();

    let error = h
        .executor
        .issues()
        .release_issue(&prefix, "handoff")
        .unwrap_err();

    assert_eq!(error.to_string(), lookup_error.to_string());
    assert_eq!(h.storage.read_events().unwrap(), events_before);
}

#[test]
fn test_reject_issue_returns_updated_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    let prefix = id.as_str()[..8].parse::<IssueIdPrefix>().unwrap();
    let issue = h
        .executor
        .issues()
        .reject_issue(&prefix, Some("obsolete"))
        .unwrap()
        .issue;
    assert_persisted(&h, issue);
}

#[test]
fn test_claim_next_returns_updated_issue() {
    let h = TestHarness::new();
    h.create_ready_issue("Task");
    let issue = h
        .executor
        .issues()
        .claim_next("agent:test".parse().unwrap(), None)
        .unwrap()
        .issue;
    assert_persisted(&h, issue);
}

#[test]
fn test_update_issue_resolved_returns_updated_issue() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    let prefix = id.as_str()[..8].parse::<IssueIdPrefix>().unwrap();
    let issue = h
        .executor
        .issues()
        .update_issue_resolved(
            &prefix,
            Some("Changed".into()),
            None,
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            IssueLocalUpdate::default(),
            false,
        )
        .unwrap()
        .issue;
    assert_persisted(&h, issue);
}

#[test]
fn test_claim_issue_returns_unchanged_issue_when_already_claimed() {
    let h = TestHarness::new();
    let id = h.create_ready_issue("Task");
    let prefix = IssueIdPrefix::from(&id);
    let assignee = "agent:test".parse().unwrap();
    let first = h.executor.issues().claim_issue(&prefix, assignee).unwrap();
    let repeated = h
        .executor
        .issues()
        .claim_issue(&prefix, "agent:test".parse().unwrap())
        .unwrap();
    assert_eq!(repeated.issue, first.issue);
    assert_persisted(&h, repeated.issue);
}
