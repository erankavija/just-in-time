//! Integration tests for CLI warning display.
//!
//! The orphan-leaf / strategic-consistency warnings are now produced by the
//! built-in GRAPH rules (`orphan-leaf` / `strategic-consistency`, origin =
//! "default") rather than the former hard-coded `check_warnings` path. These
//! tests exercise that the same create-time warnings still surface, now through
//! the rule engine.

use jit::commands::CommandExecutor;
use jit::storage::{IssueStore, JsonFileStorage};
use jit::test_taxonomy::TestTaxonomy;
use jit::validation::graph::GraphFinding;
use tempfile::TempDir;

fn setup_test_repo() -> (TempDir, CommandExecutor<JsonFileStorage>, TestTaxonomy) {
    let (temp_dir, storage, taxonomy) = jit::test_utils::setup_test_repo_with_taxonomy().unwrap();
    let layout = jit::storage::discover_repository_layout(temp_dir.path(), storage.root()).unwrap();
    (
        temp_dir,
        CommandExecutor::new(storage).with_layout(layout),
        taxonomy,
    )
}

fn type_label(taxonomy: &TestTaxonomy, level: u8) -> String {
    format!("type:{}", taxonomy.type_at_level(level))
}

fn membership_label(taxonomy: &TestTaxonomy, type_name: &str) -> String {
    format!(
        "{}:auth",
        taxonomy
            .label_associations
            .get(type_name)
            .expect("the test taxonomy associates the strategic type with a namespace")
    )
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
    let (_temp_dir, executor, taxonomy) = setup_test_repo();
    let strategic_type = taxonomy.type_at_level(2);

    // Create epic without epic:* label
    let (id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec![type_label(&taxonomy, 2)],
            None,
            None,
            false,
        )
        .unwrap();

    let warnings = warnings_for(&executor, &id);
    assert_eq!(warnings.len(), 1);
    assert_eq!(warnings[0].finding.rule, "strategic-consistency");
    assert!(warnings[0].finding.message.contains(&format!(
        "{}:*",
        taxonomy.label_associations[strategic_type]
    )));
}

#[test]
fn test_create_task_without_parent_shows_warning() {
    let (_temp_dir, executor, taxonomy) = setup_test_repo();

    // Create task without parent labels
    let (id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec![type_label(&taxonomy, 4)],
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
    let (_temp_dir, executor, taxonomy) = setup_test_repo();
    let strategic_type = taxonomy.type_at_level(2);

    // Create epic with epic:* label
    let (id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "Epic description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec![
                type_label(&taxonomy, 2),
                membership_label(&taxonomy, strategic_type),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    assert!(warnings_for(&executor, &id).is_empty());
}

#[test]
fn test_create_task_with_parent_no_warning() {
    let (_temp_dir, executor, taxonomy) = setup_test_repo();
    let strategic_type = taxonomy.type_at_level(2);

    // Create task with epic label
    let (id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "Task description".to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec![
                type_label(&taxonomy, 4),
                membership_label(&taxonomy, strategic_type),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    assert!(warnings_for(&executor, &id).is_empty());
}
