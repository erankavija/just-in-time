//! `jit worktree exec`: running a command holding the repository write lock.

use jit::output::ErrorCode;
use jit::storage::{FileLocker, RECORD_FILE_NAME, RECORD_SCRATCH_DIR};
use jit::test_cli::{jit_binary, JitCli};
use serde_json::Value;
use std::process::Output;
use std::time::Duration;

fn exec(cli: &JitCli, command: &[&str]) -> Output {
    cli.command_with_env(
        ["worktree", "exec", "--json", "--"]
            .iter()
            .chain(command)
            .collect::<Vec<_>>(),
        [("JIT_LOCK_TIMEOUT", "1")],
    )
    .output()
    .unwrap()
}

fn assert_refused(output: &Output, code: ErrorCode) {
    assert_eq!(output.status.code(), Some(code.exit_code().code()));
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(envelope["error"]["code"], code.as_str());
}

#[test]
fn test_worktree_exec_answers_the_child_exit_status() {
    let cli = JitCli::initialised().unwrap();

    let output = exec(&cli, &["sh", "-c", "exit 7"]);

    assert_eq!(output.status.code(), Some(7));
}

#[test]
fn test_worktree_exec_child_jit_mutation_fails_on_its_bounded_lock_wait() {
    let cli = JitCli::initialised().unwrap();
    let jit = jit_binary();

    let output = exec(
        &cli,
        &[
            jit.to_str().unwrap(),
            "issue",
            "create",
            "--title",
            "Child",
            "--json",
        ],
    );

    assert_refused(&output, ErrorCode::LockTimeout);
    assert_eq!(cli.run_json(["query", "all"]).unwrap()["count"], 0);
}

#[test]
fn test_worktree_exec_child_jit_read_does_not_wait_for_the_lock() {
    let cli = JitCli::initialised().unwrap();
    let jit = jit_binary();

    let output = exec(&cli, &[jit.to_str().unwrap(), "query", "all", "--json"]);

    assert!(output.status.success());
}

#[test]
fn test_worktree_exec_refuses_a_leftover_redo_record_without_applying_it() {
    let cli = JitCli::initialised().unwrap();
    let record = cli
        .root()
        .join(".jit")
        .join(RECORD_SCRATCH_DIR)
        .join(RECORD_FILE_NAME);
    std::fs::create_dir_all(record.parent().unwrap()).unwrap();
    std::fs::write(&record, "leftover").unwrap();

    let output = exec(&cli, &["sh", "-c", "touch ran"]);

    assert_refused(&output, ErrorCode::GenericError);
    let envelope: Value = serde_json::from_slice(&output.stdout).unwrap();
    let message = envelope["error"]["message"].as_str().unwrap();
    assert!(message.contains(record.to_str().unwrap()), "{message}");
    assert!(message.contains("jit recover"), "{message}");
    assert!(!cli.root().join("ran").exists());
    assert_eq!(std::fs::read_to_string(&record).unwrap(), "leftover");
}

#[test]
fn test_worktree_exec_lock_wait_expires_as_a_lock_timeout_without_running() {
    let cli = JitCli::initialised().unwrap();
    let _held = FileLocker::new(Duration::from_secs(5))
        .lock_exclusive(&cli.root().join(".jit/.repo-write.lock"))
        .unwrap();

    let output = exec(&cli, &["sh", "-c", "touch ran"]);

    assert_refused(&output, ErrorCode::LockTimeout);
    assert!(!cli.root().join("ran").exists());
}
