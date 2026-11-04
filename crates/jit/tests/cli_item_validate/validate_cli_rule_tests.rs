//! CLI integration tests for the extended `jit validate` surface (issue
//! b8ba1b10): the positional `[<id>]`, `--explain`, `--json`, error-severity
//! exit codes, the `$JIT_ISSUE_ID` gate-checker wiring, and `--schema` / MCP
//! parity for the new positional + flag.

use assert_cmd::prelude::*;
use jit::commands::test_helpers::ExecutorTestAccess;
use jit::test_cli::JitCli;
use jit::test_utils::profiled_repository_fixture;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command;

fn append_rules(cli: &JitCli, rules_toml: &str) {
    let rules_path = cli.root().join(".jit/rules.toml");
    let mut rules = fs::read_to_string(&rules_path).unwrap();
    rules.push_str(rules_toml);
    fs::write(rules_path, rules).unwrap();
}

/// An epic carrying its identifying `epic:*` label, so the built-in
/// strategic-consistency warning adds no finding beside the user rule.
const EPIC: [&str; 6] = [
    "--title",
    "An epic",
    "--label",
    "type:epic",
    "--label",
    "epic:auth",
];

/// [`EPIC`] with the `component:*` label the epic rules require.
const COMPLIANT_EPIC: [&str; 8] = [
    "--title",
    "An epic",
    "--label",
    "type:epic",
    "--label",
    "epic:auth",
    "--label",
    "component:api",
];

/// An epic whose body declares a single `[hard]` success criterion.
const EPIC_WITH_CRITERION: [&str; 6] = [
    "--title",
    "An epic",
    "--label",
    "type:epic",
    "--description",
    "## Success Criteria\n\n- [hard] REQ-01: do the thing\n",
];

const EPIC_NEEDS_COMPONENT: &str = r#"
[[rules]]
name = "epic-needs-component"
when = { type = "epic" }
severity = "error"
enforce = false
assert = { require-label = { label = "component:*", min = 1 } }
"#;

const EPIC_WARNS_WITHOUT_COMPONENT: &str = r#"
[[rules]]
name = "epic-warns-without-component"
when = { type = "epic" }
severity = "warn"
enforce = false
assert = { require-label = { label = "component:*", min = 1 } }
"#;

#[test]
fn test_validate_custom_only_rules_reject_malformed_labels() {
    let cli = JitCli::initialised().unwrap();
    let rules_path = cli.root().join(".jit/rules.toml");
    fs::write(&rules_path, EPIC_NEEDS_COMPONENT).unwrap();
    cli.command(["issue", "create", "--title", "Malformed", "--label", "BAD"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("label-format"));

    cli.create_issue(["--title", "Forced malformed", "--label", "BAD", "--force"])
        .unwrap();
    fs::write(rules_path, EPIC_NEEDS_COMPONENT).unwrap();
    cli.command(["validate"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("label-format"));
}

#[test]
fn test_config_writes_and_validate_fix_restore_defaults_in_custom_only_file() {
    for args in [
        &["config", "set", "validation.strictness", "loose"][..],
        &["validate", "--fix"][..],
        &["init"][..],
    ] {
        let cli = JitCli::initialised().unwrap();
        let rules_path = cli.root().join(".jit/rules.toml");
        fs::write(&rules_path, EPIC_NEEDS_COMPONENT).unwrap();
        cli.command(args).assert().success();
        let restored = fs::read_to_string(&rules_path).unwrap();
        assert!(restored.starts_with(EPIC_NEEDS_COMPONENT));
        let executor = jit::test_utils::file_executor(cli.root()).unwrap();
        assert!(executor
            .rules()
            .unwrap()
            .rules
            .iter()
            .any(|r| r.name == "label-format" && r.origin.as_deref() == Some("default")));
    }
}

#[test]
fn test_validate_id_passes_for_compliant_issue() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();

    cli.command(["validate", &id])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_validate_id_json_success_keeps_the_rule_report() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();

    let assert = cli.command(["validate", &id, "--json"]).assert().success();
    let json: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert!(
        json["findings"].is_array(),
        "successful report stays direct: {json}"
    );
    assert!(json.get("error").is_none());
}

#[test]
fn test_validate_id_fails_and_exits_nonzero() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(EPIC).unwrap();

    let output = cli.command(["validate", &id]).output().unwrap();
    assert_eq!(output.status.code(), Some(4));
    assert!(String::from_utf8_lossy(&output.stdout).contains("epic-needs-component"));
}

