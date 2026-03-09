//! `jit serve` — Start the JIT API and web UI server as a background daemon process.
//!
//! Manages a `jit-server` process per repository, using a PID file at
//! `.jit/server.pid.json` for lifecycle tracking and collision detection.
//! Auto-selects an available port in the range 3000–3099 when the preferred
//! port is already bound.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, Context, Result};

// ────────────────────────────────────────────────────────────────────────────
// Types
// ────────────────────────────────────────────────────────────────────────────

/// Runtime state persisted in `.jit/server.pid.json`.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct ServerPidFile {
    /// OS process ID of the running `jit-server` process.
    pub pid: u32,
    /// TCP port the server is listening on.
    pub port: u16,
    /// When the server was started (UTC).
    pub started_at: DateTime<Utc>,
    /// Absolute path to the `.jit` data directory being served.
    pub data_dir: PathBuf,
    /// Absolute path to the log file where server output is written.
    pub log_file: PathBuf,
}

/// Outcome of a `jit serve` (start) invocation.
#[derive(Debug, PartialEq)]
pub enum ServeOutcome {
    /// Server was successfully started.
    Started { pid: u32, port: u16 },
    /// A live server was already running — we printed its info and exited.
    AlreadyRunning { pid: u32, port: u16 },
}

/// Outcome of `jit serve --stop`.
#[derive(Debug, PartialEq)]
pub enum StopOutcome {
    /// Server was killed successfully.
    Stopped { pid: u32, port: u16 },
    /// No server was running (no PID file or stale PID).
    NotRunning,
}

// ────────────────────────────────────────────────────────────────────────────
// PID file helpers
// ────────────────────────────────────────────────────────────────────────────

/// Returns the canonical path of the PID file for the given data directory.
pub fn pid_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("server.pid.json")
}

/// Reads and deserialises the PID file, returning `None` if it does not exist.
pub fn read_pid_file(data_dir: &Path) -> Result<Option<ServerPidFile>> {
    let path = pid_file_path(data_dir);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).context("Failed to read PID file")?;
    let pf: ServerPidFile = serde_json::from_str(&raw).context("Malformed PID file")?;
    Ok(Some(pf))
}

/// Writes the PID file atomically (temp → rename).
pub fn write_pid_file(data_dir: &Path, pf: &ServerPidFile) -> Result<()> {
    let path = pid_file_path(data_dir);
    let tmp = path.with_extension("pid.tmp");
    let json = serde_json::to_string_pretty(pf).context("Failed to serialise PID file")?;
    std::fs::write(&tmp, json).context("Failed to write PID file tmp")?;
    std::fs::rename(&tmp, &path).context("Failed to rename PID file")?;
    Ok(())
}

/// Removes the PID file, ignoring "not found" errors.
pub fn remove_pid_file(data_dir: &Path) -> Result<()> {
    let path = pid_file_path(data_dir);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(anyhow::anyhow!("Failed to remove PID file: {e}")),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Process liveness
// ────────────────────────────────────────────────────────────────────────────

/// Returns `true` if a process with the given PID is currently alive.
///
/// On Unix this sends signal 0 (no-op) and checks the kernel response.
/// On other platforms falls back to always returning `false`.
///
/// # Safety
/// PIDs that cannot be represented as a positive `i32` (i.e. > `i32::MAX`)
/// are rejected and return `false` — casting them to `i32` would produce
/// negative values that have special `kill(2)` semantics (e.g. -1 = all
/// processes), which would be catastrophic.
pub fn is_process_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;
        // Guard: PIDs must fit in a positive i32.  Values > i32::MAX would
        // become negative after casting and `kill(-1, ...)` would send to
        // every process owned by this user.
        let Ok(pid_i32) = i32::try_from(pid) else {
            return false;
        };
        if pid_i32 <= 0 {
            return false;
        }
        kill(Pid::from_raw(pid_i32), None).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Port selection
// ────────────────────────────────────────────────────────────────────────────

/// Returns the first free TCP port in `start..=start+99`, or an error.
///
/// A port is "free" if we can successfully bind a `TcpListener` to it.
pub fn find_available_port(start: u16) -> Result<u16> {
    (start..=start.saturating_add(99))
        .find(|&port| TcpListener::bind(("127.0.0.1", port)).is_ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No available port found in range {start}–{}",
                start.saturating_add(99)
            )
        })
}

// ────────────────────────────────────────────────────────────────────────────
// Server binary discovery
// ────────────────────────────────────────────────────────────────────────────

