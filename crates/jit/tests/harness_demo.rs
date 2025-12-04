//! Demonstration of test harness usage
//!
//! This shows the recommended patterns for using the TestHarness
//! for fast, reliable in-process testing.

mod harness;

use harness::TestHarness;
use jit::domain::{Priority, State};

// ========== Query Tests ==========

#[test]
fn test_harness_query_ready() {
    let h = TestHarness::new();

    // Create ready and non-ready issues
    let ready_id = h.create_ready_issue("Ready task");
    let assigned_id = h.create_ready_issue("Assigned task");
    h.executor
        .claim_issue(&assigned_id, "agent:worker-1".to_string())
        .unwrap();

    // Query
    let ready = h.executor.query_ready().unwrap();

    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, ready_id);
}

#[test]
fn test_harness_query_by_priority() {
    let h = TestHarness::new();

    let high_id = h.create_issue_with_priority("High", Priority::High);
    let _normal_id = h.create_issue("Normal");
    let _critical_id = h.create_issue_with_priority("Critical", Priority::Critical);

    let high_issues = h.executor.query_by_priority(Priority::High).unwrap();

    assert_eq!(high_issues.len(), 1);
    assert_eq!(high_issues[0].id, high_id);
}

// ========== Issue Lifecycle Tests ==========

#[test]
fn test_harness_issue_lifecycle() {
    let h = TestHarness::new();

    // Create
    let id = h.create_issue("Task");
    assert_eq!(h.all_issues().len(), 1);

    // Update state
    h.executor
        .update_issue(&id, None, None, None, Some(State::Ready))
        .unwrap();
    let issue = h.get_issue(&id);
    assert_eq!(issue.state, State::Ready);

    // Claim
    h.executor
        .claim_issue(&id, "agent:worker-1".to_string())
        .unwrap();
    let issue = h.get_issue(&id);
    assert_eq!(issue.assignee, Some("agent:worker-1".to_string()));

    // Release
    h.executor.release_issue(&id, "timeout").unwrap();
    let issue = h.get_issue(&id);
    assert!(issue.assignee.is_none());

    // Delete
    h.executor.delete_issue(&id).unwrap();
    assert_eq!(h.all_issues().len(), 0);
}

// ========== Dependency Tests ==========

#[test]
fn test_harness_dependencies_block() {
    let h = TestHarness::new();

    let parent = h.create_issue("Parent");
    let child = h.create_issue("Child");

    // Add dependency
    h.executor.add_dependency(&child, &parent).unwrap();

    // Child should be blocked
    let child_issue = h.get_issue(&child);
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(child_issue.is_blocked(&resolved));

    // Complete parent
    h.executor
        .update_issue(&parent, None, None, None, Some(State::Done))
        .unwrap();

    // Child should be unblocked
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(!child_issue.is_blocked(&resolved));
}

#[test]
fn test_harness_cycle_detection() {
    let h = TestHarness::new();

    let issue1 = h.create_issue("Task 1");
    let issue2 = h.create_issue("Task 2");

    // Create dependency: 2 depends on 1
    h.executor.add_dependency(&issue2, &issue1).unwrap();

    // Try to create cycle: 1 depends on 2
    let result = h.executor.add_dependency(&issue1, &issue2);
    assert!(result.is_err(), "Cycle should be rejected");
}

// ========== Gate Tests ==========

#[test]
fn test_harness_gates() {
    let h = TestHarness::new();

    // Add gate definition
    h.add_gate("review", "Code Review", "Manual review", false);

    // Create issue with gate
    let id = h.create_issue_with_gates("Task", vec!["review".to_string()]);

    // Issue should NOT be blocked by pending gate (gates don't block starting work)
    let issue = h.get_issue(&id);
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(!issue.is_blocked(&resolved));
    
    // But gates do prevent completion
    assert!(issue.has_unpassed_gates());

    // Pass gate
    h.executor
        .pass_gate(&id, "review".to_string(), None)
        .unwrap();

    // Issue gates should be passed now
    let issue = h.get_issue(&id);
    assert!(!issue.has_unpassed_gates());
}

// ========== Complex Scenarios ==========

#[test]
fn test_harness_complex_workflow() {
    let h = TestHarness::new();

    // Setup gates
    h.add_gate("tests", "Tests", "Unit tests", true);
    h.add_gate("review", "Review", "Code review", false);

    // Create epic with dependencies
    let dep1 = h.create_issue_with_gates("Dependency 1", vec!["tests".to_string()]);
    let dep2 = h.create_issue_with_gates("Dependency 2", vec!["tests".to_string()]);
    let epic = h.create_issue_with_gates("Epic", vec!["review".to_string()]);

    h.executor.add_dependency(&epic, &dep1).unwrap();
    h.executor.add_dependency(&epic, &dep2).unwrap();

    // Pass gates for dependencies
    h.executor
        .pass_gate(&dep1, "tests".to_string(), None)
        .unwrap();
    h.executor
        .pass_gate(&dep2, "tests".to_string(), None)
        .unwrap();

    // Complete dependencies
    h.executor
        .update_issue(&dep1, None, None, None, Some(State::Done))
        .unwrap();
    h.executor
        .update_issue(&dep2, None, None, None, Some(State::Done))
        .unwrap();

    // Pass epic's gate
    h.executor
        .pass_gate(&epic, "review".to_string(), None)
        .unwrap();

    // Epic should now be unblocked
    let epic_issue = h.get_issue(&epic);
    let all = h.all_issues();
    let resolved: std::collections::HashMap<String, &jit::domain::Issue> =
        all.iter().map(|i| (i.id.clone(), i)).collect();
    assert!(!epic_issue.is_blocked(&resolved));
}

// ========== Performance Test Example ==========

#[test]
fn test_harness_scales_with_many_issues() {
    let h = TestHarness::new();

    // Create many issues quickly (all auto-transition to Ready since no blockers)
    for i in 0..100 {
        h.create_issue(&format!("Task {}", i));
    }

    assert_eq!(h.all_issues().len(), 100);

    // Query should be fast - all are ready since no blockers
    let ready = h.executor.query_ready().unwrap();
    assert_eq!(ready.len(), 100); // All auto-transitioned to Ready
}
