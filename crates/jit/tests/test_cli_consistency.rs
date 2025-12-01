//! TDD tests for CLI consistency improvements
//!
//! These tests define the expected behavior for consistent CLI:
//! 1. All commands support --json flag for machine-readable output
//! 2. Consistent argument order (ID first, then flags)
//! 3. Predictable output format

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> std::path::PathBuf {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
}

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .unwrap();
    temp
}

// ============================================================================
// Test: issue create --json
// ============================================================================

#[test]
fn test_issue_create_supports_json() {
    let repo = setup_repo();

    let output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test task", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "Command should succeed");

    // Should return valid JSON
    let json: Value = serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    // Should contain id field
    assert!(json["id"].is_string(), "Should have 'id' field");
    assert!(json["title"].is_string(), "Should have 'title' field");
    assert_eq!(json["title"].as_str().unwrap(), "Test task");
}

#[test]
fn test_issue_create_without_json_is_human_readable() {
    let repo = setup_repo();

    let output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test task"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain human-readable text (not JSON)
    assert!(stdout.contains("Created issue:"));

    // Should NOT be valid JSON
    assert!(serde_json::from_slice::<Value>(&output.stdout).is_err());
}

// ============================================================================
// Test: issue update --json
// ============================================================================

#[test]
fn test_issue_update_supports_json() {
    let repo = setup_repo();

    // Create issue first
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let create_json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let id = create_json["id"].as_str().unwrap();

    // Update with JSON output
    let output = Command::new(jit_binary())
        .args(["issue", "update", id, "--title", "Updated", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).expect("Output should be valid JSON");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert_eq!(json["title"].as_str().unwrap(), "Updated");
}

// ============================================================================
// Test: issue claim with consistent argument order
// ============================================================================

#[test]
fn test_issue_claim_takes_id_first() {
    let repo = setup_repo();

    // Create ready issue
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Task", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let id = json["id"].as_str().unwrap();

    // Set to ready
    Command::new(jit_binary())
        .args(["issue", "update", id, "--state", "ready"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    // Claim with ID first (NEW ORDER)
    let output = Command::new(jit_binary())
        .args(["issue", "claim", id, "--to", "agent:test", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Claim with ID-first order should work"
    );

    let json: Value = serde_json::from_slice(&output.stdout).expect("Should return JSON");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert_eq!(json["assignee"].as_str().unwrap(), "agent:test");
}

#[test]
fn test_issue_claim_supports_json() {
    let repo = setup_repo();

    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Task", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let id = json["id"].as_str().unwrap();

    Command::new(jit_binary())
        .args(["issue", "update", id, "--state", "ready"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    // Claim without --json should be human-readable
    let output_text = Command::new(jit_binary())
        .args(["issue", "claim", id, "--to", "agent:test"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output_text.stdout);
    assert!(stdout.contains("Claimed issue:") || stdout.contains("claimed"));
}

// ============================================================================
// Test: issue delete --json
// ============================================================================

#[test]
fn test_issue_delete_supports_json() {
    let repo = setup_repo();

    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Task", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let id = json["id"].as_str().unwrap();

    let output = Command::new(jit_binary())
        .args(["issue", "delete", id, "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).expect("Should return JSON");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert!(json["deleted"].as_bool().unwrap(), "deleted should be true");
}

// ============================================================================
// Test: issue release --json
// ============================================================================

#[test]
fn test_issue_release_supports_json() {
    let repo = setup_repo();

    // Create and claim issue
    let create_output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Task", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&create_output.stdout).unwrap();
    let id = json["id"].as_str().unwrap();

    Command::new(jit_binary())
        .args(["issue", "update", id, "--state", "ready"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    Command::new(jit_binary())
        .args(["issue", "claim", id, "--to", "agent:test"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    // Release with JSON
    let output = Command::new(jit_binary())
        .args(["issue", "release", id, "--reason", "Test", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).expect("Should return JSON");

    assert_eq!(json["id"].as_str().unwrap(), id);
    assert!(json["assignee"].is_null(), "Assignee should be cleared");
}

// ============================================================================
// Test: Consistent JSON structure
// ============================================================================

#[test]
fn test_json_output_has_consistent_structure() {
    let repo = setup_repo();

    // All mutation commands should return similar structure with at least:
    // - id field
    // - relevant issue data

    let output = Command::new(jit_binary())
        .args(["issue", "create", "-t", "Test", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();

    // Check for expected fields
    assert!(json.is_object(), "Root should be an object");
    assert!(json["id"].is_string(), "Should have id");
    assert!(json["title"].is_string(), "Should have title");
    assert!(json["state"].is_string(), "Should have state");
    assert!(json["priority"].is_string(), "Should have priority");
}
