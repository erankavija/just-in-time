//! Integration tests for `jit doc show` command
//!
//! Tests for:
//! - doc show without git (filesystem fallback)
//! - doc show with git
//! - doc show with --at commit

use assert_cmd::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_doc_show_without_git() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path();

    // Initialize jit (without git)
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .arg("init")
        .assert()
        .success();

    // Create an issue
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["issue", "create", "--title", "Test Issue", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = json["data"]["id"].as_str().unwrap();
    let short_id = &issue_id[..8];

    // Create a document file
    let doc_path = temp_path.join("design.md");
    fs::write(&doc_path, "# Design Document\n\nThis is a test document.").unwrap();

    // Add document reference to issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args([
            "doc",
            "add",
            short_id,
            "design.md",
            "--label",
            "Design Doc",
            "--doc-type",
            "design",
        ])
        .assert()
        .success();

    // Show document without git - should read from filesystem
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["doc", "show", short_id, "design.md"])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "doc show should work without git: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Design Document"));
    assert!(stdout.contains("This is a test document"));
}

#[test]
fn test_doc_show_with_git() {
    let temp = TempDir::new().unwrap();
    let temp_path = temp.path();

    // Initialize git repo
    Command::new("git")
        .current_dir(temp_path)
        .args(["init"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(temp_path)
        .args(["config", "user.name", "Test User"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(temp_path)
        .args(["config", "user.email", "test@example.com"])
        .assert()
        .success();

    // Initialize jit
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .arg("init")
        .assert()
        .success();

    // Create an issue
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["issue", "create", "--title", "Test Issue", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = json["data"]["id"].as_str().unwrap();
    let short_id = &issue_id[..8];

    // Create and commit a document
    let doc_path = temp_path.join("design.md");
    fs::write(&doc_path, "# Design Document\n\nCommitted content.").unwrap();

    Command::new("git")
        .current_dir(temp_path)
        .args(["add", "design.md"])
        .assert()
        .success();

    Command::new("git")
        .current_dir(temp_path)
        .args(["commit", "-m", "Add design doc"])
        .assert()
        .success();

    // Add document reference
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["doc", "add", short_id, "design.md"])
        .assert()
        .success();

    // Show document - should read from git
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp_path)
        .args(["doc", "show", short_id, "design.md"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("# Design Document"));
    assert!(stdout.contains("Committed content"));
}
