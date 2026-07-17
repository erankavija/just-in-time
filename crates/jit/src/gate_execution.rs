//! Gate execution engine for automated quality checks
//!
//! This module implements the execution of automated gates, including:
//! - Command execution with timeouts
//! - Output capture (stdout/stderr)
//! - Git commit/branch tracking
//! - Result storage for audit trail

use crate::domain::{
    DocumentReference, GateChecker, GateContext, GateRunResult, GateRunStatus, GateStage,
    GATE_RUN_SCHEMA_VERSION,
};
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Actor stamped on the `by` field of a gate run recorded by the automated
/// executor (as opposed to a human or agent attestation). A recorded pass whose
/// `updated_by` equals this value is an auto-era pass with no human attestation
/// behind it, which matters when a gate is later redefined to manual mode.
pub(crate) const AUTO_EXECUTOR: &str = "auto:executor";

/// Execute a gate checker and return the result (without context).
///
/// Convenience wrapper around [`execute_gate_checker_with_context`] that passes `None`
/// for context and no linked documents. Basic env vars (`JIT_ISSUE_ID`, `JIT_GATE_KEY`,
/// `JIT_STAGE`, `JIT_ISSUE_DOCS`, `JIT_GATE_RUN`) are still set.
pub fn execute_gate_checker(
    gate_key: &str,
    issue_id: &str,
    stage: GateStage,
    checker: &GateChecker,
    working_dir: &Path,
) -> Result<GateRunResult> {
    execute_gate_checker_with_context(gate_key, issue_id, stage, checker, working_dir, None, &[])
}

/// One entry in the `JIT_ISSUE_DOCS` env var: the fields a checker needs to
/// inline a linked document into a review prompt.
#[derive(serde::Serialize)]
struct IssueDocEnvEntry<'a> {
    path: &'a str,
    doc_type: Option<&'a str>,
    label: Option<&'a str>,
}

/// Build the JSON value of the `JIT_ISSUE_DOCS` env var from an issue's linked
/// documents.
///
/// Every gate checker process receives `JIT_ISSUE_DOCS` (alongside
/// `JIT_ISSUE_ID`, `JIT_GATE_KEY`, `JIT_STAGE`) so a checker script — e.g.
/// `scripts/ai-review.sh` — can inline linked design/plan docs into its
/// review prompt without hand-pasting them into the issue description first.
///
/// Each entry keeps only the fields a checker needs: `path`, `doc_type`, and
/// `label` (each `null` when the source [`DocumentReference`] leaves it
/// unset). The result is always a valid JSON array — `"[]"` when `documents`
/// is empty — so a checker can parse the variable unconditionally instead of
/// special-casing an absent or missing value.
pub fn build_issue_docs_env(documents: &[DocumentReference]) -> String {
    let entries: Vec<IssueDocEnvEntry> = documents
        .iter()
        .map(|doc| IssueDocEnvEntry {
            path: &doc.path,
            doc_type: doc.doc_type.as_deref(),
            label: doc.label.as_deref(),
        })
        .collect();

    // `Vec<IssueDocEnvEntry>` serialization cannot fail (plain strings and
    // options, no map keys, no floats); fall back to an empty array
    // defensively rather than panicking in library code.
    serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string())
}

