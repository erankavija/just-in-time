//! Unit tests for domain query functions
//!
//! These tests verify that domain query operations work correctly as pure functions
//! independent of CLI orchestration.

use jit::domain::identity::IssueId;
use jit::domain::*;
use jit::test_utils::issue_id;
use std::collections::HashMap;

fn make_issue(id: &IssueId, title: &str, state: State) -> Issue {
    Issue {
        id: id.clone(),
        title: title.to_string(),
        description: String::new(),
        state,
        priority: Priority::Normal,
        assignee: None,
        dependencies: Vec::new(),
        gates_required: Vec::new(),
        gates_status: HashMap::new(),
        context: HashMap::new(),
        documents: Vec::new(),
        labels: Vec::new(),
        content_format: None,
        created_at: "2024-01-01T00:00:00Z".parse().unwrap(),
        updated_at: "2024-01-01T00:00:00Z".parse().unwrap(),
        first_ready_at: None,
        claimed_at: None,
        done_at: None,
        archived_from: None,
    }
}

fn make_issue_with_deps(id: &IssueId, title: &str, state: State, deps: Vec<IssueId>) -> Issue {
    Issue {
        id: id.clone(),
        title: title.to_string(),
        description: String::new(),
        state,
        priority: Priority::Normal,
        assignee: None,
        dependencies: deps,
        gates_required: Vec::new(),
        gates_status: HashMap::new(),
        context: HashMap::new(),
        documents: Vec::new(),
        labels: Vec::new(),
        content_format: None,
        created_at: "2024-01-01T00:00:00Z".parse().unwrap(),
        updated_at: "2024-01-01T00:00:00Z".parse().unwrap(),
        first_ready_at: None,
        claimed_at: None,
        done_at: None,
        archived_from: None,
    }
}

#[test]
fn test_query_ready_finds_unassigned_ready_unblocked_issues() {
    let issues = vec![
        make_issue(&issue_id(1), "Ready issue", State::Ready),
        make_issue(&issue_id(2), "Done issue", State::Done),
        make_issue(&issue_id(3), "Another ready", State::Ready),
    ];

    let ready = jit::domain::queries::query_ready(&issues);

    assert_eq!(ready.len(), 2);
    assert!(ready.iter().any(|i| i.id == issue_id(1)));
    assert!(ready.iter().any(|i| i.id == issue_id(3)));
}

#[test]
fn test_query_ready_excludes_assigned_issues() {
    let mut issue1 = make_issue(&issue_id(1), "Ready but assigned", State::Ready);
    issue1.assignee = Some("agent:worker-1".parse().unwrap());

    let issue2 = make_issue(&issue_id(2), "Ready unassigned", State::Ready);

    let issues = vec![issue1, issue2];
    let ready = jit::domain::queries::query_ready(&issues);

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, issue_id(2));
}

#[test]
fn test_query_ready_excludes_blocked_issues() {
    let dep = make_issue(&issue_id(11), "Dependency", State::InProgress);
    let blocked = make_issue_with_deps(&issue_id(1), "Blocked", State::Ready, vec![issue_id(11)]);
    let ready = make_issue(&issue_id(2), "Ready", State::Ready);

    let issues = vec![dep, blocked, ready];
    let result = jit::domain::queries::query_ready(&issues);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, issue_id(2));
}

#[test]
fn test_query_by_state_filters_correctly() {
    let issues = vec![
        make_issue(&issue_id(1), "Ready", State::Ready),
        make_issue(&issue_id(2), "Done", State::Done),
        make_issue(&issue_id(3), "InProgress", State::InProgress),
        make_issue(&issue_id(4), "Another ready", State::Ready),
    ];

    let ready_issues = jit::domain::queries::query_by_state(&issues, State::Ready);
    assert_eq!(ready_issues.len(), 2);

    let done_issues = jit::domain::queries::query_by_state(&issues, State::Done);
    assert_eq!(done_issues.len(), 1);
    assert_eq!(done_issues[0].id, issue_id(2));
}

#[test]
fn test_query_by_priority_filters_correctly() {
    let mut issue1 = make_issue(&issue_id(1), "Critical", State::Ready);
    issue1.priority = Priority::Critical;

    let mut issue2 = make_issue(&issue_id(2), "High", State::Ready);
    issue2.priority = Priority::High;

    let mut issue3 = make_issue(&issue_id(3), "Normal", State::Ready);
    issue3.priority = Priority::Normal;

    let issues = vec![issue1, issue2, issue3];

    let critical = jit::domain::queries::query_by_priority(&issues, Priority::Critical);
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].id, issue_id(1));

    let high = jit::domain::queries::query_by_priority(&issues, Priority::High);
    assert_eq!(high.len(), 1);
    assert_eq!(high[0].id, issue_id(2));
}

