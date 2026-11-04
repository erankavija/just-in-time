//! `scratch_build` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod build_profile_policy_tests;
mod command_executor_surface_policy_tests;
mod dependency_feature_policy_tests;
mod process_state_policy_tests;
mod repository_state_cutover_tests;
