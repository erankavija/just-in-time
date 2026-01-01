//! Integration tests for gate modification flags in issue update command
//!
//! Tests the `--add-gate` and `--remove-gate` flags for `jit issue update`

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

    let output = Command::new(&jit)
        .arg("init")
        .current_dir(temp.path())
        .output()
        .expect("Failed to run jit init");

    assert!(output.status.success(), "jit init failed");
    temp
}

fn create_issue(temp: &TempDir, title: &str) -> String {
    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "create", "-t", title])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "Failed to create issue");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Extract ID from "Created issue: <ID>"
    stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .and_then(|l| l.split_whitespace().last())
        .unwrap()
        .to_string()
}

fn define_gate(temp: &TempDir, key: &str, title: &str) {
    let jit = jit_binary();
    let output = Command::new(&jit)
        .args([
            "gate",
            "define",
            key,
            "--title",
            title,
            "--description",
            "Test gate",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to define gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn get_issue_json(temp: &TempDir, id: &str) -> serde_json::Value {
    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "show", id, "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success(), "Failed to show issue");
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    json["data"].clone()
}

#[test]
fn test_add_single_gate() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--add-gate", "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to add gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = issue["gates_required"].as_array().unwrap();
    assert!(gates.contains(&serde_json::json!("tests")));
}

#[test]
fn test_add_multiple_gates_multiple_flags() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    define_gate(&temp, "clippy", "Run clippy");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            &id,
            "--add-gate",
            "tests",
            "--add-gate",
            "clippy",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to add gates: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = issue["gates_required"].as_array().unwrap();
    assert!(gates.contains(&serde_json::json!("tests")));
    assert!(gates.contains(&serde_json::json!("clippy")));
}

#[test]
fn test_add_multiple_gates_comma_separated() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    define_gate(&temp, "clippy", "Run clippy");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--add-gate", "tests,clippy"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to add gates: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    let gates = issue["gates_required"].as_array().unwrap();
    assert!(gates.contains(&serde_json::json!("tests")));
    assert!(gates.contains(&serde_json::json!("clippy")));
}

#[test]
fn test_remove_gate() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    define_gate(&temp, "clippy", "Run clippy");
    let jit = jit_binary();

    // Create issue with gates
    let output = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Test issue",
            "--gate",
            "tests,clippy",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .and_then(|l| l.split_whitespace().last())
        .unwrap();

    // Remove one gate
    let output = Command::new(&jit)
        .args(["issue", "update", id, "--remove-gate", "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to remove gate: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, id);
    let gates = issue["gates_required"].as_array().unwrap();
    assert!(!gates.contains(&serde_json::json!("tests")));
    assert!(gates.contains(&serde_json::json!("clippy")));
}

#[test]
fn test_add_and_remove_gates_combined() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    define_gate(&temp, "clippy", "Run clippy");
    define_gate(&temp, "fmt", "Run fmt");
    let jit = jit_binary();

    // Create issue with one gate
    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Test issue", "--gate", "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .and_then(|l| l.split_whitespace().last())
        .unwrap();

    // Add clippy and fmt, remove tests
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            id,
            "--add-gate",
            "clippy,fmt",
            "--remove-gate",
            "tests",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed to modify gates: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, id);
    let gates = issue["gates_required"].as_array().unwrap();
    assert!(!gates.contains(&serde_json::json!("tests")));
    assert!(gates.contains(&serde_json::json!("clippy")));
    assert!(gates.contains(&serde_json::json!("fmt")));
}

#[test]
fn test_add_gate_invalid_key_error() {
    let temp = setup_test_repo();
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--add-gate", "nonexistent"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success(), "Should fail for invalid gate");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not found") || stderr.contains("Gate"),
        "Error should mention gate not found: {}",
        stderr
    );
}

#[test]
fn test_add_gate_idempotent() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();

    // Add gate first time
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--add-gate", "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    // Add same gate again (should be no-op)
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--add-gate", "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(output.status.success());

    let issue = get_issue_json(&temp, &id);
    let gates = issue["gates_required"].as_array().unwrap();
    // Should have exactly one "tests" gate
    let test_count = gates.iter().filter(|g| g == &"tests").count();
    assert_eq!(test_count, 1, "Gate should only appear once");
}

#[test]
fn test_batch_add_gates() {
    let temp = setup_test_repo();
    define_gate(&temp, "code-review", "Code review");

    // Create multiple issues with same label
    let _id1 = create_issue(&temp, "Task 1");
    let _id2 = create_issue(&temp, "Task 2");

    // Update issues by setting label first (new issues start in ready state)
    let jit = jit_binary();
    Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "state:ready",
            "--label",
            "type:task",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Batch add gate using filter
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--add-gate",
            "code-review",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed batch gate add: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify both issues have the gate
    let output = Command::new(&jit)
        .args(["query", "label", "type:task", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = result["data"]["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2);

    for issue in issues {
        let gates = issue["gates_required"].as_array().unwrap();
        assert!(gates.contains(&serde_json::json!("code-review")));
    }
}

#[test]
fn test_gates_in_json_output() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--add-gate", "tests", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let gates = result["data"]["gates_required"].as_array().unwrap();
    assert!(gates.contains(&serde_json::json!("tests")));
}

#[test]
fn test_combine_with_other_updates() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            &id,
            "--state",
            "ready",
            "--add-gate",
            "tests",
            "--label",
            "type:task",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed combined update: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue = get_issue_json(&temp, &id);
    assert_eq!(issue["state"], "ready");
    assert!(issue["gates_required"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("tests")));
    assert!(issue["labels"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("type:task")));
}

#[test]
fn test_remove_nonexistent_gate_is_noop() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    let id = create_issue(&temp, "Test issue");

    let jit = jit_binary();
    let output = Command::new(&jit)
        .args(["issue", "update", &id, "--remove-gate", "tests"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Removing non-existent gate should succeed as no-op"
    );

    let issue = get_issue_json(&temp, &id);
    let gates = issue["gates_required"].as_array().unwrap();
    assert!(gates.is_empty());
}

#[test]
fn test_batch_remove_gates() {
    let temp = setup_test_repo();
    define_gate(&temp, "tests", "Run tests");
    let jit = jit_binary();

    // Create issues with gates
    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Task 1",
            "--gate",
            "tests",
            "--label",
            "type:task",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Task 2",
            "--gate",
            "tests",
            "--label",
            "type:task",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Batch remove gate
    let output = Command::new(&jit)
        .args([
            "issue",
            "update",
            "--filter",
            "label:type:task",
            "--remove-gate",
            "tests",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "Failed batch gate remove: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify gates removed
    let output = Command::new(&jit)
        .args(["query", "label", "type:task", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issues = result["data"]["issues"].as_array().unwrap();

    for issue in issues {
        let gates = issue["gates_required"].as_array().unwrap();
        assert!(!gates.contains(&serde_json::json!("tests")));
    }
}
