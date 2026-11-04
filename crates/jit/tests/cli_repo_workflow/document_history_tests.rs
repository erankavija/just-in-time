//! Integration tests for document history features
//!
//! Tests for:
//! - `jit doc history` - List commits for a document
//! - `jit doc show --at` - View document at specific commit
//! - `jit doc diff` - Compare document versions

use jit::test_cli::JitCli;
use std::fs;

fn write_file(cli: &JitCli, path: &str, content: &str) {
    let full_path = cli.root().join(path);
    fs::create_dir_all(full_path.parent().expect("a file sits in a directory"))
        .expect("create parent dirs");
    fs::write(full_path, content).expect("write file");
}

#[test]
fn test_doc_history_lists_commits() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    // Create document with multiple commits
    write_file(&cli, "docs/design.md", "# Design v1\nInitial design");
    cli.commit("Initial design document").unwrap();
    let commit1 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit1 = commit1.trim();

    write_file(&cli, "docs/design.md", "# Design v2\nAdded auth section");
    cli.commit("Add authentication to design").unwrap();
    let commit2 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit2 = commit2.trim();

    write_file(&cli, "docs/design.md", "# Design v3\nAdded API endpoints");
    cli.commit("Add API endpoints to design").unwrap();
    let commit3 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit3 = commit3.trim();

    // Create issue with document reference
    let issue_id = cli.create_issue(["--title", "Implement auth"]).unwrap();

    cli.run(["doc", "add", issue_id.as_str(), "docs/design.md"])
        .unwrap();

    // Test: Get document history
    let history_output = cli
        .run_json(["doc", "history", issue_id.as_str(), "docs/design.md"])
        .unwrap();

    // Should return wrapped result with commits array
    let commits = history_output["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 3, "Expected 3 commits");

    // Verify commit information (most recent first)
    assert_eq!(commits[0]["sha"].as_str().unwrap(), &commit3[..7]);
    assert!(commits[0]["message"]
        .as_str()
        .unwrap()
        .contains("API endpoints"));
    assert!(commits[0]["author"].as_str().is_some());
    assert!(commits[0]["date"].as_str().is_some());

    assert_eq!(commits[1]["sha"].as_str().unwrap(), &commit2[..7]);
    assert!(commits[1]["message"]
        .as_str()
        .unwrap()
        .contains("authentication"));

    assert_eq!(commits[2]["sha"].as_str().unwrap(), &commit1[..7]);
    assert!(commits[2]["message"]
        .as_str()
        .unwrap()
        .contains("Initial design"));
}

#[test]
fn test_doc_history_nonexistent_document() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();

    // Try to get history for document not referenced in issue
    let output = cli
        .command(["doc", "history", issue_id.as_str(), "docs/missing.md"])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not found") || stderr.contains("Document reference"));
}

