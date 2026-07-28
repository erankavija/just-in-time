//! Failure exit statuses are an invocation-form invariant: callers receive the
//! same process status whether they request human-readable or JSON output.

use std::fs;
use std::process::{Command, Output};

use jit::output::ErrorCode;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_repository() -> TempDir {
    let repository = TempDir::new().unwrap();
    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .arg("init")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    repository
}

fn run(repository: &TempDir, args: &[&str], json: bool) -> Output {
    let mut command = Command::new(jit_binary());
    command.current_dir(repository.path()).args(args);
    if json {
        command.arg("--json");
    }
    command.output().unwrap()
}

/// Observe the two process statuses instead of encoding the classification as a
/// duplicate literal in every assertion.
fn assert_failure_status_parity(repository: &TempDir, args: &[&str], expected: ErrorCode) {
    let plain = run(repository, args, false);
    let machine_readable = run(repository, args, true);

    assert!(
        !plain.status.success(),
        "plain invocation unexpectedly succeeded: {args:?}"
    );
    assert!(
        !machine_readable.status.success(),
        "JSON invocation unexpectedly succeeded: {args:?}"
    );
    assert_eq!(
        plain.status.code(),
        machine_readable.status.code(),
        "plain and JSON invocations must report the same status for {args:?}"
    );
    assert_eq!(
        plain.status.code(),
        Some(expected.exit_code().code()),
        "both invocation forms must report the mapped {expected} status for {args:?}"
    );
}

#[test]
fn test_unresolved_identifiers_have_not_found_status_parity_across_namespaces() {
    let repository = setup_repository();
    let existing_issue = run(
        &repository,
        &["issue", "create", "--title", "Existing issue"],
        true,
    );
    assert!(existing_issue.status.success());
    let existing_issue: serde_json::Value = serde_json::from_slice(&existing_issue.stdout).unwrap();
    let existing_issue = existing_issue["id"].as_str().unwrap();

    assert_failure_status_parity(
        &repository,
        &["dep", "add", existing_issue, "0000000000000000"],
        ErrorCode::IssueNotFound,
    );
    assert_failure_status_parity(
        &repository,
        &["issue", "claim", "0000000000000000", "agent:parity"],
        ErrorCode::IssueNotFound,
    );
    assert_failure_status_parity(
        &repository,
        &["gate", "add", "0000000000000000", "cargo-ci"],
        ErrorCode::IssueNotFound,
    );
}

#[test]
fn test_duplicate_gate_key_has_already_exists_status_parity() {
    let repository = setup_repository();
    let definition = [
        "gate",
        "define",
        "--title",
        "Parity gate",
        "--description",
        "Parity gate",
        "parity-gate",
    ];
    assert!(run(&repository, &definition, false).status.success());

    assert_failure_status_parity(&repository, &definition, ErrorCode::AlreadyExists);
}

#[test]
fn test_missing_preset_has_not_found_status_parity() {
    let repository = setup_repository();

    assert_failure_status_parity(
        &repository,
        &["gate", "preset", "show", "missing-preset"],
        ErrorCode::PresetError,
    );
}

#[test]
fn test_failing_preset_application_has_exit_status_parity() {
    let repository = setup_repository();

    assert_failure_status_parity(
        &repository,
        &["gate", "preset", "apply", "plan-review", "0000000000000000"],
        ErrorCode::PresetError,
    );
}

#[test]
fn test_malformed_stored_record_has_exit_status_parity() {
    let repository = setup_repository();
    fs::write(repository.path().join(".jit/index.json"), b"{").unwrap();

    assert_failure_status_parity(&repository, &["query", "all"], ErrorCode::ParseError);
}
