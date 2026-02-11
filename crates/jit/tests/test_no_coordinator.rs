//! Test that verifies JIT works without coordinator functionality
//!
//! This test defines the expected behavior after coordinator removal:
//! - All query commands work
//! - All issue commands work
//! - No coordinator commands exist
//! - Core functionality is intact

mod harness;
use harness::TestHarness;
use jit::domain::{Priority, State};
use jit::storage::IssueStore;

#[test]
fn test_query_interface_works_without_coordinator() {
    let h = TestHarness::new();

    // Create ready issues
    let id1 = h.create_ready_issue("Task 1");
    let id2 = h.create_ready_issue("Task 2");

    // Query ready issues
    let ready = h.executor.query_ready().unwrap();
    assert_eq!(ready.len(), 2);

    // Claim one issue
    h.executor
        .claim_issue(&id1, "agent:test".to_string())
        .unwrap();

    // Query by assignee
    let assigned = h.executor.query_by_assignee("agent:test").unwrap();
    assert_eq!(assigned.len(), 1);
    assert_eq!(assigned[0].id, id1);

    // Query ready again - should exclude claimed issue
    let ready = h.executor.query_ready().unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, id2);
}

#[test]
fn test_issue_lifecycle_works_without_coordinator() {
    let h = TestHarness::new();

    // Create issue
    let (id, _) = h
        .executor
        .create_issue(
            "Task".to_string(),
            "description".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    // Update issue
    let _ = h
        .executor
        .update_issue(
            &id,
            Some("Updated".to_string()),
            Some("Updated desc".to_string()),
            None,
            None,
            vec![],
            vec![],
        )
        .unwrap();

    // Claim issue
    h.executor
        .claim_issue(&id, "agent:test".to_string())
        .unwrap();

    // Release issue
    h.executor.release_issue(&id, "Test release").unwrap();

    // After release, assignee is cleared (state stays in-progress though)
    let issue = h.storage.load_issue(&id).unwrap();
    assert!(issue.assignee.is_none());
}

#[test]
fn test_dependency_graph_works_without_coordinator() {
    let h = TestHarness::new();

    let id1 = h.create_ready_issue("Task 1");
    let id2 = h.create_ready_issue("Task 2");

    // Add dependency
    h.executor.add_dependency(&id2, &id1).unwrap();

    // Query blocked issues (id2 is blocked by id1)
    let blocked = h.executor.query_blocked().unwrap();
    assert_eq!(blocked.len(), 1);
    assert_eq!(blocked[0].0.id, id2);

    // Query ready - only id1 should be ready
    let ready = h.executor.query_ready().unwrap();
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, id1);
}

#[test]
fn test_gates_work_without_coordinator() {
    let h = TestHarness::new();

    // Register a gate
    h.add_gate("test-gate", "Test Gate", "A test gate", false);

    // Create ready issue then add gate requirement
    let id = h.create_ready_issue("Task");
    let mut issue = h.storage.load_issue(&id).unwrap();
    issue.gates_required.push("test-gate".to_string());
    h.storage.save_issue(&issue).unwrap();

    // Issue should NOT be blocked by pending gate (gates don't block Ready state)
    let blocked = h.executor.query_blocked().unwrap();
    assert_eq!(blocked.len(), 0);

    // Issue should still be ready even with pending gate
    let ready = h.executor.query_ready().unwrap();
    assert_eq!(ready.len(), 1);

    // But gates prevent completion - attempting to mark Done will transition to Gated
    // and return an error (gate validation failed)
    let result = h.executor.update_issue(
        &id,
        None,
        None,
        None,
        Some(jit::domain::State::Done),
        vec![],
        vec![],
    );
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Gate validation failed"));

    let issue = h.storage.load_issue(&id).unwrap();
    assert_eq!(issue.state, jit::domain::State::Gated);

    // Pass the gate
    h.executor
        .pass_gate(&id, "test-gate".to_string(), None)
        .unwrap();

    // Issue should now auto-transition to Done
    let issue = h.storage.load_issue(&id).unwrap();
    assert_eq!(issue.state, jit::domain::State::Done);
}

#[test]
fn test_state_queries_work_without_coordinator() {
    let h = TestHarness::new();

    let id1 = h.create_ready_issue("Open task");
    let _id2 = h.create_ready_issue("Ready task");

    // Query by state
    let ready_issues = h.executor.query_by_state(State::Ready).unwrap();
    assert_eq!(ready_issues.len(), 2);

    // Claim one
    h.executor
        .claim_issue(&id1, "agent:test".to_string())
        .unwrap();

    // Query in-progress state (claimed issue)
    let in_progress = h.executor.query_by_state(State::InProgress).unwrap();
    assert_eq!(in_progress.len(), 1);
    assert_eq!(in_progress[0].id, id1);
}

#[test]
fn test_priority_queries_work_without_coordinator() {
    let h = TestHarness::new();

    h.create_issue_with_priority("High task", Priority::High);
    h.create_issue_with_priority("Normal task", Priority::Normal);
    h.create_issue_with_priority("Low task", Priority::Low);

    // Query by priority
    let high = h.executor.query_by_priority(Priority::High).unwrap();
    assert_eq!(high.len(), 1);

    let normal = h.executor.query_by_priority(Priority::Normal).unwrap();
    assert_eq!(normal.len(), 1);
}
