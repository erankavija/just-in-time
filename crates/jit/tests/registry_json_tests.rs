//! JSON-output tests for the gate registry surface (`jit gate list` / `gate show`).
//!
//! Gate-registry configuration lives solely under `jit gate`; these tests cover
//! the machine-readable shapes of its list and show verbs.

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
fn test_gate_list_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Define a gate
    Command::new(jit)
        .args([
            "gate",
            "define",
            "test-gate",
            "-t",
            "Test Gate",
            "-d",
            "A test gate",
            "--auto",
            "--checker-command",
            "true",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // List gates with JSON
    let output = Command::new(jit)
        .args(["gate", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert!(json["gates"].is_array());
    assert_eq!(json["count"], 1);
    assert_eq!(json["gates"][0]["key"], "test-gate");
    assert_eq!(json["gates"][0]["title"], "Test Gate");
    assert_eq!(json["gates"][0]["auto"], true);
}

#[test]
fn test_gate_show_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Define a gate with an example-integration snippet
    Command::new(jit)
        .args([
            "gate",
            "define",
            "test-gate",
            "-t",
            "Test Gate",
            "-d",
            "A test gate description",
            "--auto",
            "--checker-command",
            "true",
            "--example",
            "Example command",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Show gate with JSON
    let output = Command::new(jit)
        .args(["gate", "show", "test-gate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["key"], "test-gate");
    assert_eq!(json["title"], "Test Gate");
    assert_eq!(json["description"], "A test gate description");
    assert_eq!(json["auto"], true);
    assert_eq!(json["example_integration"], "Example command");
}

#[test]
fn test_gate_list_empty_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // List gates when empty
    let output = Command::new(jit)
        .args(["gate", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert!(json["gates"].is_array());
    assert_eq!(json["count"], 0);
}

#[test]
fn test_gate_define_with_stage_option() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Define a precheck gate using --stage option
    let output = Command::new(jit)
        .args([
            "gate",
            "define",
            "tdd-reminder",
            "-t",
            "TDD Reminder",
            "-d",
            "Write tests first",
            "--stage",
            "precheck",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to define gate with --stage precheck"
    );

    // Verify the gate was created with precheck stage
    let output = Command::new(jit)
        .args(["gate", "show", "tdd-reminder", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["key"], "tdd-reminder");
    assert_eq!(json["stage"], "precheck");
    assert_eq!(json["mode"], "manual");
}

#[test]
fn test_gate_define_defaults_to_postcheck() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Define gate without --stage option (should default to postcheck)
    let output = Command::new(jit)
        .args([
            "gate",
            "define",
            "code-review",
            "-t",
            "Code Review",
            "-d",
            "Manual review",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    // Verify it defaulted to postcheck
    let output = Command::new(jit)
        .args(["gate", "show", "code-review", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["key"], "code-review");
    assert_eq!(json["stage"], "postcheck");
}

#[test]
fn test_gate_define_with_invalid_stage() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Try to define gate with invalid stage value
    let output = Command::new(jit)
        .args([
            "gate",
            "define",
            "invalid-gate",
            "-t",
            "Invalid",
            "-d",
            "Invalid stage",
            "--stage",
            "invalid",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Should fail with error
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("invalid") || stderr.contains("stage"));
}
