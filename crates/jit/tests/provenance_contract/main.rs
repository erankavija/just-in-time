//! `provenance_contract` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod build_provenance_metadata_stability_tests;
mod install_dirty_flag_tests;
mod repository_inventory;
mod repository_inventory_tests;
mod repository_state_cutover_tests;
mod version_cli_tests;
