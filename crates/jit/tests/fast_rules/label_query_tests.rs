//! Tests for label-based query functionality (Phase 1.4)

use jit::commands::test_helpers::memory_executor;
use jit::domain::identity::IssueId;
use jit::domain::Priority;
use jit::storage::InMemoryStorage;

#[test]
fn test_query_all_invalid_label_pattern() {
    let storage = InMemoryStorage::new();
    let executor = memory_executor(storage);

    // Invalid pattern (no colon)
    let result = executor
        .queries()
        .query_all(None, None, None, &["invalidlabel".to_string()]);
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
        .issue
        .id
}

#[test]
fn test_search_issues_with_filters_namespace_wildcard_matches_query_all_label_set() {
    let executor = memory_executor(InMemoryStorage::new());
    create_labeled(&executor, "Task one", &["type:task"]);
    create_labeled(&executor, "Epic one", &["type:epic", "area:auth"]);
    create_labeled(&executor, "Unlabelled-ish", &["area:auth"]);

    let pattern = "type:*".to_string();
    let searched = executor
        .queries()
        .search_issues_with_filters("", None, None, None, std::slice::from_ref(&pattern))
        .unwrap();
    let queried = executor
        .queries()
        .query_all(None, None, None, &[pattern])
        .unwrap();

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
        .queries()
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
            .queries()
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
