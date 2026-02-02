//! TDD tests for jit-dispatch orchestrator
//!
//! These tests define the expected behavior of the dispatch orchestrator:
//! 1. Poll jit for ready issues
//! 2. Assign issues to available agents
//! 3. Track agent status
//! 4. Handle stalled work

use std::path::Path;
use tempfile::TempDir;

/// Test helper: Get path to jit binary (built in workspace)
fn jit_binary() -> std::path::PathBuf {
    // Assume we're in workspace, jit binary is in target/debug/
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
}

/// Test helper: Initialize a jit repository
fn init_jit_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize jit repository
    let status = std::process::Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .status()
        .unwrap();

    assert!(status.success(), "Failed to initialize jit repository");
    temp
}

/// Test helper: Create a ready issue in jit
fn create_ready_issue(repo_path: &Path, title: &str) -> String {
    // Create issue
    let output = std::process::Command::new(jit_binary())
        .args(["issue", "create", "-t", title])
        .current_dir(repo_path)
        .output()
        .unwrap();

    if !output.status.success() {
        eprintln!("Create issue failed:");
        eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
        panic!("Failed to create issue");
    }

    // Parse ID from output (first line contains "Created issue: <id>")
    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout
        .lines()
        .find(|line| line.contains("Created issue:"))
        .and_then(|line| line.split_whitespace().last())
        .expect("Failed to parse issue ID")
        .to_string();

    // Set to ready state
    let status = std::process::Command::new(jit_binary())
        .args(["issue", "update", &id, "--state", "ready"])
        .current_dir(repo_path)
        .status()
        .unwrap();

    assert!(status.success(), "Failed to update issue to ready");
    id
}

/// Test helper: Query available issues from jit
fn query_ready_issues(repo_path: &Path) -> Vec<serde_json::Value> {
    let output = std::process::Command::new(jit_binary())
        .args(["query", "available", "--json"])
        .current_dir(repo_path)
        .output()
        .unwrap();

    assert!(output.status.success(), "Failed to query available issues");

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["data"]["issues"].as_array().unwrap().to_vec()
}

#[test]
fn test_dispatch_can_query_jit_repository() {
    let repo = init_jit_repo();

    // Create some ready issues
    create_ready_issue(repo.path(), "Task 1");
    create_ready_issue(repo.path(), "Task 2");

    // Query should find them
    let ready = query_ready_issues(repo.path());
    assert_eq!(ready.len(), 2);
}

#[test]
fn test_dispatch_claims_issues_for_agents() {
    let repo = init_jit_repo();
    let id = create_ready_issue(repo.path(), "Test task");

    // Simulate dispatch claiming issue for agent
    let status = std::process::Command::new(jit_binary())
        .args(["issue", "claim", &id, "agent:test-worker"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    assert!(status.success(), "Failed to claim issue");

    // Query should now show 0 ready issues
    let ready = query_ready_issues(repo.path());
    assert_eq!(ready.len(), 0);

    // Query by assignee should show 1
    let output = std::process::Command::new(jit_binary())
        .args(["query", "all", "--assignee", "agent:test-worker", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["issues"].as_array().unwrap().len(), 1);
}

#[test]
fn test_dispatch_respects_priority() {
    let repo = init_jit_repo();

    // Create issues with different priorities
    let _low_id = create_ready_issue(repo.path(), "Low priority");
    let _high_id = create_ready_issue(repo.path(), "High priority");

    // Set priorities (low already defaults to normal)
    let high_id = _high_id;
    std::process::Command::new(jit_binary())
        .args(["issue", "update", &high_id, "--priority", "high"])
        .current_dir(repo.path())
        .status()
        .unwrap();

    // Query high priority should return 1
    let output = std::process::Command::new(jit_binary())
        .args(["query", "all", "--priority", "high", "--json"])
        .current_dir(repo.path())
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["data"]["issues"].as_array().unwrap().len(), 1);
}

// TODO: Add tests for:
// - Config file loading
// - Agent pool management
// - Periodic polling
// - Stalled work detection
// - Multiple concurrent agents
