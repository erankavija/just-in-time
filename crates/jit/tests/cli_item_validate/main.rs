//! `cli_item_validate` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod decision_kind_tests;
mod decision_risk_story_tests;
mod derived_state_repair_tests;
mod invariant_check_cli_tests;
mod invariant_registry_story_tests;
mod item_cli_tests;
mod markdown_kind_source_path_tests;
mod project_render_cli_tests;
mod registry_json_tests;
mod risk_kind_tests;
mod type_hierarchy_fix_tests;
mod validate_cli_rule_tests;
mod validate_document_tests;
mod validate_drift_builtin_tests;
mod validation_lease_tests;

fn create_issue(
    executor: &jit::commands::CommandExecutor<jit::storage::JsonFileStorage>,
    title: &str,
    body: &str,
) -> String {
    use jit::storage::IssueStore;

    let id = executor
        .create_issue(
            title.to_string(),
            body.to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap()
        .0;
    executor.storage().load_issue(&id).unwrap().short_id()
}

/// The package carrying the generic vocabulary these suites address: the type
/// hierarchy, the label namespaces, and the addressable item kinds an item is
/// resolved against.
pub(crate) const DEFAULT_PACKAGE: &str = "jit-default";

/// A fresh repository carrying the default package's vocabulary.
pub(crate) fn setup_repo_with_default_vocabulary() -> tempfile::TempDir {
    jit::test_utils::profiled_repository_fixture(
        DEFAULT_PACKAGE,
        "packages",
        Some(std::path::Path::new(env!("CARGO_BIN_EXE_jit"))),
    )
    .expect("clone a coherent default-vocabulary repository fixture")
}