/// Execute a gate checker with optional structured context.
///
/// This function runs the specified checker and captures all execution details
/// including exit code, output, timing, and git context if available.
///
/// Basic env vars (`JIT_ISSUE_ID`, `JIT_GATE_KEY`, `JIT_STAGE`, `JIT_ISSUE_DOCS`,
/// `JIT_GATE_RUN`) are always set. `JIT_ISSUE_DOCS` is built from `documents` via
/// [`build_issue_docs_env`]. When `context` is `Some`, a temporary JSON file is
/// written containing the structured context and made available via the
/// `JIT_CONTEXT_FILE` env var.
///
/// `JIT_GATE_RUN=1` marks every process in the checker's subtree as running
/// inside a gate checker (jit:7446af34 REQ-02): any `jit` invocation that
/// inherits it — including a checker SCRIPT that itself shells out to `jit`
/// (e.g. `scripts/jit-validate.sh`'s `exec jit validate "$@"`), which
/// resolves `jit` from `PATH` independently of the evaluator process — self-
/// checks its own build provenance at startup and refuses under the same
/// condition [`check_gate`](crate::commands::CommandExecutor::check_gate)
/// uses (see [`domain::build_provenance`](crate::domain::build_provenance)
/// for the full predicate), so a stale binary anywhere in the checker's
/// process tree cannot silently produce the recorded verdict. See `main`'s
/// startup dispatch (binary crate) for the child-side check.
pub fn execute_gate_checker_with_context(
    gate_key: &str,
    issue_id: &str,
    stage: GateStage,
    checker: &GateChecker,
    working_dir: &Path,
    context: Option<&GateContext>,
    documents: &[DocumentReference],
) -> Result<GateRunResult> {
    let start_time = Instant::now();
    let started_at = chrono::Utc::now();

    let git_context = get_git_context(working_dir);

    // Build base env vars that are always set
    let mut base_env = std::collections::HashMap::new();
    base_env.insert("JIT_ISSUE_ID".to_string(), issue_id.to_string());
    base_env.insert("JIT_GATE_KEY".to_string(), gate_key.to_string());
    base_env.insert("JIT_STAGE".to_string(), stage.as_str().to_string());
    base_env.insert(
        "JIT_ISSUE_DOCS".to_string(),
        build_issue_docs_env(documents),
    );
    // REQ-02 (jit:7446af34): marks every process in the checker's subtree so
    // any `jit` invocation, direct or nested, self-checks its own build
    // provenance at startup — see the doc comment above.
    base_env.insert("JIT_GATE_RUN".to_string(), "1".to_string());

    // Write context file if context is provided; otherwise explicitly clear
    // JIT_CONTEXT_FILE so it is never inherited from the parent environment
    // (e.g. when cargo test is invoked from inside a gate checker).
    let _context_tempfile = if let Some(ctx) = context {
        let mut tmpfile = tempfile::NamedTempFile::new()
            .context("Failed to create temp file for gate context")?;
        serde_json::to_writer_pretty(&mut tmpfile, ctx)
            .context("Failed to write gate context JSON")?;
        tmpfile.flush().context("Failed to flush context file")?;
        let path = tmpfile.path().to_string_lossy().to_string();
        base_env.insert("JIT_CONTEXT_FILE".to_string(), path);
        Some(tmpfile)
    } else {
        None
    };

    let execution_result = match checker {
        GateChecker::Exec {
            command,
            timeout_seconds,
            env,
            ..
        } => execute_command(command, *timeout_seconds, env, &base_env, working_dir)?,
        _ => anyhow::bail!(
            "built-in gate checkers must be executed through CommandExecutor::check_gate"
        ),
    };

    let duration = start_time.elapsed();
    let completed_at = chrono::Utc::now();

    // Parse the checker's machine-readable findings block (if any) once, at
    // record time, so every downstream view reads structured data instead of
    // re-grepping stdout. Absent or malformed blocks degrade to `None`.
    let findings = crate::domain::parse_gate_findings(&execution_result.stdout);

    let status = match execution_result.exit_code {
        Some(0) => GateRunStatus::Passed,
        // Shell could not execute the command: 127 = command not found,
        // 126 = found but not executable. Runner/infra error, not a checker verdict.
        Some(126) | Some(127) => GateRunStatus::Error,
        Some(_) => GateRunStatus::Failed,
        None => GateRunStatus::Error, // killed by signal / timeout
    };

    Ok(GateRunResult {
        schema_version: GATE_RUN_SCHEMA_VERSION,
        run_id: uuid::Uuid::new_v4().to_string(),
        gate_key: gate_key.to_string(),
        stage,
        issue_id: issue_id.to_string(),
        commit: git_context.commit,
        branch: git_context.branch,
        tree_dirty: git_context.tree_dirty,
        status,
        started_at,
        completed_at: Some(completed_at),
        duration_ms: Some(duration.as_millis() as u64),
        exit_code: execution_result.exit_code,
        stdout: execution_result.stdout,
        stderr: execution_result.stderr,
        command: execution_result.command,
        by: Some(AUTO_EXECUTOR.to_string()),
        message: None,
        findings,
    })
}

/// Result of command execution
struct CommandExecutionResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    command: String,
}

