//! Tests for strategic queries (Phase 3)

use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::domain::Priority;
use jit::storage::InMemoryStorage;
use jit::test_taxonomy::{test_taxonomy, TestTaxonomy};

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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );
    let top = taxonomy.type_at_level(1);

    // Create issues with strategic types
    let (top_id, _) = executor
        .issues()
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
        .issues()
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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );
    let container = taxonomy.type_at_level(2);

    let (container_id, _) = executor
        .issues()
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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );

    let strategic_ids: Vec<String> = taxonomy
        .strategic_types
        .iter()
        .map(|type_name| {
            let (id, _) = executor
                .issues()
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
        .issues()
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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );

    // Create only tactical issues
    executor
        .issues()
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
        .issues()
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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );
    let top = taxonomy.type_at_level(1);

    // Issue with both strategic type and tactical labels
    let (mixed_id, _) = executor
        .issues()
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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );
    let top = taxonomy.type_at_level(1);
    let container = taxonomy.type_at_level(2);

    // Strategic classification is type-based, not namespace-based: a container's
    // membership label on a leaf-typed issue does not make it strategic.
    let (_member_id, _) = executor
        .issues()
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
        .issues()
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
    let taxonomy = test_taxonomy();
    let executor = memory_executor_declaring(
        InMemoryStorage::new(),
        MemoryDeclarations {
            config: Some(&format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            )),
            ..MemoryDeclarations::default()
        },
    );

    let strategic = executor.query_strategic().unwrap();

    assert_eq!(strategic.len(), 0);
}