#[test]
fn test_doc_show_at_specific_commit() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    // Create document with evolving content
    write_file(&cli, "docs/api.md", "# API v1\n- GET /users");
    cli.commit("Add users endpoint").unwrap();
    let commit1 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit1 = commit1.trim();

    write_file(&cli, "docs/api.md", "# API v2\n- GET /users\n- POST /users");
    cli.commit("Add POST endpoint").unwrap();

    write_file(
        &cli,
        "docs/api.md",
        "# API v3\n- GET /users\n- POST /users\n- DELETE /users",
    );
    cli.commit("Add DELETE endpoint").unwrap();

    // Create issue with document reference
    let issue_id = cli.create_issue(["--title", "API work"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/api.md"])
        .unwrap();

    // Test: View document at old commit
    let output = cli
        .command([
            "doc",
            "show",
            issue_id.as_str(),
            "docs/api.md",
            "--at",
            commit1,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show v1 content
    assert!(stdout.contains("API v1"));
    assert!(stdout.contains("GET /users"));
    assert!(!stdout.contains("POST /users"));
    assert!(!stdout.contains("DELETE /users"));
}

#[test]
fn test_doc_show_at_head_by_default() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/readme.md", "# Original");
    cli.commit("Initial readme").unwrap();

    write_file(&cli, "docs/readme.md", "# Updated");
    cli.commit("Update readme").unwrap();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/readme.md"])
        .unwrap();

    // Without --at flag, should show HEAD
    let output = cli
        .command(["doc", "show", issue_id.as_str(), "docs/readme.md"])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Updated"));
    assert!(!stdout.contains("Original"));
}

#[test]
fn test_doc_show_at_invalid_commit() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/test.md", "content");
    cli.commit("Add test doc").unwrap();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/test.md"])
        .unwrap();

    // Invalid commit hash
    let output = cli
        .command([
            "doc",
            "show",
            issue_id.as_str(),
            "docs/test.md",
            "--at",
            "invalid123",
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
}

#[test]
fn test_doc_diff_between_commits() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    // Version 1
    write_file(
        &cli,
        "docs/spec.md",
        "# Spec\n\n## Feature A\nDescription A",
    );
    cli.commit("Add feature A").unwrap();
    let commit1 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit1 = commit1.trim();

    // Version 2
    write_file(
        &cli,
        "docs/spec.md",
        "# Spec\n\n## Feature A\nDescription A\n\n## Feature B\nDescription B",
    );
    cli.commit("Add feature B").unwrap();
    let commit2 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit2 = commit2.trim();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/spec.md"])
        .unwrap();

    // Test: Diff between commits
    let output = cli
        .command([
            "doc",
            "diff",
            issue_id.as_str(),
            "docs/spec.md",
            "--from",
            commit1,
            "--to",
            commit2,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show diff
    assert!(stdout.contains("+++") || stdout.contains("---") || stdout.contains("diff"));
    assert!(stdout.contains("Feature B") || stdout.contains("+## Feature B"));
}

#[test]
fn test_doc_diff_from_old_to_head() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/plan.md", "# Plan v1");
    cli.commit("Initial plan").unwrap();
    let commit1 = cli.git(["rev-parse", "HEAD"]).unwrap();
    let commit1 = commit1.trim();

    write_file(&cli, "docs/plan.md", "# Plan v2\nMore details");
    cli.commit("Update plan").unwrap();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/plan.md"])
        .unwrap();

    // Diff from old commit to HEAD (implicit)
    let output = cli
        .command([
            "doc",
            "diff",
            issue_id.as_str(),
            "docs/plan.md",
            "--from",
            commit1,
        ])
        .output()
        .unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("v2") || stdout.contains("More details"));
}

#[test]
fn test_doc_history_json_output() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/doc.md", "v1");
    cli.commit("First version").unwrap();

    write_file(&cli, "docs/doc.md", "v2");
    cli.commit("Second version").unwrap();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/doc.md"])
        .unwrap();

    // JSON output for history
    let history_output = cli
        .run_json(["doc", "history", issue_id.as_str(), "docs/doc.md"])
        .unwrap();

    let commits = history_output["commits"].as_array().unwrap();
    assert!(commits.len() >= 2);

    // Validate structure
    for commit in commits {
        assert!(commit["sha"].is_string());
        assert!(commit["author"].is_string());
        assert!(commit["date"].is_string());
        assert!(commit["message"].is_string());
    }
}

#[test]
fn test_doc_history_empty_for_new_file() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    // Create file but don't commit yet
    write_file(&cli, "docs/new.md", "content");

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();

    // Can't add document reference to uncommitted file
    // This should work - doc add doesn't validate git
    cli.run(["doc", "add", issue_id.as_str(), "docs/new.md"])
        .unwrap();

    // But history should fail or return empty
    let output = cli
        .command(["doc", "history", issue_id.as_str(), "docs/new.md"])
        .output()
        .unwrap();

    // Either fails or returns empty array
    if output.status.success() {
        let history = serde_json::from_slice::<serde_json::Value>(&output.stdout);
        if let Ok(json) = history {
            if let Some(arr) = json.as_array() {
                assert!(
                    arr.is_empty(),
                    "History should be empty for uncommitted file"
                );
            }
        }
    }
}
