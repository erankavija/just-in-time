//! Integration tests for type hierarchy auto-fix functionality

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
        .status()
        .unwrap();
    temp
}

fn run_jit(temp: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(jit_binary())
        .args(args)
        .current_dir(temp.path())
        .output()
        .unwrap()
}

fn extract_id(output: &str) -> String {
    output.split_whitespace().last().unwrap().trim().to_string()
}

#[test]
fn test_fix_unknown_type_with_suggestion() {
    let temp = setup_test_repo();

    // Create an issue with a typo in the type label
    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Fix bug",
            "--label",
            "type:taks", // typo: should be "task"
        ],
    );
    assert!(
        output.status.success(),
        "Failed to create issue: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let issue_id = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Validate should fail with unknown type
    let output = run_jit(&temp, &["validate"]);
    assert!(
        !output.status.success(),
        "Validation should fail with unknown type"
    );

    // Run validate with --fix --dry-run to preview fix
    let output = run_jit(&temp, &["validate", "--fix", "--dry-run"]);
    assert!(
        output.status.success(),
        "Dry run should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("taks"));
    assert!(stdout.contains("task"));
    assert!(stdout.contains("Would replace"));

    // Apply the fix
    let output = run_jit(&temp, &["validate", "--fix"]);
    assert!(
        output.status.success(),
        "Fix should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Applied") || stdout.contains("fix"));

    // Verify the type was fixed
    let output = run_jit(&temp, &["issue", "show", &issue_id, "--json"]);
    assert!(output.status.success());
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let labels = json["data"]["labels"].as_array().unwrap();
    assert!(labels.iter().any(|l| l.as_str() == Some("type:task")));
    assert!(!labels.iter().any(|l| l.as_str() == Some("type:taks")));

    // Validation should now pass
    let output = run_jit(&temp, &["validate"]);
    assert!(
        output.status.success(),
        "Validation should pass after fix: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_fix_unknown_type_no_suggestion() {
    let temp = setup_test_repo();

    // Create an issue with a completely unknown type
    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Unknown work",
            "--label",
            "type:completely_unknown_xyz",
        ],
    );
    assert!(output.status.success());

    // Run validate with --fix
    let output = run_jit(&temp, &["validate", "--fix"]);
    // Should not crash, but won't be able to fix
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no automatic fixes") || stdout.contains("No fixes"));
}

#[test]
fn test_fix_multiple_issues() {
    let temp = setup_test_repo();

    // Create multiple issues with typos
    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Issue 1",
            "--label",
            "type:taks", // typo
        ],
    );
    assert!(output.status.success());

    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Issue 2",
            "--label",
            "type:epik", // typo
        ],
    );
    assert!(output.status.success());

    // Run validate with --fix
    let output = run_jit(&temp, &["validate", "--fix"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("2") || stdout.contains("Applied"));

    // Validation should pass
    let output = run_jit(&temp, &["validate"]);
    assert!(output.status.success());
}

#[test]
fn test_dry_run_doesnt_modify() {
    let temp = setup_test_repo();

    // Create an issue with a typo
    let output = run_jit(
        &temp,
        &["issue", "create", "--title", "Test", "--label", "type:taks"],
    );
    assert!(output.status.success());
    let issue_id = extract_id(&String::from_utf8_lossy(&output.stdout));

    // Run --fix --dry-run
    let output = run_jit(&temp, &["validate", "--fix", "--dry-run"]);
    assert!(output.status.success());

    // Verify nothing was actually changed
    let output = run_jit(&temp, &["issue", "show", &issue_id, "--json"]);
    let json: serde_json::Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).unwrap();
    let labels = json["data"]["labels"].as_array().unwrap();
    assert!(labels.iter().any(|l| l.as_str() == Some("type:taks")));
    assert!(!labels.iter().any(|l| l.as_str() == Some("type:task")));

    // Validation should still fail
    let output = run_jit(&temp, &["validate"]);
    assert!(!output.status.success());
}

#[test]
fn test_fix_json_output() {
    let temp = setup_test_repo();

    // Create issue with typo
    let output = run_jit(
        &temp,
        &["issue", "create", "--title", "Test", "--label", "type:taks"],
    );
    assert!(output.status.success());

    // Run validate with --fix --json
    let output = run_jit(&temp, &["validate", "--fix", "--json"]);
    assert!(output.status.success());

    // Parse JSON and verify structure
    let json: serde_json::Value = serde_json::from_str(&String::from_utf8_lossy(&output.stdout))
        .expect("Should output valid JSON");

    assert_eq!(json["success"], true);
    assert!(json["data"]["fixes_applied"].as_u64().unwrap() > 0);
    assert_eq!(json["data"]["dry_run"], false);
}

#[test]
fn test_dry_run_requires_fix_flag() {
    let temp = setup_test_repo();

    // Try to use --dry-run without --fix
    let output = run_jit(&temp, &["validate", "--dry-run"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--dry-run requires --fix"));
}

#[test]
fn test_fix_with_valid_repository() {
    let temp = setup_test_repo();

    // Create a valid issue
    let output = run_jit(
        &temp,
        &[
            "issue",
            "create",
            "--title",
            "Valid issue",
            "--label",
            "type:task",
        ],
    );
    assert!(output.status.success());

    // Run --fix on already valid repository
    let output = run_jit(&temp, &["validate", "--fix"]);
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("No fixes") || stdout.contains("valid"));
}
