//! `fast_rules` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod cli_warnings_integration_tests;
mod config_loading_tests;
mod default_rules_registry_derivation_tests;
mod effective_rules_tests;
mod example_rulesets_tests;
mod label_membership_validation_tests;
mod label_query_tests;
mod label_strategic_tests;
mod local_rule_enforcement_tests;
mod namespace_unique_writethrough_tests;
mod project_render_harness_tests;
mod type_hierarchy_schema_regen_tests;
mod type_taxonomy_custom_strategic_tests;
mod type_taxonomy_warnings_tests;
mod validate_rule_runner_tests;
mod validation_tests;

fn memory_executor(
    storage: jit::storage::InMemoryStorage,
) -> jit::commands::CommandExecutor<jit::storage::InMemoryStorage> {
    let layout = storage.repository_layout();
    jit::commands::CommandExecutor::new(storage).with_layout(layout)
}

fn fixture_issue(title: String, description: String) -> jit::domain::Issue {
    let mut issue = jit::domain::Issue::draft(title, description);
    issue.id = uuid::Uuid::new_v4().to_string();
    issue
}
