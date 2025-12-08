//! Tests for strategic queries and breakdown label inheritance (Phase 3)

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;

#[test]
fn test_query_strategic_returns_milestone_issues() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create issues with strategic labels
    let milestone_id = executor
        .create_issue(
            "Release v1.0".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    let _tactical_id = executor
        .create_issue(
            "Fix bug".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:bug".to_string()],
        )
        .unwrap();

    // Query strategic issues
    let strategic = executor.query_strategic().unwrap();
    
    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, milestone_id);
}

#[test]
fn test_query_strategic_returns_epic_issues() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    let epic_id = executor
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["epic:auth".to_string()],
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();
    
    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, epic_id);
}

#[test]
fn test_query_strategic_returns_both_milestone_and_epic() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

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
            Priority::High,
            vec![],
            vec!["epic:auth".to_string()],
        )
        .unwrap();

    let _tactical_id = executor
        .create_issue(
            "Fix typo".to_string(),
            "".to_string(),
            Priority::Low,
            vec![],
            vec!["type:bug".to_string()],
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
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create only tactical issues
    executor
        .create_issue(
            "Task 1".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
        )
        .unwrap();

    executor
        .create_issue(
            "Task 2".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["component:backend".to_string()],
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();
    
    assert_eq!(strategic.len(), 0);
}

#[test]
fn test_query_strategic_includes_mixed_labels() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Issue with both strategic and tactical labels
    let mixed_id = executor
        .create_issue(
            "Auth bug fix".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec![
                "milestone:v1.0".to_string(),
                "type:bug".to_string(),
                "component:auth".to_string(),
            ],
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();
    
    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, mixed_id);
}

#[test]
fn test_query_strategic_with_custom_strategic_namespace() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Add custom strategic namespace
    executor
        .add_label_namespace("initiative", "Company-wide initiatives", false, true)
        .unwrap();

    let initiative_id = executor
        .create_issue(
            "Digital transformation".to_string(),
            "".to_string(),
            Priority::Critical,
            vec![],
            vec!["initiative:cloud-migration".to_string()],
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();
    
    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, initiative_id);
}

#[test]
fn test_query_strategic_empty_repo() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    let strategic = executor.query_strategic().unwrap();
    
    assert_eq!(strategic.len(), 0);
}

#[test]
fn test_breakdown_copies_labels_to_subtasks() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    // Create parent with strategic labels
    let parent_id = executor
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec![
                "milestone:v1.0".to_string(),
                "epic:auth".to_string(),
                "component:backend".to_string(),
            ],
        )
        .unwrap();

    // Breakdown into subtasks
    let subtasks = vec![
        ("Implement login".to_string(), "Login flow".to_string()),
        ("Implement logout".to_string(), "Logout flow".to_string()),
    ];
    
    let subtask_ids = executor
        .breakdown_issue(&parent_id, subtasks)
        .unwrap();

    assert_eq!(subtask_ids.len(), 2);

    // Verify each subtask has parent's labels
    for subtask_id in subtask_ids {
        let subtask = executor.get_issue(&subtask_id).unwrap();
        assert_eq!(subtask.labels.len(), 3);
        assert!(subtask.labels.contains(&"milestone:v1.0".to_string()));
        assert!(subtask.labels.contains(&"epic:auth".to_string()));
        assert!(subtask.labels.contains(&"component:backend".to_string()));
    }
}

#[test]
fn test_breakdown_preserves_parent_labels() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    let parent_id = executor
        .create_issue(
            "Parent".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    executor
        .breakdown_issue(&parent_id, vec![("Subtask".to_string(), "".to_string())])
        .unwrap();

    // Parent should still have its labels
    let parent = executor.get_issue(&parent_id).unwrap();
    assert_eq!(parent.labels.len(), 1);
    assert_eq!(parent.labels[0], "milestone:v1.0");
}

#[test]
fn test_breakdown_with_no_labels() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    let parent_id = executor
        .create_issue(
            "Parent".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![],
        )
        .unwrap();

    let subtask_ids = executor
        .breakdown_issue(&parent_id, vec![("Subtask".to_string(), "".to_string())])
        .unwrap();

    // Subtask should have no labels
    let subtask = executor.get_issue(&subtask_ids[0]).unwrap();
    assert_eq!(subtask.labels.len(), 0);
}

#[test]
fn test_breakdown_updates_parent_state() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    let parent_id = executor
        .create_issue(
            "Parent".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["milestone:v1.0".to_string()],
        )
        .unwrap();

    executor
        .breakdown_issue(
            &parent_id,
            vec![
                ("Subtask 1".to_string(), "".to_string()),
                ("Subtask 2".to_string(), "".to_string()),
            ],
        )
        .unwrap();

    // Parent should now be blocked (has dependencies)
    let parent = executor.get_issue(&parent_id).unwrap();
    assert_eq!(parent.dependencies.len(), 2);
    assert_eq!(parent.state, jit::domain::State::Backlog); // Blocked by deps
}

#[test]
fn test_breakdown_creates_dependency_edges() {
    let storage = InMemoryStorage::new();
    let executor = CommandExecutor::new(storage);
    executor.init().unwrap();

    let parent_id = executor
        .create_issue(
            "Parent".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["epic:feature".to_string()],
        )
        .unwrap();

    let subtask_ids = executor
        .breakdown_issue(
            &parent_id,
            vec![
                ("Sub1".to_string(), "".to_string()),
                ("Sub2".to_string(), "".to_string()),
            ],
        )
        .unwrap();

    // Verify parent depends on subtasks
    let parent = executor.get_issue(&parent_id).unwrap();
    for subtask_id in subtask_ids {
        assert!(parent.dependencies.contains(&subtask_id));
    }
}
