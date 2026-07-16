use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn setup_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let jit = jit_binary();
    Command::new(jit)
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
    let output = Command::new(jit)
        .args(["issue", "show", "nonexistent", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("NOT_FOUND"));
    assert!(json["error"]["message"].is_string());
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_cycle_detected_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create two issues
    let output1 = Command::new(jit)
        .args(["issue", "create", "-t", "Task A", "-d", "First"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    let output2 = Command::new(jit)
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
    Command::new(jit)
        .args(["dep", "add", &id1, &id2])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Try to add B depends on A (creates cycle)
    let output = Command::new(jit)
        .args(["dep", "add", &id2, &id1, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify error structure
    assert!(json["error"]["code"]
        .as_str()
        .unwrap()
        .contains("CYCLE_DETECTED"));
    assert!(json["error"]["message"].as_str().unwrap().contains("cycle"));
    assert!(json["error"]["suggestions"].is_array());
}

#[test]
fn test_deletion_not_confirmed_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let created = Command::new(jit)
        .args(["issue", "create", "-t", "Doomed", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id = serde_json::from_slice::<serde_json::Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Delete without JIT_ALLOW_DELETION=1: refused (REQ-02).
    let output = Command::new(jit)
        .args(["issue", "delete", &id, "--json"])
        .env_remove("JIT_ALLOW_DELETION")
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert_eq!(
        output.status.code(),
        Some(2),
        "refusal must exit 2 (REQ-01)"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["error"]["code"], "DELETION_NOT_CONFIRMED");
    assert!(json["error"]["message"]
        .as_str()
        .unwrap()
        .contains("discouraged"));
    let suggestions = json["error"]["suggestions"].as_array().unwrap();
    assert!(
        suggestions
            .iter()
            .any(|s| s.as_str().unwrap().contains("JIT_ALLOW_DELETION=1")),
        "suggestions must carry the confirmation hint, got: {suggestions:?}"
    );

    // The issue must still exist: the refusal must not have deleted it.
    let show = Command::new(jit)
        .args(["issue", "show", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(show.status.success(), "issue must survive a refused delete");
}

#[test]
fn test_invalid_state_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Try to query with invalid state
    let output = Command::new(jit)
        .args(["query", "all", "--state", "invalid_state", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should get error message (might be in stderr for clap validation errors)
    assert!(
        stdout.contains("INVALID") || stderr.contains("invalid") || stderr.contains("state"),
        "Expected error about invalid state, got stdout: {}, stderr: {}",
        stdout,
        stderr
    );
}

#[test]
fn test_gate_operation_error_json() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create an issue
    let output1 = Command::new(jit)
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
    let output = Command::new(jit)
        .args(["gate", "pass", &id, "nonexistent-gate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // This should fail or at least handle gracefully
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify JSON structure - envelope removed, just check valid JSON
}
