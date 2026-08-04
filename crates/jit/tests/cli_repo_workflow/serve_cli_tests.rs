//! Integration tests for `jit serve` CLI command.
//!
//! Most cases cover output contracts for `--status`, `--stop`, `--json`, stale
//! PID cleanup, and MCP schema exclusion without starting a server. The
//! foreground case at the end does start a live `jit-server`, and runs only
//! when one was built beside the `jit` binary under test.

use assert_cmd::prelude::*;
use predicates::prelude::*;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ── bounded waiting ──────────────────────────────────────────────────────────

/// Interval between two evaluations of a polled condition.
const POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Wall-clock ceiling for the `jit init` that seeds one scratch repository.
const SETUP_BUDGET: Duration = Duration::from_secs(30);

/// Grace period for a process to die once it has been signalled.
const REAP_GRACE: Duration = Duration::from_secs(5);

/// Evaluates `condition` until it holds or `deadline` passes, answering whether
/// it held.
///
/// The condition is evaluated at least once, and no sleep here runs past
/// `deadline`, so the call returns within the caller's budget whatever the
/// condition observes.
fn poll_until(deadline: Instant, mut condition: impl FnMut() -> bool) -> bool {
    loop {
        if condition() {
            return true;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL.min(remaining));
    }
}

/// Waits for `child` to exit, giving up at `deadline`; answers whether it
/// exited. A child observed exited here has also been reaped.
fn exited_by(child: &mut Child, deadline: Instant) -> bool {
    poll_until(deadline, || matches!(child.try_wait(), Ok(Some(_))))
}

/// Spawns `command` with its stdout and stderr redirected to `out` and `err`.
///
/// Files rather than pipes, so that reading a spawned process's output cannot
/// block: a pipe stays open for as long as *any* descendant holds its inherited
/// write end, so one surviving grandchild keeps a reader from ever reaching end
/// of file, while a regular file always reads to end of file (jit:76a4bd21).
fn spawn_capturing(command: &mut Command, out: &Path, err: &Path) -> std::io::Result<Child> {
    command
        .stdin(Stdio::null())
        .stdout(fs::File::create(out)?)
        .stderr(fs::File::create(err)?)
        .spawn()
}

