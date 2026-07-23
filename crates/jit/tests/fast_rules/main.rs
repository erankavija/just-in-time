//! `fast_rules` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

#[path = "../common/harness.rs"]
mod harness;

mod cli_warnings_integration_tests;
mod config_loading_tests;
mod default_rules_registry_derivation_tests;
mod derived_state_repair_tests;
mod effective_rules_tests;
mod example_rulesets_tests;
mod label_membership_validation_tests;
mod label_query_tests;
mod label_strategic_tests;
mod local_rule_enforcement_tests;
mod project_render_harness_tests;
mod type_hierarchy_schema_regen_tests;
mod type_taxonomy_custom_strategic_tests;
mod type_taxonomy_warnings_tests;
mod validate_rule_runner_tests;
mod validation_tests;

fn memory_executor(
    storage: jit::storage::InMemoryStorage,
) -> jit::commands::CommandExecutor<jit::storage::InMemoryStorage> {
    use jit::storage::IssueStore;

    if storage
        .read_repo_file(".jit/config.toml")
        .unwrap()
        .is_none()
    {
        storage.add_data_file("config.toml", "");
    }
    if storage.read_repo_file(".jit/index.json").unwrap().is_none() {
        let mut ids = storage
            .list_issues()
            .unwrap()
            .into_iter()
            .map(|issue| issue.id)
            .collect::<Vec<_>>();
        ids.sort();
        storage.add_data_file(
            "index.json",
            &serde_json::json!({
                "schema_version": 2,
                "all_ids": ids,
                "deleted_ids": [],
            })
            .to_string(),
        );
    }
    let layout = storage.repository_layout();
    jit::commands::CommandExecutor::new(storage).with_layout(layout)
}

fn fixture_issue(title: String, description: String) -> jit::domain::Issue {
    let mut issue = jit::domain::Issue::draft(title, description);
    issue.id = uuid::Uuid::new_v4().to_string();
    issue
}
