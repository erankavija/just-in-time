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
fn test_graph_deps_depth_default() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create a 3-level dependency chain: A -> B -> C
    let c = create_issue(&jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(&jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_dep(&jit, &temp, "Task A", "Root", &b);

    // Default behavior (no --depth flag) should show immediate deps only (depth 1)
    let output = Command::new(&jit)
        .args(["graph", "deps", &a, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let deps = json["data"]["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 1); // Only B (immediate dependency)
    assert_eq!(deps[0]["id"], b);

    // Verify depth is reported
    assert_eq!(json["data"]["depth"], 1);
}

#[test]
fn test_graph_deps_depth_2() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create: A -> B -> C
    let c = create_issue(&jit, &temp, "Task C", "Leaf");
    let b = create_issue_with_dep(&jit, &temp, "Task B", "Middle", &c);
    let a = create_issue_with_dep(&jit, &temp, "Task A", "Root", &b);

    // --depth 2 should show B and C
    let output = Command::new(&jit)
        .args(["graph", "deps", &a, "--depth", "2", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let deps = json["data"]["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 2); // B and C

    let dep_ids: Vec<&str> = deps.iter().map(|d| d["id"].as_str().unwrap()).collect();
    assert!(dep_ids.contains(&b.as_str()));
    assert!(dep_ids.contains(&c.as_str()));

    assert_eq!(json["data"]["depth"], 2);
}

#[test]
fn test_graph_deps_depth_unlimited() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create: A -> B -> C -> D
    let d = create_issue(&jit, &temp, "Task D", "Deep");
    let c = create_issue_with_dep(&jit, &temp, "Task C", "Level 3", &d);
    let b = create_issue_with_dep(&jit, &temp, "Task B", "Level 2", &c);
    let a = create_issue_with_dep(&jit, &temp, "Task A", "Root", &b);

    // --depth 0 should show all transitive dependencies
    let output = Command::new(&jit)
        .args(["graph", "deps", &a, "--depth", "0", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let deps = json["data"]["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 3); // B, C, D

    assert_eq!(json["data"]["depth"], 0); // 0 = unlimited
}

// Helper functions
fn create_issue(jit: &str, temp: &TempDir, title: &str, desc: &str) -> String {
    let output = Command::new(jit)
        .args(["issue", "create", "-t", title, "-d", desc])
        .current_dir(temp.path())
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

fn create_issue_with_dep(
    jit: &str,
    temp: &TempDir,
    title: &str,
    desc: &str,
    dep_id: &str,
) -> String {
    let id = create_issue(jit, temp, title, desc);
    Command::new(jit)
        .args(["dep", "add", &id, dep_id])
        .current_dir(temp.path())
        .output()
        .unwrap();
    id
}
