//! Integration tests for jit CLI
//!
//! These tests verify end-to-end functionality by running actual CLI commands

use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{}/target/debug/jit", manifest_dir)
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

#[test]
fn test_search_by_title() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues
    Command::new(&jit)
        .args(&["issue", "create", "-t", "Fix bug in parser", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(&["issue", "create", "-t", "Add feature", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(&["issue", "create", "-t", "Fix bug in lexer", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Search for "bug"
    let output = Command::new(&jit)
        .args(&["issue", "search", "bug"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 2 issue(s)"));
    assert!(stdout.contains("Fix bug in parser"));
    assert!(stdout.contains("Fix bug in lexer"));
    assert!(!stdout.contains("Add feature"));
}

#[test]
fn test_search_by_description() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args(&[
            "issue",
            "create",
            "-t",
            "Task 1",
            "-d",
            "Contains security vulnerability",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(&["issue", "create", "-t", "Task 2", "-d", "Regular task"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(&["issue", "search", "security"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 1 issue(s)"));
    assert!(stdout.contains("Task 1"));
}

#[test]
fn test_search_case_insensitive() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args(&["issue", "create", "-t", "Fix BUG", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(&["issue", "search", "bug"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 1 issue(s)"));
    assert!(stdout.contains("Fix BUG"));
}

#[test]
fn test_search_with_priority_filter() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args(&[
            "issue",
            "create",
            "-t",
            "Critical bug",
            "-d",
            "Desc",
            "--priority",
            "critical",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(&[
            "issue",
            "create",
            "-t",
            "Normal bug",
            "-d",
            "Desc",
            "--priority",
            "normal",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(&["issue", "search", "bug", "--priority", "critical"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 1 issue(s)"));
    assert!(stdout.contains("Critical bug"));
    assert!(!stdout.contains("Normal bug"));
}

#[test]
fn test_search_no_results() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args(&["issue", "create", "-t", "Task", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(&["issue", "search", "nonexistent"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 0 issue(s)"));
}

#[test]
fn test_export_dot_format() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues with dependencies
    let output1 = Command::new(&jit)
        .args(&["issue", "create", "-t", "API", "-d", "Design API"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let id1 = stdout1.trim().split_whitespace().last().unwrap();

    let output2 = Command::new(&jit)
        .args(&["issue", "create", "-t", "Backend", "-d", "Implement"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let id2 = stdout2.trim().split_whitespace().last().unwrap();

    Command::new(&jit)
        .args(&["dep", "add", id2, "--on", id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(&["graph", "export", "--format", "dot"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph issues"));
    assert!(stdout.contains(id1));
    assert!(stdout.contains(id2));
}

#[test]
fn test_export_mermaid_format() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args(&["issue", "create", "-t", "Task", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(&["graph", "export", "--format", "mermaid"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("graph LR"));
    assert!(stdout.contains("classDef"));
}
