//! Graceful shutdown of the `jit-server` binary as a real process.
//!
//! The in-crate unit tests in `src/shutdown.rs` cover the shutdown sequence's
//! logic in process. Only a real process can show the thing adopters depend on:
//! a signalled server closes its sockets, releases its port, and exits `0`
//! within a bounded time even when a connection refuses to finish on its own.

use std::fs::File;
use std::io::{ErrorKind, Read, Write};
use std::net::TcpStream;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use jit::commands::CommandExecutor;
use jit::domain::Priority;
use jit::hierarchy_templates::HierarchyTemplate;
use jit::storage::{IssueStore, JsonFileStorage};
use jit_server::shutdown::GRACEFUL_DRAIN_TIMEOUT;
use nix::sys::signal::{kill, Signal};
use nix::unistd::Pid;
use tempfile::TempDir;

/// Longest the fixture waits for a freshly spawned server to report its port.
const STARTUP_BUDGET: Duration = Duration::from_secs(30);

/// Longest an ordinary exchange (health probe, SSE head, change event) may take.
const EXCHANGE_BUDGET: Duration = Duration::from_secs(10);

/// "Promptly" for an event stream that must end on cancellation rather than
/// linger behind its keepalive.
const STREAM_EOF_BUDGET: Duration = Duration::from_secs(2);

/// REQ-03's hard external bound: the process is gone in strictly less than ten
/// seconds after the signal, never by a test-side kill.
const EXIT_BUDGET: Duration = Duration::from_secs(10);

/// Scheduling slack allowed when asserting that a survivor waited out the full
/// drain deadline before being force-closed.
const DEADLINE_SLACK: Duration = Duration::from_millis(500);

/// Blocking-read slice, short enough that a poll loop stays responsive.
const READ_POLL: Duration = Duration::from_millis(50);

// ── Repository fixture ──────────────────────────────────────────────────────

/// Builds the valid repository the server requires at startup.
fn initialize_repository(worktree_root: &Path) {
    executor_for(worktree_root)
        .initialize_fresh_repository(worktree_root, &HierarchyTemplate::default(), None)
        .expect("initialize the fixture repository");
}

fn executor_for(worktree_root: &Path) -> CommandExecutor<JsonFileStorage> {
    let storage = JsonFileStorage::new(worktree_root.join(".jit"));
    let layout = jit::storage::discover_repository_layout(worktree_root, storage.root())
        .expect("discover the fixture repository layout");
    CommandExecutor::new(storage).with_layout(layout)
}

