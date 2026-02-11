mod harness;
use harness::TestHarness;
use jit::domain::Priority;
use jit::storage::IssueStore;

#[test]
fn test_breakdown_replaces_type_label_with_child_type() {
    let harness = TestHarness::new();

    // Create parent with type:story
    let (parent_id, _) = harness
        .executor
        .create_issue(
            "User Authentication Story".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:story".to_string(), "epic:security".to_string()],
        )
        .unwrap();

    // Break down with child-type task
    let subtask_ids = harness
        .executor
        .breakdown_issue(
            &parent_id,
            "task", // child_type
            vec![
                ("Implement login".to_string(), String::new()),
                ("Add password hash".to_string(), String::new()),
            ],
            None, // No gate option
        )
        .unwrap();

    // Verify subtasks have type:task, not type:story
    for subtask_id in subtask_ids {
        let issue = harness.storage.load_issue(&subtask_id).unwrap();
        assert!(
            issue.labels.contains(&"type:task".to_string()),
            "Subtask should have type:task label"
        );
        assert!(
            !issue.labels.contains(&"type:story".to_string()),
            "Subtask should not have type:story label"
        );
        // Should still have other labels
        assert!(
            issue.labels.contains(&"epic:security".to_string()),
            "Subtask should inherit epic label"
        );
    }
}

#[test]
fn test_breakdown_preserves_non_type_labels() {
    let harness = TestHarness::new();

    // Create parent with multiple labels
    let (parent_id, _) = harness
        .executor
        .create_issue(
            "Feature Story".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec![
                "type:story".to_string(),
                "epic:onboarding".to_string(),
                "milestone:v1.0".to_string(),
                "component:backend".to_string(),
            ],
        )
        .unwrap();

    // Break down
    let subtask_ids = harness
        .executor
        .breakdown_issue(
            &parent_id,
            "task",
            vec![("Implement feature".to_string(), String::new())],
            None,
        )
        .unwrap();

    // Verify all non-type labels are preserved
    let subtask = harness.storage.load_issue(&subtask_ids[0]).unwrap();
    assert!(subtask.labels.contains(&"type:task".to_string()));
    assert!(subtask.labels.contains(&"epic:onboarding".to_string()));
    assert!(subtask.labels.contains(&"milestone:v1.0".to_string()));
    assert!(subtask.labels.contains(&"component:backend".to_string()));
    assert!(!subtask.labels.contains(&"type:story".to_string()));
}

#[test]
fn test_breakdown_works_with_custom_types() {
    let harness = TestHarness::new();

    // Create parent with custom type
    let (parent_id, _) = harness
        .executor
        .create_issue(
            "Feature Specification".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:feature".to_string()],
        )
        .unwrap();

    // Break down into custom child type
    let subtask_ids = harness
        .executor
        .breakdown_issue(
            &parent_id,
            "requirement", // Custom type
            vec![("REQ-001".to_string(), String::new())],
            None,
        )
        .unwrap();

    let subtask = harness.storage.load_issue(&subtask_ids[0]).unwrap();
    assert!(subtask.labels.contains(&"type:requirement".to_string()));
    assert!(!subtask.labels.contains(&"type:feature".to_string()));
}

#[test]
fn test_breakdown_no_gates_by_default() {
    let harness = TestHarness::new();

    // Create parent with gates
    let (parent_id, _) = harness
        .executor
        .create_issue(
            "Story with Gates".to_string(),
            String::new(),
            Priority::Normal,
            vec!["tests".to_string(), "code-review".to_string()],
            vec!["type:story".to_string()],
        )
        .unwrap();

    // Break down with no gate option (default)
    let subtask_ids = harness
        .executor
        .breakdown_issue(
            &parent_id,
            "task",
            vec![("Implement feature".to_string(), String::new())],
            None, // No gate option = no gates
        )
        .unwrap();

    // Verify subtasks have NO gates
    for subtask_id in subtask_ids {
        let issue = harness.storage.load_issue(&subtask_id).unwrap();
        assert!(
            issue.gates_required.is_empty(),
            "Subtask should have no gates by default"
        );
    }
}

#[test]
fn test_breakdown_with_gate_preset() {
    // This test will be implemented after we add PresetManager integration
    // For now, we'll test that the method signature accepts the preset parameter
    let harness = TestHarness::new();

    let (parent_id, _) = harness
        .executor
        .create_issue(
            "Story".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["type:story".to_string()],
        )
        .unwrap();

    // This will fail until we implement the gate preset logic
    // But we need the signature to exist first
    let _result = harness.executor.breakdown_issue(
        &parent_id,
        "task",
        vec![("Implement feature".to_string(), String::new())],
        Some("test-preset".to_string()), // Apply preset
    );

    // Test will be expanded once PresetManager integration is complete
}

#[test]
fn test_breakdown_with_inherit_gates() {
    let harness = TestHarness::new();

    // Define gates in registry first
    harness
        .executor
        .add_gate_definition(
            "tests".to_string(),
            "Tests Pass".to_string(),
            "All tests must pass".to_string(),
            false,
            None,
            "postcheck".to_string(),
        )
        .unwrap();

    harness
        .executor
        .add_gate_definition(
            "clippy".to_string(),
            "Clippy Pass".to_string(),
            "Clippy must pass".to_string(),
            false,
            None,
            "postcheck".to_string(),
        )
        .unwrap();

    // Create parent with gates
    let (parent_id, _) = harness
        .executor
        .create_issue(
            "Story with Gates".to_string(),
            String::new(),
            Priority::Normal,
            vec!["tests".to_string(), "clippy".to_string()],
            vec!["type:story".to_string()],
        )
        .unwrap();

    // Break down with inherit-gates flag
    let subtask_ids = harness
        .executor
        .breakdown_issue_with_inherit(
            &parent_id,
            "task",
            vec![("Implement feature".to_string(), String::new())],
            true, // inherit_gates = true
        )
        .unwrap();

    // Verify subtasks have parent's gates
    let parent = harness.storage.load_issue(&parent_id).unwrap();
    for subtask_id in subtask_ids {
        let issue = harness.storage.load_issue(&subtask_id).unwrap();
        assert_eq!(
            issue.gates_required.len(),
            parent.gates_required.len(),
            "Should inherit all parent gates"
        );
        for gate_key in &parent.gates_required {
            assert!(
                issue.gates_required.contains(gate_key),
                "Should have gate: {}",
                gate_key
            );
        }
    }
}

#[test]
fn test_breakdown_gate_preset_and_inherit_mutually_exclusive() {
    // This will be tested at the CLI level with clap's conflicts_with
    // The CLI won't allow both flags simultaneously
    // This is just a placeholder to track that we need the CLI test
}
