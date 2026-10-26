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

use jit::test_cli::JitCli;
use std::fs;

/// Assert the last event is `issue_updated` for `id`, with `fields`
/// containing "description".
fn assert_last_event_is_description_update(temp: &JitCli, id: &str) {
    let events_path = temp.root().join(".jit").join("events.jsonl");
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
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Original text."])
        .unwrap();

    temp.run(["issue", "update", &id, "--append-description", "Follow-up."])
        .unwrap();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "Original text.\n\nFollow-up."
    );
}

#[test]
fn test_append_description_to_empty_description_has_no_leading_blank_line() {
    let temp = JitCli::initialised().unwrap();
    let id = temp.create_issue(["--title", "Some issue"]).unwrap();
    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        ""
    );

    temp.run([
        "issue",
        "update",
        &id,
        "--append-description",
        "First note.",
    ])
    .unwrap();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "First note."
    );
}

#[test]
fn test_append_description_twice_accumulates_with_one_blank_line_each() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "A"])
        .unwrap();

    temp.run(["issue", "update", &id, "--append-description", "B"])
        .unwrap();
    temp.run(["issue", "update", &id, "--append-description", "C"])
        .unwrap();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "A\n\nB\n\nC"
    );
}

// ---------------------------------------------------------------------
// REQ-02: --description-file / --append-description-file (file + stdin)
// ---------------------------------------------------------------------

#[test]
fn test_description_file_replaces_description_verbatim() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Old"])
        .unwrap();
    let file_path = temp.root().join("desc.txt");
    fs::write(&file_path, "New content from file").unwrap();

    temp.run([
        "issue",
        "update",
        &id,
        "--description-file",
        file_path.to_str().unwrap(),
    ])
    .unwrap();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "New content from file"
    );
}

#[test]
fn test_append_description_file_appends_from_file() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Base"])
        .unwrap();
    let file_path = temp.root().join("extra.txt");
    fs::write(&file_path, "Extra").unwrap();

    temp.run([
        "issue",
        "update",
        &id,
        "--append-description-file",
        file_path.to_str().unwrap(),
    ])
    .unwrap();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "Base\n\nExtra"
    );
}

#[test]
fn test_description_file_dash_reads_stdin() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Old"])
        .unwrap();

    assert_cmd::Command::from_std(temp.command([
        "issue",
        "update",
        &id,
        "--description-file",
        "-",
    ]))
    .write_stdin("From stdin")
    .assert()
    .success();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "From stdin"
    );
}

#[test]
fn test_append_description_file_dash_reads_stdin_and_appends() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Base"])
        .unwrap();

    assert_cmd::Command::from_std(temp.command([
        "issue",
        "update",
        &id,
        "--append-description-file",
        "-",
    ]))
    .write_stdin("From stdin")
    .assert()
    .success();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "Base\n\nFrom stdin"
    );
}

#[test]
fn test_description_file_multi_kilobyte_content_roundtrips_verbatim() {
    let temp = JitCli::initialised().unwrap();
    let id = temp.create_issue(["--title", "Some issue"]).unwrap();

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

    let file_path = temp.root().join("big.txt");
    fs::write(&file_path, &big).unwrap();

    temp.run([
        "issue",
        "update",
        &id,
        "--description-file",
        file_path.to_str().unwrap(),
    ])
    .unwrap();

    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        big
    );
}

#[test]
fn test_append_description_file_multi_kilobyte_content_appends_verbatim() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Preamble."])
        .unwrap();

    let mut big = String::new();
    for i in 0..400 {
        big.push_str(&format!("Appended kilobyte-scale line {i}.\n"));
    }
    assert!(big.len() > 4 * 1024, "fixture should be multi-kilobyte");

    let file_path = temp.root().join("big_append.txt");
    fs::write(&file_path, &big).unwrap();

    temp.run([
        "issue",
        "update",
        &id,
        "--append-description-file",
        file_path.to_str().unwrap(),
    ])
    .unwrap();

    let expected = format!("Preamble.\n\n{big}");
    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        expected
    );
}

// ---------------------------------------------------------------------
// Mutual exclusivity (design decision: exactly one description flag)
// ---------------------------------------------------------------------

#[test]
fn test_description_and_append_description_flags_conflict() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Old"])
        .unwrap();

    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--description",
            "X",
            "--append-description",
            "Y",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));

    // Nothing was changed.
    assert_eq!(
        temp.run_json(["issue", "show", &id]).unwrap()["description"],
        "Old"
    );
}

#[test]
fn test_description_and_description_file_flags_conflict() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Old"])
        .unwrap();
    let file_path = temp.root().join("desc.txt");
    fs::write(&file_path, "New").unwrap();

    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--description",
            "X",
            "--description-file",
            file_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

#[test]
fn test_append_description_and_append_description_file_flags_conflict() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Old"])
        .unwrap();
    let file_path = temp.root().join("extra.txt");
    fs::write(&file_path, "Extra").unwrap();

    let output = temp
        .command([
            "issue",
            "update",
            &id,
            "--append-description",
            "X",
            "--append-description-file",
            file_path.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot be used with"));
}

// ---------------------------------------------------------------------
// REQ-03: help text + event log
// ---------------------------------------------------------------------

#[test]
fn test_help_documents_replace_vs_append_semantics() {
    let temp = JitCli::uninitialised().unwrap();

    let output = temp
        .command(["issue", "update", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--description-file"));
    assert!(stdout.contains("--append-description-file"));
    assert!(stdout.contains("Replace the entire description with TEXT"));
    assert!(stdout.contains("Append TEXT to the end of the existing description"));
    assert!(stdout.contains("blank line"));
}

#[test]
fn test_append_description_logs_issue_updated_event() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Original"])
        .unwrap();

    temp.run(["issue", "update", &id, "--append-description", "More"])
        .unwrap();

    assert_last_event_is_description_update(&temp, &id);
}

#[test]
fn test_description_file_update_logs_issue_updated_event() {
    let temp = JitCli::initialised().unwrap();
    let id = temp
        .create_issue(["--title", "Some issue", "--description", "Original"])
        .unwrap();
    let file_path = temp.root().join("desc.txt");
    fs::write(&file_path, "Replaced via file").unwrap();

    temp.run([
        "issue",
        "update",
        &id,
        "--description-file",
        file_path.to_str().unwrap(),
    ])
    .unwrap();

    assert_last_event_is_description_update(&temp, &id);
}

/// Batch mode (`--filter`) must REJECT the description flags rather than
/// silently ignoring them: a description edit is a per-issue replace/append
/// against one issue's existing text, so applying it across a filter match is
/// not supported. Guards against the accept-and-no-op CLI-contract violation
/// (matches the existing `--content-format` / `--type` batch-mode rejections).
#[test]
fn test_description_flags_rejected_in_filter_batch_mode() {
    let temp = JitCli::initialised().unwrap();
    let _id = temp
        .create_issue(["--title", "Some issue", "--description", "Original text"])
        .unwrap();

    // Append form.
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "state:backlog",
            "--append-description",
            "a note",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not supported with --filter"));

    // Replace-from-file form is rejected the same way. The guard fires before
    // any stdin/file read, so `-` never blocks on stdin here.
    let output = temp
        .command([
            "issue",
            "update",
            "--filter",
            "state:backlog",
            "--description-file",
            "notes.txt",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("not supported with --filter"));
}