#[test]
fn test_validate_id_json_reports_findings_and_exits_nonzero() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(EPIC).unwrap();

    let assert = cli.command(["validate", &id, "--json"]).assert().failure();
    let out = assert.get_output().stdout.clone();
    assert_eq!(assert.get_output().status.code(), Some(4));
    let json: Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(json["error"]["code"], "VALIDATION_FAILED");
    assert!(assert.get_output().stderr.is_empty());
    let findings = json["error"]["details"]["findings"].as_array().unwrap();
    assert!(findings.iter().any(|f| f["rule"] == "epic-needs-component"));
    assert!(findings.iter().any(|f| f["severity"] == "error"));
}

#[test]
fn test_validate_explain_lists_outcomes() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(EPIC).unwrap();

    let output = cli
        .command(["validate", &id, "--explain"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(4));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("epic-needs-component"));
    assert!(stdout.contains("FAIL"));
    assert!(stdout.contains("type=epic"));
}

#[test]
fn test_validate_explain_json_structure() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();

    let assert = cli
        .command(["validate", &id, "--explain", "--json"])
        .assert()
        .success();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let outcomes = json["outcomes"].as_array().unwrap();
    // The effective rule set prepends the built-in default rules, so the user
    // rule is no longer guaranteed at index 0; find it by name.
    let outcome = outcomes
        .iter()
        .find(|o| o["rule"] == "epic-needs-component")
        .expect("epic-needs-component outcome present");
    assert_eq!(outcome["scope"], "local");
    assert_eq!(outcome["passed"], true);
}

/// A state-scoped rule used to exercise `--explain`'s skip rendering: it applies
/// only to `done` issues. Against a non-`done` issue the rule is skipped, and the
/// success criterion requires `--explain` to show that the state predicate did
/// not match.
const DONE_NEEDS_SUMMARY: &str = r#"
[[rules]]
name = "done-needs-summary"
when = { state = "done" }
severity = "error"
enforce = false
assert = { require-section = { heading = "Summary" } }
"#;

#[test]
fn test_validate_explain_renders_skipped_rule_with_state_reason() {
    // The success criterion: `jit validate <id> --explain` shows whether the
    // state predicate matched. A fresh issue is not in the rule's wanted state,
    // so the rule is rendered as a [SKIP] line naming the issue state and the wanted
    // token.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, DONE_NEEDS_SUMMARY);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();

    cli.command(["validate", &id, "--explain"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[SKIP] done-needs-summary"))
        .stdout(predicate::str::contains("state predicate did not match"))
        .stdout(predicate::str::contains("wants 'done'"));
}

#[test]
fn test_validate_explain_json_includes_matched_and_skip_reason() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, DONE_NEEDS_SUMMARY);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();

    let assert = cli
        .command(["validate", &id, "--explain", "--json"])
        .assert()
        .success();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let outcomes = json["outcomes"].as_array().unwrap();
    let outcome = outcomes
        .iter()
        .find(|o| o["rule"] == "done-needs-summary")
        .expect("done-needs-summary outcome present even though it does not match");
    assert_eq!(outcome["matched"], false);
    assert_eq!(outcome["passed"], true);
    let reason = outcome["skip_reason"].as_str().unwrap();
    assert!(
        reason.contains("state predicate did not match") && reason.contains("done"),
        "skip_reason names the state dimension: {reason}"
    );
}

