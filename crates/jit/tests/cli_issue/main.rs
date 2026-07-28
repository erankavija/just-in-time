//! `cli_issue` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod batch_create_tests;
mod bulk_update_cli_tests;
mod command_alias_tests;
mod command_exit_code_projection_tests;
mod container_rollup_tests;
mod error_json_tests;
mod exit_code_tests;
mod failure_lever_registry;
mod failure_probe_fixture;
mod gate_field_contract_test;
mod invocation_exit_status_parity_tests;
mod issue_create_json_contract_test;
mod issue_create_positional_type_tests;
mod issue_search_json_tests;
mod issue_search_label_filter_tests;
mod issue_show_projection_test;
mod issue_show_shape_test;
mod issue_show_summary_test;
mod issue_status_projection_tests;
mod issue_update_description_ops_tests;
mod issue_update_lean_response_test;
mod list_envelope_tests;
mod payload_stream_purity_tests;
mod quiet_mode_tests;
mod recover_command_tests;
mod search_exit_status_tests;
mod serve_failure_envelope_tests;
mod snapshot_export_tests;
mod stored_record_classification_tests;
mod top_level_failure_envelope_tests;
mod verb_hint_tests;
