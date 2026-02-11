//! Unit tests for validation functionality
//!
//! Tests comprehensive validation including broken references and orphaned data.

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{InMemoryStorage, IssueStore};

#[test]
fn test_validation_detects_broken_dependency() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());

    // Create two issues
    let (issue1_id, _) = executor
        .create_issue(
            "Task A".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    let (issue2_id, _) = executor
        .create_issue(
            "Task B".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    // Add valid dependency
    executor.add_dependency(&issue2_id, &issue1_id).unwrap();

    // Validation should pass
    assert!(executor.validate_silent().is_ok());

    // Delete issue1, leaving broken reference in issue2
    storage.delete_issue(&issue1_id).unwrap();

    // Validation should fail
    let result = executor.validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Invalid dependency") || err_msg.contains("not found"));
}

#[test]
fn test_validation_detects_cycle() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());

    // Create two issues
    let (issue1_id, _) = executor
        .create_issue(
            "A".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();
    let (issue2_id, _) = executor
        .create_issue(
            "B".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    // Add A -> B
    executor.add_dependency(&issue1_id, &issue2_id).unwrap();

    // Manually create cycle by adding B -> A directly in storage
    let mut issue2_updated = storage.load_issue(&issue2_id).unwrap();
    issue2_updated.dependencies.push(issue1_id.clone());
    storage.save_issue(&issue2_updated).unwrap();

    // Validation should detect the cycle
    let result = executor.validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("cycle") || err_msg.contains("Cycle"));
}

#[test]
fn test_validation_passes_with_valid_graph() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());

    // Create a valid dependency graph: A -> B -> C
    let (issue_c_id, _) = executor
        .create_issue(
            "C".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();
    let (issue_b_id, _) = executor
        .create_issue(
            "B".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();
    let (issue_a_id, _) = executor
        .create_issue(
            "A".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    executor.add_dependency(&issue_b_id, &issue_c_id).unwrap();
    executor.add_dependency(&issue_a_id, &issue_b_id).unwrap();

    // Validation should pass
    assert!(executor.validate_silent().is_ok());
}

#[test]
fn test_validation_detects_multiple_broken_dependencies() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());

    let (issue1_id, _) = executor
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    // Manually add broken dependencies
    let mut issue = storage.load_issue(&issue1_id).unwrap();
    issue.dependencies.push("nonexistent1".to_string());
    issue.dependencies.push("nonexistent2".to_string());
    storage.save_issue(&issue).unwrap();

    // Validation should fail
    let result = executor.validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Invalid dependency") || err_msg.contains("not found"));
}

#[test]
fn test_validation_empty_repository() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);

    // Empty repository should validate successfully
    assert!(executor.validate_silent().is_ok());
}

#[test]
fn test_validation_detects_invalid_gate_reference() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());

    // Create issue with gate requirement
    let (_issue, _) = executor
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec!["nonexistent-gate".to_string()],
            vec![],
        )
        .unwrap();

    // Validation should fail because gate doesn't exist in registry
    let result = executor.validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Gate") || err_msg.contains("not found") || err_msg.contains("undefined")
    );
}
