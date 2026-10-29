//! Tests for label-based query functionality (Phase 1.4)

use jit::commands::test_helpers::memory_executor;
use jit::domain::identity::IssueId;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;

#[test]
fn test_query_by_label_exact_match() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage);

    // Create issues with various labels
    let (milestone_id, _) = executor
        .issues()
        .create_issue(
            "Release v1.0".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec!["milestone:v1.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (epic_id, _) = executor
        .issues()
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["epic:auth".to_string(), "milestone:v1.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (_other_id, _) = executor
        .issues()
        .create_issue(
            "Other Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v2.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    // Query exact match for milestone:v1.0
    let results = executor.query_by_label("milestone:v1.0").unwrap();

    assert_eq!(results.len(), 2);
    let result_ids: Vec<IssueId> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&milestone_id));
    assert!(result_ids.contains(&epic_id));
}

#[test]
fn test_query_by_label_invalid_pattern() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage);

    // Invalid pattern (no colon)
    let result = executor.query_by_label("invalidlabel");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid label pattern"));
}

fn create_labeled(
    executor: &jit::commands::CommandExecutor<InMemoryStorage>,
    title: &str,
    labels: &[&str],
) -> IssueId {
    executor
        .issues()
        .create_issue(
            title.to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            labels.iter().map(|l| l.to_string()).collect(),
            None,
            None,
            false,
        )
        .unwrap()
        .0
}

#[test]
fn test_search_issues_with_filters_namespace_wildcard_matches_query_by_label_set() {
    let executor = memory_executor(InMemoryStorage::new());
    create_labeled(&executor, "Task one", &["type:task"]);
    create_labeled(&executor, "Epic one", &["type:epic", "area:auth"]);
    create_labeled(&executor, "Unlabelled-ish", &["area:auth"]);

    let pattern = "type:*".to_string();
    let searched = executor
        .search_issues_with_filters("", None, None, None, std::slice::from_ref(&pattern))
        .unwrap();
    let queried = executor.query_by_labels(&[pattern]).unwrap();

    let ids = |issues: &[jit::domain::Issue]| {
        let mut ids: Vec<IssueId> = issues.iter().map(|i| i.id.clone()).collect();
        ids.sort();
        ids
    };
    assert_eq!(searched.len(), 2);
    assert_eq!(ids(&searched), ids(&queried));
}

#[test]
fn test_search_issues_with_filters_wildcard_still_applies_text_query() {
    let executor = memory_executor(InMemoryStorage::new());
    let wanted = create_labeled(&executor, "Billing task", &["type:task"]);
    create_labeled(&executor, "Auth task", &["type:task"]);
    create_labeled(&executor, "Billing note", &["area:billing"]);

    let results = executor
        .search_issues_with_filters("billing", None, None, None, &["type:*".to_string()])
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, wanted);
}

#[test]
fn test_search_issues_with_filters_malformed_pattern_is_invalid_label_pattern() {
    let executor = memory_executor(InMemoryStorage::new());
    // Search goes through the shared validator, so it rejects malformed exact
    // labels as well as malformed wildcards; `labels` owns the full matrix.
    for bad in ["type", ":*", "type:", "type:a:b"] {
        let error = executor
            .search_issues_with_filters("", None, None, None, &[bad.to_string()])
            .unwrap_err();
        assert!(
            error
                .downcast_ref::<jit::errors::InvalidLabelPatternError>()
                .is_some(),
            "{bad:?} should be rejected as an invalid label pattern: {error}"
        );
    }
}
