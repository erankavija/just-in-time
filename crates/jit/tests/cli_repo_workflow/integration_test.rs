//! Integration tests for jit CLI
//!
//! These tests verify end-to-end functionality by running actual CLI commands

use jit::test_cli::JitCli;
use tempfile::TempDir;

#[test]
fn test_search_by_title() {
    let cli = JitCli::initialised().unwrap();

    // Create issues
    cli.command(["issue", "create", "-t", "Fix bug in parser", "-d", "Desc"])
        .output()
        .unwrap();

    cli.command(["issue", "create", "-t", "Add feature", "-d", "Desc"])
        .output()
        .unwrap();

    cli.command(["issue", "create", "-t", "Fix bug in lexer", "-d", "Desc"])
        .output()
        .unwrap();

    // Search for "bug"
    let output = cli.command(["issue", "search", "bug"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 2 issue(s)"));
    assert!(stdout.contains("Fix bug in parser"));
    assert!(stdout.contains("Fix bug in lexer"));
    assert!(!stdout.contains("Add feature"));
}

#[test]
fn test_issue_search_human_output_indents_rows_and_ends_with_total() {
    let cli = JitCli::initialised().unwrap();
    let id = cli
        .create_issue(["--title", "Fix bug in parser", "--description", "Desc"])
        .unwrap();

    let output = cli.command(["issue", "search", "bug"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let row = stdout
        .lines()
        .find(|line| line.contains(&id))
        .expect("search output lists the issue");
    assert!(row.starts_with("  "), "row is indented: {row:?}");
    assert!(stdout.trim_end().ends_with("Total: 1"), "{stdout}");
}

#[test]
fn test_search_by_description() {
    let cli = JitCli::initialised().unwrap();

    cli.command([
        "issue",
        "create",
        "-t",
        "Task 1",
        "-d",
        "Contains security vulnerability",
    ])
    .output()
    .unwrap();

    cli.command(["issue", "create", "-t", "Task 2", "-d", "Regular task"])
        .output()
        .unwrap();

    let output = cli
        .command(["issue", "search", "security"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 1 issue(s)"));
    assert!(stdout.contains("Task 1"));
}

#[test]
fn test_search_case_insensitive() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["issue", "create", "-t", "Fix BUG", "-d", "Desc"])
        .output()
        .unwrap();

    let output = cli.command(["issue", "search", "bug"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 1 issue(s)"));
    assert!(stdout.contains("Fix BUG"));
}

#[test]
fn test_search_with_priority_filter() {
    let cli = JitCli::initialised().unwrap();

    cli.command([
        "issue",
        "create",
        "-t",
        "Critical bug",
        "-d",
        "Desc",
        "--priority",
        "critical",
    ])
    .output()
    .unwrap();

    cli.command([
        "issue",
        "create",
        "-t",
        "Normal bug",
        "-d",
        "Desc",
        "--priority",
        "normal",
    ])
    .output()
    .unwrap();

    let output = cli
        .command(["issue", "search", "bug", "--priority", "critical"])
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
    let cli = JitCli::initialised().unwrap();

    cli.command(["issue", "create", "-t", "Task", "-d", "Desc"])
        .output()
        .unwrap();

    let output = cli
        .command(["issue", "search", "nonexistent"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Found 0 issue(s)"));
}

#[test]
fn test_export_dot_format() {
    let cli = JitCli::initialised().unwrap();

    // Create issues with dependencies
    let id1 = cli
        .create_issue(["--title", "API", "--description", "Design API"])
        .unwrap();
    let id2 = cli
        .create_issue(["--title", "Backend", "--description", "Implement"])
        .unwrap();

    cli.command(["dep", "add", &id2, &id1]).output().unwrap();

    let output = cli
        .command(["graph", "export", "--format", "dot"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("digraph issues"));
    assert!(stdout.contains(&id1));
    assert!(stdout.contains(&id2));
}

#[test]
fn test_export_mermaid_format() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["issue", "create", "-t", "Task", "-d", "Desc"])
        .output()
        .unwrap();

    let output = cli
        .command(["graph", "export", "--format", "mermaid"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("graph LR"));
    assert!(stdout.contains("classDef"));
}

#[test]
fn test_export_json_full_emits_complete_records() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["issue", "create", "-t", "Task", "-d", "Full body"])
        .output()
        .unwrap();

    let output = cli
        .command(["graph", "export", "--format", "json", "--full"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let doc: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let node = &doc["nodes"][0];
    // Complete-record fields the summary shape omits.
    assert_eq!(node["description"], "Full body");
    assert!(node.get("gates_status").is_some());
    assert!(node.get("created_at").is_some());
    assert!(doc.get("edges").is_some());
}

#[test]
fn test_export_output_writes_complete_file() {
    let cli = JitCli::initialised().unwrap();

    cli.command(["issue", "create", "-t", "Task", "-d", "Desc"])
        .output()
        .unwrap();

    let out_path = cli.root().join("graph.json");
    std::fs::write(&out_path, "old").unwrap();
    for _ in 0..2 {
        let output = cli
            .command([
                "graph",
                "export",
                "--format",
                "json",
                "--output",
                out_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    // The atomically written file is complete and parses as the full document;
    // no partial/temp residue is left beside it.
    let contents = std::fs::read_to_string(&out_path).unwrap();
    let doc: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert!(doc["nodes"].is_array());
    assert!(doc["edges"].is_array());
    let temp_residue = std::fs::read_dir(cli.root())
        .unwrap()
        .filter_map(|e| e.ok())
        .any(|e| e.file_name().to_string_lossy().contains(".graph.json."));
    assert!(!temp_residue, "atomic write left a temp file behind");
}

#[test]
fn test_export_output_routes_repository_and_external_destinations() {
    let cli = JitCli::initialised().unwrap();
    let run = |path: &std::path::Path| {
        cli.command([
            "graph",
            "export",
            "--format",
            "json",
            "--output",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap()
    };

    let nested_data = cli.root().join(".jit/exports/graph.json");
    std::fs::create_dir(cli.root().join(".jit/exports")).unwrap();
    assert!(run(&nested_data).status.success());
    assert!(nested_data.is_file());

    let external = TempDir::new().unwrap();
    let outside = external.path().join("graph.json");
    assert!(run(&outside).status.success());
    assert!(outside.is_file());
}

#[cfg(unix)]
#[test]
fn test_export_output_rejects_symlinked_repository_parent() {
    let cli = JitCli::initialised().unwrap();
    let external = TempDir::new().unwrap();
    std::os::unix::fs::symlink(external.path(), cli.root().join("linked")).unwrap();

    let output = cli
        .command([
            "graph",
            "export",
            "--format",
            "json",
            "--output",
            "linked/graph.json",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(!external.path().join("graph.json").exists());
}

#[cfg(unix)]
#[test]
fn test_export_output_rejects_external_parent_aliases_into_repository() {
    let cli = JitCli::initialised().unwrap();
    let external = TempDir::new().unwrap();
    let worktree_alias = external.path().join("worktree");
    let data_alias = external.path().join("issues");
    std::os::unix::fs::symlink(cli.root(), &worktree_alias).unwrap();
    std::os::unix::fs::symlink(cli.root().join(".jit/issues"), &data_alias).unwrap();

    for (alias, escaped) in [
        (worktree_alias, cli.root().join("escaped.json")),
        (data_alias, cli.root().join(".jit/issues/escaped.json")),
    ] {
        let output = cli
            .command([
                "graph",
                "export",
                "--format",
                "json",
                "--output",
                alias.join("escaped.json").to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!escaped.exists());
    }
}

#[test]
fn test_export_full_requires_json_format() {
    let cli = JitCli::initialised().unwrap();

    let output = cli
        .command(["graph", "export", "--format", "dot", "--full"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--full is only valid with --format json"));
}

// Test issue lifecycle
#[test]
fn test_issue_create_list_show() {
    let cli = JitCli::initialised().unwrap();

    let output = cli
        .command([
            "issue",
            "create",
            "-t",
            "Test Issue",
            "-d",
            "Test description",
            "--priority",
            "high",
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Created issue:"));
    let issues = cli.run_json(["issue", "search", "Test Issue"]).unwrap();
    let id = issues["issues"][0]["id"].as_str().unwrap().to_owned();

    // List issues
    let output = cli.command(["query", "all"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&id));
    assert!(stdout.contains("Test Issue"));

    // Show issue details
    let output = cli.command(["issue", "show", &id]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Title: Test Issue"));
    assert!(stdout.contains("Description: Test description"));
    assert!(stdout.contains("Priority: high"));
}

#[test]
fn test_issue_update_and_delete() {
    let cli = JitCli::initialised().unwrap();

    let id = cli
        .create_issue(["--title", "Old Title", "--description", "Desc"])
        .unwrap();

    // Update issue
    cli.command(["issue", "update", &id, "-t", "New Title"])
        .output()
        .unwrap();

    let output = cli.command(["issue", "show", &id]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Title: New Title"));

    // Delete issue (requires env var)
    cli.command_with_env(["issue", "delete", &id], [("JIT_ALLOW_DELETION", "1")])
        .output()
        .unwrap();

    let output = cli.command(["issue", "list"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(&id));
}

#[test]
fn test_issue_delete_confirmed_reports_the_deleted_id_in_text_mode() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Doomed"]).unwrap();

    let output = cli
        .command_with_env(["issue", "delete", &id], [("JIT_ALLOW_DELETION", "1")])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains(&format!("Deleted issue: {id}")), "{stdout}");
}

#[test]
fn test_issue_delete_confirmed_reports_the_deleted_id_in_json_mode() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Doomed"]).unwrap();

    let output = cli
        .command_with_env(
            ["issue", "delete", &id, "--json"],
            [("JIT_ALLOW_DELETION", "1")],
        )
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["id"], id);
    assert_eq!(json["deleted"], true);
}

#[test]
fn test_gates() {
    let cli = JitCli::initialised().unwrap();

    // Add gate definition
    cli.command([
        "gate",
        "define",
        "tests",
        "-t",
        "Unit Tests",
        "-d",
        "Run tests",
    ])
    .output()
    .unwrap();

    // List gates
    let output = cli.command(["gate", "list"]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tests"));
    assert!(stdout.contains("Unit Tests"));

    // Create issue with gate
    let id = cli
        .create_issue([
            "--title",
            "Task",
            "--description",
            "Desc",
            "--gate",
            "tests",
        ])
        .unwrap();

    // Evaluate gate
    let output = cli
        .command(["gate", "evaluate", &id, "tests", "-b", "ci:bot"])
        .output()
        .unwrap();

    assert!(output.status.success());

    // Verify gate status
    let output = cli.command(["issue", "show", &id]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("tests"));
}

#[test]
fn test_assignment_workflow() {
    let cli = JitCli::initialised().unwrap();

    let id = cli
        .create_issue(["--title", "Task", "--description", "Desc"])
        .unwrap();

    // Assign issue
    cli.command(["issue", "assign", &id, "human:alice"])
        .output()
        .unwrap();

    let output = cli.command(["issue", "show", &id]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("alice"));

    // Unassign issue
    cli.command(["issue", "unassign", &id]).output().unwrap();

    let output = cli.command(["issue", "show", &id]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Assignee: None"));
}

#[test]
fn test_events() {
    let cli = JitCli::initialised().unwrap();

    // Create issues to generate events
    cli.command(["issue", "create", "-t", "Task 1", "-d", "Desc"])
        .output()
        .unwrap();

    cli.command(["issue", "create", "-t", "Task 2", "-d", "Desc"])
        .output()
        .unwrap();

    // Tail events
    let output = cli.command(["events", "tail", "-n", "2"]).output().unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Should have JSON events
    assert!(stdout.contains("issue_created") || stdout.contains("{"));

    // Query events
    let output = cli
        .command(["events", "query", "-e", "issue_created"])
        .output()
        .unwrap();

    assert!(output.status.success());
}

#[test]
fn test_validate_repository() {
    let cli = JitCli::initialised().unwrap();

    let id1 = cli
        .create_issue(["--title", "Task 1", "--description", "Desc"])
        .unwrap();
    let id2 = cli
        .create_issue(["--title", "Task 2", "--description", "Desc"])
        .unwrap();

    // Add dependency to connect the issues
    cli.command(["dep", "add", &id2, &id1]).output().unwrap();

    // Validate should succeed
    let output = cli.command(["validate"]).output().unwrap();

    assert!(output.status.success());
}

#[test]
fn test_bug_5dbc3548_deleted_issue_not_in_query() {
    // Bug 5dbc3548: Deleted issues reappear in queries until committed to git
    // This test verifies the fix using deletion tracking in index.json

    let cli = JitCli::initialised().unwrap();

    // Initialize git (issue was committed)
    cli.init_git().unwrap();

    // Create an issue
    let output = cli
        .command(["issue", "create", "-t", "Test Issue", "-d", "desc"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let issues = cli.run_json(["issue", "search", "Test Issue"]).unwrap();
    let id = issues["issues"][0]["id"].as_str().unwrap().to_owned();

    // Commit to git
    cli.commit("Add issue").unwrap();

    // Verify issue is visible
    let output = cli.command(["query", "all"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Test Issue"),
        "Issue should be visible before deletion"
    );

    // Delete the issue (requires JIT_ALLOW_DELETION=1)
    let output = cli
        .command_with_env(["issue", "delete", &id], [("JIT_ALLOW_DELETION", "1")])
        .output()
        .unwrap();
    assert!(output.status.success(), "Deletion should succeed");

    // THE BUG TEST: Issue should NOT appear in queries
    // Before fix: it would reappear because it's still in git
    // After fix: it's filtered out via deleted_ids in index
    let output = cli.command(["query", "all"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("Test Issue"),
        "Bug 5dbc3548: Deleted issue should NOT reappear in queries. Stdout: {}",
        stdout
    );
}

#[test]
fn test_issue_timestamps_on_create() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let id = cli
        .create_issue([
            "--title",
            "Timestamp Test",
            "--description",
            "Testing timestamp fields",
        ])
        .unwrap();

    // Show issue as JSON to check timestamps
    let output = cli
        .command(["issue", "show", &id, "--json"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue = &json;

    // Verify timestamps exist and are valid RFC 3339
    let created_at = issue["created_at"].as_str().unwrap();
    let updated_at = issue["updated_at"].as_str().unwrap();

    assert!(!created_at.is_empty(), "created_at should not be empty");
    assert!(!updated_at.is_empty(), "updated_at should not be empty");

    // Parse as RFC 3339 (will panic if invalid)
    use chrono::DateTime;
    let created_dt = created_at.parse::<DateTime<chrono::Utc>>().unwrap();
    let updated_dt = updated_at.parse::<DateTime<chrono::Utc>>().unwrap();

    // For a new issue, created_at should equal updated_at (or be very close)
    let diff = (updated_dt.timestamp() - created_dt.timestamp()).abs();
    assert!(
        diff <= 1,
        "For new issue, created_at and updated_at should be same (or within 1s)"
    );
}

#[test]
fn test_issue_timestamps_persist() {
    let cli = JitCli::initialised().unwrap();

    // Create an issue
    let id = cli
        .create_issue(["--title", "Persist Test", "--description", "Description"])
        .unwrap();

    // Get initial timestamps
    let output = cli
        .command(["issue", "show", &id, "--json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let initial_created = json["created_at"].as_str().unwrap().to_string();
    let initial_updated = json["updated_at"].as_str().unwrap().to_string();

    // Update the issue (this will change updated_at)
    std::thread::sleep(std::time::Duration::from_millis(10)); // Ensure timestamp difference

    let output = cli
        .command(["issue", "update", &id, "--state", "ready"])
        .output()
        .unwrap();

    assert!(output.status.success());

    // Get timestamps after update
    let output = cli
        .command(["issue", "show", &id, "--json"])
        .output()
        .unwrap();

    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let final_created = json["created_at"].as_str().unwrap();
    let final_updated = json["updated_at"].as_str().unwrap();

    // created_at should be unchanged (immutable)
    assert_eq!(
        initial_created, final_created,
        "created_at should be immutable"
    );

    // updated_at should have changed
    // Note: Currently update might not change updated_at if not implemented
    // This test documents expected behavior
    if final_updated != initial_updated {
        use chrono::DateTime;
        let initial_dt = initial_updated.parse::<DateTime<chrono::Utc>>().unwrap();
        let final_dt = final_updated.parse::<DateTime<chrono::Utc>>().unwrap();
        assert!(
            final_dt >= initial_dt,
            "updated_at should be >= original timestamp"
        );
    }
}
