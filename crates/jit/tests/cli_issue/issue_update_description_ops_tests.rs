//! `jit issue update` description flags: append vs. replace, and file/stdin
//! sources (jit:b2f9f390).
//!
//! Covers:
//!   REQ-01 `--append-description <TEXT>` appends to the end of an existing
//!          description with a separating blank line, leaving the rest
//!          intact; appending to an empty description has no leading blank
//!          line.
//!   REQ-02 `--description-file` / `--append-description-file` read from a
//!          file (or stdin via `-`), for both the replace and append forms,
//!          including a multi-kilobyte description.
//!   REQ-03 `--help` documents replace vs. append semantics, and the update
//!          still logs an `issue_updated` event (fields includes
//!          "description"), same as a plain `--description` update.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit(temp: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(temp.path());
    cmd
}

/// `assert_cmd::Command` (distinct from `std::process::Command`) is needed
/// only for `write_stdin`. Built from a `std::process::Command` via the same
/// `cargo_bin!` binary resolution the rest of the suite uses, since
/// `assert_cmd::Command::cargo_bin` is deprecated.
fn jit_assert_cmd(temp: &TempDir) -> assert_cmd::Command {
    let mut inner = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    inner.current_dir(temp.path());
    assert_cmd::Command::from_std(inner)
}

/// Set up a repo with a single issue, returning `(TempDir, issue_id)`. Pass
/// `None` to leave the description at its default (empty).
fn setup_repo_with_issue(description: Option<&str>) -> (TempDir, String) {
    let temp = TempDir::new().unwrap();
    jit(&temp).arg("init").assert().success();

    let mut args = vec!["issue", "create", "--title", "Some issue"];
    if let Some(d) = description {
        args.push("--description");
        args.push(d);
    }
    let output = jit(&temp)
        .args(&args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let stdout = String::from_utf8_lossy(&output);
    let id = stdout
        .lines()
        .find(|l| l.contains("Created issue:"))
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap()
        .to_string();
    (temp, id)
}

/// Fetch the current description via `issue show --json`.
fn description_of(temp: &TempDir, id: &str) -> String {
    let output = jit(temp)
        .args(["issue", "show", id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    json["description"]
        .as_str()
        .expect("description field must be a string")
        .to_string()
}

/// Assert the last event is `issue_updated` for `id`, with `fields`
/// containing "description".
fn assert_last_event_is_description_update(temp: &TempDir, id: &str) {
    let events_path = temp.path().join(".jit").join("events.jsonl");
    let contents = fs::read_to_string(&events_path).unwrap();
    let last_line = contents.lines().last().expect("events.jsonl is empty");
    let event: serde_json::Value = serde_json::from_str(last_line).unwrap();

    assert_eq!(event["type"], "issue_updated", "got: {}", event);
    assert_eq!(event["issue_id"], id, "got: {}", event);
    let fields = event["fields"].as_array().expect("fields must be an array");
    assert!(
        fields.iter().any(|f| f == "description"),
        "fields should contain 'description', got: {}",
        event
    );
}

// ---------------------------------------------------------------------
// REQ-01: --append-description
// ---------------------------------------------------------------------

#[test]
fn test_append_description_adds_text_with_blank_line_separator() {
    let (temp, id) = setup_repo_with_issue(Some("Original text."));

    jit(&temp)
        .args(["issue", "update", &id, "--append-description", "Follow-up."])
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "Original text.\n\nFollow-up.");
}

#[test]
fn test_append_description_to_empty_description_has_no_leading_blank_line() {
    let (temp, id) = setup_repo_with_issue(None);
    assert_eq!(description_of(&temp, &id), "");

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--append-description",
            "First note.",
        ])
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "First note.");
}

#[test]
fn test_append_description_twice_accumulates_with_one_blank_line_each() {
    let (temp, id) = setup_repo_with_issue(Some("A"));

    jit(&temp)
        .args(["issue", "update", &id, "--append-description", "B"])
        .assert()
        .success();
    jit(&temp)
        .args(["issue", "update", &id, "--append-description", "C"])
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "A\n\nB\n\nC");
}

// ---------------------------------------------------------------------
// REQ-02: --description-file / --append-description-file (file + stdin)
// ---------------------------------------------------------------------

#[test]
fn test_description_file_replaces_description_verbatim() {
    let (temp, id) = setup_repo_with_issue(Some("Old"));
    let file_path = temp.path().join("desc.txt");
    fs::write(&file_path, "New content from file").unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "New content from file");
}

#[test]
fn test_append_description_file_appends_from_file() {
    let (temp, id) = setup_repo_with_issue(Some("Base"));
    let file_path = temp.path().join("extra.txt");
    fs::write(&file_path, "Extra").unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--append-description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "Base\n\nExtra");
}

#[test]
fn test_description_file_dash_reads_stdin() {
    let (temp, id) = setup_repo_with_issue(Some("Old"));

    jit_assert_cmd(&temp)
        .args(["issue", "update", &id, "--description-file", "-"])
        .write_stdin("From stdin")
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "From stdin");
}

