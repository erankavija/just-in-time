//! `fast_docs_templates` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

// Shared in-process test harness (below the auto-discovery boundary).
#[path = "../common/harness.rs"]
mod harness;

mod ai_review_verdict_tests;
mod artifact_discovery_tests;
mod artifact_inventory_tests;
mod artifact_mutation_storage_tests;
mod artifact_plan_model_tests;
mod batch_export_tests;
mod bracket_breakdown_tests;
mod code_review_policy_test;
mod content_format_dispatch;
mod content_parser_cross_format;
mod doc_review_policy_test;
mod document_event_log_tests;
mod git_revision_tests;
mod harness_demo;
mod lock_tests;
mod planning_preset_tests;
mod research_bracket_tests;
mod schema_tests;
mod sdd_bracket_tests;
mod template_apply_atomicity_tests;
mod template_apply_tests;
mod template_binding_tests;
mod templates_loader_tests;

fn seed_memory_data_file(storage: &jit::storage::InMemoryStorage, name: &str, content: &str) {
    use jit::storage::IssueStore;

    std::fs::create_dir_all(storage.root()).unwrap();
    std::fs::write(storage.root().join(name), content).unwrap();
    storage.add_repo_file(&format!(".jit/{name}"), content);
}

fn fixture_issue(title: String, description: String) -> jit::domain::Issue {
    let mut issue = jit::domain::Issue::draft(title, description);
    issue.id = uuid::Uuid::new_v4().to_string();
    issue
}
