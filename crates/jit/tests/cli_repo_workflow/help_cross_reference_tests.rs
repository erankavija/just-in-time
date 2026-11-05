//! Help cross-reference sweep (jit:62f3bebd).
//!
//! Session mining caught agents hand-rolling projections that existing
//! commands already provide, because those commands were never surfaced at
//! the point of need: `issue status`/`gate status`/`gate status-all` went
//! unused in favor of piping `issue show --json` through `jq`, and event-log
//! verification was done by tailing the raw file instead of `jit events`.
//! These tests pin the `--help`/`--schema` cross-references that point agents
//! at the existing command instead.

use jit::test_cli::JitCli;

fn help_text(cli: &JitCli, args: &[&str]) -> String {
    cli.run(args).unwrap()
}

// ============================================================================
// REQ-01 + REQ-03: `issue show --help` points at `issue status` (compact
// view), `gate status`/`gate status-all` (per-issue gates), and summarizes
// the JSON top-level field names, including the dependency and gate
// collections.
// ============================================================================

#[test]
fn test_issue_show_help_mentions_issue_status() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "show", "--help"]);
    assert!(
        help.contains("jit issue status"),
        "issue show --help should point at jit issue status for a compact view, got: {help}"
    );
}

#[test]
fn test_issue_show_help_mentions_gate_status_commands() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "show", "--help"]);
    assert!(
        help.contains("jit gate status"),
        "issue show --help should point at the per-issue gate status command(s), got: {help}"
    );
}

#[test]
fn test_issue_show_help_summarizes_json_field_names() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "show", "--help"]);
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
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "create", "--help"]);
    assert!(
        help.contains("jit events"),
        "issue create --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_issue_update_help_mentions_events_verification() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "update", "--help"]);
    assert!(
        help.contains("jit events"),
        "issue update --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_doc_add_help_mentions_events_verification() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["doc", "add", "--help"]);
    assert!(
        help.contains("jit events"),
        "doc add --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_doc_remove_help_mentions_events_verification() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["doc", "remove", "--help"]);
    assert!(
        help.contains("jit events"),
        "doc remove --help should point at jit events for verification, got: {help}"
    );
}

#[test]
fn test_invariant_help_mentions_item_list_route() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["invariant", "--help"]);
    assert!(
        help.contains("jit item list --kind invariant"),
        "invariant --help should point at the generic invariant listing route, got: {help}"
    );
}

#[test]
fn test_profile_help_describes_repository_package_locations() {
    let cli = JitCli::uninitialised().unwrap();
    for command in ["show", "apply"] {
        let help = help_text(&cli, &["profile", command, "--help"]);
        assert!(
            help.contains("worktree package directory"),
            "profile {command} --help should describe the repository package directory, got: {help}"
        );
    }
}

// ============================================================================
// REQ-01 + REQ-02 (jit:6ad894cb): `issue create --help` describes `-d`/
// `--description` as the initial issue description/body, and states its
// actual default (an empty string) when the flag is omitted.
// ============================================================================

#[test]
fn test_issue_create_help_describes_description_as_initial_body() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "create", "--help"]);
    assert!(
        help.contains("Initial description (body) of the issue"),
        "issue create --help should describe -d/--description as the initial issue description/body, got: {help}"
    );
}

#[test]
fn test_issue_create_help_states_description_default() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["issue", "create", "--help"]);
    assert!(
        help.contains("Defaults to an empty string when omitted"),
        "issue create --help should state --description's actual default (empty string), got: {help}"
    );
}

// ============================================================================
// REQ-02: top-level help mentions `jit --schema` for JSON shapes and exit
// codes; `jit --schema` actually documents both.
// ============================================================================

#[test]
fn test_top_level_help_mentions_schema_command() {
    let cli = JitCli::uninitialised().unwrap();
    let help = help_text(&cli, &["--help"]);
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
    let cli = JitCli::uninitialised().unwrap();
    // `-h` and `--help` are both driven by the same top-level about text in
    // this CLI (long_about is disabled), so both must carry the mention.
    let help = help_text(&cli, &["-h"]);
    assert!(
        help.contains("--schema"),
        "jit -h should mention --schema, got: {help}"
    );
}

#[test]
fn test_schema_flag_documents_json_shapes_and_exit_codes() {
    let cli = JitCli::uninitialised().unwrap();
    let json = cli.run_json(["--schema"]).unwrap();

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

// ============================================================================
// REQ-02 (jit:1a63ef75): `jit --schema`'s `exit_codes` array describes the
// complete (`crate::output::ExitCode`, 0 through 6, 10 through 12, and 141) taxonomy.
// Pins the full set so a code added to the enum without a matching schema update
// fails this test instead of silently drifting.
// ============================================================================

#[test]
fn test_schema_exit_codes_match_full_taxonomy() {
    let cli = JitCli::uninitialised().unwrap();
    let json = cli.run_json(["--schema"]).unwrap();

    let codes: Vec<i64> = json["exit_codes"]
        .as_array()
        .expect("exit_codes should be an array")
        .iter()
        .map(|entry| entry["code"].as_i64().expect("code should be an integer"))
        .collect();

    let expected = [0, 1, 2, 3, 4, 5, 6, 10, 11, 12, 141];
    for code in expected {
        assert!(
            codes.contains(&code),
            "exit_codes should document code {code}, got: {codes:?}"
        );
    }
    assert_eq!(
        codes.len(),
        expected.len(),
        "exit_codes should have exactly the documented taxonomy, got: {codes:?}"
    );
}
