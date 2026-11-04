//! Unit tests for validation functionality
//!
//! Tests comprehensive validation including broken references and orphaned data.

use jit::commands::test_helpers::memory_executor;
use jit::domain::Priority;
use jit::storage::{InMemoryStorage, IssueStore};
use jit::test_utils::add_reducing_edge;
use jit::test_utils::issue_id;

#[test]
fn test_validation_detects_broken_dependency() {
    let storage = InMemoryStorage::new();
    storage.add_data_file("config.toml", "");
    let executor = memory_executor(storage.clone());

    // Create two issues
    let issue1_id = executor
        .issues()
        .create_issue(
            "Task A".to_string(),
            "".to_string(),
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

    let issue2_id = executor
        .issues()
        .create_issue(
            "Task B".to_string(),
            "".to_string(),
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

    // Add valid dependency
    add_reducing_edge(&executor, &issue2_id, &issue1_id).unwrap();

    // Validation should pass
    assert!(executor.validation().validate_silent().is_ok());

    // Add a well-formed dependency id whose target is absent.
    let mut issue2 = storage.load_issue(&issue2_id).unwrap();
    issue2.dependencies = vec![issue_id(1)];
    jit::test_harness::seed_memory_issue(&storage, &issue2);

    // Validation should fail
    let result = executor.validation().validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Invalid dependency") || err_msg.contains("not found"));
}

#[test]
fn test_validation_detects_cycle() {
    let storage = InMemoryStorage::new();
    storage.add_data_file("config.toml", "");
    let executor = memory_executor(storage.clone());

    // Create two issues
    let issue1_id = executor
        .issues()
        .create_issue(
            "A".to_string(),
            "".to_string(),
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
    let issue2_id = executor
        .issues()
        .create_issue(
            "B".to_string(),
            "".to_string(),
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

    // Add A -> B
    add_reducing_edge(&executor, &issue1_id, &issue2_id).unwrap();

    // Manually create cycle by adding B -> A directly in storage
    let mut issue2_updated = storage.load_issue(&issue2_id).unwrap();
    issue2_updated.dependencies.push(issue1_id.clone());
    jit::test_harness::seed_memory_issue(&storage, &issue2_updated);

    // Validation should detect the cycle
    let result = executor.validation().validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("cycle") || err_msg.contains("Cycle"));
}

#[test]
fn test_validation_passes_with_valid_graph() {
    let storage = InMemoryStorage::new();
    storage.add_data_file("config.toml", "");
    let executor = memory_executor(storage.clone());

    // Create a valid dependency graph: A -> B -> C
    let issue_c_id = executor
        .issues()
        .create_issue(
            "C".to_string(),
            "".to_string(),
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
    let issue_b_id = executor
        .issues()
        .create_issue(
            "B".to_string(),
            "".to_string(),
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
    let issue_a_id = executor
        .issues()
        .create_issue(
            "A".to_string(),
            "".to_string(),
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

    add_reducing_edge(&executor, &issue_b_id, &issue_c_id).unwrap();
    add_reducing_edge(&executor, &issue_a_id, &issue_b_id).unwrap();

    // Validation should pass
    assert!(executor.validation().validate_silent().is_ok());
}

#[test]
fn test_validation_detects_multiple_broken_dependencies() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage.clone());

    let issue1_id = executor
        .issues()
        .create_issue(
            "Task".to_string(),
            "".to_string(),
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

    // Manually add missing targets using canonical issue ids.
    let mut issue = storage.load_issue(&issue1_id).unwrap();
    issue.dependencies.extend([issue_id(2), issue_id(3)]);
    jit::test_harness::seed_memory_issue(&storage, &issue);

    // Validation should fail
    let result = executor.validation().validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("Invalid dependency") || err_msg.contains("not found"));
}

#[test]
fn test_validation_empty_repository() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage);

    // Empty repository should validate successfully
    assert!(executor.validation().validate_silent().is_ok());
}

#[test]
fn test_validation_detects_invalid_gate_reference() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage.clone());

    // Create issue with gate requirement
    let _issue = executor
        .issues()
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec!["nonexistent-gate".parse().unwrap()],
            vec![],
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;

    // Validation should fail because gate doesn't exist in registry
    let result = executor.validation().validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("Gate") || err_msg.contains("not found") || err_msg.contains("undefined")
    );
}

#[test]
fn test_validation_detects_isolated_issues() {
    let storage = InMemoryStorage::new();
    storage.add_data_file("config.toml", "");
    let executor = memory_executor(storage.clone());

    // Create connected issues: A -> B
    let issue_a_id = executor
        .issues()
        .create_issue(
            "Connected A".to_string(),
            "".to_string(),
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

    let issue_b_id = executor
        .issues()
        .create_issue(
            "Connected B".to_string(),
            "".to_string(),
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

    add_reducing_edge(&executor, &issue_b_id, &issue_a_id).unwrap();

    // Create an isolated issue (no deps, not a dep of anything)
    let _isolated_id = executor
        .issues()
        .create_issue(
            "Isolated Issue".to_string(),
            "".to_string(),
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

    // Validation should fail due to isolated issue
    let result = executor.validation().validate_silent();
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("isolated") || err_msg.contains("Isolated"));
}

#[test]
fn test_validation_passes_with_all_connected_issues() {
    let storage = InMemoryStorage::new();
    storage.add_data_file("config.toml", "");
    let executor = memory_executor(storage.clone());

    // Create a connected graph: A -> B -> C
    let issue_a_id = executor
        .issues()
        .create_issue(
            "Issue A".to_string(),
            "".to_string(),
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

    let issue_b_id = executor
        .issues()
        .create_issue(
            "Issue B".to_string(),
            "".to_string(),
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

    let issue_c_id = executor
        .issues()
        .create_issue(
            "Issue C".to_string(),
            "".to_string(),
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

    add_reducing_edge(&executor, &issue_b_id, &issue_a_id).unwrap();
    add_reducing_edge(&executor, &issue_c_id, &issue_b_id).unwrap();

    // All issues are connected - validation should pass
    let result = executor.validation().validate_silent();
    assert!(result.is_ok());
}

#[test]
fn test_validation_passes_with_single_issue() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage.clone());

    // Create a single issue (edge case - should not be considered isolated)
    let _issue_id = executor
        .issues()
        .create_issue(
            "Single Issue".to_string(),
            "".to_string(),
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

    // Single issue should not be considered isolated - validation should pass
    let result = executor.validation().validate_silent();
    assert!(result.is_ok());
}
