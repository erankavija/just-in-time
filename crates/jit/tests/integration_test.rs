//! Integration tests for jit CLI
//!
//! These tests verify end-to-end functionality by running actual CLI commands

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

#[test]
fn test_search_by_title() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues
    Command::new(&jit)
        .args(["issue", "create", "-t", "Fix bug in parser", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "create", "-t", "Add feature", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "create", "-t", "Fix bug in lexer", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Search for "bug"
    let output = Command::new(&jit)
        .args(["issue", "search", "bug"])
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
        .args([
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
        .args(["issue", "create", "-t", "Task 2", "-d", "Regular task"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "search", "security"])
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
        .args(["issue", "create", "-t", "Fix BUG", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "search", "bug"])
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
        .args([
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
        .args([
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
        .args(["issue", "search", "bug", "--priority", "critical"])
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
        .args(["issue", "create", "-t", "Task", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "search", "nonexistent"])
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
        .args(["issue", "create", "-t", "API", "-d", "Design API"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let stdout1 = String::from_utf8_lossy(&output1.stdout);
    let id1 = stdout1.split_whitespace().last().unwrap();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Backend", "-d", "Implement"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let stdout2 = String::from_utf8_lossy(&output2.stdout);
    let id2 = stdout2.split_whitespace().last().unwrap();

    Command::new(&jit)
        .args(["dep", "add", id2, "--on", id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["graph", "export", "--format", "dot"])
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
        .args(["issue", "create", "-t", "Task", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["graph", "export", "--format", "mermaid"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("graph LR"));
    assert!(stdout.contains("classDef"));
}

// Test issue lifecycle
#[test]
fn test_issue_create_list_show() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .args([
            "issue",
            "create",
            "-t",
            "Test Issue",
            "-d",
            "Test description",
            "--priority",
            "high",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created issue:"));
    let id = stdout.split_whitespace().last().unwrap();

    // List issues
    let output = Command::new(&jit)
        .args(["issue", "list"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(id));
    assert!(stdout.contains("Test Issue"));

    // Show issue details
    let output = Command::new(&jit)
        .args(["issue", "show", id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Title: Test Issue"));
    assert!(stdout.contains("Description: Test description"));
    assert!(stdout.contains("Priority: High"));
}

#[test]
fn test_issue_update_and_delete() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Old Title", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Update issue
    Command::new(&jit)
        .args(["issue", "update", &id, "-t", "New Title"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "show", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Title: New Title"));

    // Delete issue
    Command::new(&jit)
        .args(["issue", "delete", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "list"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&id));
}

#[test]
fn test_dependencies() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Dependency", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Dependent", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Add dependency
    let output = Command::new(&jit)
        .args(["dep", "add", &id2, "--on", &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    // Show graph
    let output = Command::new(&jit)
        .args(["graph", "show", &id2])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id1));

    // Show downstream
    let output = Command::new(&jit)
        .args(["graph", "downstream", &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id2));
}

#[test]
fn test_cycle_detection() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Issue 1", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Issue 2", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Add dependency: id2 depends on id1
    Command::new(&jit)
        .args(["dep", "add", &id2, "--on", &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Try to create cycle: id1 depends on id2 (should fail)
    let output = Command::new(&jit)
        .args(["dep", "add", &id1, "--on", &id2])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cycle") || stderr.contains("Cycle"));
}

#[test]
fn test_gates() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Add gate definition
    Command::new(&jit)
        .args([
            "registry",
            "add",
            "tests",
            "-t",
            "Unit Tests",
            "-d",
            "Run tests",
            "-a",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // List gates
    let output = Command::new(&jit)
        .args(["registry", "list"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tests"));
    assert!(stdout.contains("Unit Tests"));

    // Create issue with gate
    let output = Command::new(&jit)
        .args([
            "issue", "create", "-t", "Task", "-d", "Desc", "--gate", "tests",
        ])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Pass gate
    let output = Command::new(&jit)
        .args(["gate", "pass", &id, "tests", "-b", "ci"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());

    // Verify gate status
    let output = Command::new(&jit)
        .args(["issue", "show", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tests"));
}

#[test]
fn test_assignment_workflow() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let id = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Assign issue
    Command::new(&jit)
        .args(["issue", "assign", &id, "-t", "alice"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "show", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alice"));

    // Unassign issue
    Command::new(&jit)
        .args(["issue", "unassign", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let output = Command::new(&jit)
        .args(["issue", "show", &id])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Assignee: None"));
}

#[test]
fn test_events() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    // Create issues to generate events
    Command::new(&jit)
        .args(["issue", "create", "-t", "Task 1", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "create", "-t", "Task 2", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Tail events
    let output = Command::new(&jit)
        .args(["events", "tail", "-n", "2"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should have JSON events
    assert!(stdout.contains("issue_created") || stdout.contains("{"));

    // Query events
    let output = Command::new(&jit)
        .args(["events", "query", "-e", "issue_created"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
}

#[test]
fn test_validate_repository() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    Command::new(&jit)
        .args(["issue", "create", "-t", "Task 1", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    Command::new(&jit)
        .args(["issue", "create", "-t", "Task 2", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Validate should succeed
    let output = Command::new(&jit)
        .args(["validate"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    assert!(output.status.success());
}

#[test]
fn test_graph_roots() {
    let temp = setup_test_repo();
    let jit = jit_binary();

    let output1 = Command::new(&jit)
        .args(["issue", "create", "-t", "Root", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id1 = String::from_utf8_lossy(&output1.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    let output2 = Command::new(&jit)
        .args(["issue", "create", "-t", "Child", "-d", "Desc"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    let id2 = String::from_utf8_lossy(&output2.stdout)
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();

    // Add dependency
    Command::new(&jit)
        .args(["dep", "add", &id2, "--on", &id1])
        .current_dir(temp.path())
        .output()
        .unwrap();

    // Get roots
    let output = Command::new(&jit)
        .args(["graph", "roots"])
        .current_dir(temp.path())
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id1));
    assert!(!stdout.contains(&id2));
}
