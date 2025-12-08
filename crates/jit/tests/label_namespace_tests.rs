//! Tests for label namespace registry (Phase 2.1-2.2)

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{InMemoryStorage, IssueStore};

#[test]
fn test_init_creates_default_namespaces() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());
    executor.init().unwrap();

    // Should have standard namespaces after init
    let namespaces = storage.list_label_namespaces().unwrap();
    assert!(namespaces.contains_key("milestone"));
    assert!(namespaces.contains_key("epic"));
    assert!(namespaces.contains_key("component"));
    assert!(namespaces.contains_key("type"));
    assert!(namespaces.contains_key("team"));
}

#[test]
fn test_namespace_has_correct_properties() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());
    executor.init().unwrap();

    let namespaces = storage.list_label_namespaces().unwrap();
    
    // type should be unique
    let type_ns = namespaces.get("type").unwrap();
    assert!(type_ns.unique);
    assert!(!type_ns.strategic);
    
    // milestone should be strategic and non-unique
    let milestone_ns = namespaces.get("milestone").unwrap();
    assert!(!milestone_ns.unique);
    assert!(milestone_ns.strategic);
}

#[test]
fn test_unique_namespace_allows_single_label() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issue with type:bug
    let id = executor
        .create_issue(
            "Fix bug".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:bug".to_string()],
        )
        .unwrap();

    let issue = executor.get_issue(&id).unwrap();
    assert_eq!(issue.labels.len(), 1);
    assert_eq!(issue.labels[0], "type:bug");
}

#[test]
fn test_unique_namespace_prevents_multiple_labels() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Attempt to create issue with two type labels should fail
    let result = executor.create_issue(
        "Task".to_string(),
        "".to_string(),
        Priority::Normal,
        vec![],
        vec!["type:bug".to_string(), "type:feature".to_string()],
    );

    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("unique") || err.contains("type"));
}

#[test]
fn test_non_unique_namespace_allows_multiple_labels() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Multiple component labels should be allowed
    let id = executor
        .create_issue(
            "Cross-cutting task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![
                "component:auth".to_string(),
                "component:database".to_string(),
            ],
        )
        .unwrap();

    let issue = executor.get_issue(&id).unwrap();
    assert_eq!(issue.labels.len(), 2);
}

#[test]
fn test_update_issue_enforces_uniqueness() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issue with type:bug
    let id = executor
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:bug".to_string()],
        )
        .unwrap();

    // Try to add another type label
    let result = executor.add_label(&id, "type:feature");
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("unique"));
}

#[test]
fn test_list_label_values_in_namespace() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issues with various milestones
    executor
        .create_issue(
            "Task 1".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    executor
        .create_issue(
            "Task 2".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v2.0".to_string()],
        )
        .unwrap();

    executor
        .create_issue(
            "Task 3".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    // List values in milestone namespace
    let values = executor.list_label_values("milestone").unwrap();
    assert_eq!(values.len(), 2);
    assert!(values.contains(&"v1.0".to_string()));
    assert!(values.contains(&"v2.0".to_string()));
}

#[test]
fn test_add_custom_namespace() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage.clone());
    executor.init().unwrap();

    // Add custom namespace
    executor
        .add_label_namespace(
            "priority",
            "Custom priority labels",
            false,  // not unique
            false,  // not strategic
        )
        .unwrap();

    let namespaces = storage.list_label_namespaces().unwrap();
    assert!(namespaces.contains_key("priority"));
    assert_eq!(namespaces.get("priority").unwrap().description, "Custom priority labels");
}

#[test]
fn test_unknown_namespace_warning() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Using unknown namespace should succeed but log warning
    let result = executor.create_issue(
        "Task".to_string(),
        "".to_string(),
        Priority::Normal,
        vec![],
        vec!["unknown:value".to_string()],
    );

    // Should succeed (only warning, not error)
    assert!(result.is_ok());
}