/// Writes a graph-relevant change into the watched repository, which the
/// server's file watcher turns into an SSE `change` event.
fn record_repository_change(worktree_root: &Path) {
    executor_for(worktree_root)
        .create_issue(
            "Live change".to_string(),
            "Observed over the event stream".to_string(),
            Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .expect("create an issue in the fixture repository");
}

// ── Server process fixture ──────────────────────────────────────────────────

/// A spawned `jit-server` process, its bound port, and its captured log.
struct RunningServer {
    child: Child,
    port: u16,
    log_path: PathBuf,
}

impl RunningServer {
    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The server's captured log, with terminal styling removed so assertions
    /// read the field names and values rather than the escapes around them.
    fn log(&self) -> String {
        strip_ansi(&std::fs::read_to_string(&self.log_path).unwrap_or_default())
    }

    /// Waits for the process to exit, and returns how long that took measured
    /// from `signalled_at`. Panics rather than killing the process: a test-side
    /// kill would hide exactly the defect this suite exists to catch.
    fn wait_for_exit(&mut self, signalled_at: Instant, budget: Duration) -> (i32, Duration) {
        loop {
            match self.child.try_wait().expect("poll the server process") {
                Some(status) => {
                    assert_eq!(
                        status.signal(),
                        None,
                        "the server died from a signal instead of exiting on its own; log:\n{}",
                        self.log()
                    );
                    let code = status
                        .code()
                        .expect("a process that was not signalled has an exit code");
                    return (code, signalled_at.elapsed());
                }
                None => {
                    assert!(
                        signalled_at.elapsed() < budget,
                        "the server was still running {:?} after the signal; log:\n{}",
                        signalled_at.elapsed(),
                        self.log()
                    );
                    std::thread::sleep(READ_POLL);
                }
            }
        }
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        // Only reached when a test panicked before its own shutdown assertions;
        // leaving the process behind would leak a listener into the next test.
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Spawns the compiled `jit-server` binary against `worktree_root`, with no
/// intermediate shell or supervisor, and waits until it reports its port.
fn start_server(worktree_root: &Path, bind: &str, log_name: &str) -> RunningServer {
    let log_path = worktree_root.join(log_name);
    let log = File::create(&log_path).expect("create the server log file");
    let child = Command::new(env!("CARGO_BIN_EXE_jit-server"))
        .current_dir(worktree_root)
        .args(["--data-dir", ".jit", "--bind", bind])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().expect("clone the log handle")))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn jit-server");

    let mut server = RunningServer {
        child,
        port: 0,
        log_path,
    };
    server.port = wait_for_listening_port(&mut server);
    probe_health(server.port);
    server
}

/// Reads the bound port out of the server's own listening log line, so the
/// fixture never has to guess a free port or race the server for one.
fn wait_for_listening_port(server: &mut RunningServer) -> u16 {
    let started = Instant::now();
    loop {
        if let Some(status) = server.child.try_wait().expect("poll the server process") {
            panic!(
                "jit-server exited with {status} during startup; log:\n{}",
                server.log()
            );
        }
        if let Some(port) = parse_listening_port(&server.log()) {
            return port;
        }
        assert!(
            started.elapsed() < STARTUP_BUDGET,
            "jit-server never reported a listening address; log:\n{}",
            server.log()
        );
        std::thread::sleep(READ_POLL);
    }
}

/// The log message that reports a delivered signal.
const SIGNAL_RECEIVED: &str = "Shutdown signal received";

/// The log message that reports the deadline closing what did not finish.
const FORCE_CLOSING: &str = "force-closing";

/// The value of `field` on the log line carrying `message`.
///
/// `tracing`'s compact format renders event fields as `name=value` after the
/// message, so reading one field back is how the fixture asserts on a specific
/// reported number rather than on the shape of a whole line.
fn log_field(log: &str, message: &str, field: &str) -> String {
    let line = log
        .lines()
        .find(|line| line.contains(message))
        .unwrap_or_else(|| panic!("no log line reports {message:?}; log:\n{log}"));
    line.split(&format!("{field}="))
        .nth(1)
        .unwrap_or_else(|| panic!("the {message:?} line carries no {field} field: {line}"))
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Drops CSI escape sequences (`ESC [ … final-byte`) from captured output.
fn strip_ansi(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut characters = text.chars();
    while let Some(character) = characters.next() {
        if character != '\u{1b}' {
            plain.push(character);
            continue;
        }
        if characters.next() != Some('[') {
            continue;
        }
        characters.by_ref().find(|byte| byte.is_ascii_alphabetic());
    }
    plain
}

fn parse_listening_port(log: &str) -> Option<u16> {
    log.split("listening on http://")
        .nth(1)?
        .split_whitespace()
        .next()?
        .rsplit(':')
        .next()?
        .parse()
        .ok()
}

// ── Raw HTTP helpers ────────────────────────────────────────────────────────

fn connect(port: u16) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to the server");
    stream
        .set_read_timeout(Some(READ_POLL))
        .expect("set a read timeout");
    stream
}

fn send(stream: &mut TcpStream, request: &str) {
    stream
        .write_all(request.as_bytes())
        .expect("send the request");
    stream.flush().expect("flush the request");
}

/// Reads until `needle` appears, and returns everything read.
fn read_until(stream: &mut TcpStream, needle: &str, budget: Duration) -> String {
    let started = Instant::now();
    let mut received = String::new();
    loop {
        let mut chunk = [0u8; 2048];
        match stream.read(&mut chunk) {
            Ok(0) => panic!("connection closed while waiting for {needle:?}; read: {received:?}"),
            Ok(read) => {
                received.push_str(&String::from_utf8_lossy(&chunk[..read]));
                if received.contains(needle) {
                    return received;
                }
            }
            Err(error) if is_retryable(&error) => {}
            Err(error) => panic!("read failed while waiting for {needle:?}: {error}"),
        }
        assert!(
            started.elapsed() < budget,
            "timed out after {:?} waiting for {needle:?}; read: {received:?}",
            started.elapsed()
        );
    }
}

/// Waits until the peer closes the connection, and returns how long that took
/// measured from `since`.
fn wait_for_close(stream: &mut TcpStream, since: Instant, budget: Duration) -> Duration {
    let started = Instant::now();
    loop {
        let mut chunk = [0u8; 2048];
        match stream.read(&mut chunk) {
            Ok(0) => return since.elapsed(),
            // A forced close can arrive as a reset rather than an orderly FIN.
            Err(error) if error.kind() == ErrorKind::ConnectionReset => return since.elapsed(),
            Ok(_) => {}
            Err(error) if is_retryable(&error) => {}
            Err(error) => panic!("read failed while waiting for the close: {error}"),
        }
        assert!(
            started.elapsed() < budget,
            "the connection was still open {:?} after the signal",
            since.elapsed()
        );
    }
}

/// True when the connection is still open right now.
fn is_still_open(stream: &mut TcpStream) -> bool {
    let mut chunk = [0u8; 1];
    match stream.read(&mut chunk) {
        Ok(0) => false,
        Err(error) if error.kind() == ErrorKind::ConnectionReset => false,
        Ok(_) => true,
        Err(error) if is_retryable(&error) => true,
        Err(error) => panic!("read failed while probing the connection: {error}"),
    }
}

fn is_retryable(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
    )
}