#[test]
fn test_validate_explain_json_matched_rule_has_no_skip_reason() {
    // A matched rule keeps `matched = true` and omits `skip_reason` from JSON.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();

    let assert = cli
        .command(["validate", &id, "--explain", "--json"])
        .assert()
        .success();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let outcome = json["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|o| o["rule"] == "epic-needs-component")
        .expect("epic-needs-component outcome present");
    assert_eq!(outcome["matched"], true);
    assert!(
        outcome.get("skip_reason").is_none(),
        "matched rule omits skip_reason: {outcome:?}"
    );
}

#[test]
fn test_validate_explain_requires_id() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    cli.command(["validate", "--explain"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires an issue id"));
}

#[test]
fn test_gate_checker_via_jit_issue_id_env_var() {
    // A gate whose checker runs `jit validate "$JIT_ISSUE_ID" --json` must pass
    // when the issue is compliant. This proves the JIT_ISSUE_ID wiring end to end
    // (no `{issue}` placeholder is used).
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let checker = format!(
        "{} validate \"$JIT_ISSUE_ID\" --json",
        jit::test_cli::jit_binary().display()
    );

    cli.command([
        "gate",
        "define",
        "rules-gate",
        "--title",
        "Rules Gate",
        "--description",
        "Runs jit validate for the issue",
        "--mode",
        "auto",
        "--checker-command",
        &checker,
        "--timeout",
        "30",
    ])
    .assert()
    .success();

    // Compliant epic with the gate -> gate passes.
    let good_id = cli
        .create_issue([
            "--title",
            "Good epic",
            "--label",
            "type:epic",
            "--label",
            "component:api",
            "--gate",
            "rules-gate",
        ])
        .unwrap();

    cli.command(["gate", "evaluate", &good_id, "rules-gate"])
        .assert()
        .success();
}

#[test]
fn test_gate_checker_via_jit_issue_id_fails_for_noncompliant() {
    // The same gate must FAIL when the issue violates an error-severity rule,
    // because `jit validate "$JIT_ISSUE_ID"` exits non-zero.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let checker = format!(
        "{} validate \"$JIT_ISSUE_ID\" --json",
        jit::test_cli::jit_binary().display()
    );

    cli.command([
        "gate",
        "define",
        "rules-gate",
        "--title",
        "Rules Gate",
        "--description",
        "Runs jit validate for the issue",
        "--mode",
        "auto",
        "--checker-command",
        &checker,
        "--timeout",
        "30",
    ])
    .assert()
    .success();

    // Non-compliant epic (no component:* label) with the gate -> gate fails.
    let bad_id = cli
        .create_issue([
            "--title",
            "Bad epic",
            "--label",
            "type:epic",
            "--gate",
            "rules-gate",
        ])
        .unwrap();

    cli.command(["gate", "evaluate", &bad_id, "rules-gate"])
        .assert()
        .failure();
}

// ---------------------------------------------------------------------------
// Rework (jit:b8ba1b10): whole-repo --json must surface graph-rule findings;
// id + legacy flags must be rejected; per-issue + --explain must report
// unresolved container pointers instead of silently passing.
// ---------------------------------------------------------------------------

/// A `label-coverage` GRAPH rule at error severity. An epic declaring a `[hard]`
/// criterion with no satisfying child (child-link = any) violates it.
const COVERAGE_GRAPH_RULE: &str = r#"
[[rules]]
name = "epic-criteria-covered"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { child-link = "any", marker = "[hard]" } }
"#;

const DISABLED_GRAPH_RULE: &str = r#"
[[rules]]
name = "disabled-graph-rule"
when = { type = "epic" }
severity = "off"
assert = { label-coverage = { marker = "[hard]" } }
"#;

const SELECTED_COVERAGE_GRAPH_RULE: &str = r#"
[[rules]]
name = "selected-epic-criteria-covered"
when = { type = "epic" }
severity = "error"
assert = { label-coverage = { child-link = "dependencies", marker = "[hard]" } }
"#;

