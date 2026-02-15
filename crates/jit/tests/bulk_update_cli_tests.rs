//! Integration tests for bulk update CLI functionality
//!
//! Tests the `jit issue update` command with --filter flag for batch operations

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
    let jit = jit_binary();

    let output = Command::new(&jit)
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");

    assert!(
        output.status.success(),
        "jit init failed:\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    temp
}

fn create_issue(temp: &TempDir, title: &str, labels: &[&str]) -> String {
    let jit = jit_binary();
    let mut args = vec!["issue", "create", "-t", title];

    for label in labels {
        args.push("--label");
        args.push(label);
    }

    let output = Command::new(&jit)
        .args(&args)
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "Failed to create issue");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Extract ID from "Created issue: <ID>"
    stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .and_then(|l| l.split_whitespace().last())
        .unwrap()
        .to_string()
}

#[test]
fn test_bulk_update_requires_id_or_filter() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // No ID and no --filter should fail
    let output = Command::new(&jit)
        .args(["issue", "update", "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ID") && stderr.contains("filter"),
        "Should require either ID or --filter"
    );
}

#[test]
fn test_bulk_update_rejects_both_id_and_filter() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    create_issue(&temp, "Test", &["type:task"]);

    // Both ID and --filter should fail
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "abc123",
            "--filter",
            "state:ready",
            "--state",
            "done",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("mutually exclusive"),
        "Should reject both ID and --filter, got: {}",
        stderr
    );
}

#[test]
fn test_bulk_update_add_labels() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create multiple issues with type:task
    create_issue(&temp, "Task 1", &["type:task"]);
    create_issue(&temp, "Task 2", &["type:task"]);
    create_issue(&temp, "Epic 1", &["type:epic"]);

    // Add milestone label to all tasks
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--label",
            "milestone:v1.0",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON response
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["success"].as_bool().unwrap());

    let summary = &json["data"]["summary"];
    assert_eq!(summary["total_matched"].as_u64().unwrap(), 2); // 2 tasks
    assert_eq!(summary["total_modified"].as_u64().unwrap(), 2);
}

#[test]
fn test_bulk_update_state_transition() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create ready issues
    let id1 = create_issue(&temp, "Task 1", &["type:task"]);
    let id2 = create_issue(&temp, "Task 2", &["type:task"]);

    // Transition to ready first
    Command::new(&jit)
        .args(["issue", "update", &id1, "--state", "ready"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "update", &id2, "--state", "ready"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Bulk update to in_progress
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "state:ready",
            "--state",
            "in_progress",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(
        json["data"]["summary"]["total_modified"].as_u64().unwrap(),
        2
    );
}

#[test]
fn test_bulk_update_assignee() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create unassigned issues
    create_issue(&temp, "Task 1", &["type:task"]);
    create_issue(&temp, "Task 2", &["type:task"]);

    // Assign all tasks
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--assignee",
            "agent:worker-1",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(
        json["data"]["summary"]["total_modified"].as_u64().unwrap(),
        2
    );
}

#[test]
fn test_bulk_update_validation_errors() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create an issue with gates
    let id = create_issue(&temp, "Task with gates", &["type:task"]);

    // Define a gate
    Command::new(&jit)
        .args([
            "gate",
            "define",
            "tests",
            "--title",
            "Run tests",
            "--description",
            "Test gate",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Add gate to issue
    Command::new(&jit)
        .args(["gate", "add", &id, "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Try to transition to done without passing gates
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--state",
            "done",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success()); // Command succeeds but reports errors
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should have errors for gate requirement
    assert!(json["data"]["summary"]["total_errors"].as_u64().unwrap() > 0);
}

#[test]
fn test_bulk_update_complex_query() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create diverse issues
    create_issue(&temp, "High priority task", &["type:task", "priority:high"]);
    create_issue(&temp, "Normal priority task", &["type:task"]);
    create_issue(&temp, "High priority epic", &["type:epic", "priority:high"]);

    // Update only high-priority tasks using AND query
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:type:task AND label:priority:high",
            "--label",
            "milestone:v1.0",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should match only 1 issue (high-priority task, not epic)
    assert_eq!(
        json["data"]["summary"]["total_matched"].as_u64().unwrap(),
        1
    );
    assert_eq!(
        json["data"]["summary"]["total_modified"].as_u64().unwrap(),
        1
    );
}

#[test]
fn test_bulk_update_remove_labels() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with old label
    create_issue(&temp, "Task 1", &["type:task", "milestone:v0.9"]);
    create_issue(&temp, "Task 2", &["type:task", "milestone:v0.9"]);

    // Remove old milestone label
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:milestone:v0.9",
            "--remove-label",
            "milestone:v0.9",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(
        json["data"]["summary"]["total_modified"].as_u64().unwrap(),
        2
    );
}

#[test]
fn test_bulk_update_no_matches() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    create_issue(&temp, "Task 1", &["type:task"]);

    // Filter that matches nothing
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:nonexistent:value",
            "--state",
            "done",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(
        json["data"]["summary"]["total_matched"].as_u64().unwrap(),
        0
    );
    assert_eq!(
        json["data"]["summary"]["total_modified"].as_u64().unwrap(),
        0
    );
}
