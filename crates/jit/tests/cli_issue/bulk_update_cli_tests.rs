//! Integration tests for bulk update CLI functionality
//!
//! Tests the `jit issue update` command with --filter flag for batch operations

use jit::test_cli::JitCli;

#[test]
fn test_bulk_update_requires_id_or_filter() {
    let temp = JitCli::initialised().unwrap();

    // No ID and no --filter should fail
    let output = temp
        .command(["issue", "update", "--state", "done"])
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
    let temp = JitCli::initialised().unwrap();

    temp.create_issue(["-t", "Test", "--label", "type:task"])
        .unwrap();

    // Both ID and --filter should fail
    let output = temp
        .command([
            "issue",
            "update",
            "abc123",
            "--filter",
            "state:ready",
            "--state",
            "done",
        ])
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
    let temp = JitCli::initialised().unwrap();

    // Create multiple issues with type:task
    temp.create_issue(["-t", "Task 1", "--label", "type:task"])
        .unwrap();
    temp.create_issue(["-t", "Task 2", "--label", "type:task"])
        .unwrap();
    temp.create_issue(["-t", "Epic 1", "--label", "type:epic"])
        .unwrap();

    // Add milestone label to all tasks
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--label",
            "milestone:v1.0",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Parse JSON response
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // success field removed

    let summary = &json["summary"];
    assert_eq!(summary["total_matched"].as_u64().unwrap(), 2); // 2 tasks
    assert_eq!(summary["total_modified"].as_u64().unwrap(), 2);
}

#[test]
fn test_bulk_update_state_transition() {
    let temp = JitCli::initialised().unwrap();

    // Create ready issues
    let id1 = temp
        .create_issue(["-t", "Task 1", "--label", "type:task"])
        .unwrap();
    let id2 = temp
        .create_issue(["-t", "Task 2", "--label", "type:task"])
        .unwrap();

    // Transition to ready first
    temp.command(["issue", "update", &id1, "--state", "ready"])
        .output()
        .unwrap();

    temp.command(["issue", "update", &id2, "--state", "ready"])
        .output()
        .unwrap();

    // Bulk update to in_progress
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "state:ready",
            "--state",
            "in_progress",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["summary"]["total_modified"].as_u64().unwrap(), 2);
}

#[test]
fn test_bulk_update_assignee() {
    let temp = JitCli::initialised().unwrap();

    // Create unassigned issues
    temp.create_issue(["-t", "Task 1", "--label", "type:task"])
        .unwrap();
    temp.create_issue(["-t", "Task 2", "--label", "type:task"])
        .unwrap();

    // Assign all tasks
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--assignee",
            "agent:worker-1",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["summary"]["total_modified"].as_u64().unwrap(), 2);
}

#[test]
fn test_bulk_update_validation_errors() {
    let temp = JitCli::initialised().unwrap();

    // Create an issue with gates
    let id = temp
        .create_issue(["-t", "Task with gates", "--label", "type:task"])
        .unwrap();

    // Define a gate
    temp.command([
        "gate",
        "define",
        "tests",
        "--title",
        "Run tests",
        "--description",
        "Test gate",
    ])
    .output()
    .unwrap();

    // Add gate to issue
    temp.command(["gate", "add", &id, "tests"])
        .output()
        .unwrap();

    // Try to transition to done without passing gates
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--state",
            "done",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success()); // Command succeeds but reports errors
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should have errors for gate requirement
    assert!(json["summary"]["total_errors"].as_u64().unwrap() > 0);
}

#[test]
fn test_bulk_update_complex_query() {
    let temp = JitCli::initialised().unwrap();

    // Create diverse issues
    temp.create_issue([
        "-t",
        "High priority task",
        "--label",
        "type:task",
        "--label",
        "priority:high",
    ])
    .unwrap();
    temp.create_issue(["-t", "Normal priority task", "--label", "type:task"])
        .unwrap();
    temp.create_issue([
        "-t",
        "High priority epic",
        "--label",
        "type:epic",
        "--label",
        "priority:high",
    ])
    .unwrap();

    // Update only high-priority tasks using AND query
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:type:task AND label:priority:high",
            "--label",
            "milestone:v1.0",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should match only 1 issue (high-priority task, not epic)
    assert_eq!(json["summary"]["total_matched"].as_u64().unwrap(), 1);
    assert_eq!(json["summary"]["total_modified"].as_u64().unwrap(), 1);
}

#[test]
fn test_bulk_update_remove_labels() {
    let temp = JitCli::initialised().unwrap();

    // Create issues with old label
    temp.create_issue([
        "-t",
        "Task 1",
        "--label",
        "type:task",
        "--label",
        "milestone:v0.9",
    ])
    .unwrap();
    temp.create_issue([
        "-t",
        "Task 2",
        "--label",
        "type:task",
        "--label",
        "milestone:v0.9",
    ])
    .unwrap();

    // Remove old milestone label
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:milestone:v0.9",
            "--remove-label",
            "milestone:v0.9",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["summary"]["total_modified"].as_u64().unwrap(), 2);
}

#[test]
fn test_bulk_update_no_matches() {
    let temp = JitCli::initialised().unwrap();

    temp.create_issue(["-t", "Task 1", "--label", "type:task"])
        .unwrap();

    // Filter that matches nothing
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "label:nonexistent:value",
            "--state",
            "done",
            "--json",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["summary"]["total_matched"].as_u64().unwrap(), 0);
    assert_eq!(json["summary"]["total_modified"].as_u64().unwrap(), 0);
}

#[test]
fn test_bulk_update_invalid_non_ascii_filter_errors_without_panic() {
    let temp = JitCli::initialised().unwrap();

    for filter in ["ä:x", "label:ä state:", "é"] {
        let output = temp
            .command(["issue", "update", "--filter", filter, "--priority", "low"])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            matches!(output.status.code(), Some(code) if code != 0 && code != 101),
            "filter {filter:?} must fail with an error status, got {:?}; stderr: {stderr}",
            output.status
        );
        assert!(
            !stderr.contains("panicked") && !stderr.trim().is_empty(),
            "filter {filter:?} must report an error message, not a panic; stderr: {stderr}"
        );
    }
}
