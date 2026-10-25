//! `cli_gate` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod bracket_coverage_gate_run_test;
mod gate_cli_integration_test;
mod gate_evaluate_all_test;
mod gate_evaluate_exit_code_test;
mod gate_evaluate_rerun_test;
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

use jit::test_cli::JitCli;

/// Define auto gate `key` running `checker`.
pub(crate) fn define_auto_gate(cli: &JitCli, key: &str, checker: &str) {
    cli.run([
        "gate",
        "define",
        key,
        "--title",
        key,
        "--description",
        "Test",
        "--mode",
        "auto",
        "--checker-command",
        checker,
        "--timeout",
        "10",
    ])
    .unwrap();
}

/// Define manual gate `key`.
pub(crate) fn define_manual_gate(cli: &JitCli, key: &str) {
    cli.run([
        "gate",
        "define",
        key,
        "--title",
        key,
        "--description",
        "Test",
        "--mode",
        "manual",
    ])
    .unwrap();
}

/// Create an issue requiring `gate_keys`; returns its id.
pub(crate) fn create_issue_requiring(cli: &JitCli, gate_keys: &[&str]) -> String {
    let gates = gate_keys.iter().flat_map(|key| ["--gate", *key]);
    cli.create_issue(["--title", "Test issue"].into_iter().chain(gates))
        .unwrap()
}

/// Define auto gate `key` running `checker` and create an issue requiring it.
pub(crate) fn add_auto_gate_issue(cli: &JitCli, key: &str, checker: &str) -> String {
    define_auto_gate(cli, key, checker);
    create_issue_requiring(cli, &[key])
}

/// Define manual gate `key` and create an issue requiring it.
pub(crate) fn add_manual_gate_issue(cli: &JitCli, key: &str) -> String {
    define_manual_gate(cli, key);
    create_issue_requiring(cli, &[key])
}

/// Create an issue titled `title`; returns its id.
pub(crate) fn create_titled_issue(cli: &JitCli, title: &str) -> String {
    cli.create_issue(["--title", title]).unwrap()
}

/// `gate evaluate --force`: records a fresh run whatever its verdict.
pub(crate) fn run_gate_forced(cli: &JitCli, issue_id: &str, gate_key: &str) {
    cli.command(["gate", "evaluate", issue_id, gate_key, "--force"])
        .output()
        .unwrap();
}
