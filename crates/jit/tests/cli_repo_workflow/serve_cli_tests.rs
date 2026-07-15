//! Integration tests for `jit serve` CLI command.
//!
//! These tests cover output contracts for `--status`, `--stop`, `--json`,
//! stale PID cleanup, and MCP schema exclusion. They do NOT start a live
//! jit-server process (which would require the binary to be on PATH).

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(temp.path())
        .arg("init")
        .assert()
        .success();
    temp
}

fn jit(dir: &TempDir) -> Command {
    let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    cmd.current_dir(dir.path());
    cmd
}

fn write_stale_pid(dir: &TempDir, pid: u32, port: u16) {
    let jit_dir = dir.path().join(".jit");
    let pid_json = serde_json::json!({
        "pid": pid,
        "port": port,
        "started_at": "2024-01-01T00:00:00Z",
        "data_dir": jit_dir.to_str().unwrap(),
        "log_file": jit_dir.join("server.log").to_str().unwrap()
    });
    fs::write(
        jit_dir.join("server.pid.json"),
        serde_json::to_string_pretty(&pid_json).unwrap(),
    )
    .unwrap();
}

// ── --status: no server running ───────────────────────────────────────────────

#[test]
fn test_serve_status_not_running_human() {
    let temp = setup_repo();
    jit(&temp)
        .args(["serve", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));
}

#[test]
fn test_serve_status_not_running_json() {
    let temp = setup_repo();
    let output = jit(&temp)
        .args(["serve", "--status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: Value = serde_json::from_slice(&output).expect("stdout must be valid JSON");
    assert_eq!(v["status"], "not_running");
}

// ── --stop: no server running ─────────────────────────────────────────────────

#[test]
fn test_serve_stop_not_running_human() {
    let temp = setup_repo();
    jit(&temp)
        .args(["serve", "--stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));
}

#[test]
fn test_serve_stop_not_running_json() {
    let temp = setup_repo();
    let output = jit(&temp)
        .args(["serve", "--stop", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: Value = serde_json::from_slice(&output).expect("stdout must be valid JSON");
    assert_eq!(v["status"], "not_running");
}

// ── stale PID file cleanup ────────────────────────────────────────────────────

#[test]
fn test_serve_status_cleans_stale_pid() {
    let temp = setup_repo();
    // PID 999999 is almost certainly dead.
    write_stale_pid(&temp, 999_999, 3050);

    let pid_file = temp.path().join(".jit").join("server.pid.json");
    assert!(
        pid_file.exists(),
        "stale pid file should be present before check"
    );

    jit(&temp)
        .args(["serve", "--status"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));

    assert!(
        !pid_file.exists(),
        "stale pid file should be removed after --status"
    );
}

#[test]
fn test_serve_stop_cleans_stale_pid() {
    let temp = setup_repo();
    write_stale_pid(&temp, 999_999, 3051);

    let pid_file = temp.path().join(".jit").join("server.pid.json");
    assert!(pid_file.exists());

    jit(&temp)
        .args(["serve", "--stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));

    assert!(
        !pid_file.exists(),
        "stale pid file should be removed after --stop"
    );
}

// ── JSON status fields ────────────────────────────────────────────────────────

#[test]
fn test_serve_status_json_has_required_fields_when_not_running() {
    let temp = setup_repo();
    let output = jit(&temp)
        .args(["serve", "--status", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let v: Value = serde_json::from_slice(&output).unwrap();
    assert!(v.get("status").is_some(), "must have 'status' field");
}

// ── MCP schema exclusion ──────────────────────────────────────────────────────

#[test]
fn test_serve_not_in_mcp_schema() {
    let temp = setup_repo();
    let output = jit(&temp)
        .arg("--schema")
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();

    let schema: Value = serde_json::from_slice(&output).expect("schema must be valid JSON");
    let commands = schema["commands"]
        .as_object()
        .expect("commands must be an object");
    // 'serve' may appear in the schema but must be marked hidden (so MCP won't expose it).
    if let Some(serve_cmd) = commands.get("serve") {
        assert_eq!(
            serve_cmd["hidden"], true,
            "'serve' must be marked hidden in MCP schema"
        );
    }
    // Either absent or hidden is acceptable — just not visible (hidden: false or missing).
}

// ── --stop: stale PID with pid=0 guard ───────────────────────────────────────

#[test]
fn test_serve_stop_rejects_pid_zero_gracefully() {
    let temp = setup_repo();
    // A PID of 0 is not alive (is_process_alive returns false), so it should
    // be treated as stale and cleaned up rather than attempting to signal it.
    write_stale_pid(&temp, 0, 3052);

    jit(&temp)
        .args(["serve", "--stop"])
        .assert()
        .success()
        .stdout(predicate::str::contains("not running"));
}