/// Execute a shell command with timeout and capture output
fn execute_command(
    command: &str,
    timeout_seconds: u64,
    env: &std::collections::HashMap<String, String>,
    base_env: &std::collections::HashMap<String, String>,
    working_dir: &Path,
) -> Result<CommandExecutionResult> {
    let timeout = Duration::from_secs(timeout_seconds);

    #[cfg(unix)]
    let mut cmd = {
        use std::os::unix::process::CommandExt;
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        // Place the child in its own process group (PGID = child PID) so that
        // on timeout we can kill all descendants (e.g. test binaries spawned by
        // `cargo test`) as a group, not just the immediate shell process.
        c.process_group(0);
        c
    };

    #[cfg(windows)]
    let mut cmd = {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    };

    cmd.current_dir(working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Add base env vars (JIT_ISSUE_ID, JIT_GATE_KEY, JIT_STAGE, JIT_ISSUE_DOCS,
    // JIT_CONTEXT_FILE). Clear JIT_CONTEXT_FILE if not explicitly set, to
    // prevent leaking from the parent environment (e.g. when tests run inside
    // a gate checker).
    if !base_env.contains_key("JIT_CONTEXT_FILE") {
        cmd.env_remove("JIT_CONTEXT_FILE");
    }
    for (key, value) in base_env {
        cmd.env(key, value);
    }

    // Add checker-specific environment variables
    for (key, value) in env {
        cmd.env(key, value);
    }

    // Spawn the process
    let mut child = cmd.spawn().context("Failed to spawn command")?;

    // Drain stdout/stderr in background threads to prevent a pipe-buffer
    // deadlock.  Commands like `cargo test` spawn many child processes that
    // together can produce more data than the OS pipe buffer (~64 KB).  If we
    // don't drain the pipes concurrently, those processes block on write(2) and
    // we deadlock waiting for them to exit.
    let stdout_thread = child.stdout.take().map(|stdout| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = String::new();
            let _ = std::io::BufReader::new(stdout).read_to_string(&mut buf);
            buf
        })
    });
    let stderr_thread = child.stderr.take().map(|stderr| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = String::new();
            let _ = std::io::BufReader::new(stderr).read_to_string(&mut buf);
            buf
        })
    });

    // Wait with timeout. On timeout the helper kills the entire process GROUP
    // (PGID == child PID, set via `process_group(0)` above) BEFORE reaping the
    // leader, so any grandchildren still holding pipe ends (e.g. test binaries
    // that outlived the killed `cargo` process) release them and the reader
    // threads above can reach EOF. Killing the group before the leader is reaped
    // is essential: once the leader is reaped its PID/PGID can be recycled, and a
    // later group-kill could then signal an unrelated process group.
    let wait_result = wait_with_timeout(&mut child, timeout)?;

    let stdout = stdout_thread
        .and_then(|h| h.join().ok())
        .unwrap_or_default();
    let stderr = stderr_thread
        .and_then(|h| h.join().ok())
        .unwrap_or_default();

    Ok(CommandExecutionResult {
        exit_code: wait_result.exit_code,
        stdout,
        stderr,
        command: command.to_string(),
    })
}