#[test]
fn test_append_description_file_dash_reads_stdin_and_appends() {
    let (temp, id) = setup_repo_with_issue(Some("Base"));

    jit_assert_cmd(&temp)
        .args(["issue", "update", &id, "--append-description-file", "-"])
        .write_stdin("From stdin")
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), "Base\n\nFrom stdin");
}

#[test]
fn test_description_file_multi_kilobyte_content_roundtrips_verbatim() {
    let (temp, id) = setup_repo_with_issue(None);

    // Build a >8KB description with structure (not just one repeated byte),
    // ending in a trailing newline, to prove file content survives verbatim
    // (including the trailing newline) through the file-read path.
    let mut big = String::new();
    for i in 0..400 {
        big.push_str(&format!(
            "Line {i} of a multi-kilobyte description used to verify --description-file \
             does not truncate, retokenize, or otherwise mangle large content.\n"
        ));
    }
    assert!(big.len() > 8 * 1024, "fixture should be multi-kilobyte");

    let file_path = temp.path().join("big.txt");
    fs::write(&file_path, &big).unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_eq!(description_of(&temp, &id), big);
}

#[test]
fn test_append_description_file_multi_kilobyte_content_appends_verbatim() {
    let (temp, id) = setup_repo_with_issue(Some("Preamble."));

    let mut big = String::new();
    for i in 0..400 {
        big.push_str(&format!("Appended kilobyte-scale line {i}.\n"));
    }
    assert!(big.len() > 4 * 1024, "fixture should be multi-kilobyte");

    let file_path = temp.path().join("big_append.txt");
    fs::write(&file_path, &big).unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--append-description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    let expected = format!("Preamble.\n\n{big}");
    assert_eq!(description_of(&temp, &id), expected);
}

// ---------------------------------------------------------------------
// Mutual exclusivity (design decision: exactly one description flag)
// ---------------------------------------------------------------------

#[test]
fn test_description_and_append_description_flags_conflict() {
    let (temp, id) = setup_repo_with_issue(Some("Old"));

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--description",
            "X",
            "--append-description",
            "Y",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));

    // Nothing was changed.
    assert_eq!(description_of(&temp, &id), "Old");
}

#[test]
fn test_description_and_description_file_flags_conflict() {
    let (temp, id) = setup_repo_with_issue(Some("Old"));
    let file_path = temp.path().join("desc.txt");
    fs::write(&file_path, "New").unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--description",
            "X",
            "--description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn test_append_description_and_append_description_file_flags_conflict() {
    let (temp, id) = setup_repo_with_issue(Some("Old"));
    let file_path = temp.path().join("extra.txt");
    fs::write(&file_path, "Extra").unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--append-description",
            "X",
            "--append-description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

// ---------------------------------------------------------------------
// REQ-03: help text + event log
// ---------------------------------------------------------------------

#[test]
fn test_help_documents_replace_vs_append_semantics() {
    let temp = TempDir::new().unwrap();

    jit(&temp)
        .args(["issue", "update", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--description-file"))
        .stdout(predicate::str::contains("--append-description-file"))
        .stdout(predicate::str::contains(
            "Replace the entire description with TEXT",
        ))
        .stdout(predicate::str::contains(
            "Append TEXT to the end of the existing description",
        ))
        .stdout(predicate::str::contains("blank line"));
}

#[test]
fn test_append_description_logs_issue_updated_event() {
    let (temp, id) = setup_repo_with_issue(Some("Original"));

    jit(&temp)
        .args(["issue", "update", &id, "--append-description", "More"])
        .assert()
        .success();

    assert_last_event_is_description_update(&temp, &id);
}

#[test]
fn test_description_file_update_logs_issue_updated_event() {
    let (temp, id) = setup_repo_with_issue(Some("Original"));
    let file_path = temp.path().join("desc.txt");
    fs::write(&file_path, "Replaced via file").unwrap();

    jit(&temp)
        .args([
            "issue",
            "update",
            &id,
            "--description-file",
            file_path.to_str().unwrap(),
        ])
        .assert()
        .success();

    assert_last_event_is_description_update(&temp, &id);
}

/// Batch mode (`--filter`) must REJECT the description flags rather than
/// silently ignoring them: a description edit is a per-issue replace/append
/// against one issue's existing text, so applying it across a filter match is
/// not supported. Guards against the accept-and-no-op CLI-contract violation
/// (matches the existing `--content-format` / `--type` batch-mode rejections).
#[test]
fn test_description_flags_rejected_in_filter_batch_mode() {
    let (temp, _id) = setup_repo_with_issue(Some("Original text"));

    // Append form.
    jit(&temp)
        .args([
            "issue",
            "update",
            "--filter",
            "state:backlog",
            "--append-description",
            "a note",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported with --filter"));

    // Replace-from-file form is rejected the same way. The guard fires before
    // any stdin/file read, so `-` never blocks on stdin here.
    jit(&temp)
        .args([
            "issue",
            "update",
            "--filter",
            "state:backlog",
            "--description-file",
            "notes.txt",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("not supported with --filter"));
}
