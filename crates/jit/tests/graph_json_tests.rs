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
fn test_graph_downstream_json_output() {
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

    // Query downstream dependents
    let output = Command::new(&jit)
        .args(["graph", "downstream", &id1, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["issue_id"], id1);
    assert!(json["data"]["dependents"].is_array());
    assert_eq!(json["data"]["count"], 1);
}

#[test]
fn test_graph_roots_json_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create two issues with dependency
    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Root Task", "-d", "First"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .nth(2)
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Dependent Task", "-d", "Second"])
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

    // Query root issues
    let output = Command::new(&jit)
        .args(["graph", "roots", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify structure
    assert_eq!(json["success"], true);
    assert!(json["data"]["roots"].is_array());
    assert_eq!(json["data"]["count"], 1);
    assert_eq!(json["data"]["roots"][0]["id"], id1);
}
