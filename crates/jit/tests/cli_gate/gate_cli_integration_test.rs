//! Integration tests for gate CLI commands
//!
//! These tests verify the full CLI → CommandExecutor → Storage flow

use super::{add_auto_gate_issue, create_issue_requiring, define_auto_gate};
use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;
use std::fs;

/// Look up a single gate entry from the `gates` array of an `issue show --json`
/// response by gate key. Returns `None` when the gate is not present.
fn gate_entry<'a>(issue: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    issue["gates"].as_array()?.iter().find(|g| g["key"] == key)
}

fn add_multi_auto_gate_issue(cli: &JitCli, gates: &[(&str, &str)]) -> String {
    gates
        .iter()
        .for_each(|(key, checker)| define_auto_gate(cli, key, checker));
    let keys: Vec<&str> = gates.iter().map(|(key, _)| *key).collect();
    create_issue_requiring(cli, &keys)
}

fn enable_warn_leases(temp: &JitCli) {
    let config_path = temp.root().join(".jit").join("config.toml");
    let mut config = fs::read_to_string(&config_path).unwrap();
    config.push_str("\n[worktree]\nenforce_leases = \"warn\"\n");
    fs::write(config_path, config).unwrap();
}

#[test]
fn test_gate_status_no_prior_runs_shows_not_run_message() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");

    // gate status before any evaluate — should say not run yet, no mutation
    temp.command(["gate", "status", &issue_id, "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not been run").or(predicate::str::contains("no run")));

    // Confirm non-mutating: gate status should still be pending
    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate = gate_entry(&json, "test-gate").expect("test-gate in gates array");
    assert_eq!(gate["status"], "pending", "Expected pending, got: {gate}");
    assert!(
        gate["last_run_at"].is_null(),
        "last_run_at must be null before any run: {gate}"
    );
    assert!(
        gate["exit_code"].is_null(),
        "exit_code must be null before any run: {gate}"
    );
}

#[test]
fn test_issue_show_errors_on_corrupt_gate_run() {
    // A corrupt run record must make `issue show --json` fail loudly instead
    // of silently omitting the run. The deserialize cause and offending path
    // are in the error's source chain.
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");

    temp.run(["gate", "evaluate", &issue_id, "test-gate"])
        .unwrap();
    let issue: serde_json::Value = serde_json::from_slice(
        &fs::read(temp.root().join(format!(".jit/issues/{issue_id}.json"))).unwrap(),
    )
    .unwrap();
    let run_id =
        serde_json::from_value(issue["gates_status"]["test-gate"]["run_id"].clone()).unwrap();

    let result_path = temp.root().join(".jit").join(
        jit::repository_state::VirtualPath::gate_run_record(&issue_id.parse().unwrap(), &run_id)
            .relative()
            .as_path(),
    );
    fs::create_dir_all(result_path.parent().unwrap()).unwrap();
    fs::write(&result_path, "{ this is not valid json").unwrap();

    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    // The command must fail loudly with the gate-run load context instead of
    // silently succeeding with an omitted run.
    let envelope: serde_json::Value = serde_json::from_slice(&output)
        .unwrap_or_else(|error| panic!("expected a JSON failure envelope: {error}; {output:?}"));
    let message = envelope["error"]["message"]
        .as_str()
        .expect("failure envelope carries an error message");
    assert!(
        message.contains("Failed to load gate runs for issue"),
        "expected a contextual gate-run load error, got: {envelope}"
    );
}

#[test]
fn test_gate_status_shows_last_run_after_evaluate() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");

    // Execute the gate via pass
    temp.command(["gate", "evaluate", &issue_id, "test-gate"])
        .assert()
        .success();

    // gate status now shows last run
    temp.command(["gate", "status", &issue_id, "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"))
        .stdout(predicate::str::contains("exit 0").or(predicate::str::contains("exit_code")));
}