/// Confirms the server answers, and leaves no probe connection behind: the
/// probe asks for a close so it is not counted among the connections the
/// shutdown assertions are about.
fn probe_health(port: u16) {
    let mut stream = connect(port);
    send(
        &mut stream,
        "GET /api/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
    );
    let response = read_until(&mut stream, "\"status\":\"ok\"", EXCHANGE_BUDGET);
    assert!(
        response.starts_with("HTTP/1.1 200 "),
        "health probe answered: {response:?}"
    );
    wait_for_close(&mut stream, Instant::now(), EXCHANGE_BUDGET);
}

/// Opens an event stream and confirms it is a live SSE response.
fn open_event_stream(port: u16) -> TcpStream {
    let mut stream = connect(port);
    send(
        &mut stream,
        "GET /api/events/stream HTTP/1.1\r\nHost: localhost\r\nAccept: text/event-stream\r\n\r\n",
    );
    let head = read_until(&mut stream, "\r\n\r\n", EXCHANGE_BUDGET);
    assert!(
        head.starts_with("HTTP/1.1 200 "),
        "event stream answered: {head:?}"
    );
    assert!(
        head.to_lowercase()
            .contains("content-type: text/event-stream"),
        "event stream is not an SSE response: {head:?}"
    );
    stream
}

/// Opens a connection, completes one exchange on it, and leaves it idle in
/// HTTP keep-alive — an ordinary connection that can be retired voluntarily.
fn open_completed_connection(port: u16) -> TcpStream {
    let mut stream = connect(port);
    send(
        &mut stream,
        "GET /api/health HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    read_until(&mut stream, "\"status\":\"ok\"", EXCHANGE_BUDGET);
    stream
}

/// Opens a connection whose request never completes: the request line and a
/// header are sent, the terminating blank line never is. The server stays mid
/// message, so nothing but the drain deadline can retire this connection.
fn open_stalled_connection(port: u16) -> TcpStream {
    let mut stream = connect(port);
    send(
        &mut stream,
        "GET /api/health HTTP/1.1\r\nHost: localhost\r\n",
    );
    stream
}

// ── Signalling ──────────────────────────────────────────────────────────────

/// Sends `SIGTERM` to the server process.
///
/// `@/inv/pid-safety`: a PID that does not convert to a positive `i32` is
/// rejected before the syscall, so a lossy conversion can never turn this
/// targeted signal into `kill(-1, …)` against every process the user owns.
fn send_sigterm(pid: u32) {
    let target =
        i32::try_from(pid).unwrap_or_else(|_| panic!("PID {pid} is out of range for kill(2)"));
    assert!(
        target > 0,
        "refusing to signal PID {pid}: the value would target a process group"
    );
    kill(Pid::from_raw(target), Signal::SIGTERM).expect("send SIGTERM to the server");
}

