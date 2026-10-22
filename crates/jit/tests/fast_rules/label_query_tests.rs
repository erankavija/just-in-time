//! Tests for label-based query functionality (Phase 1.4)

use jit::domain::Priority;
use jit::storage::InMemoryStorage;

#[test]
fn test_query_by_label_exact_match() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

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
    let result_ids: Vec<String> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&milestone_id));
    assert!(result_ids.contains(&epic_id));
}

#[test]
fn test_query_by_label_wildcard_namespace() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Create issues with various milestones
    let (v1_id, _) = executor
        .issues()
        .create_issue(
            "Task v1".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v1.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (v2_id, _) = executor
        .issues()
        .create_issue(
            "Task v2".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v2.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (_no_milestone_id, _) = executor
        .issues()
        .create_issue(
            "No milestone".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["epic:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    // Query wildcard milestone:*
    let results = executor.query_by_label("milestone:*").unwrap();

    assert_eq!(results.len(), 2);
    let result_ids: Vec<String> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&v1_id));
    assert!(result_ids.contains(&v2_id));
}

#[test]
fn test_query_by_label_no_matches() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Create issue without the queried label
    let (_id, _) = executor
        .issues()
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["milestone:v1.0".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    // Query for non-existent label
    let results = executor.query_by_label("epic:auth").unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_query_by_label_empty_repo() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Query with no issues
    let results = executor.query_by_label("milestone:v1.0").unwrap();

    assert_eq!(results.len(), 0);
}

#[test]
fn test_query_by_label_wildcard_matches_all_in_namespace() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Create issues with multiple types
    let (task_id, _) = executor
        .issues()
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:task".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (bug_id, _) = executor
        .issues()
        .create_issue(
            "Bug".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:bug".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let (epic_id, _) = executor
        .issues()
        .create_issue(
            "Epic".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["type:epic".to_string(), "component:auth".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    // Query wildcard type:*
    let results = executor.query_by_label("type:*").unwrap();

    assert_eq!(results.len(), 3);
    let result_ids: Vec<String> = results.iter().map(|i| i.id.clone()).collect();
    assert!(result_ids.contains(&task_id));
    assert!(result_ids.contains(&bug_id));
    assert!(result_ids.contains(&epic_id));
}

#[test]
fn test_query_by_label_case_sensitive() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Create issue with specific case
    let (_id, _) = executor
        .issues()
        .create_issue(
            "Task".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["component:AuthService".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    // Query with different case should not match (case sensitive)
    let results = executor.query_by_label("component:authservice").unwrap();
    assert_eq!(results.len(), 0);

    // Query with exact case should match
    let results = executor.query_by_label("component:AuthService").unwrap();
    assert_eq!(results.len(), 1);
}

#[test]
fn test_query_by_label_at_prefixed_value_matches_whole() {
    // Audit (jit:7a2bbe4f) REQ-01: `query_by_label`'s `splitn(2, ':')` only
    // validates the namespace segment's charset; the ORIGINAL, un-re-split
    // `pattern` is what `domain::queries::query_by_label` (via
    // `labels::matches_pattern`'s exact-string comparison) matches against, so
    // a widened-grammar `@`-prefixed value is compared whole, never re-split
    // further.
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    let (matching_id, _) = executor
        .issues()
        .create_issue(
            "Rule-linked".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["enforces:@/rule/label-format".to_string()],
            None,
            None,
            false,
        )
        .unwrap();
    executor
        .issues()
        .create_issue(
            "Other rule-linked".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec!["enforces:@/rule/other-rule".to_string()],
            None,
            None,
            false,
        )
        .unwrap();

    let results = executor
        .query_by_label("enforces:@/rule/label-format")
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].id, matching_id);
}

#[test]
fn test_query_by_label_invalid_pattern() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Invalid pattern (no colon)
    let result = executor.query_by_label("invalidlabel");
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("Invalid label pattern"));
}

#[test]
fn test_query_by_label_multiple_labels_per_issue() {
    let storage = InMemoryStorage::new();
    let executor = crate::memory_executor(storage);

    // Create issue with multiple labels
    let (multi_id, _) = executor
        .issues()
        .create_issue(
            "Multi-labeled".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![
                "milestone:v1.0".to_string(),
                "epic:auth".to_string(),
                "component:backend".to_string(),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    // Should match any of the labels
    let results1 = executor.query_by_label("milestone:v1.0").unwrap();
    assert_eq!(results1.len(), 1);
    assert_eq!(results1[0].id, multi_id);

    let results2 = executor.query_by_label("epic:auth").unwrap();
    assert_eq!(results2.len(), 1);
    assert_eq!(results2[0].id, multi_id);

    let results3 = executor.query_by_label("component:backend").unwrap();
    assert_eq!(results3.len(), 1);
    assert_eq!(results3[0].id, multi_id);
}

fn create_labeled(
    executor: &jit::commands::CommandExecutor<InMemoryStorage>,
    title: &str,
    labels: &[&str],
) -> String {
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
    let executor = crate::memory_executor(InMemoryStorage::new());
    create_labeled(&executor, "Task one", &["type:task"]);
    create_labeled(&executor, "Epic one", &["type:epic", "area:auth"]);
    create_labeled(&executor, "Unlabelled-ish", &["area:auth"]);

    let pattern = "type:*".to_string();
    let searched = executor
        .search_issues_with_filters("", None, None, None, std::slice::from_ref(&pattern))
        .unwrap();
    let queried = executor.query_by_labels(&[pattern]).unwrap();

    let ids = |issues: &[jit::domain::Issue]| {
        let mut ids: Vec<_> = issues.iter().map(|i| i.id.clone()).collect();
        ids.sort();
        ids
    };
    assert_eq!(searched.len(), 2);
    assert_eq!(ids(&searched), ids(&queried));
}

#[test]
fn test_search_issues_with_filters_wildcard_still_applies_text_query() {
    let executor = crate::memory_executor(InMemoryStorage::new());
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
    let executor = crate::memory_executor(InMemoryStorage::new());
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
