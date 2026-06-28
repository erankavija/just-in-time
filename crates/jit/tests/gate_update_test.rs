//! REQ-16: `jit gate update <key>` edits an existing gate definition's mutable
//! fields without hand-editing the registry.
//!
//! Tests confirm:
//!   1. Updating one field changes the registry (verified via `gate show --json`).
//!   2. Fields not passed are preserved (partial update).
//!   3. Updating a non-existent key errors; `--json` yields a machine-readable error.
//!   4. Per-issue `gates_status` on an issue requiring the gate is unchanged.
//!   5. The registry write is atomic: a sibling gate stays intact and valid JSON.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();
    temp
}

/// Define an automated gate with a checker so checker-field preservation can be
/// exercised.
fn define_auto_gate(temp: &TempDir, key: &str) {
    jit(temp)
        .args([
            "gate",
            "define",
            key,
            "--title",
            "Original Title",
            "--description",
            "Original description",
            "--mode",
            "auto",
            "--checker-command",
            "cargo test",
            "--timeout",
            "120",
            "--priority",
            "50",
        ])
        .assert()
        .success();
}

/// Read `gate show <key> --json` as a JSON value.
fn gate_show(temp: &TempDir, key: &str) -> serde_json::Value {
    let out = jit(temp)
        .args(["gate", "show", key, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    serde_json::from_slice(&out).expect("gate show --json must be valid JSON")
}

/// Parse the on-disk issue JSON file (the single file under `.jit/issues/`).
fn read_only_issue_file(temp: &TempDir) -> serde_json::Value {
    let issues_dir = temp.path().join(".jit").join("issues");
    let entry = fs::read_dir(&issues_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().and_then(|s| s.to_str()) == Some("json"))
        .expect("an issue file must exist");
    let contents = fs::read_to_string(entry.path()).unwrap();
    serde_json::from_str(&contents).unwrap()
}

#[test]
fn test_gate_update_changes_field_in_registry() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests");

    jit(&temp)
        .args(["gate", "update", "tests", "--title", "All Tests Pass"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Updated gate 'tests'"));

    let gate = gate_show(&temp, "tests");
    assert_eq!(gate["title"], "All Tests Pass");
}

#[test]
fn test_gate_update_preserves_unprovided_fields() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests");

    // Change only the title.
    jit(&temp)
        .args(["gate", "update", "tests", "--title", "New Title"])
        .assert()
        .success();

    let gate = gate_show(&temp, "tests");
    // Changed field.
    assert_eq!(gate["title"], "New Title");
    // Every other field is preserved.
    assert_eq!(gate["description"], "Original description");
    assert_eq!(gate["mode"], "auto");
    assert_eq!(gate["priority"], 50);
    assert_eq!(gate["checker"]["command"], "cargo test");
    assert_eq!(gate["checker"]["timeout_seconds"], 120);
}

#[test]
fn test_gate_update_checker_field_preserves_command() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests");

    // Update only the timeout; the command must be preserved.
    jit(&temp)
        .args(["gate", "update", "tests", "--timeout", "600"])
        .assert()
        .success();

    let gate = gate_show(&temp, "tests");
    assert_eq!(gate["checker"]["timeout_seconds"], 600);
    assert_eq!(gate["checker"]["command"], "cargo test");
    // Untouched non-checker fields stay put.
    assert_eq!(gate["title"], "Original Title");
}

#[test]
fn test_gate_update_json_returns_updated_definition() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests");

    let out = jit(&temp)
        .args([
            "gate",
            "update",
            "tests",
            "--description",
            "Updated via JSON",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let value: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(value["key"], "tests");
    assert_eq!(value["description"], "Updated via JSON");
    assert_eq!(value["message"], "Updated gate 'tests'");
}

#[test]
fn test_gate_update_nonexistent_key_errors() {
    let temp = setup_repo();

    jit(&temp)
        .args(["gate", "update", "missing", "--title", "X"])
        .assert()
        .failure();
}

#[test]
fn test_gate_update_nonexistent_key_json_is_machine_readable() {
    let temp = setup_repo();

    let assert = jit(&temp)
        .args(["gate", "update", "missing", "--title", "X", "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let value: serde_json::Value = serde_json::from_slice(&out).expect("error must be valid JSON");
    assert_eq!(value["error"]["code"], "GATE_NOT_FOUND");
}

#[test]
fn test_gate_update_no_fields_errors_json() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests");

    let assert = jit(&temp)
        .args(["gate", "update", "tests", "--json"])
        .assert()
        .failure();
    let out = assert.get_output().stdout.clone();
    let value: serde_json::Value = serde_json::from_slice(&out).expect("error must be valid JSON");
    assert_eq!(value["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_gate_update_leaves_per_issue_status_unchanged() {
    let temp = setup_repo();
    // A manual gate so the issue carries a per-issue status entry.
    jit(&temp)
        .args([
            "gate",
            "define",
            "review",
            "--title",
            "Review",
            "--description",
            "Manual review",
            "--mode",
            "manual",
        ])
        .assert()
        .success();

    jit(&temp)
        .args([
            "issue",
            "create",
            "--title",
            "Needs review",
            "--gate",
            "review",
        ])
        .assert()
        .success();

    // Establish a concrete per-issue status (Passed) so the comparison is
    // meaningful rather than an empty-map equality.
    let issue_id = read_only_issue_file(&temp)["id"]
        .as_str()
        .unwrap()
        .to_string();
    jit(&temp)
        .args(["gate", "pass", &issue_id, "review", "--by", "human:tester"])
        .assert()
        .success();

    // Capture the per-issue gate state BEFORE the registry edit.
    let before = read_only_issue_file(&temp);
    let status_before = before["gates_status"]["review"].clone();
    let updated_at_before = before["updated_at"].clone();
    assert_eq!(
        status_before["status"], "passed",
        "the issue must carry a concrete per-issue status for the required gate"
    );

    // Edit the registry definition (title + description).
    jit(&temp)
        .args([
            "gate",
            "update",
            "review",
            "--title",
            "Renamed Review",
            "--description",
            "Different text",
        ])
        .assert()
        .success();

    // Registry changed...
    let gate = gate_show(&temp, "review");
    assert_eq!(gate["title"], "Renamed Review");

    // ...but the per-issue gate STATUS and the issue file are untouched.
    let after = read_only_issue_file(&temp);
    assert_eq!(
        after["gates_status"]["review"], status_before,
        "per-issue gate status must be unchanged by a registry update"
    );
    assert_eq!(
        after["updated_at"], updated_at_before,
        "a registry update must not rewrite the issue file"
    );
}

#[test]
fn test_gate_update_is_atomic_sibling_gate_intact() {
    let temp = setup_repo();
    define_auto_gate(&temp, "tests");
    jit(&temp)
        .args([
            "gate",
            "define",
            "clippy",
            "--title",
            "Clippy",
            "--description",
            "Lints",
            "--mode",
            "auto",
            "--checker-command",
            "cargo clippy",
            "--timeout",
            "90",
        ])
        .assert()
        .success();

    jit(&temp)
        .args(["gate", "update", "tests", "--timeout", "777"])
        .assert()
        .success();

    // The registry file is valid JSON and the sibling gate is intact.
    let gates_path = temp.path().join(".jit").join("gates.json");
    assert!(Path::new(&gates_path).exists());
    let registry: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&gates_path).unwrap())
            .expect("registry must remain valid JSON after an update");
    assert_eq!(
        registry["gates"]["clippy"]["checker"]["command"],
        "cargo clippy"
    );
    assert_eq!(
        registry["gates"]["clippy"]["checker"]["timeout_seconds"],
        90
    );
    assert_eq!(
        registry["gates"]["tests"]["checker"]["timeout_seconds"],
        777
    );
}