/// Wait for a child process with timeout. `exit_code` is `None` on timeout
/// (the child was killed) and otherwise the process exit code.
struct WaitResult {
    exit_code: Option<i32>,
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> Result<WaitResult> {
    let start = Instant::now();

    loop {
        match child.try_wait()? {
            Some(status) => {
                return Ok(WaitResult {
                    exit_code: status.code(),
                })
            }
            None => {
                if start.elapsed() >= timeout {
                    // Signal the whole process GROUP first, then reap the leader
                    // exactly once. The group MUST be signaled before the leader
                    // is reaped: once reaped, the PID/PGID can be recycled by the
                    // OS and a later group-kill could hit an unrelated group.
                    kill_process_group(child);
                    let _ = child.wait();
                    return Ok(WaitResult { exit_code: None });
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// On unix, send `SIGKILL` to the child's process GROUP (negative PGID) so all
/// descendants in the group die, not just the leader. PGID == child PID because
/// the child was spawned with `process_group(0)`. On other platforms, fall back
/// to killing just the leader.
#[cfg(unix)]
fn kill_process_group(child: &mut std::process::Child) {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;
    // A negative PID in kill(2) targets the process group with that PGID
    // (== the leader PID, set via `process_group(0)` at spawn). Guard the value
    // before negating: a checked u32->i32 conversion avoids wrap-around for large
    // PIDs, and rejecting PID <= 1 prevents the catastrophic `kill(-1)` (every
    // process owned by the user) and `kill(0)` (the caller's OWN group). Any
    // unexpected value simply falls back to killing the leader directly.
    match i32::try_from(child.id()) {
        Ok(pid) if pid > 1 => {
            let _ = kill(Pid::from_raw(-pid), Signal::SIGKILL);
        }
        _ => {
            let _ = child.kill();
        }
    }
}

#[cfg(not(unix))]
fn kill_process_group(child: &mut std::process::Child) {
    let _ = child.kill();
}

/// Git context information
struct GitContext {
    /// The commit paired with `tree_dirty` by [`probe_stable_git_pair`]: a
    /// HEAD read taken immediately after the tree-status probe, verified (or,
    /// past the retry bound, best-effort) to match a HEAD read taken before
    /// it, so `tree_dirty` is never attributed to the wrong commit.
    commit: Option<String>,
    branch: Option<String>,
    /// Whether the working tree differed from `commit` at capture time. `None`
    /// whenever `commit` is `None`: with no named commit there is nothing for
    /// the tree to match, so no cleanliness is claimed.
    tree_dirty: Option<bool>,
}

/// Get git context, gracefully degrading if not in a git repo.
///
/// `commit` and `tree_dirty` are read together through
/// [`probe_stable_git_pair`] so a commit landing between the two reads cannot
/// pair a cleanliness flag with the wrong commit; `branch` is read
/// independently since nothing downstream pairs it with `tree_dirty`.
fn get_git_context(working_dir: &Path) -> GitContext {
    let branch = get_git_branch(working_dir);
    let pair = probe_stable_git_pair(
        || get_git_commit(working_dir),
        || get_git_tree_dirty(working_dir),
    );

    GitContext {
        commit: pair.commit,
        branch,
        tree_dirty: pair.tree_dirty,
    }
}

/// Bound on retry attempts in [`probe_stable_git_pair`]: enough to absorb one
/// or two commits landing mid-probe under this repository's concurrent-agent
/// commit churn, without looping indefinitely if HEAD never quiesces.
const STABLE_PAIR_MAX_ATTEMPTS: u32 = 3;

/// A commit paired with a tree-dirty flag by [`probe_stable_git_pair`].
struct StableGitPair {
    commit: Option<String>,
    tree_dirty: Option<bool>,
}

/// Pair a HEAD probe with a working-tree-status probe so the recorded commit
/// is never attributed to the wrong `tree_dirty` value.
///
/// `head_probe` and `status_probe` are injected as closures — rather than
/// this function calling [`get_git_commit`]/[`get_git_tree_dirty`] directly —
/// so a unit test can simulate a commit landing between reads without
/// shelling out to git.
///
/// Each attempt reads HEAD (`h1`), then the tree status, then HEAD again
/// (`h2`). `status_probe` runs only when `h1` is `Some`: tree cleanliness is
/// meaningful only relative to a resolved commit, so a repo with no commits
/// yet never reports a fabricated clean tree. When `h1 == h2`, the two HEAD
/// reads bracket a quiescent status probe and the pair is returned
/// immediately.
///
/// If `h1 != h2` on every attempt through [`STABLE_PAIR_MAX_ATTEMPTS`], the
/// LAST attempt's pair is returned anyway, with `commit` set to that
/// attempt's `h2` — the HEAD read taken immediately after `status_probe`, the
/// closer of the two bracketing reads to when `tree_dirty` was actually
/// observed. This keeps the recorded commit bracketing the status probe on
/// its trailing side even when instability persists across every attempt.
/// The residual uncertainty this cannot remove: a commit landing during the
/// `status_probe` call itself is invisible to any number of surrounding HEAD
/// reads.
fn probe_stable_git_pair(
    mut head_probe: impl FnMut() -> Option<String>,
    mut status_probe: impl FnMut() -> Option<bool>,
) -> StableGitPair {
    let mut last = StableGitPair {
        commit: None,
        tree_dirty: None,
    };
    for _ in 0..STABLE_PAIR_MAX_ATTEMPTS {
        let h1 = head_probe();
        let tree_dirty = if h1.is_some() { status_probe() } else { None };
        let h2 = head_probe();
        let stable = h1 == h2;
        last = StableGitPair {
            commit: h2,
            tree_dirty,
        };
        if stable {
            return last;
        }
    }
    last
}

/// Resolve the current `HEAD` commit hash for `working_dir`.
///
/// Returns `None` when `working_dir` is not inside a git repository, when git
/// is unavailable, or when the `git rev-parse HEAD` invocation fails (e.g. a
/// repository with no commits yet). The value is the full 40-character SHA, the
/// same one stamped into [`GateRunResult::commit`](crate::domain::GateRunResult)
/// via [`get_git_context`], so callers can compare the two directly.
pub(crate) fn get_git_commit(working_dir: &Path) -> Option<String> {
    Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(working_dir)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
}

fn get_git_branch(working_dir: &Path) -> Option<String> {
    Command::new("git")
        .arg("rev-parse")
        .arg("--abbrev-ref")
        .arg("HEAD")
        .current_dir(working_dir)
        .output()
        .ok()
        .and_then(|output| {
            if output.status.success() {
                Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
            } else {
                None
            }
        })
}

/// Report whether `working_dir`'s tree differs from `HEAD`.
///
/// Returns `Some(true)` when `git status --porcelain` reports any change —
/// staged, unstaged, or untracked (`--untracked-files=normal`, the same probe
/// `scripts/install-jit.sh` uses to stamp build provenance) — `Some(false)` when
/// the tree is clean, and `None` when git is unavailable or `working_dir` is not
/// inside a git repository. Recorded into
/// [`GateRunResult::tree_dirty`](crate::domain::GateRunResult), so a pass
/// produced against a modified tree is distinguishable from one evidencing the
/// commit itself.
pub(crate) fn get_git_tree_dirty(working_dir: &Path) -> Option<bool> {
    Command::new("git")
        .arg("status")
        .arg("--porcelain")
        .arg("--untracked-files=normal")
        .current_dir(working_dir)
        .output()
        .ok()
        .and_then(|output| output.status.success().then_some(!output.stdout.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_execute_simple_command_success() {
        let checker = GateChecker::Exec {
            command: "echo 'hello world'".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        );

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.stdout.contains("hello world"));
    }

    #[test]
    fn test_execute_command_failure() {
        let checker = GateChecker::Exec {
            command: "exit 1".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        );

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status, GateRunStatus::Failed);
        assert_eq!(result.exit_code, Some(1));
    }

    #[test]
    fn test_execute_command_timeout() {
        let checker = GateChecker::Exec {
            command: "sleep 10".to_string(),
            timeout_seconds: 1,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        );

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status, GateRunStatus::Error);
        assert_eq!(result.exit_code, None); // No exit code on timeout
    }

    #[cfg(unix)]
    #[test]
    fn test_timeout_kills_process_group_and_does_not_deadlock() {
        // The command spawns a long-lived BACKGROUND grandchild that inherits the
        // stdout pipe and would keep it open for 60s. With the group-kill on
        // timeout, the whole process group (including the grandchild) is killed,
        // the reader threads reach EOF, and the call returns promptly instead of
        // deadlocking on pipe drain. This exercises the reap-then-signal fix (#3):
        // the group is signaled BEFORE the leader is reaped.
        let checker = GateChecker::Exec {
            // Background a sleeper that holds the inherited stdout fd, then the
            // shell itself sleeps past the timeout.
            command: "sleep 60 & sleep 30".to_string(),
            timeout_seconds: 1,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let start = Instant::now();
        let result = execute_gate_checker(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        )
        .unwrap();

        // Returns shortly after the 1s timeout (well under the 30/60s sleeps),
        // proving the background grandchild's pipe was released by the group-kill.
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "timeout should return promptly, took {:?}",
            start.elapsed()
        );
        assert_eq!(result.status, GateRunStatus::Error);
        assert_eq!(result.exit_code, None);
    }

    #[test]
    fn test_git_context_graceful_degradation() {
        // Test in a non-git directory
        let temp_dir = std::env::temp_dir();
        let context = get_git_context(&temp_dir);

        // Should not panic, just return None values
        assert!(context.commit.is_none() || !context.commit.unwrap().is_empty());
        assert!(context.branch.is_none() || !context.branch.unwrap().is_empty());
    }

    /// Initialize a git repo at `dir` with one commit; returns nothing but leaves
    /// a committed `seed.txt`. Panics on any git failure so a broken environment
    /// surfaces loudly rather than as a misleading assertion.
    fn init_committed_repo(dir: &Path) {
        let git = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?} failed");
        };
        git(&["init", "-q"]);
        git(&["config", "user.name", "Test"]);
        git(&["config", "user.email", "test@example.com"]);
        std::fs::write(dir.join("seed.txt"), "seed\n").expect("write seed");
        git(&["add", "."]);
        git(&["commit", "-q", "-m", "seed"]);
    }

    /// REQ-01: the tree probe reports a committed tree clean and a modified tree
    /// dirty, and counts an untracked file as dirty.
    #[test]
    fn test_get_git_tree_dirty_reports_clean_then_dirty() {
        let temp = tempfile::TempDir::new().unwrap();
        init_committed_repo(temp.path());

        assert_eq!(get_git_tree_dirty(temp.path()), Some(false));

        std::fs::write(temp.path().join("seed.txt"), "changed\n").unwrap();
        assert_eq!(get_git_tree_dirty(temp.path()), Some(true));

        // Restore, then an untracked file alone still reads dirty.
        std::fs::write(temp.path().join("seed.txt"), "seed\n").unwrap();
        assert_eq!(get_git_tree_dirty(temp.path()), Some(false));
        std::fs::write(temp.path().join("untracked.txt"), "new\n").unwrap();
        assert_eq!(get_git_tree_dirty(temp.path()), Some(true));
    }

    /// git is optional (@/charter/D-4): outside a git repository the probe yields
    /// `None` rather than a fabricated clean tree.
    #[test]
    fn test_get_git_tree_dirty_is_none_without_git() {
        let temp = tempfile::TempDir::new().unwrap();
        assert_eq!(get_git_tree_dirty(temp.path()), None);
    }

    /// Tree cleanliness is only recorded relative to a resolved commit: a repo
    /// with no commits yet has no `commit`, so `tree_dirty` stays `None` even
    /// though `git status` would succeed.
    #[test]
    fn test_get_git_context_ties_tree_dirty_to_commit() {
        let temp = tempfile::TempDir::new().unwrap();
        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(temp.path())
                .status()
                .expect("git runs")
        };
        assert!(git(&["init", "-q"]).success());
        std::fs::write(temp.path().join("untracked.txt"), "x\n").unwrap();

        let context = get_git_context(temp.path());
        assert!(context.commit.is_none());
        assert_eq!(context.tree_dirty, None);
    }

    /// TOCTOU fix: a commit landing between the two HEAD reads of the first
    /// attempt (`h1 = "a"`, `h2 = "b"`) is discarded rather than paired with
    /// that attempt's `tree_dirty`. The second attempt observes a quiescent
    /// HEAD (`"b"` both times) and its pair — not the first attempt's — is
    /// the one returned.
    #[test]
    fn test_probe_stable_git_pair_retries_until_stable() {
        let head_calls = std::cell::Cell::new(0u32);
        let head_probe = || {
            let n = head_calls.get();
            head_calls.set(n + 1);
            match n {
                0 => Some("a".to_string()), // attempt 1, h1
                1 => Some("b".to_string()), // attempt 1, h2 -> mismatch, retry
                _ => Some("b".to_string()), // attempt 2, h1 and h2 -> stable
            }
        };
        let status_calls = std::cell::Cell::new(0u32);
        let status_probe = || {
            let n = status_calls.get();
            status_calls.set(n + 1);
            match n {
                0 => Some(true),  // attempt 1's (discarded) tree_dirty
                _ => Some(false), // attempt 2's tree_dirty
            }
        };

        let pair = probe_stable_git_pair(head_probe, status_probe);

        assert_eq!(pair.commit.as_deref(), Some("b"));
        assert_eq!(pair.tree_dirty, Some(false));
        assert_eq!(
            head_calls.get(),
            4,
            "two HEAD reads per attempt, two attempts"
        );
        assert_eq!(status_calls.get(), 2);
    }

    /// TOCTOU fix, give-up bound: when HEAD changes on every read (never
    /// stable), the probe stops after [`STABLE_PAIR_MAX_ATTEMPTS`] attempts
    /// and returns the LAST attempt's pair, with `commit` set to that
    /// attempt's `h2` — the HEAD read bracketing the trailing side of the
    /// status probe.
    #[test]
    fn test_probe_stable_git_pair_gives_up_after_max_attempts() {
        let head_calls = std::cell::Cell::new(0u32);
        let head_probe = || {
            let n = head_calls.get();
            head_calls.set(n + 1);
            Some(format!("commit-{n}"))
        };

        let pair = probe_stable_git_pair(head_probe, || Some(true));

        assert_eq!(head_calls.get(), STABLE_PAIR_MAX_ATTEMPTS * 2);
        let last_h2 = format!("commit-{}", STABLE_PAIR_MAX_ATTEMPTS * 2 - 1);
        assert_eq!(pair.commit, Some(last_h2));
        assert_eq!(pair.tree_dirty, Some(true));
    }

    /// The seam preserves the existing tie-to-commit rule: when HEAD never
    /// resolves, the status probe never runs (and the unresolved pair is
    /// accepted on the first attempt, since `None == None`).
    #[test]
    fn test_probe_stable_git_pair_skips_status_probe_when_head_unresolved() {
        let status_calls = std::cell::Cell::new(0u32);
        let pair = probe_stable_git_pair(
            || None,
            || {
                status_calls.set(status_calls.get() + 1);
                Some(false)
            },
        );

        assert_eq!(pair.commit, None);
        assert_eq!(pair.tree_dirty, None);
        assert_eq!(status_calls.get(), 0);
    }

    /// REQ-01/REQ-04: a recorded gate run stamps `tree_dirty == Some(false)` when
    /// the checker starts against a clean committed tree.
    #[test]
    fn test_execute_gate_checker_records_clean_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        init_committed_repo(temp.path());

        let checker = GateChecker::Exec {
            command: "true".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };
        let result =
            execute_gate_checker("g", "issue-1", GateStage::Postcheck, &checker, temp.path())
                .unwrap();

        assert!(result.commit.is_some());
        assert_eq!(result.tree_dirty, Some(false));
    }

    /// REQ-01/REQ-04: a recorded gate run stamps `tree_dirty == Some(true)` when
    /// the working tree carries an uncommitted change at checker start.
    #[test]
    fn test_execute_gate_checker_records_dirty_tree() {
        let temp = tempfile::TempDir::new().unwrap();
        init_committed_repo(temp.path());
        std::fs::write(temp.path().join("seed.txt"), "dirtied\n").unwrap();

        let checker = GateChecker::Exec {
            command: "true".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };
        let result =
            execute_gate_checker("g", "issue-1", GateStage::Postcheck, &checker, temp.path())
                .unwrap();

        assert!(result.commit.is_some());
        assert_eq!(result.tree_dirty, Some(true));
    }

    #[test]
    fn test_execute_with_environment_variables() {
        let mut env = HashMap::new();
        env.insert("TEST_VAR".to_string(), "test_value".to_string());

        #[cfg(unix)]
        let command = "echo $TEST_VAR";
        #[cfg(windows)]
        let command = "echo %TEST_VAR%";

        let checker = GateChecker::Exec {
            command: command.to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env,
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        );

        assert!(result.is_ok());
        let result = result.unwrap();
        assert_eq!(result.status, GateRunStatus::Passed);
        assert!(result.stdout.contains("test_value"));
    }

    #[test]
    fn test_basic_env_vars_always_set() {
        // JIT_ISSUE_ID, JIT_GATE_KEY, JIT_STAGE should be set on every gate run
        let checker = GateChecker::Exec {
            command: "echo \"ID=$JIT_ISSUE_ID KEY=$JIT_GATE_KEY STAGE=$JIT_STAGE\"".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker(
            "my-gate",
            "issue-123",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert!(result.stdout.contains("ID=issue-123"));
        assert!(result.stdout.contains("KEY=my-gate"));
        assert!(result.stdout.contains("STAGE=postcheck"));
    }

    #[test]
    fn test_build_issue_docs_env_empty_when_no_documents() {
        assert_eq!(build_issue_docs_env(&[]), "[]");
    }

    #[test]
    fn test_build_issue_docs_env_serializes_path_type_and_label() {
        let docs = vec![
            DocumentReference {
                path: "dev/active/plan.md".to_string(),
                commit: None,
                label: Some("Implementation Plan".to_string()),
                doc_type: Some("design".to_string()),
                format: None,
                assets: Vec::new(),
            },
            // A doc with no label/doc_type still serializes, as explicit nulls.
            DocumentReference::new("NOTES.md".to_string()),
        ];

        let json = build_issue_docs_env(&docs);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entries = parsed.as_array().unwrap();
        assert_eq!(entries.len(), 2);

        assert_eq!(entries[0]["path"], "dev/active/plan.md");
        assert_eq!(entries[0]["doc_type"], "design");
        assert_eq!(entries[0]["label"], "Implementation Plan");

        assert_eq!(entries[1]["path"], "NOTES.md");
        assert!(entries[1]["doc_type"].is_null());
        assert!(entries[1]["label"].is_null());
    }

    #[test]
    fn test_issue_docs_env_var_set_with_linked_documents() {
        // REQ-01: a gate checker process receives JIT_ISSUE_DOCS as a
        // machine-readable list of {path, doc_type, label} for the issue's
        // linked documents.
        let documents = vec![DocumentReference {
            path: "dev/active/my-plan.md".to_string(),
            commit: None,
            label: Some("Plan".to_string()),
            doc_type: Some("design".to_string()),
            format: None,
            assets: Vec::new(),
        }];

        let checker = GateChecker::Exec {
            command: "echo \"$JIT_ISSUE_DOCS\"".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker_with_context(
            "my-gate",
            "issue-123",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &documents,
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        let parsed: serde_json::Value = serde_json::from_str(result.stdout.trim())
            .expect("JIT_ISSUE_DOCS should be valid JSON");
        let entries = parsed.as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["path"], "dev/active/my-plan.md");
        assert_eq!(entries[0]["doc_type"], "design");
        assert_eq!(entries[0]["label"], "Plan");
    }

    #[test]
    fn test_issue_docs_env_var_empty_array_when_no_linked_documents() {
        // REQ-01 empty case: no linked docs -> JIT_ISSUE_DOCS is present and
        // set to an empty JSON array, never absent or unset.
        let checker = GateChecker::Exec {
            command: "echo \"$JIT_ISSUE_DOCS\"".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker(
            "my-gate",
            "issue-123",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.stdout.trim(), "[]");
    }

    #[test]
    fn test_context_file_written_when_context_provided() {
        use crate::domain::GateContext;

        let context = GateContext {
            schema_version: 1,
            prompt: Some("Review the code".to_string()),
            issue: serde_json::json!({"id": "issue-123", "title": "Test issue"}),
            gate: serde_json::json!({"key": "review", "title": "Code Review"}),
            run_history: vec![],
        };

        // Checker reads the context file and echoes its content
        let checker = GateChecker::Exec {
            command: "cat $JIT_CONTEXT_FILE".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: true,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker_with_context(
            "review",
            "issue-123",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            Some(&context),
            &[],
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);

        // Parse the stdout as JSON and verify structure
        let parsed: serde_json::Value =
            serde_json::from_str(&result.stdout).expect("Context file should contain valid JSON");
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["prompt"], "Review the code");
        assert_eq!(parsed["issue"]["id"], "issue-123");
        assert_eq!(parsed["gate"]["key"], "review");
        assert!(parsed["run_history"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_context_file_cleaned_up_after_execution() {
        use crate::domain::GateContext;

        let context = GateContext {
            schema_version: 1,
            prompt: None,
            issue: serde_json::json!({}),
            gate: serde_json::json!({}),
            run_history: vec![],
        };

        // Checker prints the context file path so we can check it was cleaned up
        let checker = GateChecker::Exec {
            command: "echo $JIT_CONTEXT_FILE".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: true,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker_with_context(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            Some(&context),
            &[],
        )
        .unwrap();

        let context_path = result.stdout.trim();
        assert!(
            !context_path.is_empty(),
            "JIT_CONTEXT_FILE should have been set"
        );
        assert!(
            !std::path::Path::new(context_path).exists(),
            "Context file should be cleaned up after execution"
        );
    }

    #[test]
    fn test_no_context_file_when_no_context() {
        // When no context is provided, JIT_CONTEXT_FILE should not be set
        let checker = GateChecker::Exec {
            command: "echo \"CTX=${JIT_CONTEXT_FILE:-unset}\"".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker_with_context(
            "test-gate",
            "test-issue",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert!(result.stdout.contains("CTX=unset"));
    }

    #[test]
    fn test_context_includes_run_history() {
        use crate::domain::{GateContext, GateRunResult, GateRunStatus as RS};

        let previous_run = GateRunResult {
            schema_version: 1,
            run_id: "prev-run-1".to_string(),
            gate_key: "review".to_string(),
            stage: GateStage::Postcheck,
            issue_id: "issue-123".to_string(),
            commit: Some("abc123".to_string()),
            branch: Some("main".to_string()),
            tree_dirty: None,
            status: RS::Failed,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(100),
            exit_code: Some(1),
            stdout: "Previous review feedback".to_string(),
            stderr: String::new(),
            command: "review-checker".to_string(),
            by: Some(AUTO_EXECUTOR.to_string()),
            message: None,
            findings: None,
        };

        let context = GateContext {
            schema_version: 1,
            prompt: Some("Review again".to_string()),
            issue: serde_json::json!({"id": "issue-123"}),
            gate: serde_json::json!({"key": "review"}),
            run_history: vec![previous_run],
        };

        let checker = GateChecker::Exec {
            command: "cat $JIT_CONTEXT_FILE".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: true,
            prompt: None,
            prompt_file: None,
        };

        let temp_dir = std::env::temp_dir();
        let result = execute_gate_checker_with_context(
            "review",
            "issue-123",
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            Some(&context),
            &[],
        )
        .unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&result.stdout).unwrap();
        let history = parsed["run_history"].as_array().unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0]["run_id"], "prev-run-1");
        assert_eq!(history[0]["status"], "failed");
        assert_eq!(history[0]["stdout"], "Previous review feedback");
    }
}
