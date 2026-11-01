//! CLI coverage for verbatim issue descriptions supplied outside argv.
//!
//! The file forms deliberately exercise the same source-reading contract as
//! `issue update`: `-` means stdin, and content survives without trimming.

use jit::test_cli::{created_issue_id, JitCli};
use std::fs;

#[test]
fn test_create_description_file_preserves_special_bytes_and_trailing_blank_line() {
    let temp = JitCli::initialised().unwrap();
    let source = b"# Markdown\n\n`literal` and $HOME\n\n";
    let path = temp.root().join("description.md");
    fs::write(&path, source).unwrap();

    let created = temp
        .run_json([
            "issue",
            "create",
            "--title",
            "From file",
            "--description-file",
            path.to_str().unwrap(),
        ])
        .unwrap();
    assert_eq!(
        temp.run_json(["issue", "show", &created_issue_id(&created).unwrap()])
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .as_bytes(),
        source
    );
}

#[test]
fn test_create_description_file_dash_preserves_stdin_bytes_and_trailing_blank_line() {
    let temp = JitCli::initialised().unwrap();
    let source = "## From stdin\n\n`literal` and $HOME\n\n";

    let output = assert_cmd::Command::from_std(temp.command([
        "issue",
        "create",
        "--title",
        "From stdin",
        "--description-file",
        "-",
        "--json",
    ]))
    .write_stdin(source)
    .output()
    .unwrap();

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let created: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        temp.run_json(["issue", "show", &created_issue_id(&created).unwrap()])
            .unwrap()["description"]
            .as_str()
            .unwrap()
            .as_bytes(),
        source.as_bytes()
    );
}

#[test]
fn test_create_description_and_description_file_are_mutually_exclusive() {
    let temp = JitCli::initialised().unwrap();
    let path = temp.root().join("description.md");
    fs::write(&path, "from file").unwrap();

    let output = temp
        .command([
            "issue",
            "create",
            "--title",
            "Conflicting descriptions",
            "--description",
            "inline",
            "--description-file",
            path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[test]
fn test_create_missing_description_file_is_usage_error_naming_path() {
    let temp = JitCli::initialised().unwrap();
    let missing = temp.root().join("missing-description.md");

    let output = temp
        .command([
            "issue",
            "create",
            "--title",
            "Missing description",
            "--description-file",
            missing.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(missing.to_str().unwrap()),
        "stderr: {stderr}"
    );
}

#[test]
fn test_create_unreadable_description_path_is_usage_error_naming_path() {
    let temp = JitCli::initialised().unwrap();
    let directory = temp.root().join("description-directory");
    fs::create_dir(&directory).unwrap();

    let output = temp
        .command([
            "issue",
            "create",
            "--title",
            "Unreadable description",
            "--description-file",
            directory.to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(directory.to_str().unwrap()),
        "stderr: {stderr}"
    );
}

#[test]
fn test_create_help_documents_file_description_forms() {
    let temp = JitCli::uninitialised().unwrap();

    let output = temp
        .command(["issue", "create", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--description-file"));
    assert!(stdout.contains("stdin"));
    assert!(stdout.contains("verbatim"));
}
