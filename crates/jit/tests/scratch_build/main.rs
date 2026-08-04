//! `scratch_build` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod assemble_package_argument_contract_tests;
mod build_profile_policy_tests;
mod dependency_feature_policy_tests;
mod merged_tree_gate_verification_tests;
mod rust_build_budget_checker_tests;
mod stale_binary_child_process_tests;
mod stale_binary_json_exit_tests;
