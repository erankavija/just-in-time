//! Integration tests for CLI warning display.
//!
//! The orphan-leaf / strategic-consistency warnings are now produced by the
//! built-in GRAPH rules (`orphan-leaf` / `strategic-consistency`, origin =
//! "default") rather than the former hard-coded `check_warnings` path. These
//! tests exercise that the same create-time warnings still surface, now through
//! the rule engine.

use jit::commands::CommandExecutor;
use jit::hierarchy_templates::HierarchyTemplate;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::validation::graph::GraphFinding;
use tempfile::TempDir;

fn setup_test_repo() -> (TempDir, CommandExecutor<JsonFileStorage>) {
    let temp_dir = TempDir::new().unwrap();
    let storage = JsonFileStorage::new(temp_dir.path().join(".jit"));
    let initial_layout =
        jit::storage::discover_repository_layout(temp_dir.path(), storage.root()).unwrap();
    CommandExecutor::new(storage.clone())
        .with_layout(initial_layout)
        .initialize_fresh_repository(temp_dir.path(), &HierarchyTemplate::default(), None)
        .unwrap();
    let layout = jit::storage::discover_repository_layout(temp_dir.path(), storage.root()).unwrap();
    (temp_dir, CommandExecutor::new(storage).with_layout(layout))
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
    let (_temp_dir, executor) = setup_test_repo();

    // Create epic without epic:* label
    let (id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:epic".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let warnings = warnings_for(&executor, &id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].finding.rule, "strategic-consistency");
    assert!(warnings[0].finding.message.contains("epic:*"));
}

#[test]
fn test_create_task_without_parent_shows_warning() {
    let (_temp_dir, executor) = setup_test_repo();

    // Create task without parent labels
    let (id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let warnings = warnings_for(&executor, &id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].finding.rule, "orphan-leaf");
    assert!(warnings[0].finding.message.contains("orphaned leaf"));
}

#[test]
fn test_create_epic_with_label_no_warning() {
    let (_temp_dir, executor) = setup_test_repo();

    // Create epic with epic:* label
    let (id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    assert!(warnings_for(&executor, &id).is_empty());
}

#[test]
fn test_create_task_with_parent_no_warning() {
    let (_temp_dir, executor) = setup_test_repo();

    // Create task with epic label
    let (id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec!["type:task".to_string(), "epic:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    assert!(warnings_for(&executor, &id).is_empty());
}
