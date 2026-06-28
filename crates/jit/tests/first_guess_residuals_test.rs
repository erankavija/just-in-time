//! Integration tests for REQ-05 first-guess command/flag residuals (jit:a1623187).
//!
//! Each test pins one residual from the issue's success criteria:
//!   1. `gate define --auto` records mode=auto.
//!   2. `doc add --title` is an alias for the human label (`--label`).
//!   3. A gate-inspection call with the gate key and issue id transposed yields an
//!      actionable did-you-mean (non-zero) rather than a silent misparse.
//!   4. A top-level reverse-deps / issue-listing first guess (`jit rdeps`,
//!      `jit list`) resolves to the canonical command.

use assert_cmd::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit() -> Command {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    jit()
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

/// Create an issue and return its short id (parsed from the "Created issue:" line).
fn create_issue(temp: &TempDir, title: &str) -> String {
    let output = jit()
        .current_dir(temp.path())
        .args(["issue", "create", "--title", title])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let s = String::from_utf8_lossy(&output);
    s.lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

// 1. `gate define --auto` sets mode to auto.
#[test]
fn test_gate_define_auto_sets_mode_auto() {
    let temp = setup_repo();

    jit()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "auto-gate",
            "--title",
            "Auto Gate",
            "--description",
            "Defined via --auto",
            "--auto",
            "--checker-command",
            "true",
        ])
        .assert()
        .success();

    let output = jit()
        .current_dir(temp.path())
        .args(["gate", "show", "auto-gate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["mode"], "auto", "expected --auto to record mode=auto");
}

// 2. `doc add --title X` sets the human label identically to `--label X`.
#[test]
fn test_doc_add_title_alias_sets_label() {
    let temp = setup_repo();
    let id = create_issue(&temp, "Doc host");
    fs::write(temp.path().join("design.md"), "# Design\n").unwrap();

    let output = jit()
        .current_dir(temp.path())
        .args([
            "doc",
            "add",
            &id,
            "design.md",
            "--title",
            "Design Notes",
            "--json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        json["document"]["label"], "Design Notes",
        "--title should populate the human label like --label"
    );
}

// 3. Transposed gate-inspection args yield an actionable did-you-mean (non-zero).
#[test]
fn test_gate_check_transposed_args_did_you_mean() {
    let temp = setup_repo();
    let id = create_issue(&temp, "Gated issue");
    jit()
        .current_dir(temp.path())
        .args([
            "gate",
            "define",
            "tests",
            "--title",
            "Tests",
            "--description",
            "Test gate",
        ])
        .assert()
        .success();

    // Canonical form is `jit gate check <issue> <gate-key>`. Pass them transposed.
    let assert = jit()
        .current_dir(temp.path())
        .args(["gate", "check", "tests", &id])
        .assert()
        .failure();
    let stderr = String::from_utf8_lossy(&assert.get_output().stderr).to_lowercase();
    assert!(
        stderr.contains("transposed") && stderr.contains("did you mean"),
        "expected an actionable did-you-mean about transposed args, got: {stderr}"
    );

    // The --json surface carries the same actionable suggestion and a non-zero exit.
    let json_out = jit()
        .current_dir(temp.path())
        .args(["gate", "check", "tests", &id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&json_out).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
    assert_eq!(json["error"]["details"]["transposed"], true);
    let suggestions = json["error"]["suggestions"].as_array().unwrap();
    assert!(
        suggestions.iter().any(|s| s
            .as_str()
            .unwrap()
            .contains(&format!("gate check {} tests", id))),
        "expected suggestion with the corrected argument order"
    );
}

// 4a. Top-level `jit rdeps` routes to the canonical `jit graph rdeps`.
#[test]
fn test_top_level_rdeps_routes_to_graph_rdeps() {
    let temp = setup_repo();
    let blocker = create_issue(&temp, "Blocker");
    let dependent = create_issue(&temp, "Dependent");
    // Dependent depends on blocker => blocker's rdeps includes dependent.
    jit()
        .current_dir(temp.path())
        .args(["dep", "add", &dependent, &blocker])
        .assert()
        .success();

    let via_alias = jit()
        .current_dir(temp.path())
        .args(["rdeps", &blocker, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let via_canonical = jit()
        .current_dir(temp.path())
        .args(["graph", "rdeps", &blocker, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let alias_json: serde_json::Value = serde_json::from_slice(&via_alias).unwrap();
    let canonical_json: serde_json::Value = serde_json::from_slice(&via_canonical).unwrap();
    assert_eq!(
        alias_json, canonical_json,
        "`jit rdeps` must resolve identically to `jit graph rdeps`"
    );
    // And it actually surfaces the dependent.
    assert!(
        String::from_utf8_lossy(&via_alias).contains(&dependent),
        "rdeps output should list the dependent issue"
    );
}

// 4b. Top-level `jit list` routes to the canonical `jit issue list`.
#[test]
fn test_top_level_list_routes_to_issue_list() {
    let temp = setup_repo();
    create_issue(&temp, "Alpha");
    create_issue(&temp, "Beta");

    let via_alias = jit()
        .current_dir(temp.path())
        .args(["list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let via_canonical = jit()
        .current_dir(temp.path())
        .args(["issue", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let alias_json: serde_json::Value = serde_json::from_slice(&via_alias).unwrap();
    let canonical_json: serde_json::Value = serde_json::from_slice(&via_canonical).unwrap();
    assert_eq!(
        alias_json, canonical_json,
        "`jit list` must resolve identically to `jit issue list`"
    );
}
