//! Integration tests for `jit validate` document reference checking.
//!
//! Covers the case where a document is linked to an issue but not yet
//! committed — validate should pass as long as the file exists in the
//! working tree.

use jit::test_cli::JitCli;
use std::fs;

/// A document linked to an issue exists only in the working tree (not yet
/// committed). `jit validate` should succeed.
#[test]
fn test_validate_passes_for_uncommitted_document() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    // An initial commit so HEAD exists.
    fs::write(cli.root().join("README.md"), "# Repo\n").unwrap();
    cli.commit("initial commit").unwrap();

    let issue_id = cli.create_issue(["--title", "Task with doc"]).unwrap();
    let doc_path = "docs/design.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "# Design\n").unwrap();
    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();

    cli.run(["validate"])
        .expect("validate passes with the document only in the working tree");
}

/// A filesystem validation may resolve an unpinned document from Git `HEAD`
/// when it is absent from the working tree.
#[test]
fn test_validate_passes_for_document_only_in_head() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();

    let issue_id = cli
        .create_issue(["--title", "Task with tracked doc"])
        .unwrap();
    let doc_path = "docs/tracked.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "# Tracked\n").unwrap();
    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();
    cli.commit("tracked document").unwrap();
    fs::remove_file(cli.root().join(doc_path)).unwrap();

    cli.run(["validate"])
        .expect("validate resolves the document from HEAD");
}

/// `jit doc add` on an `.html` file stores `format == "html"` in the issue.
#[test]
fn test_doc_add_html_stores_format_html() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    fs::write(cli.root().join("README.md"), "# Repo\n").unwrap();
    cli.commit("initial commit").unwrap();

    let issue_id = cli.create_issue(["--title", "HTML doc task"]).unwrap();
    let doc_path = "docs/index.html";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(
        cli.root().join(doc_path),
        "<!DOCTYPE html><html><body><p>Hello</p></body></html>",
    )
    .unwrap();
    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();

    let json = cli.run_json(["issue", "show", &issue_id]).unwrap();
    let format = json["documents"][0]["format"]
        .as_str()
        .expect("format field missing");
    assert_eq!(format, "html", "expected format 'html', got '{format}'");
}

/// A document linked to an issue doesn't exist anywhere (not in working tree,
/// not in git). `jit validate` should fail.
#[test]
fn test_validate_fails_for_missing_document() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap();
    fs::write(cli.root().join("README.md"), "# Repo\n").unwrap();
    cli.commit("initial commit").unwrap();

    let issue_id = cli
        .create_issue(["--title", "Task with broken doc"])
        .unwrap();
    // A temporary file lets `doc add` succeed; removing it leaves neither HEAD
    // nor the working tree holding the document.
    let doc_path = "docs/ghost.md";
    fs::create_dir_all(cli.root().join("docs")).unwrap();
    fs::write(cli.root().join(doc_path), "temp").unwrap();
    cli.run(["doc", "add", &issue_id, doc_path]).unwrap();
    fs::remove_file(cli.root().join(doc_path)).unwrap();

    let output = cli.command(["validate"]).output().unwrap();
    assert!(
        !output.status.success(),
        "validate should fail for missing document"
    );
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        combined.contains("not found"),
        "expected 'not found' in output, got: {combined}"
    );
}