#[test]
fn test_query_by_assignee_filters_correctly() {
    let mut issue1 = make_issue(&issue_id(1), "Assigned to alice", State::InProgress);
    issue1.assignee = Some("agent:alice".parse().unwrap());

    let mut issue2 = make_issue(&issue_id(2), "Assigned to bob", State::InProgress);
    issue2.assignee = Some("agent:bob".parse().unwrap());

    let issue3 = make_issue(&issue_id(3), "Unassigned", State::Ready);

    let issues = vec![issue1, issue2, issue3];

    let alice_issues = jit::domain::queries::query_by_assignee(&issues, "agent:alice");
    assert_eq!(alice_issues.len(), 1);
    assert_eq!(alice_issues[0].id, issue_id(1));

    let bob_issues = jit::domain::queries::query_by_assignee(&issues, "agent:bob");
    assert_eq!(bob_issues.len(), 1);
    assert_eq!(bob_issues[0].id, issue_id(2));
}

#[test]
fn test_query_closed_includes_done_and_rejected() {
    let issues = vec![
        make_issue(&issue_id(1), "Done", State::Done),
        make_issue(&issue_id(2), "Ready", State::Ready),
        make_issue(&issue_id(3), "Rejected", State::Rejected),
        make_issue(&issue_id(4), "InProgress", State::InProgress),
    ];

    let closed = jit::domain::queries::query_closed(&issues);

    assert_eq!(closed.len(), 2);
    assert!(closed.iter().any(|i| i.id == issue_id(1)));
    assert!(closed.iter().any(|i| i.id == issue_id(3)));
}

#[test]
fn test_query_blocked_finds_issues_with_unmet_deps() {
    let dep1 = make_issue(&issue_id(11), "Unmet dep", State::InProgress);
    let dep2 = make_issue(&issue_id(12), "Done dep", State::Done);
    let blocked = make_issue_with_deps(
        &issue_id(21),
        "Blocked issue",
        State::Backlog,
        vec![issue_id(11), issue_id(12)],
    );
    let ready = make_issue(&issue_id(22), "Ready", State::Ready);

    let issues = vec![dep1, dep2, blocked, ready];
    let blocked_result = jit::domain::queries::query_blocked(&issues);

    use jit::domain::queries::BlockingReason;
    assert_eq!(blocked_result.len(), 1);
    assert_eq!(blocked_result[0].0.id, issue_id(21));
    assert!(blocked_result[0].1.iter().any(|r| matches!(
        r,
        BlockingReason::Dependency { id, .. } if *id == issue_id(11)
    )));
    // Display round-trips to the canonical reason string.
    assert!(blocked_result[0].1.iter().any(|r| r
        .to_string()
        .contains(&format!("dependency:{}", issue_id(11)))));
}

#[test]
fn test_query_by_label_matches_exact() {
    let mut issue1 = make_issue(&issue_id(1), "Epic issue", State::Ready);
    issue1.labels = vec!["type:epic".to_string()];

    let mut issue2 = make_issue(&issue_id(2), "Task issue", State::Ready);
    issue2.labels = vec!["type:task".to_string()];

    let mut issue3 = make_issue(&issue_id(3), "Another epic", State::Ready);
    issue3.labels = vec!["type:epic".to_string()];

    let issues = vec![issue1, issue2, issue3];

    let epics = jit::domain::queries::query_by_label(&issues, "type:epic");
    assert_eq!(epics.len(), 2);
    assert!(epics.iter().any(|i| i.id == issue_id(1)));
    assert!(epics.iter().any(|i| i.id == issue_id(3)));
}

#[test]
fn test_query_by_label_matches_wildcard() {
    let mut issue1 = make_issue(&issue_id(1), "Epic", State::Ready);
    issue1.labels = vec!["epic:auth".to_string()];

    let mut issue2 = make_issue(&issue_id(2), "Task", State::Ready);
    issue2.labels = vec!["epic:api".to_string()];

    let issue3 = make_issue(&issue_id(3), "No epic", State::Ready);

    let issues = vec![issue1, issue2, issue3];

    let with_epic = jit::domain::queries::query_by_label(&issues, "epic:*");
    assert_eq!(with_epic.len(), 2);
    assert!(with_epic.iter().any(|i| i.id == issue_id(1)));
    assert!(with_epic.iter().any(|i| i.id == issue_id(2)));
}
