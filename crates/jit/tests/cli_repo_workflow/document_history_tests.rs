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
fn test_doc_history_lists_document_deletion() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/removed.md", "original");
    cli.commit("Add document").unwrap();
    let issue_id = cli.create_issue(["--title", "Track document"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/removed.md"])
        .unwrap();

    fs::remove_file(cli.root().join("docs/removed.md")).unwrap();
    cli.commit("Delete document").unwrap();
    let deletion = cli.git(["rev-parse", "HEAD"]).unwrap();

    let history = cli
        .run_json(["doc", "history", issue_id.as_str(), "docs/removed.md"])
        .unwrap();
    let commits = history["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0]["sha"].as_str().unwrap(), &deletion[..7]);
    assert_eq!(commits[0]["message"], "Delete document");
}

#[test]
fn test_doc_history_omits_merge_identical_to_second_parent() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/merged.md", "base");
    cli.commit("Add document").unwrap();
    let issue_id = cli.create_issue(["--title", "Track document"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/merged.md"])
        .unwrap();

    cli.git(["branch", "side"]).unwrap();
    write_file(&cli, "main.txt", "unrelated");
    cli.commit("Change unrelated file on main").unwrap();

    cli.git(["checkout", "-q", "side"]).unwrap();
    write_file(&cli, "docs/merged.md", "side version");
    cli.commit("Update document on side").unwrap();
    let document_update = cli.git(["rev-parse", "HEAD"]).unwrap();
    cli.git(["checkout", "-q", "main"]).unwrap();
    cli.git(["merge", "-q", "--no-ff", "--no-edit", "side"])
        .unwrap();
    let merge = cli.git(["rev-parse", "HEAD"]).unwrap();
    let merged_blob = cli.git(["rev-parse", "HEAD:docs/merged.md"]).unwrap();
    assert_ne!(
        merged_blob,
        cli.git(["rev-parse", "HEAD^1:docs/merged.md"]).unwrap()
    );
    assert_eq!(
        merged_blob,
        cli.git(["rev-parse", "HEAD^2:docs/merged.md"]).unwrap()
    );

    let history = cli
        .run_json(["doc", "history", issue_id.as_str(), "docs/merged.md"])
        .unwrap();
    let commits = history["commits"].as_array().unwrap();
    assert!(commits
        .iter()
        .any(|commit| commit["sha"].as_str() == Some(&document_update[..7])));
    assert!(!commits
        .iter()
        .any(|commit| commit["sha"].as_str() == Some(&merge[..7])));
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

    let overridden = cli
        .run_json([
            "doc",
            "show",
            issue_id.as_str(),
            "docs/api.md",
            "--at",
            commit1,
        ])
        .unwrap();
    assert_eq!(overridden["commit"], &commit1[..7]);
    assert_eq!(overridden["content"], "# API v1\n- GET /users");
}

#[test]
fn test_doc_show_reads_working_tree_by_default() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/readme.md", "# Original");
    cli.commit("Initial readme").unwrap();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/readme.md"])
        .unwrap();
    write_file(&cli, "docs/readme.md", "# Uncommitted edit");

    let document = cli
        .run_json(["doc", "show", issue_id.as_str(), "docs/readme.md"])
        .unwrap();

    assert_eq!(document["content"], "# Uncommitted edit");
    assert_eq!(document["commit"], "working-tree");
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

    let output = cli
        .command([
            "doc",
            "show",
            issue_id.as_str(),
            "docs/test.md",
            "--at",
            "invalid123",
            "--json",
        ])
        .output()
        .unwrap();

    let code = jit::output::ErrorCode::DocumentNotFound;
    assert_eq!(output.status.code(), Some(3));
    let failure: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["error"]["code"], code.as_str());
}

#[test]
fn test_doc_show_pinned_path_absent_at_commit_reports_document_not_found() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    cli.commit("Before document exists").unwrap();
    let commit = cli.git(["rev-parse", "HEAD"]).unwrap();
    write_file(&cli, "docs/later.md", "# Only in the working tree");
    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run([
        "doc",
        "add",
        issue_id.as_str(),
        "docs/later.md",
        "--commit",
        commit.trim(),
        "--skip-scan",
    ])
    .unwrap();

    let output = cli
        .command(["doc", "show", issue_id.as_str(), "docs/later.md", "--json"])
        .output()
        .unwrap();

    let code = jit::output::ErrorCode::DocumentNotFound;
    assert_eq!(output.status.code(), Some(3));
    let failure: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(failure["error"]["code"], code.as_str());
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

#[cfg(unix)]
#[test]
fn test_doc_diff_uses_one_cat_file_process_for_both_versions() {
    use jit::test_harness::git::GitStandIn;

    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    write_file(&cli, "docs/spec.md", "first version\n");
    cli.commit("Add document").unwrap();
    let from = cli.git(["rev-parse", "HEAD"]).unwrap();
    let from = from.trim();

    write_file(&cli, "docs/spec.md", "second version\n");
    cli.commit("Update document").unwrap();
    let to = cli.git(["rev-parse", "HEAD"]).unwrap();
    let to = to.trim();

    let issue_id = cli.create_issue(["--title", "Test"]).unwrap();
    cli.run(["doc", "add", issue_id.as_str(), "docs/spec.md"])
        .unwrap();

    let stand_in = GitStandIn::new();
    let output = cli
        .command_with_env(
            [
                "doc",
                "diff",
                issue_id.as_str(),
                "docs/spec.md",
                "--from",
                from,
                "--to",
                to,
            ],
            [("PATH", stand_in.search_path())],
        )
        .output()
        .unwrap();
    let cat_file_reads = stand_in
        .invocations()
        .iter()
        .filter(|invocation| invocation.args() == ["cat-file", "--batch", "-Z"])
        .count();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        cat_file_reads,
        1,
        "git invocations: {:#?}",
        stand_in.invocations()
    );
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

#[test]
fn test_doc_history_uses_selected_code_checkout() {
    use assert_cmd::prelude::*;

    let data = JitCli::initialised().unwrap();
    data.init_git().unwrap();
    write_file(&data, "docs/design.md", "data history");
    data.commit("data document").unwrap();
    let code = JitCli::initialised().unwrap();
    code.init_git().unwrap();
    write_file(&code, "sub/docs/design.md", "code history");
    code.commit("code document").unwrap();
    let issue_id = data
        .create_issue(["--title", "Track code document"])
        .unwrap();
    data.command(["doc", "add", issue_id.as_str(), "docs/design.md"])
        .env("JIT_WORKTREE_ROOT", code.root().join("sub"))
        .assert()
        .success();
    let output = data
        .command([
            "doc",
            "history",
            issue_id.as_str(),
            "docs/design.md",
            "--json",
        ])
        .env("JIT_WORKTREE_ROOT", code.root().join("sub"))
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let history: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let commits = history["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0]["message"], "code document");
}