/// Reads a capture file, answering with the empty string when it cannot be read.
fn read_capture(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_default()
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn setup_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    // The captures live outside the repository being seeded, so `jit init` sees
    // the empty directory it expects.
    let capture = TempDir::new().unwrap();
    let (out, err) = (capture.path().join("out"), capture.path().join("err"));

    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command.current_dir(temp.path()).arg("init");
    let mut child = spawn_capturing(&mut command, &out, &err).expect("spawn jit init");

    if !exited_by(&mut child, Instant::now() + SETUP_BUDGET) {
        let _ = child.kill();
        let _ = exited_by(&mut child, Instant::now() + REAP_GRACE);
        panic!(
            "`jit init` did not finish within {SETUP_BUDGET:?} in {}",
            temp.path().display()
        );
    }
    let status = child.try_wait().unwrap().expect("exited above");
    assert!(
        status.success(),
        "`jit init` failed with {status}\n--- stdout ---\n{}\n--- stderr ---\n{}",
        read_capture(&out),
        read_capture(&err)
    );
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

/// Wall-clock ceiling for the whole foreground-serve case below.
///
/// Every wait that case performs takes its deadline from this budget, so the
/// case reaches a verdict inside it even on a host where the server never comes
/// up at all — it cannot hold a continuous-integration job open until the job's
/// own execution ceiling cancels it (jit:76a4bd21).
#[cfg(unix)]
const FG_BUDGET: Duration = Duration::from_secs(90);

/// The share of [`FG_BUDGET`] spent waiting for the parent to announce its port.
#[cfg(unix)]
const FG_PORT_BUDGET: Duration = Duration::from_secs(20);

/// The share spent polling for a first served HTTP response.
#[cfg(unix)]
const FG_SERVING_BUDGET: Duration = Duration::from_secs(20);

/// The share spent killing the spawned process group and confirming it drained.
#[cfg(unix)]
const FG_TEARDOWN_BUDGET: Duration = Duration::from_secs(15);

/// Ceiling on one connect, write, or read of a single HTTP liveness probe.
#[cfg(unix)]
const FG_PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// The phases have to fit inside the stated total, or the total is not the
/// bound it claims to be.
#[cfg(unix)]
const _: () = assert!(
    SETUP_BUDGET.as_secs()
        + FG_PORT_BUDGET.as_secs()
        + FG_SERVING_BUDGET.as_secs()
        + FG_TEARDOWN_BUDGET.as_secs()
        <= FG_BUDGET.as_secs(),
    "the foreground-serve phase budgets must fit inside FG_BUDGET"
);

/// Sends `SIGKILL` to every process in the group `pgid` leads.
///
/// A negative PID in `kill(2)` addresses a process group, which is how the
/// `jit-server` grandchild is reached: the foreground path spawns it from a
/// plain `Command` with no `process_group` call of its own (unlike the
/// daemonizing path in `commands::serve::start_server`), so it stays in the
/// group this test creates for the `jit` parent. The conversion is guarded
/// before the negation (`@/invariant/pid-safety`): a value that does not fit a
/// positive `i32`, or one at or below 1, would turn the signal into
/// `kill(-1, …)` — every process this user owns.
#[cfg(unix)]
fn kill_process_group(pgid: u32) -> nix::Result<()> {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    match i32::try_from(pgid) {
        Ok(leader) if leader > 1 => kill(Pid::from_raw(-leader), Signal::SIGKILL),
        _ => Err(nix::errno::Errno::EINVAL),
    }
}

/// Reports whether any process still belongs to the group `pgid` leads.
///
/// Signal 0 performs the kernel's existence and permission checks without
/// delivering anything: `ESRCH` is the answer that the group has no members
/// left, while `EPERM` means it has members this test may not signal — still a
/// surviving process.
#[cfg(unix)]
fn process_group_survives(pgid: u32) -> bool {
    use nix::sys::signal::kill;
    use nix::unistd::Pid;
    match i32::try_from(pgid) {
        Ok(leader) if leader > 1 => !matches!(
            kill(Pid::from_raw(-leader), None),
            Err(nix::errno::Errno::ESRCH)
        ),
        _ => false,
    }
}

/// `jit serve --fg` must serve end to end even though the parent process holds
/// the bootstrap → repository recovery lock. Foreground `serve` is classified
/// as requiring pre-service recovery dispatch, so the parent acquires that lock
/// at startup; it then blocks on `child.wait()`. The spawned `jit-server` child
/// runs its own bootstrap recovery on startup, which needs the same
/// cross-process lock. Before the fix the parent retained the lock for its whole
/// lifetime, so the child could never acquire it — it timed out and the server
/// never came up. The parent must release its recovery session before waiting.
///
/// The case drives real processes, so it is bounded by construction: every wait
/// takes its deadline from [`FG_BUDGET`], output is captured to files rather
/// than pipes no reader can close, and teardown signals the whole spawned
/// process group and then confirms the group drained.
#[test]
#[cfg(unix)]
fn test_serve_fg_serves_when_child_runs_bootstrap_recovery() {
    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::os::unix::process::CommandExt;

    let started = Instant::now();
    let overall = started + FG_BUDGET;
    // Each phase gets its own share, clamped so no phase can push the case past
    // the total this test states.
    let phase = |budget: Duration| (Instant::now() + budget).min(overall);

    // `commands::serve::find_server_binary` resolves a sibling of the running
    // `jit` first, and `cargo test --workspace` builds that sibling. A narrower
    // invocation such as `cargo test -p jit --features html,xml` never builds
    // it, and this test must not build one for itself: the build it would spawn
    // has to take the cargo lock the build running this very test already
    // holds, and nothing bounds that wait (jit:76a4bd21).
    let server_bin: std::path::PathBuf =
        std::path::PathBuf::from(assert_cmd::cargo::cargo_bin!("jit")).with_file_name("jit-server");
    if !server_bin.exists() {
        eprintln!(
            "SKIP: no jit-server at {}, so `jit serve --fg` has no server to \
             spawn. `cargo test --workspace` builds it; a narrower invocation \
             needs `cargo build -p jit-server` first.",
            server_bin.display()
        );
        return;
    }

    // A real HTTP round-trip, not a bare TCP connect: the parent binds the
    // listening socket and hands its fd to the child, so the kernel accepts
    // connections into the listen backlog even while the child is still
    // blocked on the recovery lock and serving nothing. Only a completed
    // HTTP response proves the child actually adopted the socket and is
    // serving.
    let server_responds = |port: u16| -> bool {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let Ok(mut stream) = TcpStream::connect_timeout(&addr, FG_PROBE_TIMEOUT) else {
            return false;
        };
        let _ = stream.set_read_timeout(Some(FG_PROBE_TIMEOUT));
        let _ = stream.set_write_timeout(Some(FG_PROBE_TIMEOUT));
        if stream
            .write_all(b"GET /api/health HTTP/1.0\r\nHost: localhost\r\n\r\n")
            .is_err()
        {
            return false;
        }
        let mut buf = [0u8; 5];
        stream.read_exact(&mut buf).is_ok() && &buf == b"HTTP/"
    };

    let temp = setup_repo();
    let capture = TempDir::new().unwrap();
    let (out_path, err_path) = (capture.path().join("out"), capture.path().join("err"));

    // Own process group so the whole tree — the `jit` parent AND the
    // `jit-server` child it blocks on, which shares this group in foreground
    // mode — is reaped together; killing only the parent would orphan a live
    // server.
    let mut command = Command::new(assert_cmd::cargo::cargo_bin!("jit"));
    command
        .current_dir(temp.path())
        .args(["serve", "--fg", "--port", "0"])
        // Bound the pre-fix failure window: a regressed parent that keeps the
        // lock makes the child give up after this timeout instead of at the
        // 5s default, so the assertion below fails fast rather than dragging.
        .env("JIT_LOCK_TIMEOUT", "3")
        .process_group(0);
    let mut child =
        spawn_capturing(&mut command, &out_path, &err_path).expect("spawn jit serve --fg");
    let pgid = child.id();

    // The parent announces the chosen port before spawning the server.
    let announced = || {
        read_capture(&out_path)
            .lines()
            .find_map(parse_localhost_port)
    };
    let port_deadline = phase(FG_PORT_BUDGET);
    let mut announced_port = None;
    poll_until(port_deadline, || {
        announced_port = announced();
        // A parent that has exited will print nothing more, and the read above
        // already took its final output.
        announced_port.is_some() || matches!(child.try_wait(), Ok(Some(_)))
    });
    // Close the window between that last read and the exit check.
    let port = announced_port.or_else(announced);

    // Poll until the server answers an HTTP request, or the parent exits early
    // (the pre-fix symptom: the child's lock wait times out and the parent's
    // child.wait() returns).
    let serving_deadline = phase(FG_SERVING_BUDGET);
    let mut served = false;
    if let Some(port) = port {
        poll_until(serving_deadline, || {
            served = server_responds(port);
            served || matches!(child.try_wait(), Ok(Some(_)))
        });
    }

    // Signal the group before reaping the leader: once the leader is reaped its
    // PID — which is this PGID — becomes recyclable, and a later group kill
    // could then land on an unrelated group.
    let killed = kill_process_group(pgid);
    let teardown_deadline = phase(FG_TEARDOWN_BUDGET);
    let reaped = exited_by(&mut child, teardown_deadline);
    let drained = poll_until(teardown_deadline, || !process_group_survives(pgid));

    let out = read_capture(&out_path);
    let err = read_capture(&err_path);
    let elapsed = started.elapsed();

    assert!(
        reaped,
        "the jit parent did not exit within its {FG_TEARDOWN_BUDGET:?} teardown \
         budget after SIGKILL to its own process group (kill: {killed:?})"
    );
    if !drained {
        // Whether the announced port still answers separates a live survivor
        // from a member the kernel has killed but nothing has reaped yet.
        let still_serving = port.is_some_and(server_responds);
        panic!(
            "a process this test spawned outlived it: process group {pgid} still \
             has members after SIGKILL and a {FG_TEARDOWN_BUDGET:?} teardown \
             budget (kill: {killed:?}); announced port still answers HTTP: \
             {still_serving}"
        );
    }
    assert!(
        elapsed <= FG_BUDGET,
        "the case took {elapsed:?}, past the {FG_BUDGET:?} it budgets"
    );
    assert!(
        served,
        "jit serve --fg never began serving within {FG_BUDGET:?} (took \
         {elapsed:?}): {}\n--- stdout ---\n{out}\n--- stderr ---\n{err}",
        match port {
            Some(port) => format!(
                "it announced port {port} and never answered HTTP there, which is \
                 what a parent still holding the bootstrap recovery lock its child \
                 needs looks like."
            ),
            None => "it never announced a port.".to_owned(),
        }
    );
}
