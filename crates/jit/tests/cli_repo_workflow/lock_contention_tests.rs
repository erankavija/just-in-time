//! Cross-process lock contention: a jit command queued behind another holder
//! waits for the configured lock wait, and a wait that expires is reported as a
//! lock timeout rather than as the command's own failure.

use jit::storage::FileLocker;
use jit::test_cli::JitCli;
use serde_json::Value;
use std::process::Output;
use std::time::Duration;

fn succeeded(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
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
    assert!(
        !output.status.success(),
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
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("root").unwrap();
    let id = cli.create_issue(["Claimable"]).unwrap();
    let locks = cli.root().join(".git/jit/locks");
    std::fs::create_dir_all(&locks).unwrap();
    let _held = FileLocker::new(Duration::from_secs(5))
        .lock_exclusive(&locks.join("claims.lock"))
        .unwrap();

    let started = std::time::Instant::now();
    let output = cli
        .command_with_env(
            [
                "claim",
                "acquire",
                &id,
                "--agent-id",
                "agent:lock-test",
                "--json",
            ],
            [("JIT_LOCK_TIMEOUT", "1")],
        )
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
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("root").unwrap();
    let held = FileLocker::new(Duration::from_secs(5))
        .lock_exclusive(&cli.root().join(".jit-bootstrap.lock"))
        .unwrap();
    let contenders = ["First", "Second"].map(|title| {
        cli.command(["issue", "create", title, "--json"])
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
    assert_eq!(
        cli.run_json(["issue", "list"]).unwrap()["issues"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
fn test_validate_reads_while_a_writer_holds_the_bootstrap_lock() {
    let cli = JitCli::initialised().unwrap();
    cli.create_issue(["Validated"]).unwrap();
    let _held = FileLocker::new(Duration::from_secs(5))
        .lock_exclusive(&cli.root().join(".jit-bootstrap.lock"))
        .unwrap();

    // A read waiting on the holder would time out after one second.
    let output = cli
        .command_with_env(["validate", "--json"], [("JIT_LOCK_TIMEOUT", "1")])
        .output()
        .unwrap();

    assert_eq!(envelope(&succeeded(output))["valid"], true);
}
