use jit::commands::IssueLocalUpdate;
use jit::domain::identity::IssueIdPrefix;
use jit::domain::Issue;
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
        .release_issue(&prefix, "finished attempt")
        .unwrap();
    assert_persisted(&h, issue);
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