const SELECTED_BREAKDOWN_COVERAGE_RULE: &str = r#"
[[rules]]
name = "selected-breakdown-criteria-covered"
when = { type = "breakdown" }
severity = "error"
assert = { label-coverage = { child-link = "dependencies", marker = "[hard]", container-from-label = "brackets" } }
"#;

#[test]
fn test_validate_whole_repo_json_includes_graph_error_findings_and_exits_nonzero() {
    // Regression for finding #1: a whole-repo `jit validate --json` with an
    // error-severity GRAPH rule violation must EMIT the structured rule report
    // including the graph finding, and exit non-zero — not bail via the generic
    // error path that drops the structured report.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, COVERAGE_GRAPH_RULE);
    let _id = cli.create_issue(EPIC_WITH_CRITERION).unwrap();

    let assert = cli.command(["validate", "--json"]).assert().failure();
    let out = assert.get_output().stdout.clone();
    let json: Value = serde_json::from_slice(&out).unwrap();
    let details = &json["error"]["details"];
    assert_eq!(
        details["valid"], false,
        "whole-repo run must be invalid: {json}"
    );
    let findings = details["rule_findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|f| f["rule"] == "epic-criteria-covered" && f["severity"] == "error"),
        "graph-rule finding must appear in the structured report: {findings:?}"
    );
}

#[test]
fn test_validate_retains_rule_findings_with_integrity_failure() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(EPIC).unwrap();
    let issue_path = cli.root().join(".jit/issues").join(format!("{id}.json"));
    let mut issue: Value = serde_json::from_str(&fs::read_to_string(&issue_path).unwrap()).unwrap();
    issue["dependencies"] = serde_json::json!(["00000000-0000-4000-8000-00000000dead"]);
    fs::write(&issue_path, serde_json::to_vec_pretty(&issue).unwrap()).unwrap();

    cli.command(["validate"])
        .assert()
        .failure()
        .stdout(predicate::str::contains("[epic-needs-component]"))
        .stdout(predicate::str::contains("does not exist"));

    let output = cli
        .command(["validate", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    let details = &json["error"]["details"];
    assert_eq!(details["valid"], false);
    assert!(details["integrity_error"]
        .as_str()
        .is_some_and(|message| message.contains("does not exist")));
    assert_eq!(details["error_count"], 1);
    assert!(details["rule_findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|finding| finding["rule"] == "epic-needs-component"));
}

#[test]
fn test_validate_id_with_fix_is_rejected() {
    // Finding #2: `--fix` is repo-wide and must not be silently scoped to an id.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();
    cli.command(["validate", &id, "--fix"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot be combined with a positional",
        ));
}

fn configure_review_placeholder(cli: &JitCli) {
    fs::write(
        cli.root().join(".jit/gates.toml"),
        r#"[[gates]]
version = 1
key = "review-test"
title = "Review"
description = "External review placeholder"
stage = "postcheck"
mode = "auto"
priority = 100

[gates.checker]
type = "review_placeholder"
"#,
    )
    .unwrap();
}

#[test]
fn test_validate_reports_review_placeholder_warning_in_human_and_json_output() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    configure_review_placeholder(&cli);

    cli.command(["validate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[review-placeholder]"))
        .stdout(predicate::str::contains("review-test"));

    let output = cli
        .command(["validate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["valid"], true);
    assert_eq!(json["warning_count"], 1);
    assert_eq!(json["warnings"][0]["rule"], "review-placeholder");
    assert_eq!(json["rule_findings"][0]["rule"], "review-placeholder");
}

#[test]
fn test_validate_scope_with_only_warnings_prints_findings_without_pass_line_and_succeeds() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_WARNS_WITHOUT_COMPONENT);
    let container = cli.create_issue(EPIC).unwrap();

    let assert = cli
        .command(["validate", "--scope", &container])
        .assert()
        .success();
    let output = assert.get_output();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("warning [epic-warns-without-component]"),
        "warnings-only scope run must list its findings, got: {stdout}"
    );
    assert!(
        !stdout.contains("Scope validation passed"),
        "warnings-only findings must not be followed by a pass line, got: {stdout}"
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn test_validate_id_with_branch_drift_is_rejected() {
    // Finding #2: `--branch-drift` is repo-wide and incompatible with a positional id.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();
    cli.command(["validate", &id, "--branch-drift"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot be combined with a positional",
        ));
}

#[test]
fn test_validate_id_with_leases_is_rejected() {
    // Finding #2: `--leases` is repo-wide and incompatible with a positional id.
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, EPIC_NEEDS_COMPONENT);
    let id = cli.create_issue(COMPLIANT_EPIC).unwrap();
    cli.command(["validate", &id, "--leases"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "cannot be combined with a positional",
        ));
}

#[test]
fn test_validate_rule_cli_resolves_partial_id_and_isolates_selected_application() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, SELECTED_COVERAGE_GRAPH_RULE);
    let selected = cli.create_issue(EPIC_WITH_CRITERION).unwrap();
    let child = cli
        .create_issue([
            "--title",
            "Selected implementation",
            "--label",
            "type:task",
            "--label",
            &format!("satisfies:{}/REQ-01", &selected[..8]),
        ])
        .unwrap();
    cli.command(["dep", "add", &selected, &child])
        .assert()
        .success();
    let _unrelated = cli.create_issue(EPIC_WITH_CRITERION).unwrap();
    let selected_prefix = &selected[..8];

    let clean = cli
        .command([
            "validate",
            selected_prefix,
            "--rule",
            "selected-epic-criteria-covered",
            "--json",
        ])
        .assert()
        .success();
    let clean_json: Value = serde_json::from_slice(&clean.get_output().stdout).unwrap();
    assert!(clean_json["findings"].as_array().unwrap().is_empty());

    let failing = cli.create_issue(EPIC_WITH_CRITERION).unwrap();
    let failed = cli
        .command([
            "validate",
            &failing,
            "--rule",
            "selected-epic-criteria-covered",
            "--json",
        ])
        .assert()
        .failure();
    let failed_json: Value = serde_json::from_slice(&failed.get_output().stdout).unwrap();
    assert_eq!(failed.get_output().status.code(), Some(4));
    assert_eq!(failed_json["error"]["code"], "VALIDATION_FAILED");
    assert!(failed.get_output().stderr.is_empty());
    let findings = failed_json["error"]["details"]["findings"]
        .as_array()
        .unwrap();
    assert!(findings.iter().any(|finding| {
        finding["rule"] == "selected-epic-criteria-covered"
            && finding["issue_id"] == failing
            && finding["message"]
                .as_str()
                .unwrap_or_default()
                .contains("REQ-01")
    }));
}