#[test]
fn test_gate_status_shows_last_run_after_failure() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 1");

    // Execute (will fail)
    temp.command(["gate", "evaluate", &issue_id, "test-gate"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Gate 'test-gate' failed"))
        .stderr(predicate::str::contains("jit gate status"));

    // gate status shows the failure
    temp.command(["gate", "status", &issue_id, "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("failed"));

    // Non-mutating: gate status itself didn't change status further
    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate = gate_entry(&json, "test-gate").expect("test-gate in gates array");
    assert_eq!(gate["status"], "failed", "Expected failed, got: {gate}");
}

#[test]
fn test_gate_evaluate_json_failure_matches_persisted_status() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(
        &temp,
        "test-gate",
        "printf checker-out && printf checker-err >&2 && exit 1",
    );

    let output = temp
        .command(["gate", "evaluate", &issue_id, "test-gate", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    assert_eq!(json["error"]["details"]["key"], "test-gate");
    assert_eq!(
        json["error"]["details"]["status"],
        jit::domain::GateRunStatus::Failed.as_str()
    );
    assert_eq!(
        json["error"]["details"]["checker_result"]["status"],
        "failed"
    );
    assert_eq!(json["error"]["details"]["checker_result"]["exit_code"], 1);
    assert!(
        json["error"]["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|suggestion| suggestion.as_str().unwrap().contains("jit gate status")),
        "expected remediation suggestion in JSON: {json}"
    );

    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let issue: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate = gate_entry(&issue, "test-gate").expect("test-gate in gates array");
    assert_eq!(gate["status"], "failed");
    assert_eq!(json["error"]["details"]["status"], gate["status"]);
    // The failed run is enriched onto the gate view.
    assert_eq!(gate["exit_code"], 1);
    assert!(gate["last_run_at"].is_string());
}

// jit:62f3bebd REQ-01 — gate failure output points at the run-history view
// (`gate status <id> <gate> --all`), which mining found agents rebuilding by
// hand instead of discovering.
#[test]
fn test_gate_evaluate_json_failure_suggests_run_history_view() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 1");

    let output = temp
        .command(["gate", "evaluate", &issue_id, "test-gate", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    assert!(
        json["error"]["suggestions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|suggestion| {
                let s = suggestion.as_str().unwrap();
                s.contains("jit gate status") && s.contains("--all")
            }),
        "expected a run-history (--all) suggestion in JSON: {json}"
    );
}

#[test]
fn test_gate_evaluate_stderr_failure_suggests_run_history_view() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 1");

    let output = temp
        .command(["gate", "evaluate", &issue_id, "test-gate"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--all"),
        "expected the non-JSON gate failure message to mention --all for run history, got: {stderr}"
    );
}

/// Commit the repository and lease `issue_id` to `agent:holder`.
fn lease_to_another_agent(temp: &JitCli, issue_id: &str) {
    temp.init_git().unwrap().commit("base").unwrap();
    temp.command_with_env(
        ["claim", "acquire", issue_id, "--ttl", "600"],
        [("JIT_AGENT_ID", "agent:holder")],
    )
    .assert()
    .success();
}

#[test]
fn test_gate_update_json_success_suppresses_human_lease_warnings() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");
    enable_warn_leases(&temp);
    lease_to_another_agent(&temp, &issue_id);

    let output = temp
        .command_with_env(
            [
                "gate",
                "update",
                "test-gate",
                "--title",
                "Updated",
                "--json",
            ],
            [("JIT_AGENT_ID", "agent:editor")],
        )
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(response["title"], "Updated");
}

#[test]
fn test_gate_preset_apply_partial_batch_retains_lease_warnings_in_both_modes() {
    let temp = JitCli::initialised().unwrap();
    let source = add_auto_gate_issue(&temp, "test-gate", "exit 0");
    temp.run(["gate", "preset", "create", &source, "team"])
        .unwrap();
    let target = super::create_titled_issue(&temp, "Target");
    enable_warn_leases(&temp);
    lease_to_another_agent(&temp, &target);

    for json in [false, true] {
        let mut args = vec!["gate", "preset", "apply", "team", &target, "00000000"];
        if json {
            args.push("--json");
        }
        let output = temp
            .command_with_env(args, [("JIT_AGENT_ID", "agent:editor")])
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(3));
        if json {
            assert!(output.stderr.is_empty(), "{output:?}");
            let response: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert!(response["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .any(|warning| { warning.as_str().is_some_and(|text| text.contains(&target)) }));
            assert!(response["error"]["details"]["success"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["issue_id"] == target));
        } else {
            assert!(String::from_utf8_lossy(&output.stderr).contains("Warning:"));
            assert!(String::from_utf8_lossy(&output.stdout).contains("Errors (1)"));
        }
    }
}

#[test]
fn test_gate_evaluate_json_failure_includes_lease_warnings() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 1");
    enable_warn_leases(&temp);
    lease_to_another_agent(&temp, &issue_id);

    let output = temp
        .command_with_env(
            ["gate", "evaluate", &issue_id, "test-gate", "--json"],
            [("JIT_AGENT_ID", "agent:evaluator")],
        )
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["error"]["code"], "GATE_FAILED");
    let warnings = json["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|warning| warning
            .as_str()
            .unwrap()
            .contains("is leased by agent:holder")),
        "expected lease warning beside the error: {json}"
    );
}

#[test]
fn test_gate_evaluate_human_failure_prints_lease_warnings() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 1");
    enable_warn_leases(&temp);
    lease_to_another_agent(&temp, &issue_id);

    temp.command_with_env(
        ["gate", "evaluate", &issue_id, "test-gate"],
        [("JIT_AGENT_ID", "agent:evaluator")],
    )
    .assert()
    .failure()
    .stderr(predicate::str::contains("Warning:"))
    .stderr(predicate::str::contains("is leased by agent:holder"))
    .stderr(predicate::str::contains("Gate 'test-gate' failed"));
}

#[test]
fn test_gate_status_is_non_mutating() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");

    // Two gate status calls — neither should mutate
    for _ in 0..2 {
        temp.command(["gate", "status", &issue_id, "test-gate"])
            .assert()
            .success();
    }

    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate = gate_entry(&json, "test-gate").expect("test-gate in gates array");
    assert_eq!(
        gate["status"], "pending",
        "Expected pending after check-only, got: {gate}"
    );
}

