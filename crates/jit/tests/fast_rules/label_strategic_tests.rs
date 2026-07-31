//! Tests for strategic queries (Phase 3)

use jit::commands::test_helpers::declared_test_taxonomy;
use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{InMemoryStorage, IssueStore};

/// An executor over a repository that declares the type hierarchy these tests
/// classify against. `query_strategic` reads that declaration through a
/// `ConfigManager` rooted at the store, so the fixture writes it there as well
/// as into the memory image.
fn strategic_executor() -> CommandExecutor<InMemoryStorage> {
    let storage = InMemoryStorage::new();
    let config = format!(
        "[worktree]\nenforce_leases = \"off\"\n\n{}",
        declared_test_taxonomy()
    );
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), &config).unwrap();
    storage.add_data_file("config.toml", &config);
    crate::memory_executor(storage)
}

#[test]
fn test_query_strategic_returns_milestone_issues() {
    let executor = strategic_executor();

    // Create issues with strategic types
    let (milestone_id, _) = executor
        .create_issue(
            "Release v1.0".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["type:milestone".to_string(), "milestone:v1.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (_tactical_id, _) = executor
        .create_issue(
            "Fix bug".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:bug".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    // Query strategic issues
    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, milestone_id);
}

#[test]
fn test_query_strategic_returns_epic_issues() {
    let executor = strategic_executor();

    let (epic_id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, epic_id);
}

#[test]
fn test_query_strategic_returns_both_milestone_and_epic() {
    let executor = strategic_executor();

    let (milestone_id, _) = executor
        .create_issue(
            "Release v1.0".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["type:milestone".to_string(), "milestone:v1.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (epic_id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["type:epic".to_string(), "epic:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (_tactical_id, _) = executor
        .create_issue(
            "Fix typo".to_string(),
            "".to_string(),
            Priority::Low,
            vec![],
            vec!["type:bug".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 2);
    let ids: Vec<String> = strategic.iter().map(|i| i.id.clone()).collect();
    assert!(ids.contains(&milestone_id));
    assert!(ids.contains(&epic_id));
}

#[test]
fn test_query_strategic_excludes_tactical_only() {
    let executor = strategic_executor();

    // Create only tactical issues
    executor
        .create_issue(
            "Task 1".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    executor
        .create_issue(
            "Task 2".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["component:backend".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 0);
}

#[test]
fn test_query_strategic_includes_mixed_labels() {
    let executor = strategic_executor();

    // Issue with both strategic type and tactical labels
    let (mixed_id, _) = executor
        .create_issue(
            "Auth milestone".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec![
                "type:milestone".to_string(),
                "milestone:v1.0".to_string(),
                "component:auth".to_string(),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, mixed_id);
}

#[test]
fn test_query_strategic_with_custom_strategic_namespace() {
    let executor = strategic_executor();

    // Strategic classification is type-based, not namespace-based
    // No need to add custom namespace - config handles this

    // Create issue with initiative label but no strategic type
    let (_initiative_id, _) = executor
        .create_issue(
            "Digital transformation".to_string(),
            "".to_string(),
            Priority::Critical,
            vec![],
            vec![
                "initiative:cloud-migration".to_string(),
                "type:task".to_string(),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    // Create issue with strategic type
    let (milestone_id, _) = executor
        .create_issue(
            "Launch".to_string(),
            "".to_string(),
            Priority::Critical,
            vec![],
            vec!["type:milestone".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    // Only the milestone should be returned (strategic query is type-based)
    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, milestone_id);
}

#[test]
fn test_query_strategic_empty_repo() {
    let executor = strategic_executor();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 0);
}
