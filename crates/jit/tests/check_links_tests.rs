use assert_cmd::assert::OutputAssertExt;
use predicates::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

/// Integration tests for jit doc check-links command
///
/// Tests follow acceptance criteria from issue fb6e2e31:
/// 1. Validate all documents in scope
/// 2. Check asset existence (working tree or git)
/// 3. Validate internal doc links resolve
/// 4. Detect broken relative vs root-relative links
/// 5. Scope filtering (all vs issue:ID)
/// 6. Exit codes (0=valid, 1=errors, 2=warnings)
/// 7. JSON output support
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

    fn init_repo(&self) {
        Self::run_git_static(&self.repo_path, &["init"]);
        Self::run_git_static(&self.repo_path, &["config", "user.name", "Test User"]);
        Self::run_git_static(
            &self.repo_path,
            &["config", "user.email", "test@example.com"],
        );
        Self::run_jit_static(&self.repo_path, &["init"]);
    }

    fn repo_path(&self) -> &PathBuf {
        &self.repo_path
    }

    fn run_git_static(path: &PathBuf, args: &[&str]) {
        Command::new("git")
            .current_dir(path)
            .args(args)
            .status()
            .expect("git command failed");
    }

    fn run_git(&self, args: &[&str]) -> std::process::ExitStatus {
        Command::new("git")
            .current_dir(&self.repo_path)
            .args(args)
            .status()
            .expect("git command failed")
    }

    fn run_jit_static(path: &PathBuf, args: &[&str]) {
        Command::new(env!("CARGO_BIN_EXE_jit"))
            .current_dir(path)
            .args(args)
            .status()
            .expect("jit command failed");
    }

    fn run_jit(&self, args: &[&str]) -> assert_cmd::assert::Assert {
        Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(&self.repo_path)
            .args(args)
            .assert()
    }

    fn create_issue(&self, title: &str, description: &str) -> String {
        let output = Command::new(assert_cmd::cargo::cargo_bin!("jit"))
            .current_dir(&self.repo_path)
            .args([
                "issue",
                "create",
                "--title",
                title,
                "--description",
                description,
                "--json",
            ])
            .output()
            .expect("create issue failed");

        if !output.status.success() {
            eprintln!("Failed to create issue:");
            eprintln!("stdout: {}", String::from_utf8_lossy(&output.stdout));
            eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
            panic!("issue creation failed");
        }

        let json: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("Failed to parse JSON from issue create");
        json["id"].as_str().expect("No ID in response").to_string()
    }
}

