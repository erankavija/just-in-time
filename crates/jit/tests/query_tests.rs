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

#[allow(dead_code)]
fn jit_binary_old() -> String {
    let mut path = std::env::current_exe().unwrap();
    path.pop();
    if path.ends_with("deps") {
        path.pop();
    }
    path.push("jit");
    path.to_str().unwrap().to_string()
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let jit = jit_binary();
    Command::new(&jit)
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

#[test]
fn test_query_ready_returns_unblocked_issues() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create multiple issues with different states
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 1", "-d", "Ready task"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let _id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 2", "-d", "In progress task"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Create task 3 with a gate to keep it in Open state
    Command::new(&jit)
        .args([
            "registry",
            "add",
            "test-gate",
            "--title",
            "Test",
            "--desc",
            "Test",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output3 = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Task 3",
            "-d",
            "Blocked task",
            "--gate",
            "test-gate",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let _id3 = String::from_utf8_lossy(&output3.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // id1 is auto-ready (no blockers), id2 move to in_progress, id3 is also ready (gates don't block Ready)
    Command::new(&jit)
        .args(["issue", "update", &id2, "--state", "in_progress"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query ready issues
    let output = Command::new(&jit)
        .args(["query", "available", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should return two ready issues (id1 and id3, gates don't block Ready state)
    assert_eq!(json["count"], 2);
}

#[test]
fn test_query_ready_excludes_assigned_issues() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Add gate for blocking
    Command::new(&jit)
        .args([
            "registry", "add", "block", "--title", "Block", "--desc", "Block",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let jit = jit_binary();

    // Create two ready issues
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 1", "-d", "Unassigned"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 2", "-d", "Assigned"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Both auto-transition to ready since no blockers

    // Claim id2
    Command::new(&jit)
        .args(["issue", "claim", &id2, "agent:worker-1"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query ready issues
    let output = Command::new(&jit)
        .args(["query", "available", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should return only unassigned ready issue
    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id1);
}

#[test]
fn test_query_blocked_returns_issues_with_reasons() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create parent and child issues
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Parent", "-d", "Dependency"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let parent_id = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Child", "-d", "Depends on parent"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let child_id = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Add dependency
    Command::new(&jit)
        .args(["dep", "add", &child_id, &parent_id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query blocked issues
    let output = Command::new(&jit)
        .args(["query", "blocked", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Child should be blocked
    assert!(json["count"].as_u64().unwrap() >= 1);
    let child_issue = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["id"] == child_id)
        .unwrap();

    assert!(!child_issue["blocked_reasons"]
        .as_array()
        .unwrap()
        .is_empty());
    // blocked_reasons are now strings like "dependency:abc123 (Title:State)"
    let first_reason = child_issue["blocked_reasons"][0].as_str().unwrap();
    assert!(first_reason.starts_with("dependency:"));
}

#[test]
fn test_query_by_assignee() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with different assignees
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 1", "-d", "For worker 1"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 2", "-d", "For worker 2"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Assign to different agents
    Command::new(&jit)
        .args(["issue", "claim", &id1, "agent:worker-1"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "claim", &id2, "agent:worker-2"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query by assignee (use --full to get assignee field in response)
    let output = Command::new(&jit)
        .args([
            "query",
            "all",
            "--assignee",
            "agent:worker-1",
            "--full",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id1);
    assert_eq!(json["issues"][0]["assignee"], "agent:worker-1");
}

#[test]
fn test_issue_release() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create and assign issue
    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "-d", "Test release"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    Command::new(&jit)
        .args(["issue", "claim", &id, "agent:worker-1"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Release the issue
    let output = Command::new(&jit)
        .args(["issue", "release", &id, "timeout"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    // Verify issue is unassigned
    let output = Command::new(&jit)
        .args(["issue", "show", &id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(json["assignee"].is_null());
}

#[test]
fn test_query_by_state() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with different states
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 1", "-d", "Open"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let _id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task 2", "-d", "Done"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    Command::new(&jit)
        .args(["issue", "update", &id2, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query by state
    let output = Command::new(&jit)
        .args(["query", "all", "--state", "done", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id2);
    assert_eq!(json["issues"][0]["state"], "done");
}

#[test]
fn test_query_by_priority() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with different priorities
    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Critical",
            "-d",
            "Urgent",
            "--priority",
            "critical",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Low",
            "-d",
            "Not urgent",
            "--priority",
            "low",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query by priority
    let output = Command::new(&jit)
        .args(["query", "all", "--priority", "critical", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["priority"], "critical");
}

#[test]
fn test_query_closed_returns_done_and_rejected() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with different states
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Completed", "-d", "Done task"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Rejected", "-d", "Won't do"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output3 = Command::new(&jit)
        .args(["issue", "create", "-t", "Ready", "-d", "Still open"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let _id3 = String::from_utf8_lossy(&output3.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Set states
    Command::new(&jit)
        .args(["issue", "update", &id1, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "reject", &id2])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query closed issues
    let output = Command::new(&jit)
        .args(["query", "closed", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should return both Done and Rejected
    assert_eq!(json["count"], 2);

    let issue_ids: Vec<String> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap().to_string())
        .collect();

    assert!(issue_ids.contains(&id1));
    assert!(issue_ids.contains(&id2));
}

#[test]
fn test_query_available_sorts_by_priority() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with different priorities (in reverse order)
    let output_low = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Low Priority Task",
            "-d",
            "Should be last",
            "--priority",
            "low",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_low = String::from_utf8_lossy(&output_low.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output_normal = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Normal Priority Task",
            "-d",
            "Should be third",
            "--priority",
            "normal",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_normal = String::from_utf8_lossy(&output_normal.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output_high = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "High Priority Task",
            "-d",
            "Should be second",
            "--priority",
            "high",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_high = String::from_utf8_lossy(&output_high.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output_critical = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Critical Priority Task",
            "-d",
            "Should be first",
            "--priority",
            "critical",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_critical = String::from_utf8_lossy(&output_critical.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Query available issues with JSON output
    let output = Command::new(&jit)
        .args(["query", "available", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Should return all 4 issues
    assert_eq!(json["count"], 4);
    let issues = json["issues"].as_array().unwrap();

    // Verify they are sorted by priority: Critical > High > Normal > Low
    assert_eq!(issues[0]["id"], id_critical);
    assert_eq!(issues[0]["priority"], "critical");

    assert_eq!(issues[1]["id"], id_high);
    assert_eq!(issues[1]["priority"], "high");

    assert_eq!(issues[2]["id"], id_normal);
    assert_eq!(issues[2]["priority"], "normal");

    assert_eq!(issues[3]["id"], id_low);
    assert_eq!(issues[3]["priority"], "low");
}

// ── query_all: combined filters ───────────────────────────────────────────────

#[test]
fn test_query_all_no_filters_returns_all() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    for title in &["Task A", "Task B", "Task C"] {
        Command::new(&jit)
            .args(["issue", "create", "-t", title, "-d", "desc"])
            .current_dir(temp.path())
            .output()
            .unwrap();
    }

    let output = Command::new(&jit)
        .args(["query", "all", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 3);
}

#[test]
fn test_query_all_combined_state_and_priority() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // high + done
    let out = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "High Done",
            "-d",
            "d",
            "--priority",
            "high",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_high_done = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // high + ready (should not appear)
    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "High Ready",
            "-d",
            "d",
            "--priority",
            "high",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // low + done (should not appear)
    let out = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Low Done",
            "-d",
            "d",
            "--priority",
            "low",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_low_done = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    Command::new(&jit)
        .args(["issue", "update", &id_high_done, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    Command::new(&jit)
        .args(["issue", "update", &id_low_done, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args([
            "query",
            "all",
            "--state",
            "done",
            "--priority",
            "high",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id_high_done);
}

#[test]
fn test_query_all_combined_state_and_assignee() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let out = Command::new(&jit)
        .args(["issue", "create", "-t", "Task A", "-d", "d"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_a = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let out = Command::new(&jit)
        .args(["issue", "create", "-t", "Task B", "-d", "d"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_b = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Assign both to the same agent without changing state
    for id in &[&id_a, &id_b] {
        Command::new(&jit)
            .args(["issue", "update", id, "--assignee", "agent:worker-1"])
            .current_dir(temp.path())
            .output()
            .unwrap();
    }

    // Move only id_a to in_progress
    Command::new(&jit)
        .args(["issue", "update", &id_a, "--state", "in_progress"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args([
            "query",
            "all",
            "--state",
            "in_progress",
            "--assignee",
            "agent:worker-1",
            "--full",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id_a);
}

#[test]
fn test_query_all_combined_state_and_label() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let out = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Tagged Ready",
            "-d",
            "d",
            "--label",
            "component:api",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_tagged = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // ready but no label — should not appear
    Command::new(&jit)
        .args(["issue", "create", "-t", "Untagged Ready", "-d", "d"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // tagged but done — should not appear
    let out = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Tagged Done",
            "-d",
            "d",
            "--label",
            "component:api",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id_tagged_done = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();
    Command::new(&jit)
        .args(["issue", "update", &id_tagged_done, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args([
            "query",
            "all",
            "--state",
            "ready",
            "--label",
            "component:api",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 1);
    assert_eq!(json["issues"][0]["id"], id_tagged);
}

#[test]
fn test_query_all_returns_empty_when_no_match() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Low Ready",
            "-d",
            "d",
            "--priority",
            "low",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Filter for critical — nothing matches
    let output = Command::new(&jit)
        .args(["query", "all", "--priority", "critical", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 0);
    assert!(json["issues"].as_array().unwrap().is_empty());
}

// ── query_by_assignee: additional cases ──────────────────────────────────────

#[test]
fn test_query_by_assignee_no_match() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let out = Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "-d", "d"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    Command::new(&jit)
        .args(["issue", "claim", &id, "agent:worker-1"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["query", "all", "--assignee", "agent:nobody", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 0);
}

#[test]
fn test_query_by_assignee_multiple_issues_same_agent() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let mut ids = vec![];
    for title in &["Task 1", "Task 2", "Task 3"] {
        let out = Command::new(&jit)
            .args(["issue", "create", "-t", title, "-d", "d"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        ids.push(
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .last()
                .unwrap()
                .to_string(),
        );
    }

    // Assign first two to agent:worker-1, third to agent:worker-2
    for id in &ids[..2] {
        Command::new(&jit)
            .args(["issue", "claim", id, "agent:worker-1"])
            .current_dir(temp.path())
            .output()
            .unwrap();
    }
    Command::new(&jit)
        .args(["issue", "claim", &ids[2], "agent:worker-2"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args([
            "query",
            "all",
            "--assignee",
            "agent:worker-1",
            "--full",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 2);

    let returned_ids: Vec<&str> = json["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    assert!(returned_ids.contains(&ids[0].as_str()));
    assert!(returned_ids.contains(&ids[1].as_str()));
    assert!(!returned_ids.contains(&ids[2].as_str()));
}

// ── query_by_priority: additional cases ──────────────────────────────────────

#[test]
fn test_query_by_priority_empty_results() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Low",
            "-d",
            "d",
            "--priority",
            "low",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["query", "all", "--priority", "critical", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    assert_eq!(json["count"], 0);
}

#[test]
fn test_query_by_priority_all_levels() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let priorities = ["critical", "high", "normal", "low"];
    let mut ids = std::collections::HashMap::new();

    for p in &priorities {
        let out = Command::new(&jit)
            .args(["issue", "create", "-t", p, "-d", "d", "--priority", p])
            .current_dir(temp.path())
            .output()
            .unwrap();
        ids.insert(
            *p,
            String::from_utf8_lossy(&out.stdout)
                .split_whitespace()
                .last()
                .unwrap()
                .to_string(),
        );
    }

    for p in &priorities {
        let output = Command::new(&jit)
            .args(["query", "all", "--priority", p, "--json"])
            .current_dir(temp.path())
            .output()
            .unwrap();

        assert!(output.status.success());
        let json: serde_json::Value =
            serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
        assert_eq!(json["count"], 1, "Expected 1 issue for priority={p}");
        assert_eq!(json["issues"][0]["id"], ids[p]);
        assert_eq!(json["issues"][0]["priority"], *p);
    }
}
