//! Stored-record decode failures remain typed across the library/binary boundary
//! and report the registered parse code in machine-readable CLI output.

use std::fs;
use std::process::Command;

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

#[test]
fn test_repository_index_parse_error_survives_anyhow_wrapping_as_public_type() {
    let source = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
    let error = anyhow::Error::new(jit::repository_state::RepositoryIndexError::Parse(source))
        .context("failed to load stored repository index");

    assert!(error
        .downcast_ref::<jit::repository_state::RepositoryIndexError>()
        .is_some());
}

#[test]
fn test_query_all_json_classifies_malformed_stored_index_as_parse_error() {
    let repository = setup_repository();
    fs::write(repository.path().join(".jit/index.json"), b"{").unwrap();

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["query", "all", "--json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let envelope: serde_json::Value = serde_json::from_slice(&output.stdout)
        .expect("malformed stored index should emit a JSON error envelope");
    assert_eq!(envelope["error"]["code"], "PARSE_ERROR");
    assert_ne!(envelope["error"]["code"], "GENERIC_ERROR");
}

#[test]
fn test_query_all_json_preserves_valid_stored_index_behavior() {
    let repository = setup_repository();

    let output = Command::new(jit_binary())
        .current_dir(repository.path())
        .args(["query", "all", "--json"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "valid stored index failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["count"], 0);
    assert_eq!(response["issues"], serde_json::json!([]));
}