#[test]
fn test_check_all_valid_links_exits_0() {
    let ctx = TestContext::new();
    ctx.init_repo();

    // Create issue with document that has valid assets
    let issue_id = ctx.create_issue("Test issue", "Test description");

    // Create a markdown document with valid local asset
    let doc_path = "docs/test-doc.md";
    let asset_path = "docs/assets/diagram.png";

    fs::create_dir_all(ctx.repo_path().join("docs/assets")).unwrap();
    fs::write(
        ctx.repo_path().join(doc_path),
        "# Test Doc\n\n![Diagram](assets/diagram.png)\n",
    )
    .unwrap();
    fs::write(ctx.repo_path().join(asset_path), b"fake-png-data").unwrap();

    // Add document with scanned assets
    ctx.run_jit(&["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .success();

    // Check links should pass with exit code 0
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .success()
        .stdout(predicate::str::contains("All documents valid"));
}

#[test]
fn test_missing_assets_exits_1() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Test description");

    // Create document referencing missing asset
    let doc_path = "docs/broken-doc.md";
    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(
        ctx.repo_path().join(doc_path),
        "# Broken Doc\n\n![Missing](assets/missing.png)\n",
    )
    .unwrap();

    ctx.run_jit(&["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .success();

    // Check links should fail with exit code 1
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .code(1)
        .stdout(predicate::str::contains("Errors found"))
        .stdout(predicate::str::contains("missing"));
}

#[test]
fn test_broken_internal_doc_links_exits_1() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Test description");

    // Create document with broken link to another doc
    let doc_path = "docs/doc-with-broken-link.md";
    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(
        ctx.repo_path().join(doc_path),
        "# Doc\n\nSee [missing doc](nonexistent.md) for details.\n",
    )
    .unwrap();

    ctx.run_jit(&["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .success();

    // Check links should fail with exit code 1
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .code(1)
        .stdout(predicate::str::contains("Errors found"))
        .stdout(predicate::str::contains("broken"));
}

#[test]
fn test_risky_relative_links_warns() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Test description");

    // Create document with risky (but valid) relative path
    let doc_path = "docs/subdir/risky-doc.md";
    fs::create_dir_all(ctx.repo_path().join("docs/subdir")).unwrap();
    fs::create_dir_all(ctx.repo_path().join("assets")).unwrap();

    // Deep relative path that exists but is fragile
    fs::write(
        ctx.repo_path().join(doc_path),
        "# Risky\n\n![Asset](../../assets/image.png)\n",
    )
    .unwrap();
    fs::write(ctx.repo_path().join("assets/image.png"), b"data").unwrap();

    ctx.run_jit(&["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .success();

    // Check links should warn with exit code 2
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .code(2)
        .stdout(predicate::str::contains("Warnings"))
        .stdout(predicate::str::contains("risky").or(predicate::str::contains("relative")));
}

#[test]
fn test_scope_filtering_all() {
    let ctx = TestContext::new();
    ctx.init_repo();

    // Create multiple issues with documents
    let issue1 = ctx.create_issue("Issue 1", "Description");
    let issue2 = ctx.create_issue("Issue 2", "Description");

    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(ctx.repo_path().join("docs/doc1.md"), "# Doc 1").unwrap();
    fs::write(ctx.repo_path().join("docs/doc2.md"), "# Doc 2").unwrap();

    ctx.run_jit(&["doc", "add", &issue1, "docs/doc1.md", "--label", "Doc1"])
        .success();
    ctx.run_jit(&["doc", "add", &issue2, "docs/doc2.md", "--label", "Doc2"])
        .success();

    // Scope all should check both documents
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .success()
        .stdout(predicate::str::contains("2 document(s)"));
}

#[test]
fn test_scope_filtering_issue_id() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue1 = ctx.create_issue("Issue 1", "Description");
    let issue2 = ctx.create_issue("Issue 2", "Description");

    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(ctx.repo_path().join("docs/doc1.md"), "# Doc 1").unwrap();
    fs::write(ctx.repo_path().join("docs/doc2.md"), "# Doc 2").unwrap();

    ctx.run_jit(&["doc", "add", &issue1, "docs/doc1.md", "--label", "Doc1"])
        .success();
    ctx.run_jit(&["doc", "add", &issue2, "docs/doc2.md", "--label", "Doc2"])
        .success();

    // Scope issue:ID should check only that issue's documents
    let scope = format!("issue:{}", &issue1[..8]);
    ctx.run_jit(&["doc", "check-links", "--scope", &scope])
        .success()
        .stdout(predicate::str::contains("1 document(s)"));
}

#[test]
fn test_json_output_structure() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Description");

    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(ctx.repo_path().join("docs/test.md"), "# Test").unwrap();

    ctx.run_jit(&["doc", "add", &issue_id, "docs/test.md", "--label", "Test"])
        .success();

    // JSON output should have expected structure
    let output = ctx
        .run_jit(&["doc", "check-links", "--scope", "all", "--json"])
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();

    // Validate JSON structure
    assert!(json["success"].as_bool().unwrap());
    assert!(json["data"]["valid"].is_boolean());
    assert!(json["data"]["errors"].is_array());
    assert!(json["data"]["warnings"].is_array());
    assert!(json["data"]["summary"].is_object());
    assert!(json["data"]["summary"]["total_documents"].is_number());
}

#[test]
fn test_invalid_scope_format() {
    let ctx = TestContext::new();
    ctx.init_repo();

    // Invalid scope should error
    ctx.run_jit(&["doc", "check-links", "--scope", "invalid"])
        .failure()
        .stderr(predicate::str::contains("Invalid scope"));
}

#[test]
fn test_empty_repository_no_documents() {
    let ctx = TestContext::new();
    ctx.init_repo();

    // No documents should exit cleanly
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .success()
        .stdout(
            predicate::str::contains("No documents found")
                .or(predicate::str::contains("0 document")),
        );
}

#[test]
fn test_external_urls_generate_warnings() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Description");

    // Document with external URL
    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(
        ctx.repo_path().join("docs/external.md"),
        "# External\n\n![Remote](https://example.com/image.png)\n",
    )
    .unwrap();

    ctx.run_jit(&[
        "doc",
        "add",
        &issue_id,
        "docs/external.md",
        "--label",
        "Test",
    ])
    .success();

    // External URLs should warn but not error
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .code(2)
        .stdout(predicate::str::contains("Warnings"))
        .stdout(predicate::str::contains("external").or(predicate::str::contains("URL")));
}

#[test]
fn test_missing_document_file() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Description");

    // Create and add document
    fs::create_dir_all(ctx.repo_path().join("docs")).unwrap();
    fs::write(ctx.repo_path().join("docs/temp.md"), "# Temp").unwrap();

    ctx.run_jit(&["doc", "add", &issue_id, "docs/temp.md", "--label", "Test"])
        .success();

    // Delete the document file
    fs::remove_file(ctx.repo_path().join("docs/temp.md")).unwrap();

    // Check should report missing document
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .code(1)
        .stdout(
            predicate::str::contains("missing_document").or(predicate::str::contains("not found")),
        );
}

#[test]
fn test_git_versioned_asset_exists() {
    let ctx = TestContext::new();
    ctx.init_repo();

    let issue_id = ctx.create_issue("Test issue", "Description");

    // Create document with asset, commit it
    fs::create_dir_all(ctx.repo_path().join("docs/assets")).unwrap();
    let doc_path = "docs/versioned.md";
    let asset_path = "docs/assets/old.png";

    fs::write(
        ctx.repo_path().join(doc_path),
        "# Versioned\n\n![Old](assets/old.png)\n",
    )
    .unwrap();
    fs::write(ctx.repo_path().join(asset_path), b"old-data").unwrap();

    // Commit to git
    assert!(ctx.run_git(&["add", "."]).success());
    assert!(ctx
        .run_git(&["commit", "-m", "Add document with asset"])
        .success());

    // Add document reference
    ctx.run_jit(&["doc", "add", &issue_id, doc_path, "--label", "Test"])
        .success();

    // Remove asset from working tree
    fs::remove_file(ctx.repo_path().join(asset_path)).unwrap();

    // Check should pass because asset exists in git
    ctx.run_jit(&["doc", "check-links", "--scope", "all"])
        .success()
        .stdout(predicate::str::contains("valid"));
}
