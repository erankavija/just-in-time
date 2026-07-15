//! `scratch_build` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod build_profile_policy_tests;
mod merged_commit_build_verification_tests;
mod stale_binary_child_process_tests;
mod stale_binary_json_exit_tests;
