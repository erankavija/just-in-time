//! Integration tests for CLI warning display

use jit::commands::CommandExecutor;
use jit::storage::{IssueStore, JsonFileStorage};
use tempfile::TempDir;

fn setup_test_storage() -> (TempDir, JsonFileStorage) {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());
    storage.init().unwrap();
    (temp_dir, storage)
}

#[test]
fn test_create_epic_without_label_shows_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);
    
    // Create epic without epic:* label
    let id = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
        )
        .unwrap();
    
    // Check for warnings
    let warnings = executor.check_warnings(&id).unwrap();
    assert_eq!(warnings.len(), 1);
    
    // Verify it's a strategic label warning
    match &warnings[0] {
        jit::type_hierarchy::ValidationWarning::MissingStrategicLabel { type_name, expected_namespace, .. } => {
            assert_eq!(type_name, "epic");
            assert_eq!(expected_namespace, "epic");
        }
        _ => panic!("Expected MissingStrategicLabel warning"),
    }
}

#[test]
fn test_create_task_without_parent_shows_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);
    
    // Create task without parent labels
    let id = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
        )
        .unwrap();
    
    // Check for warnings
    let warnings = executor.check_warnings(&id).unwrap();
    assert_eq!(warnings.len(), 1);
    
    // Verify it's an orphan warning
    match &warnings[0] {
        jit::type_hierarchy::ValidationWarning::OrphanedLeaf { type_name, .. } => {
            assert_eq!(type_name, "task");
        }
        _ => panic!("Expected OrphanedLeaf warning"),
    }
}

#[test]
fn test_create_epic_with_label_no_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);
    
    // Create epic with epic:* label
    let id = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
        )
        .unwrap();
    
    // Check for warnings
    let warnings = executor.check_warnings(&id).unwrap();
    assert_eq!(warnings.len(), 0);
}

#[test]
fn test_create_task_with_parent_no_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);
    
    // Create task with epic label
    let id = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:task".to_string(), "epic:auth".to_string()],
        )
        .unwrap();
    
    // Check for warnings
    let warnings = executor.check_warnings(&id).unwrap();
    assert_eq!(warnings.len(), 0);
}
