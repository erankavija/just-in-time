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

// ── foreground serve end-to-end: parent must release the recovery lock ────────

/// Extracts the port from a `Starting server on http://localhost:<port> …` line.
#[cfg(unix)]
fn parse_localhost_port(line: &str) -> Option<u16> {
    const MARK: &str = "http://localhost:";
    let rest = &line[line.find(MARK)? + MARK.len()..];
    rest.chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// Resolves the `jit-server` binary the foreground path will spawn.
///
/// `find_server_binary` looks for a sibling of the `jit` executable first, and
/// `cargo test --workspace` (the CI path) always builds that sibling. An
/// isolated `cargo test -p jit --test cli_repo_workflow` may not have, so build
/// it best-effort to keep this test runnable in isolation too.
#[cfg(unix)]
fn ensure_jit_server_binary() {
    use std::process::Command;
    let jit_bin: std::path::PathBuf = assert_cmd::cargo::cargo_bin!("jit").into();
    let server_bin = jit_bin.with_file_name("jit-server");
    if server_bin.exists() {
        return;
    }
    let status = Command::new(env!("CARGO"))
        .args(["build", "-p", "jit-server"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .status()
        .expect("spawn cargo build -p jit-server");
    assert!(
        status.success() && server_bin.exists(),
        "failed to build the jit-server sibling required by the serve --fg test"
    );
}

/// `jit serve --fg` must serve end to end even though the parent process holds
/// the bootstrap → repository recovery lock. Foreground `serve` is classified
/// as requiring pre-service recovery dispatch, so the parent acquires that lock
/// at startup; it then blocks on `child.wait()`. The spawned `jit-server` child
/// runs its own bootstrap recovery on startup, which needs the same
/// cross-process lock. Before the fix the parent retained the lock for its whole
/// lifetime, so the child could never acquire it — it timed out and the server
/// never came up. The parent must release its recovery session before waiting.
#[test]
#[cfg(unix)]
fn test_serve_fg_serves_when_child_runs_bootstrap_recovery() {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpStream;
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    // A real HTTP round-trip, not a bare TCP connect: the parent binds the
    // listening socket and hands its fd to the child, so the kernel accepts
    // connections into the listen backlog even while the child is still
    // blocked on the recovery lock and serving nothing. Only a completed
    // HTTP response proves the child actually adopted the socket and is
    // serving.
    let server_responds = |port: u16| -> bool {
        let Ok(mut stream) = TcpStream::connect(("127.0.0.1", port)) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_millis(500)));
        let _ = stream.set_write_timeout(Some(Duration::from_millis(500)));
        if stream
            .write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .is_err()
        {
            return false;
        }
        let mut buf = [0u8; 5];
        stream.read_exact(&mut buf).is_ok() && &buf == b"HTTP/"
    };

    ensure_jit_server_binary();
    let temp = setup_repo();

    // Own process group so the whole tree — the `jit` parent AND the
    // `jit-server` child it blocks on (which shares this group in foreground
    // mode) — can be reaped together; killing only the parent would orphan a
    // live server and wedge the stdout drain below on an open pipe.
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command
        .current_dir(temp.path())
        .args(["serve", "--fg", "--port", "0"])
        // Bound the pre-fix failure window: a regressed parent that keeps the
        // lock makes the child give up after this timeout instead of at the
        // 5s default, so the assertion below fails fast rather than dragging.
        .env("JIT_LOCK_TIMEOUT", "3")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().expect("spawn jit serve --fg");
    let pgid = child.id();

    // Drain both pipes on their own threads: the parent prints the chosen port
    // before spawning the server, and unread pipes would eventually block the
    // server child. Each thread returns its captured text for failure output.
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let (port_tx, port_rx) = mpsc::channel::<u16>();
    let out_handle = thread::spawn(move || {
        let mut collected = String::new();
        let mut sent = false;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if !sent {
                if let Some(port) = parse_localhost_port(&line) {
                    let _ = port_tx.send(port);
                    sent = true;
                }
            }
            collected.push_str(&line);
            collected.push('\n');
        }
        collected
    });
    let err_handle = thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });

    let reap = |child: &mut std::process::Child| {
        // Negative PID signals the whole process group (parent + server child).
        let _ = Command::new("kill")
            .arg("-KILL")
            .arg(format!("-{pgid}"))
            .status();
        let _ = child.wait();
    };

    let port = match port_rx.recv_timeout(Duration::from_secs(20)) {
        Ok(port) => port,
        Err(_) => {
            reap(&mut child);
            panic!(
                "jit serve --fg never announced a port\n--- stderr ---\n{}",
                err_handle.join().unwrap_or_default()
            );
        }
    };

    // Poll until the server answers an HTTP request, or the parent exits early
    // (the pre-fix symptom: the child's lock wait times out and the parent's
    // child.wait() returns).
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut served = false;
    while Instant::now() < deadline {
        if server_responds(port) {
            served = true;
            break;
        }
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }

    reap(&mut child);
    let out = out_handle.join().unwrap_or_default();
    let err = err_handle.join().unwrap_or_default();
    assert!(
        served,
        "jit serve --fg never began serving on port {port}: the parent held the \
         bootstrap recovery lock while the child needed it.\n\
         --- stdout ---\n{out}\n--- stderr ---\n{err}"
    );
}
