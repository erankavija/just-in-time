//! Tests for label-based query functionality (Phase 1.4)

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;

#[test]
fn test_query_by_label_exact_match() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issues with various labels
    let milestone_id = executor
        .create_issue(
            "Release v1.0".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    let epic_id = executor
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["epic:auth".to_string(), "milestone:v1.0".to_string()],
        )
        .unwrap();

    let _other_id = executor
        .create_issue(
            "Other Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v2.0".to_string()],
        )
        .unwrap();

    // Query exact match for milestone:v1.0
    let results = executor.query_by_label("milestone:v1.0").unwrap();

    assert_eq!(results.len(), 2);
    let result_ids: Vec<String> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&milestone_id));
    assert!(result_ids.contains(&epic_id));
}

#[test]
fn test_query_by_label_wildcard_namespace() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issues with various milestones
    let v1_id = executor
        .create_issue(
            "Task v1".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    let v2_id = executor
        .create_issue(
            "Task v2".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v2.0".to_string()],
        )
        .unwrap();

    let _no_milestone_id = executor
        .create_issue(
            "No milestone".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["epic:auth".to_string()],
        )
        .unwrap();

    // Query wildcard milestone:*
    let results = executor.query_by_label("milestone:*").unwrap();

    assert_eq!(results.len(), 2);
    let result_ids: Vec<String> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&v1_id));
    assert!(result_ids.contains(&v2_id));
}

#[test]
fn test_query_by_label_no_matches() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issue without the queried label
    let _id = executor
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    // Query for non-existent label
    let results = executor.query_by_label("epic:auth").unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_query_by_label_empty_repo() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Query with no issues
    let results = executor.query_by_label("milestone:v1.0").unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_query_by_label_wildcard_matches_all_in_namespace() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issues with multiple types
    let task_id = executor
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
        )
        .unwrap();

    let bug_id = executor
        .create_issue(
            "Bug".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:bug".to_string()],
        )
        .unwrap();

    let epic_id = executor
        .create_issue(
            "Epic".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "component:auth".to_string()],
        )
        .unwrap();

    // Query wildcard type:*
    let results = executor.query_by_label("type:*").unwrap();

    assert_eq!(results.len(), 3);
    let result_ids: Vec<String> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&task_id));
    assert!(result_ids.contains(&bug_id));
    assert!(result_ids.contains(&epic_id));
}

#[test]
fn test_query_by_label_case_sensitive() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issue with specific case
    let _id = executor
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["component:AuthService".to_string()],
        )
        .unwrap();

    // Query with different case should not match (case sensitive)
    let results = executor.query_by_label("component:authservice").unwrap();
    assert_eq!(results.len(), 0);

    // Query with exact case should match
    let results = executor.query_by_label("component:AuthService").unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_query_by_label_invalid_pattern() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Invalid pattern (no colon)
    let result = executor.query_by_label("invalidlabel");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid label pattern"));
}

#[test]
fn test_query_by_label_multiple_labels_per_issue() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issue with multiple labels
    let multi_id = executor
        .create_issue(
            "Multi-labeled".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![
                "milestone:v1.0".to_string(),
                "epic:auth".to_string(),
                "component:backend".to_string(),
            ],
        )
        .unwrap();

    // Should match any of the labels
    let results1 = executor.query_by_label("milestone:v1.0").unwrap();
    assert_eq!(results1.len(), 1);
    assert_eq!(results1[0].id, multi_id);

    let results2 = executor.query_by_label("epic:auth").unwrap();
    assert_eq!(results2.len(), 1);
    assert_eq!(results2[0].id, multi_id);

    let results3 = executor.query_by_label("component:backend").unwrap();
    assert_eq!(results3.len(), 1);
    assert_eq!(results3[0].id, multi_id);
}
