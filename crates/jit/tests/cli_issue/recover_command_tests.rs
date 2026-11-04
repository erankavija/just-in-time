//! Tests for the jit recover command

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;
use predicates::prelude::*;
use serde_json::Value;

#[test]
fn test_recover_command_exists() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    std::fs::write(cli.root().join("README.md"), "# Test").unwrap();
    cli.commit("Initial commit").unwrap();
    cli.run(["init"]).unwrap();

    cli.command(["recover"]).assert().success();
}

#[test]
fn test_recover_with_json_output() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    std::fs::write(cli.root().join("README.md"), "# Test").unwrap();
    cli.commit("Initial commit").unwrap();
    cli.run(["init"]).unwrap();

    cli.command(["recover", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"success\""));
}

#[test]
fn test_recover_json_failure_writes_registered_pretty_envelope_to_stdout() {
    let cli = JitCli::initialised().unwrap();

    cli.init_git().unwrap();

    let assert = cli.command(["recover", "--json"]).assert().failure();
    let output = assert.get_output();
    let stdout = std::str::from_utf8(&output.stdout).unwrap();
    let stderr = std::str::from_utf8(&output.stderr).unwrap();
    let envelope: Value = serde_json::from_str(stdout).expect("stdout must be one JSON document");

    assert!(
        stdout.contains("\n  \"error\":"),
        "stdout must be pretty JSON: {stdout}"
    );
    assert!(
        stdout.ends_with('\n'),
        "stdout must end with one newline: {stdout}"
    );
    assert_eq!(envelope["error"]["code"], "recovery_failed");
    assert!(
        !stderr.contains('{'),
        "stderr must not contain JSON: {stderr}"
    );
}

#[test]
fn test_recover_reports_actions_taken() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    std::fs::write(cli.root().join("README.md"), "# Test").unwrap();
    cli.commit("Initial commit").unwrap();
    cli.run(["init"]).unwrap();

    cli.command(["recover"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Recovery complete"));
}

#[test]
fn test_recover_help_describes_purpose() {
    let cli = JitCli::uninitialised().unwrap();
    cli.command(["recover", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("recovery"))
        .stdout(predicate::str::contains("stale"));
}

#[test]
fn test_recover_reports_the_leftover_redo_record_its_session_applied() {
    use jit::storage::{commit_redo_record, DataRoot, PutFile, RedoRecord};

    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    std::fs::write(cli.root().join("README.md"), "# Test").unwrap();
    cli.commit("Initial commit").unwrap();
    cli.run(["init"]).unwrap();
    let published = cli.root().join("applied-by-recovery.md");
    let record = RedoRecord::new(
        Vec::new(),
        vec![PutFile {
            path: jit::repository_state::VirtualPath::worktree("applied-by-recovery.md").unwrap(),
            bytes: b"published by the leftover record\n".to_vec(),
            mode: jit::repository_state::FileMode::Regular,
        }],
        Vec::new(),
        None,
    )
    .unwrap();
    commit_redo_record(&DataRoot::open(&cli.root().join(".jit")).unwrap(), &record).unwrap();

    let output = cli
        .command(["recover", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let value: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(
        value["redo_records_applied"], 1,
        "the applied record must reach the recover command's count, got {value}"
    );
    assert!(
        value.get("transactions_recovered").is_none(),
        "the superseded key must be gone, got {value}"
    );
    assert!(
        published.exists(),
        "the startup session must have applied the record it counted"
    );
}

/// A leftover record the session cannot read fails `jit recover` as a recovery
/// failure; applying one is the command's own work.
#[test]
fn test_recover_json_classifies_an_unreadable_leftover_record_as_recovery_failed() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    std::fs::write(cli.root().join("README.md"), "# Test").unwrap();
    cli.commit("Initial commit").unwrap();
    cli.run(["init"]).unwrap();
    let scratch = cli
        .root()
        .join(".jit")
        .join(jit::storage::RECORD_SCRATCH_DIR);
    std::fs::create_dir_all(&scratch).unwrap();
    std::fs::write(scratch.join(jit::storage::RECORD_FILE_NAME), b"no header").unwrap();

    let output = cli
        .command(["recover", "--json"])
        .assert()
        .failure()
        .get_output()
        .stdout
        .clone();

    let envelope: Value = serde_json::from_slice(&output).unwrap();
    assert_eq!(envelope["error"]["code"], "recovery_failed", "{envelope}");
}
