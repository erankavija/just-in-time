//! Integration tests for label query JSON output

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
fn test_query_label_json_exact_match() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issue with label
    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Milestone Task",
            "--label",
            "milestone:v1.0",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query with JSON
    let output = Command::new(&jit)
        .args([
            "query",
            "all",
            "--label",
            "milestone:v1.0",
            "--full",
            "--json",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["filters"]["label"], "milestone:v1.0");
    assert_eq!(json["data"]["count"], 1);
    assert!(json["data"]["issues"].is_array());
    assert_eq!(json["data"]["issues"].as_array().unwrap().len(), 1);

    let issue = &json["data"]["issues"][0];
    assert_eq!(issue["title"], "Milestone Task");
    assert!(issue["labels"]
        .as_array()
        .unwrap()
        .contains(&serde_json::json!("milestone:v1.0")));
}

#[test]
fn test_query_label_json_wildcard() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create multiple issues with milestone labels
    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Task v1",
            "--label",
            "milestone:v1.0",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Task v2",
            "--label",
            "milestone:v2.0",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query with wildcard
    let output = Command::new(&jit)
        .args(["query", "all", "--label", "milestone:*", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["filters"]["label"], "milestone:*");
    assert_eq!(json["data"]["count"], 2);
    assert_eq!(json["data"]["issues"].as_array().unwrap().len(), 2);
}

#[test]
fn test_query_label_json_no_matches() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issue without the queried label
    Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "--label", "milestone:v1.0"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query for different label
    let output = Command::new(&jit)
        .args(["query", "all", "--label", "epic:auth", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["filters"]["label"], "epic:auth");
    assert_eq!(json["data"]["count"], 0);
    assert_eq!(json["data"]["issues"].as_array().unwrap().len(), 0);
}

#[test]
fn test_query_label_json_invalid_pattern() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Query with invalid pattern (no colon)
    let output = Command::new(&jit)
        .args(["query", "all", "--label", "invalidlabel", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Error might be in JSON or plain text depending on where validation happens
    if !stdout.is_empty() && stdout.starts_with("{") {
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["code"], "INVALID_LABEL_PATTERN");
        assert!(json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Invalid label pattern"));
        assert!(json["error"]["suggestions"].is_array());
        let suggestions_text = json["error"]["suggestions"][0].as_str().unwrap();
        assert!(suggestions_text.contains("namespace:value"));
    } else {
        // Validation error from query_by_label
        assert!(stderr.contains("Invalid") || stderr.contains("invalid"));
    }
}

#[test]
fn test_query_label_text_output() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issue with label
    Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Test Task",
            "--label",
            "milestone:v1.0",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query without --json flag
    let output = Command::new(&jit)
        .args(["query", "all", "--label", "milestone:v1.0"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(stdout.contains("All issues (filtered):"));
    assert!(stdout.contains("Test Task"));
    assert!(stdout.contains("Total: 1"));
}

#[test]
fn test_query_label_with_uppercase_namespace() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Query with uppercase namespace (should fail validation)
    let output = Command::new(&jit)
        .args(["query", "all", "--label", "Milestone:v1.0", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Error might be in JSON or plain text
    if !stdout.is_empty() && stdout.starts_with("{") {
        let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(json["success"], false);
        assert_eq!(json["error"]["code"], "INVALID_LABEL_PATTERN");
    } else {
        assert!(stderr.contains("Invalid") || stderr.contains("invalid"));
    }
}

#[test]
fn test_query_label_wildcard_with_no_matches() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issue with different namespace
    Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "--label", "epic:auth"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Query wildcard that doesn't match
    let output = Command::new(&jit)
        .args(["query", "all", "--label", "milestone:*", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(json["success"], true);
    assert_eq!(json["data"]["count"], 0);
}
