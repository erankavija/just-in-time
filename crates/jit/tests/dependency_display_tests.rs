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
fn test_issue_show_enriched_dependencies() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create dependency issues with different states
    let output_dep1 = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Dependency 1",
            "-d",
            "First dependency",
            "--priority",
            "high",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let dep1_id = String::from_utf8_lossy(&output_dep1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output_dep2 = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Dependency 2",
            "-d",
            "Second dependency",
            "--priority",
            "normal",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let dep2_id = String::from_utf8_lossy(&output_dep2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Mark dep1 as done
    Command::new(&jit)
        .args(["issue", "update", &dep1_id, "--state", "done"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Create main issue with dependencies
    let output_main = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Main Issue",
            "-d",
            "Has dependencies",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let main_id = String::from_utf8_lossy(&output_main.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Add dependencies
    Command::new(&jit)
        .args(["dep", "add", &main_id, &dep1_id, &dep2_id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Test JSON output
    let output = Command::new(&jit)
        .args(["issue", "show", &main_id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // Verify dependencies are enriched with metadata
    let deps = json["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 2);

    // Each dependency should have: id, title, state, priority
    for dep in deps {
        assert!(dep["id"].is_string());
        assert!(dep["title"].is_string());
        assert!(dep["state"].is_string());
        assert!(dep["priority"].is_string());
    }

    // Verify dep1 is marked as done
    let dep1 = deps.iter().find(|d| d["id"] == dep1_id).unwrap();
    assert_eq!(dep1["state"], "done");
    assert_eq!(dep1["title"], "Dependency 1");
    assert_eq!(dep1["priority"], "high");

    // Verify dep2 is ready
    let dep2 = deps.iter().find(|d| d["id"] == dep2_id).unwrap();
    assert_eq!(dep2["state"], "ready");
    assert_eq!(dep2["title"], "Dependency 2");
    assert_eq!(dep2["priority"], "normal");

    // Test human-readable output
    let output = Command::new(&jit)
        .args(["issue", "show", &main_id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show dependencies with short hash, title, and state
    assert!(stdout.contains("Dependencies"));
    assert!(stdout.contains(&dep1_id[..8])); // Short hash
    assert!(stdout.contains("Dependency 1"));
    assert!(stdout.contains("[done]"));
    assert!(stdout.contains(&dep2_id[..8]));
    assert!(stdout.contains("Dependency 2"));
    assert!(stdout.contains("[ready]"));

    // Should show summary
    assert!(stdout.contains("1/2 complete") || stdout.contains("1 of 2 complete"));
}

#[test]
fn test_issue_show_no_dependencies() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issue without dependencies
    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Standalone", "-d", "No deps"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Test JSON output
    let output = Command::new(&jit)
        .args(["issue", "show", &id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let deps = json["dependencies"].as_array().unwrap();
    assert_eq!(deps.len(), 0);

    // Test human output
    let output = Command::new(&jit)
        .args(["issue", "show", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Dependencies: None") || stdout.contains("Dependencies (0"));
}
