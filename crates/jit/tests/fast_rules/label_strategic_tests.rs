//! Tests for strategic queries (Phase 3)

use jit::commands::test_helpers::{memory_executor_declaring, MemoryDeclarations};
use jit::domain::identity::IssueId;
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

    let strategic_ids: Vec<IssueId> = taxonomy
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
    let ids: Vec<IssueId> = strategic.iter().map(|i| i.id.clone()).collect();
    assert!(strategic_ids.iter().all(|id| ids.contains(id)));
}
