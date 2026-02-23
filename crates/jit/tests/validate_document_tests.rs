//! Integration tests for `jit validate` document reference checking.
//!
//! Covers the case where a document is linked to an issue but not yet
//! committed — validate should pass as long as the file exists in the
//! working tree.

use assert_cmd::assert::OutputAssertExt;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

struct TestContext {
    #[allow(dead_code)]
    temp_dir: TempDir,
    repo_path: PathBuf,
}

impl TestContext {
    fn new() -> Self {
        let temp_dir = TempDir::new().expect("create temp dir");
        let repo_path = temp_dir.path().to_path_buf();
        Self {
            temp_dir,
            repo_path,
        }
    }

    fn init_git(&self) {
        self.git(&["init"]);
        self.git(&["config", "user.name", "Test User"]);
        self.git(&["config", "user.email", "test@example.com"]);
    }

    fn git(&self, args: &[&str]) {
        Command::new("git")
            .current_dir(&self.repo_path)
            .args(args)
            .status()
            .expect("git command failed");
    }

    fn jit(&self, args: &[&str]) -> std::process::Output {
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(&self.repo_path)
            .args(args)
            .output()
            .expect("jit command failed")
    }

    fn jit_assert(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(&self.repo_path)
            .args(args)
            .assert()
    }

    fn create_issue(&self, title: &str) -> String {
        let output = self.jit(&["issue", "create", "--title", title, "--json"]);
        assert!(output.status.success(), "issue create failed");
        let json: serde_json::Value = serde_json::from_slice(&output.stdout).expect("parse JSON");
        json["id"].as_str().expect("no id").to_string()
    }

    /// Commit all current files so HEAD exists.
    fn commit_all(&self, message: &str) {
        self.git(&["add", "."]);
        self.git(&["commit", "-m", message]);
    }
}

/// A document linked to an issue exists only in the working tree (not yet
/// committed). `jit validate` should succeed.
#[test]
fn test_validate_passes_for_uncommitted_document() {
    let ctx = TestContext::new();
    ctx.init_git();
    ctx.jit_assert(&["init"]).success();

    // Make an initial commit so HEAD exists.
    fs::write(ctx.repo_path.join("README.md"), "# Repo\n").unwrap();
    ctx.commit_all("initial commit");

    // Create issue and a document file (NOT committed).
    let issue_id = ctx.create_issue("Task with doc");
    let doc_path = "docs/design.md";
    fs::create_dir_all(ctx.repo_path.join("docs")).unwrap();
    fs::write(ctx.repo_path.join(doc_path), "# Design\n").unwrap();

    // Link the document to the issue (still uncommitted).
    ctx.jit_assert(&["doc", "add", &issue_id, doc_path])
        .success();

    // Validate should pass even though the document is not yet committed.
    ctx.jit_assert(&["validate"]).success();
}

/// A document linked to an issue doesn't exist anywhere (not in working tree,
/// not in git). `jit validate` should fail.
#[test]
fn test_validate_fails_for_missing_document() {
    let ctx = TestContext::new();
    ctx.init_git();
    ctx.jit_assert(&["init"]).success();

    // Make an initial commit so HEAD exists.
    fs::write(ctx.repo_path.join("README.md"), "# Repo\n").unwrap();
    ctx.commit_all("initial commit");

    // Manually inject a document reference pointing to a non-existent file.
    let issue_id = ctx.create_issue("Task with broken doc");

    // Create a temporary doc file to allow `doc add` to succeed, then delete it.
    let doc_path = "docs/ghost.md";
    fs::create_dir_all(ctx.repo_path.join("docs")).unwrap();
    fs::write(ctx.repo_path.join(doc_path), "temp").unwrap();
    ctx.jit_assert(&["doc", "add", &issue_id, doc_path])
        .success();

    // Remove the file — now neither HEAD nor working tree has it.
    fs::remove_file(ctx.repo_path.join(doc_path)).unwrap();

    // Validate should fail.
    let output = ctx.jit(&["validate"]);
    assert!(
        !output.status.success(),
        "validate should fail for missing document"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{}{}", stderr, stdout);
    assert!(
        combined.contains("not found"),
        "expected 'not found' in output, got: {combined}"
    );
}
