//! Help cross-reference sweep (jit:62f3bebd).
//!
//! Session mining caught agents hand-rolling projections that existing
//! commands already provide, because those commands were never surfaced at
//! the point of need: `issue status`/`gate status`/`gate status-all` went
//! unused in favor of piping `issue show --json` through `jq`, and event-log
//! verification was done by tailing the raw file instead of `jit events`.
//! These tests pin the `--help`/`--schema` cross-references that point agents
//! at the existing command instead.

use std::process::Command;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

fn help_text(args: &[&str]) -> String {
    let output = Command::new(jit_binary()).args(args).output().unwrap();
    assert!(
        output.status.success(),
        "args {:?} should exit 0, stderr: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

// ============================================================================
// REQ-01 + REQ-03: `issue show --help` points at `issue status` (compact
// view), `gate status`/`gate status-all` (per-issue gates), and summarizes
// the JSON top-level field names, including the dependency and gate
// collections.
// ============================================================================

#[test]
fn test_issue_show_help_mentions_issue_status() {
    let help = help_text(&["issue", "show", "--help"]);
    assert!(
        help.contains("jit issue status"),
        "issue show --help should point at jit issue status for a compact view, got: {help}"
    );
}

#[test]
fn test_issue_show_help_mentions_gate_status_commands() {
    let help = help_text(&["issue", "show", "--help"]);
    assert!(
        help.contains("jit gate status"),
        "issue show --help should point at the per-issue gate status command(s), got: {help}"
    );
}

#[test]
fn test_issue_show_help_summarizes_json_field_names() {
    let help = help_text(&["issue", "show", "--help"]);
    for field in ["dependencies", "unmet_dependencies", "gates", "documents"] {
        assert!(
            help.contains(field),
            "issue show --help should summarize the JSON field `{field}`, got: {help}"
        );
    }
}

// ============================================================================
// REQ-01: issue and doc mutation help points at the events commands for
// verifying the recorded change.
// ============================================================================

#[test]
fn test_issue_create_help_mentions_events_verification() {
    let help = help_text(&["issue", "create", "--help"]);
    assert!(
        help.contains("jit events"),
        "issue create --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_issue_update_help_mentions_events_verification() {
    let help = help_text(&["issue", "update", "--help"]);
    assert!(
        help.contains("jit events"),
        "issue update --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_doc_add_help_mentions_events_verification() {
    let help = help_text(&["doc", "add", "--help"]);
    assert!(
        help.contains("jit events"),
        "doc add --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_doc_remove_help_mentions_events_verification() {
    let help = help_text(&["doc", "remove", "--help"]);
    assert!(
        help.contains("jit events"),
        "doc remove --help should point at jit events for verification, got: {help}"
    );
}

// ============================================================================
// REQ-02: top-level help mentions `jit --schema` for JSON shapes and exit
// codes; `jit --schema` actually documents both.
// ============================================================================

#[test]
fn test_top_level_help_mentions_schema_command() {
    let help = help_text(&["--help"]);
    assert!(
        help.contains("--schema"),
        "jit --help should mention --schema, got: {help}"
    );
    assert!(
        help.to_lowercase().contains("exit code"),
        "jit --help should say --schema covers exit codes, got: {help}"
    );
}

#[test]
fn test_short_help_also_mentions_schema_command() {
    // `-h` and `--help` are both driven by the same top-level about text in
    // this CLI (long_about is disabled), so both must carry the mention.
    let help = help_text(&["-h"]);
    assert!(
        help.contains("--schema"),
        "jit -h should mention --schema, got: {help}"
    );
}

#[test]
fn test_schema_flag_documents_json_shapes_and_exit_codes() {
    let output = Command::new(jit_binary()).arg("--schema").output().unwrap();
    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();

    // Exit codes are documented at the top level.
    let exit_codes = json["exit_codes"]
        .as_array()
        .expect("exit_codes should be an array");
    assert!(!exit_codes.is_empty(), "exit_codes should be non-empty");

    // JSON shapes are documented per-command via output.success_schema.
    let show_output = &json["commands"]["issue"]["subcommands"]["show"]["output"];
    assert!(
        show_output["success_schema"].is_object(),
        "issue show's schema entry should carry a success_schema, got: {show_output}"
    );
}
