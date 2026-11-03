//! Test that verifies JIT works without coordinator functionality
//!
//! Worked `TestHarness` examples of the issue lifecycle and of gates.
use jit::declarations::GateMode;
use jit::domain::identity::IssueIdPrefix;
use jit::domain::Priority;
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;

#[test]
fn test_issue_lifecycle_works_without_coordinator() {
    let h = TestHarness::new();

    // Create issue
    let id = h
        .executor
        .issues()
        .create_issue(
            "Task".to_string(),
            "description".to_string(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;

    // Update issue
    let _ = h
        .executor
        .issues()
        .update_issue(
            &id,
            Some("Updated".to_string()),
            Some(jit::commands::DescriptionUpdate::Replace(
                "Updated desc".to_string(),
            )),
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap();

    // Claim issue
    h.executor
        .issues()
        .claim_issue(&IssueIdPrefix::from(&id), "agent:test".parse().unwrap())
        .unwrap();

    // Release issue
    h.executor
        .issues()
        .release_issue(&IssueIdPrefix::from(&id), "Test release")
        .unwrap();

    // After release, assignee is cleared (state stays in-progress though)
    let issue = h.storage.load_issue(&id).unwrap();
    assert!(issue.assignee.is_none());
}

#[test]
fn test_gates_work_without_coordinator() {
    let h = TestHarness::new();

    // Register a gate
    h.add_gate(
        &"test-gate".parse().unwrap(),
        "Test Gate",
        "A test gate",
        GateMode::Manual,
        None,
    );

    // Create ready issue then add gate requirement
    let id = h.create_ready_issue("Task");
    let mut issue = h.storage.load_issue(&id).unwrap();
    issue.gates_required.push("test-gate".parse().unwrap());
    jit::test_harness::seed_memory_issue(&h.storage, &issue);

    // Gates prevent completion - attempting to mark Done will transition to Gated
    // and return an error (gate validation failed)
    let result = h.executor.issues().update_issue(
        &id,
        None,
        None,
        None,
        Some(jit::domain::State::Done),
        vec![],
        vec![],
        None,
        None,
        false,
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
        .gates()
        .pass_gate(
            &id,
            &"test-gate".parse().unwrap(),
            Some("human:reviewer".parse().unwrap()),
            false,
        )
        .unwrap();

    // Issue should now auto-transition to Done
    let issue = h.storage.load_issue(&id).unwrap();
    assert_eq!(issue.state, jit::domain::State::Done);
}