#[test]
fn test_validate_rule_cli_selection_errors_do_not_pass_vacuously() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(
        &cli,
        &format!("{COVERAGE_GRAPH_RULE}\n{DISABLED_GRAPH_RULE}"),
    );
    let epic = cli.create_issue(EPIC_WITH_CRITERION).unwrap();
    let task = cli
        .create_issue(["--title", "A task", "--label", "type:task"])
        .unwrap();

    for (id, rule, message) in [
        (&epic, "missing-graph-rule", "was not found"),
        (&epic, "disabled-graph-rule", "is off"),
        (
            &task,
            "epic-criteria-covered",
            "does not match selected issue",
        ),
    ] {
        let assert = cli
            .command(["validate", id, "--rule", rule, "--json"])
            .assert()
            .failure();
        assert_eq!(assert.get_output().status.code(), Some(4));
        let json: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
        let findings = json["error"]["details"]["findings"].as_array().unwrap();
        assert!(
            findings.iter().any(|finding| finding["message"]
                .as_str()
                .unwrap_or_default()
                .contains(message)),
            "{rule} must identify its selection error: {findings:?}"
        );
    }

    cli.command(["validate", "--rule", "epic-criteria-covered"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("requires a positional issue id"));
}

#[test]
fn test_validate_rule_cli_reports_missing_selected_container() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    append_rules(&cli, SELECTED_BREAKDOWN_COVERAGE_RULE);
    let breakdown = cli
        .create_issue([
            "--title",
            "Breakdown with missing target",
            "--label",
            "type:breakdown",
            "--label",
            "brackets:missing-target",
        ])
        .unwrap();

    let assert = cli
        .command([
            "validate",
            &breakdown,
            "--rule",
            "selected-breakdown-criteria-covered",
            "--json",
        ])
        .assert()
        .failure();
    let json: Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    let findings = json["error"]["details"]["findings"].as_array().unwrap();
    assert!(findings.iter().any(|finding| {
        finding["rule"] == "selected-breakdown-criteria-covered"
            && finding["message"]
                .as_str()
                .unwrap_or_default()
                .contains("names no known issue")
    }));
}

