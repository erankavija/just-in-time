//! Verify the `jit issue show --json` root shape:
//! - a `short_id` field (first 8 chars of the full UUID) is always present
//! - `labels` and `dependencies` are always JSON arrays (empty `[]` when none),
//!   never absent or null
//!
//! Covered across lifecycle states (backlog and an issue carrying labels +
//! dependencies) so agents can parse the output without defensive guards.

use assert_cmd::prelude::*;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir, args: &[&str]) -> Vec<u8> {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone()
}

fn create_issue(temp: &TempDir, title: &str) -> String {
    let stdout = jit(
        temp,
        &["issue", "create", "--title", title, "--description", "Body"],
    );
    let stdout = String::from_utf8_lossy(&stdout);
    stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn show_json(temp: &TempDir, id: &str) -> serde_json::Value {
    let output = jit(temp, &["issue", "show", id, "--json"]);
    serde_json::from_slice(&output).unwrap()
}

#[test]
fn test_show_root_has_short_id_matching_id_prefix() {
    let temp = TempDir::new().unwrap();
    jit(&temp, &["init"]);
    let id = create_issue(&temp, "Backlog issue");

    let json = show_json(&temp, &id);
    let short = json["short_id"]
        .as_str()
        .unwrap_or_else(|| panic!("short_id must be a string at root; got: {}", json));
    assert_eq!(short.len(), 8, "short_id must be 8 chars; got {:?}", short);
    assert_eq!(
        short,
        &id[0..8],
        "short_id must equal id[0..8]; got {:?} vs {:?}",
        short,
        &id[0..8]
    );
}

#[test]
fn test_show_labels_and_dependencies_are_empty_arrays_when_none() {
    let temp = TempDir::new().unwrap();
    jit(&temp, &["init"]);
    let id = create_issue(&temp, "Bare issue");

    // New issues may carry a default type label; strip it so we can assert the
    // genuinely-empty case still serializes as `[]` rather than being omitted.
    let labels: Vec<String> = show_json(&temp, &id)["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap().to_string())
        .collect();
    for label in &labels {
        jit(&temp, &["issue", "update", &id, "--remove-label", label]);
    }

    let json = show_json(&temp, &id);

    assert!(
        json["labels"].is_array(),
        "labels must be an array; got: {}",
        json["labels"]
    );
    assert_eq!(
        json["labels"].as_array().unwrap().len(),
        0,
        "labels must be an empty array when none set; got: {}",
        json["labels"]
    );

    assert!(
        json["dependencies"].is_array(),
        "dependencies must be an array; got: {}",
        json["dependencies"]
    );
    assert_eq!(
        json["dependencies"].as_array().unwrap().len(),
        0,
        "dependencies must be an empty array when none set; got: {}",
        json["dependencies"]
    );
}

#[test]
fn test_show_labels_and_dependencies_populated() {
    let temp = TempDir::new().unwrap();
    jit(&temp, &["init"]);
    let dep_id = create_issue(&temp, "Prerequisite");
    let id = create_issue(&temp, "Dependent issue");

    jit(&temp, &["issue", "update", &id, "--label", "type:task"]);
    jit(&temp, &["dep", "add", &id, &dep_id]);

    let json = show_json(&temp, &id);

    // Still arrays, now non-empty, and short_id still present.
    assert!(json["short_id"].is_string());
    let labels = json["labels"].as_array().expect("labels array");
    assert!(
        labels.iter().any(|l| l == "type:task"),
        "labels must contain added label; got: {}",
        json["labels"]
    );
    let deps = json["dependencies"].as_array().expect("dependencies array");
    assert_eq!(
        deps.len(),
        1,
        "dependencies must list the added dependency; got: {}",
        json["dependencies"]
    );
}

#[test]
fn test_show_short_id_present_across_states() {
    let temp = TempDir::new().unwrap();
    jit(&temp, &["init"]);
    let id = create_issue(&temp, "Lifecycle issue");

    // Backlog (just created).
    assert!(show_json(&temp, &id)["short_id"].is_string());

    // Move to in_progress and re-check the shape holds.
    jit(&temp, &["issue", "update", &id, "--state", "in_progress"]);
    let json = show_json(&temp, &id);
    assert_eq!(json["short_id"].as_str(), Some(&id[0..8]));
    assert!(json["labels"].is_array());
    assert!(json["dependencies"].is_array());
}