#[test]
fn test_gate_status_json_output() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");

    // Run the gate first so there's a result to show
    temp.command(["gate", "evaluate", &issue_id, "test-gate"])
        .assert()
        .success();

    let output = temp
        .command(["gate", "status", &issue_id, "test-gate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(json["run_id"].is_string(), "Missing run_id");
    assert!(json["key"].is_string(), "Missing key");
    assert!(json["status"].is_string(), "Missing status");
    assert!(!json["exit_code"].is_null(), "Missing exit_code");
    assert!(json["stdout"].is_string(), "Missing stdout");
    assert!(json["stderr"].is_string(), "Missing stderr");
    assert!(json["started_at"].is_string(), "Missing started_at");
}

#[test]
fn test_gate_evaluate_auto_executes_checker() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", "exit 0");

    temp.command(["gate", "evaluate", &issue_id, "test-gate"])
        .assert()
        .success();

    // The gate should be passed and enriched with its run.
    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let gate = gate_entry(&json, "test-gate").expect("test-gate in gates array");
    assert_eq!(gate["status"], "passed", "Expected passed, got: {gate}");
    assert_eq!(
        gate["exit_code"], 0,
        "exit_code should reflect the run: {gate}"
    );
    assert!(
        gate["last_run_at"].is_string(),
        "last_run_at should be set after a run: {gate}"
    );

    // A gate run result file should exist in .jit/gate-runs/
    let gate_runs_dir = temp.root().join(".jit").join("gate-runs");
    assert!(gate_runs_dir.exists(), ".jit/gate-runs/ should exist");
    let entries: Vec<_> = std::fs::read_dir(&gate_runs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .collect();
    assert!(
        !entries.is_empty(),
        "Expected at least one gate run result saved"
    );
}

#[test]
fn test_gate_define_manual_via_cli() {
    let temp = JitCli::initialised().unwrap();

    // Define a manual gate
    temp.command([
        "gate",
        "define",
        "code-review",
        "--title",
        "Code Review",
        "--description",
        "Human code review",
        "--stage",
        "postcheck",
        "--mode",
        "manual",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Defined gate 'code-review'"));

    // Verify gate was created by listing
    temp.command(["gate", "list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("code-review"))
        .stdout(predicate::str::contains("Code Review"));
}

#[test]
fn test_gate_define_automated_via_cli() {
    let temp = JitCli::initialised().unwrap();

    // Define an automated gate with checker
    temp.command([
        "gate",
        "define",
        "unit-tests",
        "--title",
        "Unit Tests",
        "--description",
        "Run all unit tests",
        "--stage",
        "postcheck",
        "--mode",
        "auto",
        "--checker-command",
        "cargo test",
        "--timeout",
        "300",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Defined gate 'unit-tests'"));

    // Show gate details
    temp.command(["gate", "show", "unit-tests"])
        .assert()
        .success()
        .stdout(predicate::str::contains("unit-tests"))
        .stdout(predicate::str::contains("Unit Tests"))
        .stdout(predicate::str::contains("cargo test"));
}

#[test]
fn test_gate_define_auto_without_checker_fails() {
    let temp = JitCli::initialised().unwrap();

    // Try to define auto gate without checker - should fail
    temp.command([
        "gate",
        "define",
        "bad-gate",
        "--title",
        "Bad Gate",
        "--description",
        "Missing checker",
        "--mode",
        "auto",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("checker"));
}

/// REQ-01: `--checker-command` with no explicit `--mode` yields an automated
/// gate — the checker must actually run rather than being silently discarded
/// by a defaulted-to-manual gate.
#[test]
fn test_gate_define_checker_command_without_mode_infers_auto() {
    let temp = JitCli::initialised().unwrap();

    temp.command([
        "gate",
        "define",
        "inferred-auto",
        "--title",
        "Inferred Auto",
        "--description",
        "No --mode, just a checker command",
        "--checker-command",
        "echo ok",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Defined gate 'inferred-auto'"));

    let output = temp
        .command(["gate", "show", "inferred-auto", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["mode"], "auto");
    assert_eq!(json["checker"]["command"], "echo ok");
}

/// REQ-02: an explicit `--mode manual` combined with `--checker-command` is a
/// usage error (exit 2) naming the conflict, not a silent checker drop.
#[test]
fn test_gate_define_explicit_manual_with_checker_command_is_usage_error() {
    let temp = JitCli::initialised().unwrap();

    temp.command([
        "gate",
        "define",
        "conflicted",
        "--title",
        "Conflicted",
        "--description",
        "Explicit manual + checker",
        "--mode",
        "manual",
        "--checker-command",
        "echo ok",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("manual"))
    .stderr(predicate::str::contains("checker"));

    // Nothing should have been persisted to the registry.
    temp.command(["gate", "show", "conflicted"])
        .assert()
        .failure();
}

/// REQ-02 (JSON path): the same conflict, machine-readable.
#[test]
fn test_gate_define_explicit_manual_with_checker_command_json_error() {
    let temp = JitCli::initialised().unwrap();

    let output = temp
        .command([
            "gate",
            "define",
            "conflicted-json",
            "--title",
            "Conflicted",
            "--description",
            "Explicit manual + checker",
            "--mode",
            "manual",
            "--checker-command",
            "echo ok",
            "--json",
        ])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value =
        serde_json::from_slice(&output).expect("error must be valid JSON");
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

/// REQ-01: `--auto` still wins over an explicit `--mode manual` (documented
/// convenience-flag precedence) even when a checker command is present, so it
/// is not treated as the REQ-02 conflict.
#[test]
fn test_gate_define_auto_flag_overrides_explicit_manual_mode() {
    let temp = JitCli::initialised().unwrap();

    temp.command([
        "gate",
        "define",
        "auto-wins",
        "--title",
        "Auto Wins",
        "--description",
        "auto overrides mode manual",
        "--mode",
        "manual",
        "--auto",
        "--checker-command",
        "echo ok",
    ])
    .assert()
    .success();

    let output = temp
        .command(["gate", "show", "auto-wins", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(json["mode"], "auto");
}

#[test]
fn test_gate_list_json() {
    let temp = JitCli::initialised().unwrap();

    // Define a gate
    temp.command([
        "gate",
        "define",
        "test-gate",
        "--title",
        "Test",
        "--description",
        "Test gate",
    ])
    .assert()
    .success();

    // List gates in JSON format
    let output = temp
        .command(["gate", "list", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    assert!(json["gates"].is_array());
}

#[test]
fn test_gate_show_json() {
    let temp = JitCli::initialised().unwrap();

    // Define a gate
    temp.command([
        "gate",
        "define",
        "test-gate",
        "--title",
        "Test Gate",
        "--description",
        "Test",
    ])
    .assert()
    .success();

    // Show gate in JSON format
    let output = temp
        .command(["gate", "show", "test-gate", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    // success field removed
    assert_eq!(json["key"], "test-gate");
    assert_eq!(json["title"], "Test Gate");
}

#[test]
fn test_gate_remove() {
    let temp = JitCli::initialised().unwrap();

    // Define a gate
    temp.command([
        "gate",
        "define",
        "test-gate",
        "--title",
        "Test",
        "--description",
        "Test",
    ])
    .assert()
    .success();

    // Remove the gate
    temp.command(["gate", "remove", "test-gate"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Removed gate 'test-gate'"));

    // Verify it's gone
    temp.command(["gate", "show", "test-gate"])
        .assert()
        .failure();
}

/// REQ-02 (jit:bb7d57a2), @/inv/event-log: `jit gate define` appends a
/// `gate_definition_created` event to the registry-scoped audit log, mirroring
/// `gate_update_test.rs`'s `test_gate_update_appends_event` for
/// `gate_definition_updated`.
#[test]
fn test_gate_define_appends_event() {
    let temp = JitCli::initialised().unwrap();

    temp.command([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Run tests",
    ])
    .assert()
    .success();

    let events_path = temp.root().join(".jit").join("events.jsonl");
    let contents = fs::read_to_string(&events_path).unwrap();
    let found = contents.lines().any(|line| {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        v["type"] == "gate_definition_created" && v["gate_key"] == "tests"
    });
    assert!(
        found,
        "a gate_definition_created event for 'tests' must be logged"
    );
}

/// REQ-02 (jit:bb7d57a2), @/inv/event-log: `jit gate remove` appends a
/// `gate_definition_removed` event to the registry-scoped audit log, mirroring
/// `gate_update_test.rs`'s `test_gate_update_appends_event` for
/// `gate_definition_updated`.
#[test]
fn test_gate_remove_appends_event() {
    let temp = JitCli::initialised().unwrap();

    temp.command([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "Run tests",
    ])
    .assert()
    .success();

    temp.command(["gate", "remove", "tests"]).assert().success();

    let events_path = temp.root().join(".jit").join("events.jsonl");
    let contents = fs::read_to_string(&events_path).unwrap();
    let found = contents.lines().any(|line| {
        let v: serde_json::Value = serde_json::from_str(line).unwrap();
        v["type"] == "gate_definition_removed" && v["gate_key"] == "tests"
    });
    assert!(
        found,
        "a gate_definition_removed event for 'tests' must be logged"
    );
}

#[test]
fn test_gate_status_single() {
    let temp = JitCli::initialised().unwrap();

    // Define an automated gate that succeeds
    temp.command([
        "gate",
        "define",
        "quick-check",
        "--title",
        "Quick Check",
        "--description",
        "Quick check",
        "--mode",
        "auto",
        "--checker-command",
        "exit 0",
        "--timeout",
        "10",
    ])
    .assert()
    .success();

    // Create an issue with the gate
    let issue_id = temp
        .create_issue(["--title", "Test issue", "--gate", "quick-check"])
        .unwrap();
    let issue_id = issue_id.as_str();

    // Run the gate first (gate evaluate executes the checker for auto gates)
    temp.command(["gate", "evaluate", issue_id, "quick-check"])
        .assert()
        .success();

    // Now inspect the last run result (gate status is inspection-only)
    temp.command(["gate", "status", issue_id, "quick-check"])
        .assert()
        .success()
        .stdout(predicate::str::contains("passed"));
}

#[test]
fn test_gate_status_all() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_multi_auto_gate_issue(
        &temp,
        &[
            ("gate-1", "printf gate-1-out && exit 0"),
            ("gate-2", "printf gate-2-out && exit 0"),
        ],
    );

    temp.command(["gate", "evaluate", &issue_id, "gate-1"])
        .assert()
        .success();
    temp.command(["gate", "evaluate", &issue_id, "gate-2"])
        .assert()
        .success();

    temp.command(["gate", "status-all", &issue_id])
        .assert()
        .success()
        .stdout(predicate::str::contains("Gate 'gate-1' last run: passed"))
        .stdout(predicate::str::contains("Gate 'gate-2' last run: passed"))
        .stdout(predicate::str::contains("gate-1-out"))
        .stdout(predicate::str::contains("gate-2-out"));
}

#[test]
fn test_gate_status_all_no_prior_runs_is_non_mutating() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_multi_auto_gate_issue(&temp, &[("gate-1", "exit 0"), ("gate-2", "exit 0")]);

    // All required gates pending -> strict `status-all` fails (non-mutating).
    temp.command(["gate", "status-all", &issue_id])
        .assert()
        .failure()
        .stdout(predicate::str::contains("gate-1"))
        .stdout(predicate::str::contains("not been run yet"))
        .stdout(predicate::str::contains("gate-2"));

    let output = temp
        .command(["issue", "show", &issue_id, "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    for gate_key in ["gate-1", "gate-2"] {
        let gate = gate_entry(&json, gate_key).expect("gate in gates array");
        assert_eq!(
            gate["status"], "pending",
            "Expected pending for {gate_key}, got: {gate}"
        );
        assert!(
            gate["last_run_at"].is_null(),
            "last_run_at must be null for un-run {gate_key}: {gate}"
        );
    }
}

#[test]
fn test_gate_status_all_json_output_reports_not_run_gates() {
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_multi_auto_gate_issue(
        &temp,
        &[
            ("gate-1", "printf gate-1-out && exit 0"),
            ("gate-2", "printf gate-2-out && exit 0"),
        ],
    );

    temp.command(["gate", "evaluate", &issue_id, "gate-1"])
        .assert()
        .success();

    // gate-1 passed, gate-2 still pending -> strict `status-all` fails.
    let output = temp
        .command(["gate", "status-all", &issue_id, "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let envelope: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let json = &envelope["error"]["details"];
    assert_eq!(json["total"].as_u64(), Some(2));
    assert_eq!(json["passed"].as_u64(), Some(1));
    assert_eq!(json["results"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        json["results"][0]["key"].as_str(),
        Some("gate-1"),
        "Expected only gate-1 to have a recorded run"
    );
    assert_eq!(
        json["not_run"].as_array().and_then(|items| items.first()),
        Some(&serde_json::Value::String("gate-2".to_string()))
    );
}

#[test]
fn test_gate_define_with_env_vars() {
    let temp = JitCli::initialised().unwrap();

    // Define a gate with --env flags
    temp.command([
        "gate",
        "define",
        "review",
        "--title",
        "AI Review",
        "--description",
        "AI-powered code review",
        "--mode",
        "auto",
        "--checker-command",
        "echo ok",
        "--env",
        "REVIEWER_AGENT=copilot -s",
        "--env",
        "MODEL=test-review-model",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains("Defined gate 'review'"));

    // Verify env vars persisted via --json
    let output = temp
        .command(["gate", "show", "review", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let checker = &json["checker"];
    assert_eq!(checker["env"]["REVIEWER_AGENT"], "copilot -s");
    assert_eq!(checker["env"]["MODEL"], "test-review-model");
}

#[test]
fn test_gate_define_env_invalid_format_fails() {
    let temp = JitCli::initialised().unwrap();

    // --env without = should fail
    temp.command([
        "gate",
        "define",
        "bad-env",
        "--title",
        "Bad",
        "--description",
        "Bad env",
        "--mode",
        "auto",
        "--checker-command",
        "echo ok",
        "--env",
        "NO_EQUALS_SIGN",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("KEY=VALUE"));
}

#[test]
fn test_gate_env_vars_passed_to_checker() {
    let temp = JitCli::initialised().unwrap();

    // Define a gate with env vars; checker prints them
    temp.command([
        "gate",
        "define",
        "env-test",
        "--title",
        "Env Test",
        "--description",
        "Test env passing",
        "--mode",
        "auto",
        "--checker-command",
        "echo FOO=$FOO BAZ=$BAZ",
        "--env",
        "FOO=bar",
        "--env",
        "BAZ=qux",
    ])
    .assert()
    .success();

    // Create issue with the gate
    let issue_id = temp
        .create_issue(["--title", "Test issue", "--gate", "env-test"])
        .unwrap();
    let issue_id = issue_id.as_str();

    // Run the gate (gate evaluate executes checker for auto gates)
    temp.command(["gate", "evaluate", issue_id, "env-test"])
        .assert()
        .success();

    // Inspect last run result — checker should have seen the env vars
    let output = temp
        .command(["gate", "status", issue_id, "env-test", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let json: serde_json::Value = serde_json::from_slice(&output).unwrap();
    let stdout = json["stdout"].as_str().unwrap();
    assert!(
        stdout.contains("FOO=bar"),
        "Expected FOO=bar in stdout: {}",
        stdout
    );
    assert!(
        stdout.contains("BAZ=qux"),
        "Expected BAZ=qux in stdout: {}",
        stdout
    );
}

#[test]
fn test_gate_status_shows_complete_report_without_truncation() {
    // Checker that produces 25 lines of stdout (more than the old 20-line truncation limit)
    let checker_cmd =
        "sh -c 'i=1; while [ $i -le 25 ]; do echo \"output-line-$i\"; i=$((i+1)); done'";
    let temp = JitCli::initialised().unwrap();
    let issue_id = add_auto_gate_issue(&temp, "test-gate", checker_cmd);

    // Run the gate to record a result with 25 lines of stdout
    temp.command(["gate", "evaluate", &issue_id, "test-gate"])
        .assert()
        .success();

    // gate status must emit the full stored report — line 21 must appear
    let output = temp
        .command(["gate", "status", &issue_id, "test-gate"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let stdout_str = String::from_utf8_lossy(&output);

    assert!(
        stdout_str.contains("output-line-21"),
        "gate status must emit the full report; line 21 was absent. Output:\n{}",
        stdout_str
    );

    assert!(
        !stdout_str.contains("Full output:"),
        "gate status must not substitute a file path for the report body. Output:\n{}",
        stdout_str
    );
}

#[test]
fn test_gate_fail_gate_not_required_is_invalid_argument() {
    let temp = JitCli::initialised().unwrap();

    let id = temp.create_issue(["--title", "No gates"]).unwrap();

    let output = temp
        .command(["gate", "fail", &id, "not-a-gate", "--json"])
        .output()
        .unwrap();

    assert!(
        !output.status.success(),
        "stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"]["code"], "INVALID_ARGUMENT");
}

#[test]
fn test_issue_responses_do_not_open_unreferenced_gate_runs() {
    let cli = JitCli::initialised().unwrap();
    let child = add_auto_gate_issue(&cli, "test-gate", "exit 0");
    cli.run(["gate", "evaluate", &child, "test-gate"]).unwrap();
    let parent = cli.create_issue(["--title", "Parent"]).unwrap();
    cli.run(["dep", "add", &parent, &child]).unwrap();
    let path = cli.root().join(".jit").join(
        jit::repository_state::VirtualPath::gate_run_record(
            &child.parse().unwrap(),
            &jit::test_utils::gate_run_id(1),
        )
        .relative()
        .as_path(),
    );
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, "{ corrupt unreferenced run").unwrap();

    for args in [
        vec!["issue", "show", &child],
        vec!["issue", "status", &child],
        vec!["issue", "children", &parent],
        vec!["issue", "create", "--title", "New"],
    ] {
        cli.run_json(args).unwrap();
    }
}
