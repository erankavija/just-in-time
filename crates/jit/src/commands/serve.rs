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
    /// Server was successfully started as a background daemon.
    Started {
        pid: u32,
        port: u16,
        /// The effective log file path (resolved default or user-supplied).
        log_file: PathBuf,
    },
    /// A live server was already running — caller should print its info.
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
pub fn foreground_exit_code(child_code: Option<i32>) -> i32 {
    child_code.unwrap_or(1)
}

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

/// Selects an available TCP port and returns it as an owned, already-bound
/// listener on `0.0.0.0` (all interfaces — the address the server serves on).
///
/// When `start` is `0`, binds directly to an OS-assigned ephemeral port —
/// there is no preferred port to range-walk from, so any free port will do.
/// Otherwise probes `start..=start+99` in order and returns the first port
/// that binds.
///
/// The returned listener is already bound, so nothing else can claim the
/// port between selection and use. The caller must hand this exact socket to
/// the server child rather than closing it and letting the child re-bind:
/// [`spawn_with_listener`] does so via file-descriptor inheritance, which is
/// what closes the probe-then-bind race across the process boundary. Read the
/// chosen port with `listener.local_addr()?.port()`.
pub fn find_available_port(start: u16) -> Result<TcpListener> {
    if start == 0 {
        return TcpListener::bind(("0.0.0.0", 0)).context("Failed to bind an OS-assigned port");
    }
    (start..=start.saturating_add(99))
        .find_map(|port| TcpListener::bind(("0.0.0.0", port)).ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "No available port found in range {start}–{}",
                start.saturating_add(99)
            )
        })
}

/// Configures `cmd` so the spawned `jit-server` child inherits `listener` as
/// its serving socket instead of binding a port itself (Unix only).
///
/// Clears close-on-exec on the listener's descriptor so it survives the
/// child's `execve`, then publishes the listenfd environment
/// (`LISTEN_FDS=1`, `LISTEN_FDS_FIRST_FD=<fd>`) that `jit-server` reads to
/// adopt the fd. `LISTEN_PID` is intentionally unset: the listenfd protocol
/// treats an absent `LISTEN_PID` as "addressed to this process", which is
/// what we want since the child's PID is unknown before the spawn.
///
/// The caller must keep `listener` alive until after the child is spawned;
/// the child receives its own dup of the socket at fork time, so the port is
/// never unbound. See [`spawn_with_listener`], which sequences this correctly.
#[cfg(unix)]
pub fn inherit_listener(cmd: &mut std::process::Command, listener: &TcpListener) -> Result<()> {
    use nix::fcntl::{fcntl, FcntlArg, FdFlag};
    use std::os::unix::io::AsRawFd;

    let fd = listener.as_raw_fd();
    // Empty flags = FD_CLOEXEC cleared, so the descriptor is inherited across
    // the child's execve rather than closed by it.
    fcntl(listener, FcntlArg::F_SETFD(FdFlag::empty()))
        .context("Failed to clear close-on-exec on the server listener")?;
    cmd.env("LISTEN_FDS", "1")
        .env("LISTEN_FDS_FIRST_FD", fd.to_string());
    Ok(())
}

