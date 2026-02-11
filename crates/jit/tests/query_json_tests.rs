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
    Command::new(&jit)
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

#[test]
fn test_query_ready_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create a ready issue
    Command::new(&jit)
        .args(["issue", "create", "-t", "Ready Task", "-d", "Test"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query ready with --json
    let output = Command::new(&jit)
        .args(["query", "available", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert!(json["data"]["issues"].is_array());
    assert_eq!(json["data"]["count"], 1);
    assert!(json["metadata"]["timestamp"].is_string());
    assert_eq!(json["metadata"]["version"], "0.2.1");
}

#[test]
fn test_query_blocked_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create two issues with dependency
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task A", "-d", "First"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task B", "-d", "Second"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    // Add dependency: id2 depends on id1
    Command::new(&jit)
        .args(["dep", "add", &id2, &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query blocked with --json
    let output = Command::new(&jit)
        .args(["query", "blocked", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert!(json["data"]["issues"].is_array());
    assert_eq!(json["data"]["count"], 1);
    assert!(json["data"]["issues"][0]["blocked_reasons"].is_array());

    // blocked_reasons is now an array of strings, not objects
    let blocked_reasons = json["data"]["issues"][0]["blocked_reasons"]
        .as_array()
        .unwrap();
    assert_eq!(blocked_reasons.len(), 1);
    let reason_str = blocked_reasons[0].as_str().unwrap();
    assert!(reason_str.starts_with("dependency:"));
}

#[test]
fn test_query_assignee_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create and assign an issue
    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "-d", "Test"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    Command::new(&jit)
        .args(["issue", "assign", &id, "copilot:test"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query by assignee with --json
    let output = Command::new(&jit)
        .args(["query", "all", "--assignee", "copilot:test", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure (filters field removed)
    assert_eq!(json["success"], true);
    assert!(json["data"]["issues"].is_array());
    assert_eq!(json["data"]["count"], 1);
}

#[test]
fn test_query_state_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create an issue (will be in ready state since no dependencies)
    Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "-d", "Test"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query by state with --json (issues default to ready when unblocked)
    let output = Command::new(&jit)
        .args(["query", "all", "--state", "ready", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure (filters field removed)
    assert_eq!(json["success"], true);
    assert!(json["data"]["issues"].is_array());
    assert_eq!(json["data"]["count"], 1);
}

#[test]
fn test_query_priority_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create an issue with high priority
    Command::new(&jit)
        .args([
            "issue", "create", "-t", "Urgent", "-d", "Test", "-p", "high",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query by priority with --json
    let output = Command::new(&jit)
        .args(["query", "all", "--priority", "high", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure (filters field removed)
    assert_eq!(json["success"], true);
    assert!(json["data"]["issues"].is_array());
    assert_eq!(json["data"]["count"], 1);
}
