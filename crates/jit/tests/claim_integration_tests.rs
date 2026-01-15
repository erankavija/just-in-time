//! Integration tests for claim CLI commands
//!
//! Tests the full claim workflow: acquire → list → release
//! Verifies actual binary execution, exit codes, and output formats.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Setup a test repository with git and jit initialized
fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize git (required for worktree detection)
    Command::new("git")
        .current_dir(temp.path())
        .args(["init"])
        .status()
        .unwrap();

    Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.email", "test@example.com"])
        .status()
        .unwrap();

    Command::new("git")
        .current_dir(temp.path())
        .args(["config", "user.name", "Test User"])
        .status()
        .unwrap();

    // Initialize jit
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();

    // Initialize control plane (for claims)
    let git_dir = temp.path().join(".git");
    fs::create_dir_all(git_dir.join("jit/locks")).unwrap();
    fs::write(git_dir.join("jit/claims.jsonl"), "").unwrap();

    // Create initial commit (required for claims)
    fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
    Command::new("git")
        .current_dir(temp.path())
        .args(["add", "."])
        .status()
        .unwrap();
    Command::new("git")
        .current_dir(temp.path())
        .args(["commit", "-m", "Initial commit"])
        .status()
        .unwrap();

    temp
}

/// Create a test issue and return its ID
fn create_issue(repo_path: &Path, title: &str) -> String {
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo_path)
        .args(["issue", "create", "--title", title, "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    json["data"]["id"].as_str().unwrap().to_string()
}

#[test]
fn test_claim_acquire_happy_path() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("Acquired lease"));
}

#[test]
fn test_claim_acquire_json_output() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim with JSON output
    let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["success"], true);
    assert!(json["data"]["lease_id"].is_string());
    assert_eq!(json["data"]["issue_id"], issue_id);
    assert_eq!(json["data"]["ttl_secs"], 600);
}

#[test]
fn test_claim_acquire_already_claimed_error() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // First claim succeeds
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    // Second claim should fail
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-2",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already claimed"));
}

#[test]
fn test_claim_list_shows_active_leases() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire a claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    // List should show the lease
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&issue_id))
        .stdout(predicate::str::contains("agent:test-1"));
}

#[test]
fn test_claim_list_json_output() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire a claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["data"]["lease_id"].as_str().unwrap();

    // List claims with JSON
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    assert!(list_output.status.success());

    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    assert_eq!(list_json["success"], true);
    assert!(list_json["data"]["leases"].is_array());

    let leases = list_json["data"]["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);
    assert_eq!(leases[0]["issue_id"], issue_id);
}

#[test]
fn test_claim_release_happy_path() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
            "--json",
        ])
        .output()
        .unwrap();

    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["data"]["lease_id"].as_str().unwrap();

    // Release claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "release", lease_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Released lease"));
}

#[test]
fn test_claim_release_not_found_error() {
    let temp = setup_repo();

    // Try to release non-existent lease
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test")
        .args(["claim", "release", "01FAKE0000000000000000000"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Lease").and(
            predicate::str::contains("not found").or(predicate::str::contains("No active lease")),
        ));
}

#[test]
fn test_claim_workflow_end_to_end() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue for Workflow");

    // Step 1: Acquire claim
    let acquire_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:workflow-test",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(acquire_output.status.success());
    let acquire_json: Value = serde_json::from_slice(&acquire_output.stdout).unwrap();
    let lease_id = acquire_json["data"]["lease_id"].as_str().unwrap();

    // Step 2: List claims and verify
    let list_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    assert!(list_output.status.success());
    let list_json: Value = serde_json::from_slice(&list_output.stdout).unwrap();
    let leases = list_json["data"]["leases"].as_array().unwrap();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0]["lease_id"], lease_id);

    // Step 3: Release claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:workflow-test")
        .args(["claim", "release", lease_id])
        .assert()
        .success();

    // Step 4: Verify lease is gone
    let list_after_output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args(["claim", "list", "--json"])
        .output()
        .unwrap();

    let list_after_json: Value = serde_json::from_slice(&list_after_output.stdout).unwrap();
    let leases_after = list_after_json["data"]["leases"].as_array().unwrap();
    assert_eq!(
        leases_after.len(),
        0,
        "Lease should be removed after release"
    );
}

#[test]
fn test_claim_status_shows_lease_details() {
    let temp = setup_repo();
    let issue_id = create_issue(temp.path(), "Test Issue");

    // Acquire claim
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            &issue_id,
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .success();

    // Check status
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .env("JIT_AGENT_ID", "agent:test-1")
        .args(["claim", "status"])
        .assert()
        .success()
        .stdout(predicate::str::contains(&issue_id))
        .stdout(predicate::str::contains("agent:test-1"));
}

#[test]
fn test_claim_acquire_nonexistent_issue_error() {
    let temp = setup_repo();

    // Try to claim non-existent issue
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .args([
            "claim",
            "acquire",
            "00000000-0000-0000-0000-000000000000",
            "--ttl",
            "600",
            "--agent-id",
            "agent:test-1",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not found").or(predicate::str::contains("Issue")));
}
