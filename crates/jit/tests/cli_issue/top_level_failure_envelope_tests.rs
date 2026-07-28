//! Structural machine-readable failures rendered by the top-level CLI boundary.

use jit::output::ErrorCode;
use std::fs;
use std::process::{Command, Output};
use std::str::FromStr;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_repository() -> TempDir {
    let repository = TempDir::new().expect("create temporary repository");
    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .arg("init")
        .output()
        .expect("initialize repository");
    assert!(output.status.success(), "init failed: {output:?}");
    repository
}

fn parse_single_error(output: &Output) -> (serde_json::Value, ErrorCode) {
    let stdout = std::str::from_utf8(&output.stdout).expect("stdout is UTF-8");
    let envelope: serde_json::Value =
        serde_json::from_str(stdout).expect("stdout contains exactly one JSON document");
    assert_eq!(envelope.as_object().map(|object| object.len()), Some(1));
    let code = ErrorCode::from_str(
        envelope["error"]["code"]
            .as_str()
            .expect("error envelope carries a code"),
    )
    .expect("top-level error code is registered");
    (envelope, code)
}

#[test]
fn test_propagated_config_parse_failure_emits_registered_envelope_and_human_diagnostic() {
    let repository = setup_repository();
    fs::write(
        repository.path().join(".jit/config.toml"),
        "this is not valid TOML [[[",
    )
    .expect("corrupt repository config");

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["label", "namespaces", "--json"])
        .output()
        .expect("run propagated failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ParseError);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert!(envelope["error"]["message"].is_string());
    assert!(
        String::from_utf8_lossy(&output.stderr).starts_with("Error: "),
        "human diagnostic remains on stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_stored_record_parse_failure_uses_typed_registered_code() {
    let repository = setup_repository();
    fs::write(repository.path().join(".jit/index.json"), "{")
        .expect("corrupt stored repository index");

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["query", "all", "--json"])
        .output()
        .expect("run stored-record failure fixture");

    let (_, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::ParseError);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
}

#[test]
fn test_command_specific_error_keeps_one_envelope_and_its_details() {
    let repository = setup_repository();

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["issue", "show", "missing", "--json"])
        .output()
        .expect("run command-specific failure fixture");

    let (envelope, code) = parse_single_error(&output);
    assert_eq!(code, ErrorCode::IssueNotFound);
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    assert_eq!(envelope["error"]["details"]["issue_id"], "missing");
    assert!(
        output.stderr.is_empty(),
        "handler-owned error stays unchanged"
    );
}
