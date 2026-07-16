//! `cli_item_validate` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod decision_kind_tests;
mod decision_risk_story_tests;
mod invariant_check_cli_tests;
mod invariant_registry_story_tests;
mod item_cli_tests;
mod markdown_kind_source_path_tests;
mod project_render_cli_tests;
mod registry_json_tests;
mod risk_kind_tests;
mod type_hierarchy_fix_tests;
mod validate_cli_rule_tests;
mod validate_document_tests;
mod validate_drift_builtin_tests;
mod validation_lease_tests;