/// Finds the `jit-server` binary.
///
/// Search order:
/// 1. Sibling of the current executable (covers `cargo install` and release builds)
/// 2. `which jit-server` (covers PATH-based installs)
pub fn find_server_binary() -> Result<PathBuf> {
    // 1. Sibling of current executable
    if let Ok(exe) = std::env::current_exe() {
        let sibling = exe.with_file_name("jit-server");
        if sibling.exists() {
            return Ok(sibling);
        }
    }

    // 2. PATH
    which::which("jit-server").map_err(|_| {
        anyhow::anyhow!(
            "jit-server binary not found. \
             Ensure it is installed and on PATH, or built alongside jit."
        )
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Start / stop logic
// ────────────────────────────────────────────────────────────────────────────

/// Options for starting the server.
#[derive(Debug)]
pub struct ServeOptions {
    /// Absolute path to the `.jit` data directory.
    pub data_dir: PathBuf,
    /// Preferred starting port (auto-selects if unavailable).
    pub preferred_port: u16,
    /// Log file path. Defaults to `<data_dir>/server.log`.
    pub log_file: Option<PathBuf>,
    /// Keep process in foreground instead of daemonizing.
    pub foreground: bool,
}

/// Checks for a running server; if alive returns `AlreadyRunning`, otherwise
/// starts a new daemonized `jit-server` process and returns `Started`.
pub fn start_server(opts: ServeOptions) -> Result<ServeOutcome> {
    let data_dir = &opts.data_dir;

    // Check existing PID file
    if let Some(pf) = read_pid_file(data_dir)? {
        if is_process_alive(pf.pid) {
            return Ok(ServeOutcome::AlreadyRunning {
                pid: pf.pid,
                port: pf.port,
            });
        }
        // Stale PID — clean up
        remove_pid_file(data_dir)?;
    }

    let port = find_available_port(opts.preferred_port)?;
    let log_file = opts.log_file.unwrap_or_else(|| data_dir.join("server.log"));
    let server_bin = find_server_binary()?;

    let bind_addr = format!("0.0.0.0:{port}");
    let data_dir_str = data_dir
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("data_dir is not valid UTF-8"))?;

    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)
        .context("Cannot open log file")?;

    let mut cmd = std::process::Command::new(&server_bin);
    cmd.arg("--data-dir")
        .arg(data_dir_str)
        .arg("--bind")
        .arg(&bind_addr);

    if opts.foreground {
        // Run in foreground — block until the process exits.
        let status = cmd.status().context("Failed to start jit-server")?;
        if !status.success() {
            bail!("jit-server exited with status {status}");
        }
        // In foreground mode there is no PID to track.
        return Ok(ServeOutcome::Started { pid: 0, port });
    }

    // Daemonize: redirect stdin to /dev/null, stdout/stderr to log file.
    let dev_null = std::fs::File::open("/dev/null")
        .or_else(|_| std::fs::File::open("NUL")) // Windows fallback
        .context("Cannot open /dev/null")?;

    let log_stderr = log.try_clone().context("Cannot clone log file handle")?;

    cmd.stdin(dev_null).stdout(log).stderr(log_stderr);

    // Detach from parent's process group so the child survives terminal closure.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    let child = cmd.spawn().context("Failed to spawn jit-server")?;

    let pid = child.id();

    // Brief pause to let the server bind its port before we record the PID.
    std::thread::sleep(Duration::from_millis(300));

    let pf = ServerPidFile {
        pid,
        port,
        started_at: Utc::now(),
        data_dir: data_dir.to_path_buf(),
        log_file: log_file.clone(),
    };
    write_pid_file(data_dir, &pf)?;

    Ok(ServeOutcome::Started { pid, port })
}

/// Stops the running server for the given data directory.
pub fn stop_server(data_dir: &Path) -> Result<StopOutcome> {
    let pf = match read_pid_file(data_dir)? {
        Some(pf) if is_process_alive(pf.pid) => pf,
        Some(_) => {
            // Stale PID — clean up silently.
            remove_pid_file(data_dir)?;
            return Ok(StopOutcome::NotRunning);
        }
        None => return Ok(StopOutcome::NotRunning),
    };

    let pid = pf.pid;
    let port = pf.port;

    #[cfg(unix)]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        let pid_i32 =
            i32::try_from(pid).with_context(|| format!("PID {pid} is out of range for kill(2)"))?;
        kill(Pid::from_raw(pid_i32), Signal::SIGTERM)
            .with_context(|| format!("Failed to send SIGTERM to PID {pid}"))?;
    }
    #[cfg(not(unix))]
    {
        bail!(
            "Stopping the server via --stop is not supported on this platform. \
             Terminate the jit-server process manually."
        );
    }

    remove_pid_file(data_dir)?;
    Ok(StopOutcome::Stopped { pid, port })
}

