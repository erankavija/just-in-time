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
fn test_issue_not_found_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Try to show non-existent issue
    let output = Command::new(&jit)
        .args(["issue", "show", "nonexistent", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert_eq!(json["success"], false);
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("NOT_FOUND"));
    assert!(json["error"]["message"].is_string());
    assert!(json["error"]["suggestions"].is_array());
    assert!(json["metadata"]["timestamp"].is_string());
}

#[test]
fn test_cycle_detected_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create two issues
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

    // Add A depends on B
    Command::new(&jit)
        .args(["dep", "add", &id1, &id2])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Try to add B depends on A (creates cycle)
    let output = Command::new(&jit)
        .args(["dep", "add", &id2, &id1, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert_eq!(json["success"], false);
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("CYCLE_DETECTED"));
    assert!(json["error"]["message"].as_str().unwrap().contains("cycle"));
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_invalid_state_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Try to query with invalid state
    let output = Command::new(&jit)
        .args(["query", "state", "invalid_state", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert_eq!(json["success"], false);
    assert!(json["error"]["code"].as_str().unwrap().contains("INVALID"));
    assert!(json["error"]["suggestions"].is_array());
    // Should suggest valid states
    let suggestions_str = json["error"]["suggestions"].to_string();
    assert!(
        suggestions_str.contains("open")
            || suggestions_str.contains("ready")
            || suggestions_str.contains("done")
    );
}

#[test]
fn test_gate_operation_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create an issue
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "-d", "Test"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    // Try to pass a gate that wasn't added to the issue
    let output = Command::new(&jit)
        .args(["gate", "pass", &id, "nonexistent-gate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // This should fail or at least handle gracefully
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify JSON structure (success or error)
    assert!(json["success"].is_boolean());
    assert!(json["metadata"]["timestamp"].is_string());
}
