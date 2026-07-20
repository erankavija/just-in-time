//! `fast_issue` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

// Shared in-process test harness (below the auto-discovery boundary).
#[path = "../common/harness.rs"]
mod harness;

mod archived_semantics_tests;
mod bulk_operations_tests;
mod dep_add_atomic_tests;
mod dependency_met_tests;
mod domain_queries_tests;
mod graph_rule_validation_tests;
mod hierarchy_vectors_test;
mod lifecycle_timestamps_tests;
mod short_hash_tests;
mod state_transition_fix_tests;
mod strictness_enforcement_tests;
mod strictness_transition_tests;
mod test_no_coordinator;
mod transition_graph_enforcement_tests;
mod transitive_reduction_validation_tests;

fn fixture_issue(title: String, description: String) -> jit::domain::Issue {
    let mut issue = jit::domain::Issue::draft(title, description);
    issue.id = uuid::Uuid::new_v4().to_string();
    issue
}
