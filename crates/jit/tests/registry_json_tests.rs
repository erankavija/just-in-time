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
fn test_registry_list_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Add a gate definition
    Command::new(&jit)
        .args([
            "registry",
            "add",
            "test-gate",
            "-t",
            "Test Gate",
            "-d",
            "A test gate",
            "--auto",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // List gates with JSON
    let output = Command::new(&jit)
        .args(["registry", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert!(json["data"]["gates"].is_array());
    assert_eq!(json["data"]["count"], 1);
    assert_eq!(json["data"]["gates"][0]["key"], "test-gate");
    assert_eq!(json["data"]["gates"][0]["title"], "Test Gate");
    assert_eq!(json["data"]["gates"][0]["auto"], true);
    assert!(json["metadata"]["timestamp"].is_string());
}

#[test]
fn test_registry_show_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Add a gate definition
    Command::new(&jit)
        .args([
            "registry",
            "add",
            "test-gate",
            "-t",
            "Test Gate",
            "-d",
            "A test gate description",
            "--auto",
            "-e",
            "Example command",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Show gate with JSON
    let output = Command::new(&jit)
        .args(["registry", "show", "test-gate", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["key"], "test-gate");
    assert_eq!(json["data"]["title"], "Test Gate");
    assert_eq!(json["data"]["description"], "A test gate description");
    assert_eq!(json["data"]["auto"], true);
    assert_eq!(json["data"]["example_integration"], "Example command");
    assert!(json["metadata"]["timestamp"].is_string());
}

#[test]
fn test_registry_list_empty_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // List gates when empty
    let output = Command::new(&jit)
        .args(["registry", "list", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert!(json["data"]["gates"].is_array());
    assert_eq!(json["data"]["count"], 0);
}

#[test]
fn test_registry_add_with_stage_option() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Add a precheck gate using --stage option
    let output = Command::new(&jit)
        .args([
            "registry",
            "add",
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
        "Failed to add gate with --stage precheck"
    );

    // Verify the gate was created with precheck stage
    let output = Command::new(&jit)
        .args(["registry", "show", "tdd-reminder", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["data"]["key"], "tdd-reminder");
    assert_eq!(json["data"]["stage"], "precheck");
    assert_eq!(json["data"]["mode"], "manual");
}

#[test]
fn test_registry_add_defaults_to_postcheck() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Add gate without --stage option (should default to postcheck)
    let output = Command::new(&jit)
        .args([
            "registry",
            "add",
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
    let output = Command::new(&jit)
        .args(["registry", "show", "code-review", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["data"]["key"], "code-review");
    assert_eq!(json["data"]["stage"], "postcheck");
}

#[test]
fn test_registry_add_with_invalid_stage() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Try to add gate with invalid stage value
    let output = Command::new(&jit)
        .args([
            "registry",
            "add",
            "invalid-gate",
            "-t",
            "Invalid",
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
