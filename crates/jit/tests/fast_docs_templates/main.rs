//! `fast_docs_templates` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

use jit::test_utils::fixture_issue;

mod ai_review_verdict_tests;
mod archive_path_layout_tests;
mod artifact_discovery_tests;
mod artifact_inventory_tests;
mod artifact_plan_model_tests;
mod batch_export_tests;
mod content_format_dispatch;
mod content_parser_cross_format;
mod document_event_log_tests;
mod git_module_tests;
mod git_revision_tests;
mod harness_demo;
mod planning_preset_tests;
mod research_bracket_tests;
mod schema_tests;
mod sdd_bracket_tests;
mod template_apply_atomicity_tests;
mod template_apply_tests;
mod template_binding_tests;
mod templates_loader_tests;

fn seed_memory_data_file(storage: &jit::storage::InMemoryStorage, name: &str, content: &str) {
    let virtual_path = jit::repository_state::VirtualPath::data(name).unwrap();
    let host_path = storage.repository_layout().resolve(&virtual_path).unwrap();
    std::fs::create_dir_all(host_path.parent().unwrap()).unwrap();
    std::fs::write(host_path, content).unwrap();
    storage.add_data_file(name, content);
}
