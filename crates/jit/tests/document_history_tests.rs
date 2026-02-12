//! Integration tests for document history features
//!
//! Tests for:
//! - `jit doc history` - List commits for a document
//! - `jit doc show --at` - View document at specific commit
//! - `jit doc diff` - Compare document versions

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Test helper: Initialize a git repository with test commits
struct GitTestRepo {
    #[allow(dead_code)]
    temp_dir: TempDir,
    repo_path: std::path::PathBuf,
}

impl GitTestRepo {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("create temp dir");
        let repo_path = temp_dir.path().to_path_buf();

        // Initialize git repo
        Self::run_git(&repo_path, &["init"]);
        Self::run_git(&repo_path, &["config", "user.name", "Test User"]);
        Self::run_git(&repo_path, &["config", "user.email", "test@example.com"]);

        Self {
            temp_dir,
            repo_path,
        }
    }

    fn run_git(path: &Path, args: &[&str]) {
        let status = Command::new("git")
            .current_dir(path)
            .args(args)
            .status()
            .expect("git command failed");
        assert!(status.success(), "git command failed: {:?}", args);
    }

    fn run_git_output(path: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .current_dir(path)
            .args(args)
            .output()
            .expect("git command failed");
        assert!(output.status.success(), "git command failed: {:?}", args);
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn write_file(&self, path: &str, content: &str) {
        let full_path = self.repo_path.join(path);
        if let Some(parent) = full_path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(full_path, content).expect("write file");
    }

    fn commit(&self, message: &str) {
        Self::run_git(&self.repo_path, &["add", "-A"]);
        Self::run_git(&self.repo_path, &["commit", "-m", message]);
    }

    fn get_commit_hash(&self, rev: &str) -> String {
        Self::run_git_output(&self.repo_path, &["rev-parse", rev])
    }

    fn path(&self) -> &Path {
        &self.repo_path
    }

    fn init_jit(&self) {
        let status = Command::new(env!("CARGO_BIN_EXE_jit"))
            .current_dir(self.path())
            .arg("init")
            .status()
            .expect("jit init failed");
        assert!(status.success());
    }

    fn run_jit(&self, args: &[&str]) -> std::process::Output {
        Command::new(env!("CARGO_BIN_EXE_jit"))
            .current_dir(self.path())
            .args(args)
            .output()
            .expect("jit command failed")
    }

    fn run_jit_json(&self, args: &[&str]) -> serde_json::Value {
        let mut full_args = args.to_vec();
        full_args.push("--json");
        let output = self.run_jit(&full_args);
        assert!(
            output.status.success(),
            "jit command failed: {} - {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("parse JSON output")
    }
}

#[test]
fn test_doc_history_lists_commits() {
    let repo = GitTestRepo::new();
    repo.init_jit();

    // Create document with multiple commits
    repo.write_file("docs/design.md", "# Design v1\nInitial design");
    repo.commit("Initial design document");
    let commit1 = repo.get_commit_hash("HEAD");

    repo.write_file("docs/design.md", "# Design v2\nAdded auth section");
    repo.commit("Add authentication to design");
    let commit2 = repo.get_commit_hash("HEAD");

    repo.write_file("docs/design.md", "# Design v3\nAdded API endpoints");
    repo.commit("Add API endpoints to design");
    let commit3 = repo.get_commit_hash("HEAD");

    // Create issue with document reference
    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Implement auth"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();

    repo.run_jit(&["doc", "add", issue_id, "docs/design.md"]);

    // Test: Get document history
    let history_output = repo.run_jit_json(&["doc", "history", issue_id, "docs/design.md"]);

    // Should return wrapped result with commits array
    assert!(history_output["success"].as_bool().unwrap());
    let commits = history_output["data"]["commits"].as_array().unwrap();
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
    let repo = GitTestRepo::new();
    repo.init_jit();

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();

    // Try to get history for document not referenced in issue
    let output = repo.run_jit(&["doc", "history", issue_id, "docs/missing.md"]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("not found") || stderr.contains("Document reference"));
}

#[test]
fn test_doc_show_at_specific_commit() {
    let repo = GitTestRepo::new();
    repo.init_jit();

    // Create document with evolving content
    repo.write_file("docs/api.md", "# API v1\n- GET /users");
    repo.commit("Add users endpoint");
    let commit1 = repo.get_commit_hash("HEAD");

    repo.write_file("docs/api.md", "# API v2\n- GET /users\n- POST /users");
    repo.commit("Add POST endpoint");

    repo.write_file(
        "docs/api.md",
        "# API v3\n- GET /users\n- POST /users\n- DELETE /users",
    );
    repo.commit("Add DELETE endpoint");

    // Create issue with document reference
    let create_output = repo.run_jit_json(&["issue", "create", "--title", "API work"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();
    repo.run_jit(&["doc", "add", issue_id, "docs/api.md"]);

    // Test: View document at old commit
    let output = repo.run_jit(&["doc", "show", issue_id, "docs/api.md", "--at", &commit1]);

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
    let repo = GitTestRepo::new();
    repo.init_jit();

    repo.write_file("docs/readme.md", "# Original");
    repo.commit("Initial readme");

    repo.write_file("docs/readme.md", "# Updated");
    repo.commit("Update readme");

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();
    repo.run_jit(&["doc", "add", issue_id, "docs/readme.md"]);

    // Without --at flag, should show HEAD
    let output = repo.run_jit(&["doc", "show", issue_id, "docs/readme.md"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Updated"));
    assert!(!stdout.contains("Original"));
}

#[test]
fn test_doc_show_at_invalid_commit() {
    let repo = GitTestRepo::new();
    repo.init_jit();

    repo.write_file("docs/test.md", "content");
    repo.commit("Add test doc");

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();
    repo.run_jit(&["doc", "add", issue_id, "docs/test.md"]);

    // Invalid commit hash
    let output = repo.run_jit(&[
        "doc",
        "show",
        issue_id,
        "docs/test.md",
        "--at",
        "invalid123",
    ]);

    assert!(!output.status.success());
}

#[test]
fn test_doc_diff_between_commits() {
    let repo = GitTestRepo::new();
    repo.init_jit();

    // Version 1
    repo.write_file("docs/spec.md", "# Spec\n\n## Feature A\nDescription A");
    repo.commit("Add feature A");
    let commit1 = repo.get_commit_hash("HEAD");

    // Version 2
    repo.write_file(
        "docs/spec.md",
        "# Spec\n\n## Feature A\nDescription A\n\n## Feature B\nDescription B",
    );
    repo.commit("Add feature B");
    let commit2 = repo.get_commit_hash("HEAD");

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();
    repo.run_jit(&["doc", "add", issue_id, "docs/spec.md"]);

    // Test: Diff between commits
    let output = repo.run_jit(&[
        "doc",
        "diff",
        issue_id,
        "docs/spec.md",
        "--from",
        &commit1,
        "--to",
        &commit2,
    ]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show diff
    assert!(stdout.contains("+++") || stdout.contains("---") || stdout.contains("diff"));
    assert!(stdout.contains("Feature B") || stdout.contains("+## Feature B"));
}

#[test]
fn test_doc_diff_from_old_to_head() {
    let repo = GitTestRepo::new();
    repo.init_jit();

    repo.write_file("docs/plan.md", "# Plan v1");
    repo.commit("Initial plan");
    let commit1 = repo.get_commit_hash("HEAD");

    repo.write_file("docs/plan.md", "# Plan v2\nMore details");
    repo.commit("Update plan");

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();
    repo.run_jit(&["doc", "add", issue_id, "docs/plan.md"]);

    // Diff from old commit to HEAD (implicit)
    let output = repo.run_jit(&["doc", "diff", issue_id, "docs/plan.md", "--from", &commit1]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("v2") || stdout.contains("More details"));
}

#[test]
fn test_doc_history_json_output() {
    let repo = GitTestRepo::new();
    repo.init_jit();

    repo.write_file("docs/doc.md", "v1");
    repo.commit("First version");

    repo.write_file("docs/doc.md", "v2");
    repo.commit("Second version");

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();
    repo.run_jit(&["doc", "add", issue_id, "docs/doc.md"]);

    // JSON output for history
    let history_output = repo.run_jit_json(&["doc", "history", issue_id, "docs/doc.md"]);

    assert!(history_output["success"].as_bool().unwrap());
    let commits = history_output["data"]["commits"].as_array().unwrap();
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
    let repo = GitTestRepo::new();
    repo.init_jit();

    // Create file but don't commit yet
    repo.write_file("docs/new.md", "content");

    let create_output = repo.run_jit_json(&["issue", "create", "--title", "Test"]);
    let issue_id = create_output["data"]["id"].as_str().unwrap();

    // Can't add document reference to uncommitted file
    let output = repo.run_jit(&["doc", "add", issue_id, "docs/new.md"]);

    // This should work - doc add doesn't validate git
    assert!(output.status.success());

    // But history should fail or return empty
    let output = repo.run_jit(&["doc", "history", issue_id, "docs/new.md"]);

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
