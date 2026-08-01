//! Tests for strategic queries (Phase 3)

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::storage::{InMemoryStorage, IssueStore};
use jit::test_taxonomy::{test_taxonomy, TestTaxonomy};

/// An executor over a repository that declares the type hierarchy these tests
/// classify against, together with the declaration it was configured from:
/// every type name below is read from that value rather than repeated.
/// `query_strategic` reads the declaration through a `ConfigManager` rooted at
/// the store, so the fixture writes it there as well as into the memory image.
fn strategic_executor() -> (CommandExecutor<InMemoryStorage>, TestTaxonomy) {
    let taxonomy = test_taxonomy();
    let storage = InMemoryStorage::new();
    let config = format!(
        "[worktree]\nenforce_leases = \"off\"\n\n{}",
        taxonomy.config_fragment()
    );
    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join("config.toml"), &config).unwrap();
    storage.add_data_file("config.toml", &config);
    (crate::memory_executor(storage), taxonomy)
}

/// The `type:<name>` label for a declared type.
fn type_label(name: &str) -> String {
    format!("type:{name}")
}

/// The membership label a declared type's own namespace carries.
fn membership_label(taxonomy: &TestTaxonomy, type_name: &str, value: &str) -> String {
    let namespace = taxonomy
        .label_associations
        .get(type_name)
        .expect("the declared vocabulary associates a membership namespace with this type");
    format!("{namespace}:{value}")
}

#[test]
fn test_query_strategic_returns_top_level_issues() {
    let (executor, taxonomy) = strategic_executor();
    let top = taxonomy.type_at_level(1);

    // Create issues with strategic types
    let (top_id, _) = executor
        .create_issue(
            "Release v1.0".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec![type_label(top), membership_label(&taxonomy, top, "v1.0")],
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
            vec![type_label(taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
        .unwrap();

    // Query strategic issues
    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, top_id);
}

#[test]
fn test_query_strategic_returns_second_level_issues() {
    let (executor, taxonomy) = strategic_executor();
    let container = taxonomy.type_at_level(2);

    let (container_id, _) = executor
        .create_issue(
            "Auth System".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec![
                type_label(container),
                membership_label(&taxonomy, container, "auth"),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, container_id);
}

#[test]
fn test_query_strategic_returns_every_declared_strategic_type() {
    let (executor, taxonomy) = strategic_executor();

    let strategic_ids: Vec<String> = taxonomy
        .strategic_types
        .iter()
        .map(|type_name| {
            let (id, _) = executor
                .create_issue(
                    format!("A {type_name}"),
                    "".to_string(),
                    Priority::High,
                    vec![],
                    vec![
                        type_label(type_name),
                        membership_label(&taxonomy, type_name, "scope"),
                    ],
                    None,
                    None,
                    false,
                )
                .unwrap();
            id
        })
        .collect();

    let (_tactical_id, _) = executor
        .create_issue(
            "Fix typo".to_string(),
            "".to_string(),
            Priority::Low,
            vec![],
            vec![type_label(taxonomy.type_at_level(4))],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), taxonomy.strategic_types.len());
    let ids: Vec<String> = strategic.iter().map(|i| i.id.clone()).collect();
    assert!(strategic_ids.iter().all(|id| ids.contains(id)));
}

#[test]
fn test_query_strategic_excludes_tactical_only() {
    let (executor, taxonomy) = strategic_executor();

    // Create only tactical issues
    executor
        .create_issue(
            "Task 1".to_string(),
            "".to_string(),
            Priority::Normal,
            vec![],
            vec![type_label(taxonomy.type_at_level(4))],
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
            vec!["area:backend".to_string()],
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
    let (executor, taxonomy) = strategic_executor();
    let top = taxonomy.type_at_level(1);

    // Issue with both strategic type and tactical labels
    let (mixed_id, _) = executor
        .create_issue(
            "Auth release".to_string(),
            "".to_string(),
            Priority::High,
            vec![],
            vec![
                type_label(top),
                membership_label(&taxonomy, top, "v1.0"),
                "area:auth".to_string(),
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
fn test_query_strategic_ignores_membership_label_without_strategic_type() {
    let (executor, taxonomy) = strategic_executor();
    let top = taxonomy.type_at_level(1);
    let container = taxonomy.type_at_level(2);

    // Strategic classification is type-based, not namespace-based: a container's
    // membership label on a leaf-typed issue does not make it strategic.
    let (_member_id, _) = executor
        .create_issue(
            "Digital transformation".to_string(),
            "".to_string(),
            Priority::Critical,
            vec![],
            vec![
                membership_label(&taxonomy, container, "cloud-migration"),
                type_label(taxonomy.type_at_level(4)),
            ],
            None,
            None,
            false,
        )
        .unwrap();

    // Create issue with strategic type
    let (top_id, _) = executor
        .create_issue(
            "Launch".to_string(),
            "".to_string(),
            Priority::Critical,
            vec![],
            vec![type_label(top)],
            None,
            None,
            false,
        )
        .unwrap();

    let strategic = executor.query_strategic().unwrap();

    // Only the strategically typed issue is returned.
    assert_eq!(strategic.len(), 1);
    assert_eq!(strategic[0].id, top_id);
}

#[test]
fn test_query_strategic_empty_repo() {
    let (executor, _taxonomy) = strategic_executor();

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 0);
}
