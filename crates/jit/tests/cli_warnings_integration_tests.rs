//! Integration tests for CLI warning display.
//!
//! The orphan-leaf / strategic-consistency warnings are now produced by the
//! built-in GRAPH rules (`default:orphan-leaf` / `default:strategic-consistency`)
//! rather than the former hard-coded `check_warnings` path. These tests exercise
//! that the same create-time warnings still surface, now through the rule engine.

use jit::commands::CommandExecutor;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::validation::graph::GraphFinding;
use tempfile::TempDir;

fn setup_test_storage() -> (TempDir, JsonFileStorage) {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path());
    storage.init().unwrap();
    (temp_dir, storage)
}

/// Graph-rule findings attributed to `id`, the rule-engine replacement for the
/// former `executor.check_warnings(&id)`.
fn warnings_for(executor: &CommandExecutor<JsonFileStorage>, id: &str) -> Vec<GraphFinding> {
    let issues = executor.storage().list_issues().unwrap();
    executor
        .evaluate_graph_rules(&issues)
        .unwrap()
        .into_iter()
        .filter(|gf| gf.issue_id.as_deref() == Some(id))
        .collect()
}

#[test]
fn test_create_epic_without_label_shows_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);

    // Create epic without epic:* label
    let (id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            false,
        )
        .unwrap();

    let warnings = warnings_for(&executor, &id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].finding.rule, "default:strategic-consistency");
    assert!(warnings[0].finding.message.contains("epic:*"));
}

#[test]
fn test_create_task_without_parent_shows_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);

    // Create task without parent labels
    let (id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            false,
        )
        .unwrap();

    let warnings = warnings_for(&executor, &id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].finding.rule, "default:orphan-leaf");
    assert!(warnings[0].finding.message.contains("orphaned leaf"));
}

#[test]
fn test_create_epic_with_label_no_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);

    // Create epic with epic:* label
    let (id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
            false,
        )
        .unwrap();

    assert!(warnings_for(&executor, &id).is_empty());
}

#[test]
fn test_create_task_with_parent_no_warning() {
    let (_temp_dir, storage) = setup_test_storage();
    let executor = CommandExecutor::new(storage);

    // Create task with epic label
    let (id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:task".to_string(), "epic:auth".to_string()],
            false,
        )
        .unwrap();

    assert!(warnings_for(&executor, &id).is_empty());
}
