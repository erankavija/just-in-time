//! Unit tests for domain query functions
//!
//! These tests verify that domain query operations work correctly as pure functions
//! independent of CLI orchestration.

use jit::domain::*;
use std::collections::HashMap;

fn make_issue(id: &str, title: &str, state: State) -> Issue {
    Issue {
        id: id.to_string(),
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
    }
}

fn make_issue_with_deps(id: &str, title: &str, state: State, deps: Vec<String>) -> Issue {
    Issue {
        id: id.to_string(),
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
    }
}

#[test]
fn test_query_ready_finds_unassigned_ready_unblocked_issues() {
    let issues = vec![
        make_issue("id1", "Ready issue", State::Ready),
        make_issue("id2", "Done issue", State::Done),
        make_issue("id3", "Another ready", State::Ready),
    ];

    let ready = jit::domain::queries::query_ready(&issues);

    assert_eq!(ready.len(), 2);
    assert!(ready.iter().any(|i| i.id == "id1"));
    assert!(ready.iter().any(|i| i.id == "id3"));
}

#[test]
fn test_query_ready_excludes_assigned_issues() {
    let mut issue1 = make_issue("id1", "Ready but assigned", State::Ready);
    issue1.assignee = Some("agent:worker-1".to_string());

    let issue2 = make_issue("id2", "Ready unassigned", State::Ready);

    let issues = vec![issue1, issue2];
    let ready = jit::domain::queries::query_ready(&issues);

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, "id2");
}

#[test]
fn test_query_ready_excludes_blocked_issues() {
    let dep = make_issue("dep1", "Dependency", State::InProgress);
    let blocked = make_issue_with_deps("id1", "Blocked", State::Ready, vec!["dep1".to_string()]);
    let ready = make_issue("id2", "Ready", State::Ready);

    let issues = vec![dep, blocked, ready];
    let result = jit::domain::queries::query_ready(&issues);

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].id, "id2");
}

#[test]
fn test_query_by_state_filters_correctly() {
    let issues = vec![
        make_issue("id1", "Ready", State::Ready),
        make_issue("id2", "Done", State::Done),
        make_issue("id3", "InProgress", State::InProgress),
        make_issue("id4", "Another ready", State::Ready),
    ];

    let ready_issues = jit::domain::queries::query_by_state(&issues, State::Ready);
    assert_eq!(ready_issues.len(), 2);

    let done_issues = jit::domain::queries::query_by_state(&issues, State::Done);
    assert_eq!(done_issues.len(), 1);
    assert_eq!(done_issues[0].id, "id2");
}

#[test]
fn test_query_by_priority_filters_correctly() {
    let mut issue1 = make_issue("id1", "Critical", State::Ready);
    issue1.priority = Priority::Critical;

    let mut issue2 = make_issue("id2", "High", State::Ready);
    issue2.priority = Priority::High;

    let mut issue3 = make_issue("id3", "Normal", State::Ready);
    issue3.priority = Priority::Normal;

    let issues = vec![issue1, issue2, issue3];

    let critical = jit::domain::queries::query_by_priority(&issues, Priority::Critical);
    assert_eq!(critical.len(), 1);
    assert_eq!(critical[0].id, "id1");

    let high = jit::domain::queries::query_by_priority(&issues, Priority::High);
    assert_eq!(high.len(), 1);
    assert_eq!(high[0].id, "id2");
}

#[test]
fn test_query_by_assignee_filters_correctly() {
    let mut issue1 = make_issue("id1", "Assigned to alice", State::InProgress);
    issue1.assignee = Some("agent:alice".to_string());

    let mut issue2 = make_issue("id2", "Assigned to bob", State::InProgress);
    issue2.assignee = Some("agent:bob".to_string());

    let issue3 = make_issue("id3", "Unassigned", State::Ready);

    let issues = vec![issue1, issue2, issue3];

    let alice_issues = jit::domain::queries::query_by_assignee(&issues, "agent:alice");
    assert_eq!(alice_issues.len(), 1);
    assert_eq!(alice_issues[0].id, "id1");

    let bob_issues = jit::domain::queries::query_by_assignee(&issues, "agent:bob");
    assert_eq!(bob_issues.len(), 1);
    assert_eq!(bob_issues[0].id, "id2");
}

#[test]
fn test_query_closed_includes_done_and_rejected() {
    let issues = vec![
        make_issue("id1", "Done", State::Done),
        make_issue("id2", "Ready", State::Ready),
        make_issue("id3", "Rejected", State::Rejected),
        make_issue("id4", "InProgress", State::InProgress),
    ];

    let closed = jit::domain::queries::query_closed(&issues);

    assert_eq!(closed.len(), 2);
    assert!(closed.iter().any(|i| i.id == "id1"));
    assert!(closed.iter().any(|i| i.id == "id3"));
}

#[test]
fn test_query_blocked_finds_issues_with_incomplete_deps() {
    let dep1 = make_issue("dep1", "Incomplete dep", State::InProgress);
    let dep2 = make_issue("dep2", "Done dep", State::Done);
    let blocked = make_issue_with_deps(
        "blocked1",
        "Blocked issue",
        State::Backlog,
        vec!["dep1".to_string(), "dep2".to_string()],
    );
    let ready = make_issue("ready1", "Ready", State::Ready);

    let issues = vec![dep1, dep2, blocked, ready];
    let blocked_result = jit::domain::queries::query_blocked(&issues);

    assert_eq!(blocked_result.len(), 1);
    assert_eq!(blocked_result[0].0.id, "blocked1");
    assert!(blocked_result[0].1.iter().any(|r| r.contains("dep1")));
}

#[test]
fn test_query_by_label_matches_exact() {
    let mut issue1 = make_issue("id1", "Epic issue", State::Ready);
    issue1.labels = vec!["type:epic".to_string()];

    let mut issue2 = make_issue("id2", "Task issue", State::Ready);
    issue2.labels = vec!["type:task".to_string()];

    let mut issue3 = make_issue("id3", "Another epic", State::Ready);
    issue3.labels = vec!["type:epic".to_string()];

    let issues = vec![issue1, issue2, issue3];

    let epics = jit::domain::queries::query_by_label(&issues, "type:epic");
    assert_eq!(epics.len(), 2);
    assert!(epics.iter().any(|i| i.id == "id1"));
    assert!(epics.iter().any(|i| i.id == "id3"));
}

#[test]
fn test_query_by_label_matches_wildcard() {
    let mut issue1 = make_issue("id1", "Epic", State::Ready);
    issue1.labels = vec!["epic:auth".to_string()];

    let mut issue2 = make_issue("id2", "Task", State::Ready);
    issue2.labels = vec!["epic:api".to_string()];

    let issue3 = make_issue("id3", "No epic", State::Ready);

    let issues = vec![issue1, issue2, issue3];

    let with_epic = jit::domain::queries::query_by_label(&issues, "epic:*");
    assert_eq!(with_epic.len(), 2);
    assert!(with_epic.iter().any(|i| i.id == "id1"));
    assert!(with_epic.iter().any(|i| i.id == "id2"));
}
