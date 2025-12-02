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
