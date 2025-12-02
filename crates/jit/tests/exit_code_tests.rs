//! Integration tests for standardized exit codes
//!
//! Tests that the CLI returns appropriate exit codes for different error scenarios.

use std::fs;
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

/// Helper to create a test environment with jit initialized
fn setup_test_env() -> TempDir {
    let temp_dir = TempDir::new().unwrap();
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());
    temp_dir
}

#[test]
fn test_exit_code_success() {
    let temp_dir = setup_test_env();

    // Successful command should return exit code 0
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Test issue"])
        .status()
        .unwrap();
    
    assert!(status.success());
    assert_eq!(status.code(), Some(0));
}

#[test]
fn test_exit_code_not_found() {
    let temp_dir = setup_test_env();

    // Issue not found should return exit code 3
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", "nonexistent"])
        .output()
        .unwrap();
    
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(3));
    // Error message is "Failed to read file" for non-existent issues
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("Failed to read file") || stderr.contains("not found"));
}

#[test]
fn test_exit_code_validation_failed_cycle() {
    let temp_dir = setup_test_env();

    // Create two issues
    let output1 = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Task A", "--json"])
        .output()
        .unwrap();
    assert!(output1.status.success());
    let json1: serde_json::Value = serde_json::from_slice(&output1.stdout).unwrap();
    // Issue creation returns the issue directly, not wrapped in JsonOutput
    let id1 = json1["id"].as_str().expect("id1 should exist");

    let output2 = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Task B", "--json"])
        .output()
        .unwrap();
    assert!(output2.status.success());
    let json2: serde_json::Value = serde_json::from_slice(&output2.stdout).unwrap();
    let id2 = json2["id"].as_str().expect("id2 should exist");

    // Add dependency A -> B
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", id1, id2])
        .status()
        .unwrap();
    assert!(status.success());

    // Try to add B -> A (would create cycle) - should return exit code 4
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["dep", "add", id2, id1])
        .output()
        .unwrap();
    
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("cycle"));
}

#[test]
fn test_exit_code_invalid_argument() {
    let temp_dir = setup_test_env();

    // Invalid priority should return exit code 2
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Test", "--priority", "invalid"])
        .output()
        .unwrap();
    
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn test_exit_code_io_error() {
    // Try to run command in non-initialized directory
    // This should return exit code 3 (not found) because data directory doesn't exist
    let temp_dir = TempDir::new().unwrap();
    
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "list"])
        .output()
        .unwrap();
    
    assert!(!output.status.success());
    // File not found is code 3, not 10
    assert_eq!(output.status.code(), Some(3));
    assert!(String::from_utf8_lossy(&output.stderr).contains("data"));
}

#[test]
fn test_exit_code_already_exists() {
    let temp_dir = setup_test_env();

    // Add a gate to registry
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "Test gate", "test-gate"])
        .status()
        .unwrap();
    assert!(status.success());

    // Try to add same gate again - should return exit code 6
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["registry", "add", "--title", "Test gate", "test-gate"])
        .output()
        .unwrap();
    
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(6));
    assert!(String::from_utf8_lossy(&output.stderr).contains("already exists"));
}

#[test]
fn test_exit_code_json_error_format() {
    let temp_dir = setup_test_env();

    // Error with --json flag should still return appropriate exit code
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "show", "nonexistent", "--json"])
        .output()
        .unwrap();

    // Should have exit code 3 (not found)
    assert_eq!(output.status.code(), Some(3));

    // Should have valid JSON error
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["success"], false);
    assert_eq!(json["error"]["code"], "ISSUE_NOT_FOUND");
}

#[test]
fn test_exit_code_validation_command() {
    let temp_dir = setup_test_env();

    // Validation should succeed with exit code 0 when no issues
    let status = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["validate"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(status.code(), Some(0));

    // Create an issue and manually corrupt the data to cause validation failure
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["issue", "create", "--title", "Test", "--json"])
        .output()
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let id = json["id"].as_str().expect("id should exist");

    // Corrupt the issue file by adding invalid dependency reference
    let issue_path = temp_dir.path().join("data").join("issues").join(format!("{}.json", id));
    let mut issue_data: serde_json::Value = serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue_data["dependencies"] = serde_json::json!(["nonexistent"]);
    fs::write(&issue_path, serde_json::to_string_pretty(&issue_data).unwrap()).unwrap();

    // Validation should fail with exit code 4
    let output = Command::new(jit_binary())
        .current_dir(&temp_dir)
        .args(["validate"])
        .output()
        .unwrap();
    
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stderr).contains("Invalid"));
}

#[test]
fn test_exit_code_help_and_version() {
    // --help should return exit code 0
    let status = Command::new(jit_binary())
        .arg("--help")
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(status.code(), Some(0));

    // Help for subcommand should also return 0
    let status = Command::new(jit_binary())
        .args(["issue", "--help"])
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(status.code(), Some(0));
}

