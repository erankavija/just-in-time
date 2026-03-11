//! Gate execution engine for automated quality checks
//!
//! This module implements the execution of automated gates, including:
//! - Command execution with timeouts
//! - Output capture (stdout/stderr)
//! - Git commit/branch tracking
//! - Result storage for audit trail

use crate::domain::{GateChecker, GateContext, GateRunResult, GateRunStatus, GateStage};
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Execute a gate checker and return the result (without context).
///
/// Convenience wrapper around [`execute_gate_checker_with_context`] that passes `None`
/// for context. Basic env vars (`JIT_ISSUE_ID`, `JIT_GATE_KEY`, `JIT_STAGE`) are still set.
pub fn execute_gate_checker(
    gate_key: &str,
    issue_id: &str,
    stage: GateStage,
    checker: &GateChecker,
    working_dir: &Path,
) -> Result<GateRunResult> {
    execute_gate_checker_with_context(gate_key, issue_id, stage, checker, working_dir, None)
}

/// Execute a gate checker with optional structured context.
///
/// This function runs the specified checker and captures all execution details
/// including exit code, output, timing, and git context if available.
///
/// Basic env vars (`JIT_ISSUE_ID`, `JIT_GATE_KEY`, `JIT_STAGE`) are always set.
/// When `context` is `Some`, a temporary JSON file is written containing the
/// structured context and made available via the `JIT_CONTEXT_FILE` env var.
pub fn execute_gate_checker_with_context(
    gate_key: &str,
    issue_id: &str,
    stage: GateStage,
    checker: &GateChecker,
    working_dir: &Path,
    context: Option<&GateContext>,
) -> Result<GateRunResult> {
    let start_time = Instant::now();
    let started_at = chrono::Utc::now();

    let git_context = get_git_context(working_dir);

    // Build base env vars that are always set
    let stage_str = match stage {
        GateStage::Precheck => "precheck",
        GateStage::Postcheck => "postcheck",
    };
    let mut base_env = std::collections::HashMap::new();
    base_env.insert("JIT_ISSUE_ID".to_string(), issue_id.to_string());
    base_env.insert("JIT_GATE_KEY".to_string(), gate_key.to_string());
    base_env.insert("JIT_STAGE".to_string(), stage_str.to_string());

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
    };

    let duration = start_time.elapsed();
    let completed_at = chrono::Utc::now();

    let status = match execution_result.exit_code {
        Some(0) => GateRunStatus::Passed,
        Some(_) => GateRunStatus::Failed,
        None => GateRunStatus::Error,
    };

    Ok(GateRunResult {
        schema_version: 1,
        run_id: uuid::Uuid::new_v4().to_string(),
        gate_key: gate_key.to_string(),
        stage,
        issue_id: issue_id.to_string(),
        commit: git_context.commit,
        branch: git_context.branch,
        status,
        started_at,
        completed_at: Some(completed_at),
        duration_ms: Some(duration.as_millis() as u64),
        exit_code: execution_result.exit_code,
        stdout: execution_result.stdout,
        stderr: execution_result.stderr,
        command: execution_result.command,
        by: Some("auto:executor".to_string()),
        message: None,
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

    // Add base env vars (JIT_ISSUE_ID, JIT_GATE_KEY, JIT_STAGE, JIT_CONTEXT_FILE)
    // Clear JIT_CONTEXT_FILE if not explicitly set, to prevent leaking from the
    // parent environment (e.g. when tests run inside a gate checker).
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

    // Save PID before waiting; needed to kill the process group on timeout.
    #[cfg(unix)]
    let child_pid = child.id();

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

    // Wait with timeout
    let wait_result = wait_with_timeout(&mut child, timeout)?;

    // On timeout, kill the entire process group so that any grandchildren still
    // holding pipe ends (e.g. test binaries that outlived the killed `cargo`
    // process) release them and the reader threads above can reach EOF.
    #[cfg(unix)]
    if wait_result.timed_out {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid;
        // A negative PID in kill(2) targets the process group with that PGID.
        // PGID == child_pid because we called process_group(0) above.
        let _ = kill(Pid::from_raw(-(child_pid as i32)), Signal::SIGKILL);
    }

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

/// Wait for a child process with timeout
struct WaitResult {
    timed_out: bool,
    exit_code: Option<i32>,
}

fn wait_with_timeout(child: &mut std::process::Child, timeout: Duration) -> Result<WaitResult> {
    let start = Instant::now();

    loop {
        match child.try_wait()? {
            Some(status) => {
                return Ok(WaitResult {
                    timed_out: false,
                    exit_code: status.code(),
                })
            }
            None => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(WaitResult {
                        timed_out: true,
                        exit_code: None,
                    });
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Git context information
struct GitContext {
    commit: Option<String>,
    branch: Option<String>,
}

/// Get git context, gracefully degrading if not in a git repo
fn get_git_context(working_dir: &Path) -> GitContext {
    let commit = get_git_commit(working_dir);
    let branch = get_git_branch(working_dir);

    GitContext { commit, branch }
}

fn get_git_commit(working_dir: &Path) -> Option<String> {
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

    #[test]
    fn test_git_context_graceful_degradation() {
        // Test in a non-git directory
        let temp_dir = std::env::temp_dir();
        let context = get_git_context(&temp_dir);

        // Should not panic, just return None values
        assert!(context.commit.is_none() || !context.commit.unwrap().is_empty());
        assert!(context.branch.is_none() || !context.branch.unwrap().is_empty());
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
            status: RS::Failed,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(100),
            exit_code: Some(1),
            stdout: "Previous review feedback".to_string(),
            stderr: String::new(),
            command: "review-checker".to_string(),
            by: Some("auto:executor".to_string()),
            message: None,
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
