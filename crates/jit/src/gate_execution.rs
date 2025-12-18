//! Gate execution engine for automated quality checks
//!
//! This module implements the execution of automated gates, including:
//! - Command execution with timeouts
//! - Output capture (stdout/stderr)
//! - Git commit/branch tracking
//! - Result storage for audit trail

use crate::domain::{GateChecker, GateRunResult, GateRunStatus, GateStage};
use anyhow::{Context, Result};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Execute a gate checker and return the result
///
/// This function runs the specified checker and captures all execution details
/// including exit code, output, timing, and git context if available.
pub fn execute_gate_checker(
    gate_key: &str,
    issue_id: &str,
    stage: GateStage,
    checker: &GateChecker,
    working_dir: &Path,
) -> Result<GateRunResult> {
    let start_time = Instant::now();
    let started_at = chrono::Utc::now();

    // Get git context (gracefully degrade if not in repo)
    let git_context = get_git_context(working_dir);

    // Execute the command based on checker type
    let execution_result = match checker {
        GateChecker::Exec {
            command,
            timeout_seconds,
            env,
            ..
        } => execute_command(command, *timeout_seconds, env, working_dir)?,
    };

    let duration = start_time.elapsed();
    let completed_at = chrono::Utc::now();

    // Determine status from exit code
    let status = match execution_result.exit_code {
        Some(0) => GateRunStatus::Passed,
        Some(_) => GateRunStatus::Failed,
        None => GateRunStatus::Error, // Timeout or signal
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
    working_dir: &Path,
) -> Result<CommandExecutionResult> {
    let timeout = Duration::from_secs(timeout_seconds);

    #[cfg(unix)]
    let mut cmd = {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
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

    // Add environment variables
    for (key, value) in env {
        cmd.env(key, value);
    }

    // Spawn the process
    let mut child = cmd.spawn().context("Failed to spawn command")?;

    // Wait with timeout
    let wait_result = wait_with_timeout(&mut child, timeout)?;

    let output = child.wait_with_output()?;

    Ok(CommandExecutionResult {
        exit_code: if wait_result.timed_out {
            None
        } else {
            output.status.code()
        },
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        command: command.to_string(),
    })
}

/// Wait for a child process with timeout
struct WaitResult {
    timed_out: bool,
}

fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Result<WaitResult> {
    let start = Instant::now();

    loop {
        match child.try_wait()? {
            Some(_status) => return Ok(WaitResult { timed_out: false }),
            None => {
                if start.elapsed() >= timeout {
                    // Kill the process on timeout
                    let _ = child.kill();
                    let _ = child.wait();
                    return Ok(WaitResult { timed_out: true });
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
}
