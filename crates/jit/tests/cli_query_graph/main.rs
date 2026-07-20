//! `cli_query_graph` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod batch_export_cli_tests;
mod check_links_tests;
mod dep_add_redundancy_cli_tests;
mod dependency_display_tests;
mod graph_depth_tests;
mod graph_json_tests;
mod hierarchy_tree_tests;
mod label_constraints_tests;
mod label_filter_repeatable_query_tests;
mod label_hierarchy_e2e_test;
mod label_query_json_tests;
mod query_json_tests;
mod query_tests;
mod remote_document_tls_tests;
mod scope_validation_tests;
mod search_tests;

fn fixture_issue(title: String, description: String) -> jit::domain::Issue {
    let mut issue = jit::domain::Issue::draft(title, description);
    issue.id = uuid::Uuid::new_v4().to_string();
    issue
}
