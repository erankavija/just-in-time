//! Cross-process lock contention: a jit command queued behind another holder
//! waits for the configured lock wait, and a wait that expires is reported as a
//! lock timeout rather than as the command's own failure.

use jit::storage::FileLocker;
use serde_json::Value;
use std::path::Path;
use std::process::{Command, Output};
use std::time::Duration;
use tempfile::TempDir;

fn jit(repository: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jit"));
    command
        .args(args)
        .env_remove("JIT_LOCK_TIMEOUT")
        .current_dir(repository);
    command
}

fn succeeded(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_repository_with_jit() -> TempDir {
    let repository = TempDir::new().unwrap();
    [
        vec!["init", "-q"],
        vec!["config", "user.name", "Lock Test"],
        vec!["config", "user.email", "lock@example.com"],
        vec!["commit", "-q", "--allow-empty", "-m", "root"],
    ]
    .iter()
    .for_each(|args| {
        succeeded(
            Command::new("git")
                .args(args)
                .current_dir(repository.path())
                .output()
                .unwrap(),
        );
    });
    succeeded(jit(repository.path(), &["init"]).output().unwrap());
    repository
}

fn envelope(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "no JSON envelope: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_lock_timeout(output: &Output) {
    assert_eq!(
        output.status.code(),
        Some(jit::output::ExitCode::LockTimeout.code()),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        envelope(output)["error"]["code"],
        jit::output::ErrorCode::LockTimeout.as_str()
    );
}

#[test]
fn test_claim_coordination_lock_honors_configured_wait() {
    let repository = git_repository_with_jit();
    let id = envelope(&succeeded(
        jit(
            repository.path(),
            &["issue", "create", "Claimable", "--json"],
        )
        .output()
        .unwrap(),
    ))["id"]
        .as_str()
        .unwrap()
        .to_string();
    let locks = repository.path().join(".git/jit/locks");
    std::fs::create_dir_all(&locks).unwrap();
    let _held = FileLocker::new(Duration::from_secs(5))
        .lock_exclusive(&locks.join("claims.lock"))
        .unwrap();

    let started = std::time::Instant::now();
    let output = jit(
        repository.path(),
        &[
            "claim",
            "acquire",
            &id,
            "--agent-id",
            "agent:lock-test",
            "--json",
        ],
    )
    .env("JIT_LOCK_TIMEOUT", "1")
    .output()
    .unwrap();

    assert_lock_timeout(&output);
    // The one-second configured wait, not the default, bounded the claim wait.
    assert!(
        started.elapsed() < Duration::from_secs(jit::runtime_defaults::LOCK_TIMEOUT_SECS / 2),
        "claim coordination ignored JIT_LOCK_TIMEOUT: waited {:?}",
        started.elapsed()
    );
}

#[test]
fn test_concurrent_mutations_queue_behind_a_holder_under_the_default_wait() {
    let repository = git_repository_with_jit();
    let held = FileLocker::new(Duration::from_secs(5))
        .lock_exclusive(&repository.path().join(".jit-bootstrap.lock"))
        .unwrap();
    let contenders = ["First", "Second"].map(|title| {
        jit(repository.path(), &["issue", "create", title, "--json"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    });

    // Both contenders start while the lock is held and must queue behind it.
    std::thread::sleep(Duration::from_secs(2));
    drop(held);

    contenders.into_iter().for_each(|contender| {
        succeeded(contender.wait_with_output().unwrap());
    });
    let listed = envelope(&succeeded(
        jit(repository.path(), &["issue", "list", "--json"])
            .output()
            .unwrap(),
    ));
    assert_eq!(listed["count"], 2);
}
