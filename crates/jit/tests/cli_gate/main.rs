//! `cli_gate` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod bracket_coverage_gate_run_test;
mod gate_cli_integration_test;
mod gate_evaluate_all_test;
mod gate_evaluate_exit_code_test;
mod gate_evaluate_skip_at_head_test;
mod gate_evaluation_durability_test;
mod gate_findings_test;
mod gate_key_flag_test;
mod gate_modification_cli_tests;
mod gate_preset_apply_json_failure_test;
mod gate_status_all_lean_response_test;
mod gate_status_all_strict_test;
mod gate_status_history_flat_test;
mod gate_update_test;
mod nested_checker_recovery_test;
