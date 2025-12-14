//! Tests for type hierarchy warning-level validations

use jit::domain::Issue;
use jit::type_hierarchy::{HierarchyConfig, ValidationWarning};

/// Helper to create a test hierarchy config
fn test_config() -> HierarchyConfig {
    HierarchyConfig::default()
}

#[test]
fn test_epic_without_epic_label_warns() {
    let config = test_config();
    let mut epic = Issue::new("Auth System".to_string(), "Epic description".to_string());
    epic.labels = vec!["type:epic".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &epic);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, "epic");
            assert_eq!(expected_namespace, "epic");
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_milestone_without_milestone_label_warns() {
    let config = test_config();
    let mut milestone = Issue::new(
        "v1.0 Release".to_string(),
        "Milestone description".to_string(),
    );
    milestone.labels = vec!["type:milestone".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &milestone);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::MissingStrategicLabel {
            type_name,
            expected_namespace,
            ..
        } => {
            assert_eq!(type_name, "milestone");
            assert_eq!(expected_namespace, "milestone");
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_task_non_strategic_no_warning() {
    let config = test_config();
    let mut task = Issue::new("Login API".to_string(), "Task description".to_string());
    task.labels = vec!["type:task".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &task);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_epic_with_epic_label_no_warning() {
    let config = test_config();
    let mut epic = Issue::new("Auth System".to_string(), "Epic description".to_string());
    epic.labels = vec!["type:epic".to_string(), "epic:auth".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &epic);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_milestone_with_milestone_label_no_warning() {
    let config = test_config();
    let mut milestone = Issue::new(
        "v1.0 Release".to_string(),
        "Milestone description".to_string(),
    );
    milestone.labels = vec!["type:milestone".to_string(), "milestone:v1.0".to_string()];

    let warnings = jit::type_hierarchy::validate_strategic_labels(&config, &milestone);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_task_without_parent_labels_warns() {
    let config = test_config();
    let mut task = Issue::new("Login API".to_string(), "Task description".to_string());
    task.labels = vec!["type:task".to_string()];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &task);

    assert_eq!(warnings.len(), 1);
    match &warnings[0] {
        ValidationWarning::OrphanedLeaf { type_name, .. } => {
            assert_eq!(type_name, "task");
        }
        _ => panic!("Expected OrphanedLeaf warning"),
    }
}

#[test]
fn test_task_with_epic_label_no_warning() {
    let config = test_config();
    let mut task = Issue::new("Login API".to_string(), "Task description".to_string());
    task.labels = vec!["type:task".to_string(), "epic:auth".to_string()];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &task);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_task_with_milestone_label_no_warning() {
    let config = test_config();
    let mut task = Issue::new("Login API".to_string(), "Task description".to_string());
    task.labels = vec!["type:task".to_string(), "milestone:v1.0".to_string()];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &task);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_epic_non_leaf_no_warning() {
    let config = test_config();
    let mut epic = Issue::new("Auth System".to_string(), "Epic description".to_string());
    epic.labels = vec!["type:epic".to_string()];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &epic);

    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_task_with_multiple_parent_labels_no_warning() {
    let config = test_config();
    let mut task = Issue::new("Login API".to_string(), "Task description".to_string());
    task.labels = vec![
        "type:task".to_string(),
        "epic:auth".to_string(),
        "milestone:v1.0".to_string(),
    ];

    let warnings = jit::type_hierarchy::validate_orphans(&config, &task);

    assert_eq!(warnings.len(), 0);
}