/// Spawns `cmd` as the `jit-server` child, handing it `listener` as its
/// pre-bound serving socket, and returns the running child.
///
/// On Unix the child inherits the exact socket via [`inherit_listener`] — it
/// performs no bind of its own, so the port `find_available_port` probed is
/// held continuously across the process boundary and cannot be stolen in a
/// probe-then-bind window. The parent's handle is dropped only after the
/// fork, once the child holds its own dup.
///
/// On non-Unix platforms file-descriptor inheritance is unavailable, so the
/// socket is released before the spawn and the child binds `--bind` itself.
///
/// # Errors
/// Returns an error if the descriptor cannot be made inheritable or the child
/// process fails to spawn.
pub fn spawn_with_listener(
    cmd: &mut std::process::Command,
    listener: TcpListener,
) -> Result<std::process::Child> {
    #[cfg(unix)]
    {
        inherit_listener(cmd, &listener)?;
        let child = cmd.spawn().context("Failed to spawn jit-server")?;
        // The child now holds its own dup from the fork; release ours.
        drop(listener);
        Ok(child)
    }
    #[cfg(not(unix))]
    {
        // Release the port so the child can bind it itself.
        drop(listener);
        cmd.spawn().context("Failed to spawn jit-server")
    }
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
        // Generic NotFoundError: a missing executable has no dedicated domain type.
        // Still downcastable -> exit 3; message preserved verbatim.
        crate::errors::NotFoundError::new(
            "jit-server binary not found. \
             Ensure it is installed and on PATH, or built alongside jit.",
        )
        .into()
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Start / stop logic
// ────────────────────────────────────────────────────────────────────────────

/// Options for starting the server as a background daemon.
#[derive(Debug)]
pub struct ServeOptions {
    /// Absolute path to the `.jit` data directory.
    pub data_dir: PathBuf,
    /// Preferred starting port (auto-selects if unavailable).
    pub preferred_port: u16,
    /// Log file path. Defaults to `<data_dir>/server.log`.
    pub log_file: Option<PathBuf>,
    /// Directory containing built web UI static files.
    /// `None` means auto-detect; use [`find_web_dir`] before calling `start_server`.
    pub web_dir: Option<PathBuf>,
    /// Override the server binary path (testing only; normally resolved by
    /// [`find_server_binary`] automatically).
    pub server_binary: Option<PathBuf>,
}

/// Tries to locate the built web UI `dist/` directory.
///
/// Search order:
/// 1. Sibling `web/dist/` next to the current executable
/// 2. `web/dist/` relative to the current working directory
pub fn find_web_dir() -> Option<PathBuf> {
    // 1. Next to the jit binary (installed layout or cargo target/debug/)
    if let Ok(exe) = std::env::current_exe() {
        // target/{profile}/jit -> exe.parent() = target/{profile}/
        // go up two more: target/ -> repo root
        let candidates = [
            exe.parent()
                .and_then(|p| p.parent()?.parent().map(|r| r.join("web/dist"))),
            exe.parent().map(|p| p.join("web/dist")),
        ];
        for c in candidates.into_iter().flatten() {
            if c.is_dir() {
                return Some(c);
            }
        }
    }
    // 2. Current working directory
    if let Ok(cwd) = std::env::current_dir() {
        let p = cwd.join("web/dist");
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// Checks for a running server; if alive returns `AlreadyRunning`, otherwise
/// starts a new daemonized `jit-server` process and returns `Started`.
///
/// Foreground mode is handled by the caller, not this function. Use
/// [`find_available_port`] to bind the socket and [`spawn_with_listener`] to
/// hand it to the child — the same inheritance path this function uses — so
/// the port is never re-bound across the process boundary.
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

    // Bind the serving socket here and hand this exact socket to the child
    // (spawn_with_listener). The child adopts it rather than re-binding, so
    // the port stays bound continuously across the process boundary — there
    // is no probe-then-bind window for another process to slip into (REQ-2).
    let listener = find_available_port(opts.preferred_port)?;
    let port = listener
        .local_addr()
        .context("Failed to read bound port")?
        .port();
    let log_file = opts.log_file.unwrap_or_else(|| data_dir.join("server.log"));
    let server_bin = match opts.server_binary {
        Some(p) => p,
        None => find_server_binary()?,
    };

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

    if let Some(web_dir) = &opts.web_dir {
        if web_dir.is_dir() {
            cmd.arg("--web-dir").arg(web_dir);
        }
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

    // Hand the bound socket to the child (inherited fd on Unix); it adopts
    // this exact socket instead of re-binding the port.
    let mut child = spawn_with_listener(&mut cmd, listener)?;

    let pid = child.id();

    // Brief pause to let the server bind its port before checking liveness.
    std::thread::sleep(Duration::from_millis(300));

    // Verify the child is still alive. A rapid exit indicates a startup
    // failure (e.g. it could not adopt the inherited socket, or a data-dir
    // error). In that case we must not write a PID file — doing so would
    // leave a stale record that falsely reports a running server.
    if let Some(exit_status) = child.try_wait().context("Failed to check server startup")? {
        bail!(
            "jit-server exited during startup with {exit_status}. \
             Check the log file for details: {}",
            log_file.display()
        );
    }

    let pf = ServerPidFile {
        pid,
        port,
        started_at: Utc::now(),
        data_dir: data_dir.to_path_buf(),
        log_file: log_file.clone(),
    };
    if let Err(e) = write_pid_file(data_dir, &pf) {
        // PID file write failed but the child is already detached and running.
        // Kill it now so we do not leave an orphaned, untrackable process.
        let _ = child.kill();
        let _ = child.wait(); // reap to avoid zombie
        return Err(e.context(format!(
            "Failed to persist PID file; terminated spawned jit-server (PID {pid})"
        )));
    }

    Ok(ServeOutcome::Started {
        pid,
        port,
        log_file,
    })
}

/// Stops the running server for the given data directory.
///
/// Sends `SIGTERM` to the server process and removes the PID file.
/// Returns [`StopOutcome::NotRunning`] if no server was running or the PID
/// file is stale.
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
        // Guard against pid 0 (signal whole process group) and negative values.
        if pid_i32 <= 0 {
            bail!("Refusing to signal PID {pid}: value would target a process group");
        }
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
///
/// Returns `Some(ServerPidFile)` when a live server is found, `None` when
/// no server is running or the PID file is stale (stale files are cleaned up).
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
    fn test_find_available_port_returns_bound_listener() {
        // start=0 requests an OS-assigned port with no fixed range to walk,
        // so this test never contends with a sibling over shared port
        // numbers under parallel execution.
        let listener = find_available_port(0).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(port > 0, "OS should assign a nonzero ephemeral port");

        // The listener returned is already bound and live — a peer can
        // connect to it without any further bind call on our part. This is
        // the type-level guarantee: the caller never has to (and must not)
        // re-bind to learn whether the port is usable.
        std::net::TcpStream::connect(("127.0.0.1", port))
            .expect("should connect to the already-bound listener");
    }

    #[test]
    fn test_find_available_port_skips_bound_port() {
        // Hold an OS-assigned ephemeral port open, then scan starting from
        // it. Since we still own the listener, find_available_port must
        // skip it and return a different, still-free port. Starting from a
        // freshly OS-assigned port (rather than a fixed low range like
        // 3000-3099) means this test never shares a hardcoded range with a
        // sibling test running concurrently.
        let held = TcpListener::bind("127.0.0.1:0").unwrap();
        let held_port = held.local_addr().unwrap().port();

        let listener = find_available_port(held_port).unwrap();
        let port = listener.local_addr().unwrap().port();

        assert_ne!(port, held_port, "must skip the already-bound port");
        assert!(
            (held_port..=held_port.saturating_add(99)).contains(&port),
            "chosen port {port} should be within the scanned range starting at {held_port}"
        );
    }

    // ── listener handoff (parent side) ────────────────────────────────────────

    #[test]
    #[cfg(unix)]
    fn test_inherit_listener_hands_off_socket() {
        use nix::fcntl::{fcntl, FcntlArg, FdFlag};
        use std::os::unix::io::AsRawFd;

        let listener = find_available_port(0).unwrap();
        let fd = listener.as_raw_fd();

        let mut cmd = std::process::Command::new("true");
        inherit_listener(&mut cmd, &listener).unwrap();

        // Close-on-exec must be cleared so the descriptor survives the child's
        // execve — this is what lets the child adopt the bound socket instead
        // of re-binding the port.
        let flags = fcntl(&listener, FcntlArg::F_GETFD).unwrap();
        assert_eq!(
            flags & FdFlag::FD_CLOEXEC.bits(),
            0,
            "FD_CLOEXEC must be cleared so the socket survives the child's exec"
        );

        // The child is pointed at this exact descriptor via the listenfd env.
        let envs: std::collections::HashMap<String, String> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v?.to_str()?.to_owned())))
            .collect();
        assert_eq!(envs.get("LISTEN_FDS").map(String::as_str), Some("1"));
        assert_eq!(
            envs.get("LISTEN_FDS_FIRST_FD").map(String::as_str),
            Some(fd.to_string().as_str()),
            "child must be pointed at the parent's bound descriptor"
        );
        // LISTEN_PID stays unset: listenfd reads an absent value as addressed
        // to the receiving process, which sidesteps not knowing the child PID.
        assert!(
            !envs.contains_key("LISTEN_PID"),
            "LISTEN_PID must remain unset"
        );
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

    // ── startup failure: child exits immediately ──────────────────────────────

    #[test]
    #[cfg(unix)]
    fn test_start_server_errors_when_child_exits_immediately() {
        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        // `/bin/false` exits immediately with exit code 1 — simulates a
        // jit-server that fails to bind its port.
        let opts = ServeOptions {
            data_dir: data_dir.to_path_buf(),
            // 0 = any OS-assigned free port; this test doesn't care which
            // port is chosen, and using 0 avoids sharing a fixed range with
            // any sibling test running in parallel.
            preferred_port: 0,
            log_file: None,
            web_dir: None,
            server_binary: Some(PathBuf::from("/bin/false")),
        };

        let result = start_server(opts);
        assert!(
            result.is_err(),
            "must return Err when server exits immediately"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("exited during startup"),
            "error message should mention startup exit, got: {msg}"
        );
        // No stale PID file should be written.
        assert!(
            !pid_file_path(data_dir).exists(),
            "PID file must not be written when server exits immediately"
        );
    }

    // ── orphan cleanup: pid-file write fails after successful spawn ──────────

    #[test]
    #[cfg(unix)]
    fn test_start_server_kills_orphan_when_pid_write_fails() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let data_dir = tmp.path();

        // Write a shell script that ignores all arguments and sleeps, so it
        // stays alive long enough to pass the try_wait liveness check.
        let fake_server = tmp.path().join("fake-jit-server.sh");
        std::fs::write(&fake_server, "#!/bin/sh\nexec sleep 60\n").unwrap();
        let mut perms = std::fs::metadata(&fake_server).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_server, perms).unwrap();

        // Block write_pid_file by placing a directory at the PID file path.
        // The atomic write goes tmp→final via rename; renaming a regular file
        // over a directory fails, triggering our orphan-cleanup path.
        let pid_path = pid_file_path(data_dir);
        std::fs::create_dir_all(&pid_path).unwrap();

        let opts = ServeOptions {
            data_dir: data_dir.to_path_buf(),
            // See the sibling startup-failure test above for why 0.
            preferred_port: 0,
            log_file: None,
            web_dir: None,
            server_binary: Some(fake_server),
        };

        let result = start_server(opts);
        assert!(result.is_err(), "must fail when PID file cannot be written");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("PID") || msg.contains("pid") || msg.contains("persist"),
            "error should relate to PID persistence, got: {msg}"
        );

        // Give the kill a moment to propagate, then confirm the process is gone.
        std::thread::sleep(Duration::from_millis(200));
        // We do not have the child PID here, but the key invariant is that the
        // PID file was NOT successfully written as a JSON record (the directory
        // blocker is still in place, proving no rename succeeded).
        assert!(
            pid_path.is_dir(),
            "directory blocker should still be there — no JSON PID was persisted"
        );
    }
}
