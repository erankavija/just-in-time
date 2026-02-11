//! TDD tests for `jit issue search` JSON output compaction
//!
//! Tests verify that search returns compact results by default (MinimalIssue)
//! and full results only when --full flag is provided.

use serde_json::Value;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest_dir)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("target/debug/jit")
        .to_string_lossy()
        .to_string()
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();

    // Initialize jit repo properly
    let jit = jit_binary();
    Command::new(&jit)
        .current_dir(temp.path())
        .arg("init")
        .output()
        .unwrap();

    // Create first test issue via CLI (ensures proper indexing)
    Command::new(&jit)
        .current_dir(temp.path())
        .args([
            "issue", "create",
            "--title", "Test authentication feature",
            "--description", "This is a detailed description about implementing authentication with JWT tokens and middleware.",
            "--priority", "high",
            "--label", "type:task",
            "--label", "epic:auth",
            "--label", "component:api",
        ])
        .output()
        .unwrap();

    // Create second test issue
    Command::new(&jit)
        .current_dir(temp.path())
        .args([
            "issue",
            "create",
            "--title",
            "Add authentication middleware",
            "--description",
            "Implement middleware for JWT validation",
            "--label",
            "type:task",
            "--label",
            "epic:auth",
        ])
        .output()
        .unwrap();

    temp
}

#[test]
fn test_search_returns_compact_by_default() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .current_dir(temp.path())
        .arg("issue")
        .arg("search")
        .arg("authentication")
        .arg("--json")
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &json["data"];
    let issues = data["issues"].as_array().unwrap();

    assert!(!issues.is_empty(), "Should find authentication issues");

    // Check first issue has ONLY compact fields
    let first_issue = &issues[0];

    // Should have these fields (MinimalIssue)
    assert!(first_issue.get("id").is_some());
    assert!(first_issue.get("short_id").is_some());
    assert!(first_issue.get("title").is_some());
    assert!(first_issue.get("state").is_some());
    assert!(first_issue.get("priority").is_some());
    assert!(first_issue.get("assignee").is_some());
    assert!(first_issue.get("labels").is_some());

    // Should NOT have these fields (full Issue)
    assert!(
        first_issue.get("description").is_none(),
        "Compact mode should not include description"
    );
    assert!(
        first_issue.get("dependencies").is_none(),
        "Compact mode should not include dependencies"
    );
    assert!(
        first_issue.get("gates_required").is_none(),
        "Compact mode should not include gates_required"
    );
    assert!(
        first_issue.get("gates_status").is_none(),
        "Compact mode should not include gates_status"
    );
    assert!(
        first_issue.get("documents").is_none(),
        "Compact mode should not include documents"
    );
    assert!(
        first_issue.get("context").is_none(),
        "Compact mode should not include context"
    );
}

#[test]
fn test_search_returns_full_with_flag() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .current_dir(temp.path())
        .arg("issue")
        .arg("search")
        .arg("authentication")
        .arg("--json")
        .arg("--full")
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &json["data"];
    let issues = data["issues"].as_array().unwrap();

    assert!(!issues.is_empty(), "Should find authentication issues");

    // Check first issue has ALL fields
    let first_issue = &issues[0];

    // Should have all Issue fields
    assert!(first_issue.get("id").is_some());
    assert!(first_issue.get("title").is_some());
    assert!(
        first_issue.get("description").is_some(),
        "--full should include description"
    );
    assert!(first_issue.get("state").is_some());
    assert!(first_issue.get("priority").is_some());
    assert!(first_issue.get("assignee").is_some());
    assert!(first_issue.get("labels").is_some());
    assert!(
        first_issue.get("dependencies").is_some(),
        "--full should include dependencies"
    );
    assert!(
        first_issue.get("gates_required").is_some(),
        "--full should include gates_required"
    );
    assert!(
        first_issue.get("gates_status").is_some(),
        "--full should include gates_status"
    );
    assert!(
        first_issue.get("documents").is_some(),
        "--full should include documents"
    );
    assert!(
        first_issue.get("context").is_some(),
        "--full should include context"
    );
}

#[test]
fn test_search_compact_has_short_id() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .current_dir(temp.path())
        .arg("issue")
        .arg("search")
        .arg("authentication")
        .arg("--json")
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &json["data"];
    let issues = data["issues"].as_array().unwrap();

    assert!(!issues.is_empty());

    let first_issue = &issues[0];
    let short_id = first_issue["short_id"].as_str().unwrap();
    let full_id = first_issue["id"].as_str().unwrap();

    // short_id should be first 8 chars of full ID
    assert_eq!(short_id.len(), 8);
    assert!(full_id.starts_with(short_id));
}

#[test]
fn test_search_compact_preserves_count() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .current_dir(temp.path())
        .arg("issue")
        .arg("search")
        .arg("authentication")
        .arg("--json")
        .output()
        .unwrap();

    assert!(output.status.success());

    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let data = &json["data"];
    let issues = data["issues"].as_array().unwrap();
    let count = data["count"].as_u64().unwrap();

    // Count should match actual array length
    assert_eq!(count, issues.len() as u64);
    assert_eq!(count, 2); // We created 2 issues with "authentication"
}
