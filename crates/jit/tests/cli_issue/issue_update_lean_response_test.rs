//! Verify `jit issue update --json` returns a lightweight confirmation
//! response (id, short_id, state, updated_at) instead of the full issue body.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn setup_repo_with_issue(description: &str) -> (TempDir, String) {
    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Some issue",
            "--description",
            description,
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);
    let id = stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();
    (temp, id)
}

#[test]
fn test_update_json_omits_description() {
    let big_description = "A".repeat(5000);
    let (temp, id) = setup_repo_with_issue(&big_description);

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "update", &id, "--priority", "high", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let data = &json;

    assert!(
        data.get("description").is_none(),
        "lean update response must not include description; got: {}",
        data
    );

    let serialized = serde_json::to_string(&json).unwrap();
    assert!(
        !serialized.contains(&big_description),
        "description content leaked into update response"
    );
}

#[test]
fn test_update_json_contains_id_short_id_state_updated_at() {
    let (temp, id) = setup_repo_with_issue("short");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "update", &id, "--priority", "high", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let data = &json;

    assert!(data["id"].is_string(), "missing id; got: {}", data);
    assert!(
        data["short_id"].is_string(),
        "missing short_id; got: {}",
        data
    );
    assert!(data["state"].is_string(), "missing state; got: {}", data);
    assert!(
        data["updated_at"].is_string(),
        "missing updated_at; got: {}",
        data
    );
}

#[test]
fn test_update_json_omits_enriched_dependencies() {
    let (temp, id) = setup_repo_with_issue("short");

    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["issue", "update", &id, "--priority", "high", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let data = &json;

    assert!(
        data.get("dependencies").is_none() && data.get("labels").is_none(),
        "lean update response must not include full issue fields; got: {}",
        data
    );
}