#[test]
fn test_schema_exposes_validate_positional_and_explain_flag() {
    // MCP parity: the new positional `id` arg and `--explain` flag MUST appear in
    // `jit --schema` so the MCP server auto-generates a working tool (DR §9.3).
    let output = JitCli::uninitialised()
        .unwrap()
        .command(["--schema"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let validate = &json["commands"]["validate"];

    let args = validate["args"].as_array().unwrap();
    assert!(
        args.iter().any(|a| a["name"] == "id"),
        "validate must expose positional `id`: {args:?}"
    );

    let flags = validate["flags"].as_array().unwrap();
    assert!(
        flags.iter().any(|f| f["name"] == "explain"),
        "validate must expose --explain flag: {flags:?}"
    );
    assert!(
        flags.iter().any(|f| f["name"] == "rule"),
        "validate must expose --rule for selected graph-rule reproduction"
    );
    let fix = flags.iter().find(|flag| flag["name"] == "fix").unwrap();
    assert!(
        fix["description"]
            .as_str()
            .unwrap()
            .contains("derived-state repairs publish through one redo record"),
        "schema must document the repair contract: {fix}"
    );
}

#[test]
fn test_mcp_generates_validate_tool_with_id_and_explain() {
    // The MCP server generates tools from `jit --schema`. Run its tool-generator
    // over the real schema and assert the `jit_validate` tool carries the new
    // `id` and `explain` properties (proves a WORKING tool is generated for the
    // new surface, not just that the schema contains the fields).
    let schema_out = JitCli::uninitialised()
        .unwrap()
        .command(["--schema"])
        .output()
        .unwrap();
    assert!(schema_out.status.success());
    let schema_json = String::from_utf8(schema_out.stdout).unwrap();

    let mcp_dir = mcp_server_dir();
    let script = r#"
import { generateTools } from './lib/tool-generator.js';
let data = '';
process.stdin.on('data', c => data += c);
process.stdin.on('end', () => {
  const schema = JSON.parse(data);
  const tools = generateTools(schema, false);
  const tool = tools.find(t => t.name === 'jit_validate');
  if (!tool) { console.error('no jit_validate tool'); process.exit(2); }
  const props = tool.inputSchema.properties || {};
  if (!props.id) { console.error('missing id property'); process.exit(3); }
  if (!props.explain) { console.error('missing explain property'); process.exit(4); }
  if (!props.rule) { console.error('missing rule property'); process.exit(5); }
  if (!props.fix?.description?.includes('derived-state repairs publish through one redo record')) {
    console.error('missing fix repair contract'); process.exit(6);
  }
  console.log('ok');
});
"#;

    let mut child = Command::new("node")
        .current_dir(&mcp_dir)
        .args(["--input-type=module", "-e", script])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect(
            "node is required for the MCP tool-generation parity test and could not be spawned",
        );
    {
        use std::io::Write;
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(schema_json.as_bytes())
            .unwrap();
    }
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "MCP tool generation parity failed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok"));
}

/// Locate the `mcp-server` directory relative to the crate manifest.
fn mcp_server_dir() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR is crates/jit; mcp-server is two levels up.
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("mcp-server")
}