/// Returns the current server status for the given data directory.
pub fn server_status(data_dir: &Path) -> Result<Option<ServerPidFile>> {
    match read_pid_file(data_dir)? {
        Some(pf) if is_process_alive(pf.pid) => Ok(Some(pf)),
        Some(_) => {
            // Stale — clean up
            remove_pid_file(data_dir)?;
            Ok(None)
        }
        None => Ok(None),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Unit tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── PID file round-trip ──────────────────────────────────────────────────

    #[test]
    fn test_pid_file_round_trip() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let pf = ServerPidFile {
            pid: 42,
            port: 3000,
            started_at: Utc::now(),
            data_dir: data_dir.to_path_buf(),
            log_file: data_dir.join("server.log"),
        };

        write_pid_file(data_dir, &pf).unwrap();
        assert!(pid_file_path(data_dir).exists());

        let read_back = read_pid_file(data_dir).unwrap().unwrap();
        assert_eq!(read_back.pid, pf.pid);
        assert_eq!(read_back.port, pf.port);
        assert_eq!(read_back.data_dir, pf.data_dir);
    }

    #[test]
    fn test_read_pid_file_returns_none_when_missing() {
        let tmp = TempDir::new().unwrap();
        let result = read_pid_file(tmp.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_write_pid_file_is_atomic() {
        // The tmp file must not linger after a successful write.
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let pf = ServerPidFile {
            pid: 1,
            port: 3001,
            started_at: Utc::now(),
            data_dir: data_dir.to_path_buf(),
            log_file: data_dir.join("server.log"),
        };
        write_pid_file(data_dir, &pf).unwrap();
        let tmp_path = pid_file_path(data_dir).with_extension("pid.tmp");
        assert!(
            !tmp_path.exists(),
            "tmp file should be removed after rename"
        );
    }

    #[test]
    fn test_remove_pid_file_idempotent() {
        let tmp = TempDir::new().unwrap();
        // Should not error even when file does not exist.
        remove_pid_file(tmp.path()).unwrap();
        remove_pid_file(tmp.path()).unwrap();
    }

    #[test]
    fn test_remove_pid_file_deletes_existing_file() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();
        let pf = ServerPidFile {
            pid: 99,
            port: 3005,
            started_at: Utc::now(),
            data_dir: data_dir.to_path_buf(),
            log_file: data_dir.join("server.log"),
        };
        write_pid_file(data_dir, &pf).unwrap();
        assert!(pid_file_path(data_dir).exists());

        remove_pid_file(data_dir).unwrap();
        assert!(!pid_file_path(data_dir).exists());
    }

    // ── Process liveness ────────────────────────────────────────────────────

    /// Spawns a short-lived child process and returns its PID after it exits.
    /// The returned PID is guaranteed to be a valid positive i32 but dead.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true")
            .spawn()
            .expect("failed to spawn 'true'");
        let pid = child.id();
        child.wait().expect("failed to wait on child");
        pid
    }

    #[test]
    fn test_is_process_alive_current_process() {
        let my_pid = std::process::id();
        assert!(is_process_alive(my_pid), "current process should be alive");
    }

    #[test]
    fn test_is_process_alive_rejects_nonexistent_pid() {
        assert!(
            !is_process_alive(dead_pid()),
            "exited process should not be alive"
        );
    }

    #[test]
    fn test_is_process_alive_rejects_overflowing_pid() {
        // u32::MAX as i32 = -1 which would signal all processes — must return false.
        assert!(!is_process_alive(u32::MAX));
    }

    // ── Port selection ───────────────────────────────────────────────────────

    #[test]
    fn test_find_available_port_returns_free_port() {
        let port = find_available_port(3000).unwrap();
        assert!((3000..=3099).contains(&port));
        // Verify we can actually bind to it.
        TcpListener::bind(("127.0.0.1", port)).unwrap();
    }

    #[test]
    fn test_find_available_port_skips_bound_port() {
        // Bind the preferred port so find_available_port must skip it.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let bound_port = listener.local_addr().unwrap().port();

        // Ask for that specific port; it should return a different one (if any free).
        if bound_port >= 3000 && bound_port <= 3099 {
            let port = find_available_port(bound_port).unwrap();
            assert_ne!(port, bound_port);
        }
    }

    // ── server_status with stale PID ─────────────────────────────────────────

    #[test]
    fn test_server_status_cleans_stale_pid_file() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        // Write a PID file with a known-dead process.
        let pf = ServerPidFile {
            pid: dead_pid(),
            port: 3000,
            started_at: Utc::now(),
            data_dir: data_dir.to_path_buf(),
            log_file: data_dir.join("server.log"),
        };
        write_pid_file(data_dir, &pf).unwrap();

        let status = server_status(data_dir).unwrap();
        assert!(status.is_none(), "dead PID should be cleaned up");
        assert!(
            !pid_file_path(data_dir).exists(),
            "stale PID file should be removed"
        );
    }

    #[test]
    fn test_server_status_returns_none_when_no_pid_file() {
        let tmp = TempDir::new().unwrap();
        let status = server_status(tmp.path()).unwrap();
        assert!(status.is_none());
    }

    // ── stop_server with no running server ──────────────────────────────────

    #[test]
    fn test_stop_server_when_not_running() {
        let tmp = TempDir::new().unwrap();
        let outcome = stop_server(tmp.path()).unwrap();
        assert_eq!(outcome, StopOutcome::NotRunning);
    }

    #[test]
    fn test_stop_server_cleans_stale_pid() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        let pf = ServerPidFile {
            pid: dead_pid(),
            port: 3000,
            started_at: Utc::now(),
            data_dir: data_dir.to_path_buf(),
            log_file: data_dir.join("server.log"),
        };
        write_pid_file(data_dir, &pf).unwrap();

        let outcome = stop_server(data_dir).unwrap();
        assert_eq!(outcome, StopOutcome::NotRunning);
        assert!(!pid_file_path(data_dir).exists());
    }
}