/// Asserts the server keeps its work in process: no child whose lifetime a
/// shutdown would additionally have to coordinate.
fn assert_no_child_processes(pid: u32) {
    // Linux exposes the child list directly; elsewhere the fixture's direct
    // spawn (no shell, no supervisor) is the only guarantee available.
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/task/{pid}/children");
        let children = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("cannot read {path}: {error}"));
        assert!(
            children.trim().is_empty(),
            "jit-server spawned child process(es): {children:?}"
        );
    }
    #[cfg(not(target_os = "linux"))]
    let _ = pid;
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn test_jit_server_shutdown_force_closes_a_stalled_connection_and_exits_zero() {
    let repository = TempDir::new().expect("create the fixture repository directory");
    initialize_repository(repository.path());
    let mut server = start_server(repository.path(), "127.0.0.1:0", "server.log");

    // Two live event streams, plus a completing and a non-completing ordinary
    // connection: the four the shutdown logs must account for.
    let mut first_stream = open_event_stream(server.port);
    let mut second_stream = open_event_stream(server.port);
    record_repository_change(repository.path());
    read_until(&mut first_stream, "event: change", EXCHANGE_BUDGET);
    read_until(&mut second_stream, "event: change", EXCHANGE_BUDGET);
    let mut completed = open_completed_connection(server.port);
    let mut stalled = open_stalled_connection(server.port);
    assert_no_child_processes(server.pid());

    let signalled_at = Instant::now();
    send_sigterm(server.pid());

    // Cancellation reaches the event streams, so they end instead of waiting
    // out their keepalive interval.
    let first_eof = wait_for_close(&mut first_stream, signalled_at, STREAM_EOF_BUDGET);
    let second_eof = wait_for_close(&mut second_stream, signalled_at, STREAM_EOF_BUDGET);
    assert!(
        first_eof < STREAM_EOF_BUDGET && second_eof < STREAM_EOF_BUDGET,
        "event streams reached EOF after {first_eof:?} and {second_eof:?}"
    );
    // An idle keep-alive connection is retired by the graceful shutdown itself.
    wait_for_close(&mut completed, signalled_at, STREAM_EOF_BUDGET);
    // The connection that cannot finish is still open partway through the drain.
    //
    // Only probe while the probe can still observe that: the three waits above
    // are each entitled to STREAM_EOF_BUDGET, so together they may consume more
    // than GRACEFUL_DRAIN_TIMEOUT before reaching this line, at which point a
    // CORRECT server has already force-closed the survivor and an unconditional
    // probe fails on conforming behaviour. Under `cargo test --workspace` that
    // is what happens; in isolation the waits return in milliseconds and the
    // probe lands mid-drain. Skipping the probe costs no coverage — the
    // stalled_closed_after assertion below states the same entitlement
    // unconditionally, measured after the fact instead of sampled during.
    if signalled_at.elapsed() + DEADLINE_SLACK < GRACEFUL_DRAIN_TIMEOUT {
        assert!(
            is_still_open(&mut stalled),
            "the stalled connection was dropped before the drain deadline"
        );
    }

    let stalled_closed_after = wait_for_close(&mut stalled, signalled_at, EXIT_BUDGET);
    assert!(
        stalled_closed_after + DEADLINE_SLACK >= GRACEFUL_DRAIN_TIMEOUT,
        "the stalled connection was closed after {stalled_closed_after:?}, \
         before the drain deadline it was entitled to"
    );

    let (exit_code, exited_after) = server.wait_for_exit(signalled_at, EXIT_BUDGET);
    assert_eq!(exit_code, 0, "server log:\n{}", server.log());
    assert!(
        exited_after < EXIT_BUDGET,
        "the server took {exited_after:?} to exit"
    );

    let log = server.log();
    assert_eq!(log_field(&log, SIGNAL_RECEIVED, "signal"), "SIGTERM");
    assert_eq!(
        log_field(&log, SIGNAL_RECEIVED, "drain_deadline_secs"),
        GRACEFUL_DRAIN_TIMEOUT.as_secs().to_string()
    );
    assert_eq!(
        log_field(&log, SIGNAL_RECEIVED, "open_connections"),
        "4",
        "the log must account for both event streams and both ordinary connections"
    );
    assert_eq!(
        log_field(&log, FORCE_CLOSING, "open_connections"),
        "1",
        "the log must report the connection the deadline force-closed"
    );
    assert!(
        log.contains("Shutdown complete"),
        "the shutdown log does not record clean completion:\n{log}"
    );
}

#[test]
fn test_jit_server_shutdown_releases_the_port_for_an_immediate_restart() {
    let repository = TempDir::new().expect("create the fixture repository directory");
    initialize_repository(repository.path());
    let mut first = start_server(repository.path(), "127.0.0.1:0", "first.log");
    let port = first.port;

    let signalled_at = Instant::now();
    send_sigterm(first.pid());
    let (exit_code, exited_after) = first.wait_for_exit(signalled_at, EXIT_BUDGET);
    assert_eq!(exit_code, 0, "server log:\n{}", first.log());
    assert!(
        exited_after < GRACEFUL_DRAIN_TIMEOUT,
        "a server with nothing to drain waited {exited_after:?} instead of stopping at once"
    );

    // No lingering listener: the successor binds the same port immediately.
    let mut second = start_server(
        repository.path(),
        &format!("127.0.0.1:{port}"),
        "second.log",
    );
    assert_eq!(second.port, port);

    let restart_signalled_at = Instant::now();
    send_sigterm(second.pid());
    let (restart_exit_code, _) = second.wait_for_exit(restart_signalled_at, EXIT_BUDGET);
    assert_eq!(restart_exit_code, 0, "server log:\n{}", second.log());
}
