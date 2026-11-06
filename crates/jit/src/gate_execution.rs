//! Gate execution engine for automated quality checks
//!
//! This module implements the execution of automated gates, including:
//! - Command execution with timeouts
//! - Output capture (stdout/stderr)
//! - Git commit/branch tracking
//! - Result storage for audit trail

use crate::declarations::{GateChecker, GateStage};
use crate::domain::identity::{GateKey, GateRunId, IssueId};
use crate::domain::relative_path::RootRelativePath;
use crate::domain::repository_inputs::{InputsDigest, InputsDigestBuilder, RepositoryInputs};
use crate::domain::{
    Assignee, DocumentReference, GateContext, GateRunResult, GateRunStatus, GATE_RUN_SCHEMA_VERSION,
};
use crate::storage::git::Head;
use anyhow::{Context, Result};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

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
/// `contrib/gates/ai-review.sh` — can inline linked design/plan docs into its
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

/// What a gate run records itself as being about.
#[derive(Debug, Clone, Copy)]
pub struct GateRunSubject<'a> {
    /// The run id the mutation publishing this run pre-derived.
    pub run_id: &'a GateRunId,
    pub gate_key: &'a GateKey,
    pub issue_id: &'a IssueId,
    pub stage: GateStage,
}

/// Execute a gate checker with optional structured context.
///
/// This function runs the specified checker and captures all execution details
/// including exit code, output, timing, and the supplied worktree head.
///
/// Basic env vars (`JIT_ISSUE_ID`, `JIT_GATE_KEY`, `JIT_STAGE`, `JIT_ISSUE_DOCS`)
/// are always set. `JIT_ISSUE_DOCS` is built from `documents` via
/// [`build_issue_docs_env`]. When `context` is `Some`, a temporary JSON file is
/// written containing the structured context and made available via the
/// `JIT_CONTEXT_FILE` env var.
pub fn execute_gate_checker_with_context(
    GateRunSubject {
        run_id,
        gate_key,
        issue_id,
        stage,
    }: GateRunSubject<'_>,
    checker: &GateChecker,
    working_dir: &Path,
    head: Option<&Head>,
    context: Option<&GateContext>,
    documents: &[DocumentReference],
) -> Result<GateRunResult> {
    let start_time = Instant::now();
    let started_at = chrono::Utc::now();

    let commit = head.map(|head| head.commit().as_str().to_string());
    let branch = head.map(|head| head.branch_label().to_string());

    // Build base env vars that are always set
    let mut base_env = std::collections::HashMap::new();
    base_env.insert("JIT_ISSUE_ID".to_string(), issue_id.to_string());
    base_env.insert("JIT_GATE_KEY".to_string(), gate_key.to_string());
    base_env.insert("JIT_STAGE".to_string(), stage.as_str().to_string());
    base_env.insert(
        "JIT_ISSUE_DOCS".to_string(),
        build_issue_docs_env(documents),
    );
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
        _ => anyhow::bail!("built-in gate checkers must be executed through GateOps::check_gate"),
    };

    let duration = start_time.elapsed();
    let completed_at = chrono::Utc::now();

    // Parse the checker's machine-readable findings block (if any) once, at
    // record time, so every downstream view reads structured data instead of
    // re-grepping stdout. Absent or malformed blocks degrade to `None`.
    let findings = crate::domain::gate_findings::parse_gate_findings(&execution_result.stdout);

    let status = match execution_result.exit_code {
        Some(0) => GateRunStatus::Passed,
        // Shell could not execute the command: 127 = command not found,
        // 126 = found but not executable. Runner/infra error, not a checker verdict.
        Some(126) | Some(127) => GateRunStatus::Error,
        // The checker reports it could not judge (EX_TEMPFAIL).
        Some(CHECKER_TEMPFAIL_EXIT) => GateRunStatus::Error,
        Some(_) => GateRunStatus::Failed,
        None => GateRunStatus::Error, // killed by signal / timeout
    };

    Ok(GateRunResult {
        schema_version: GATE_RUN_SCHEMA_VERSION,
        run_id: run_id.clone(),
        gate_key: gate_key.clone(),
        stage,
        issue_id: issue_id.clone(),
        commit,
        branch,
        status,
        started_at,
        completed_at: Some(completed_at),
        duration_ms: Some(duration.as_millis() as u64),
        exit_code: execution_result.exit_code,
        stdout: execution_result.stdout,
        stderr: execution_result.stderr,
        command: execution_result.command,
        by: Some(Assignee::AUTO_EXECUTOR),
        message: None,
        findings,
        // Stamped by the evaluator, which holds the gate declaration the
        // digest is taken over.
        inputs_digest: None,
        origin: crate::domain::GateVerdictOrigin::Executed,
        evaluation_stability: crate::domain::GateEvaluationStability::Stable,
    })
}

/// Exit status (`EX_TEMPFAIL`) by which an exec checker reports an
/// infrastructure fault instead of a verdict on the repository.
pub(crate) const CHECKER_TEMPFAIL_EXIT: i32 = 75;

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
    match crate::process::process_group_target(child.id()) {
        Some(target) => {
            let _ = kill(Pid::from_raw(target), Signal::SIGKILL);
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

/// Digest the working-tree content of every repository file `inputs` covers.
///
/// The walk is over the *working tree*, not a commit: a source file edited but
/// not committed, and one created but never `git add`ed, both change the digest,
/// because both change what a checker compiles or reads.
///
/// **A file present beneath a declared root is digested unless a declared
/// pattern excludes it.** The repository's ignore rules are deliberately not
/// consulted: they describe what a project does not version, which is a
/// different question from what a checker reads, and material a checker really
/// does read is routinely ignored — a `node_modules` tree decides what a test
/// suite runs against. Letting the ignore rules drop such a path would leave the
/// digest unmoved while the checker's inputs changed, which is the one failure
/// this mechanism must not have. Generated material a checker does not read is
/// therefore excluded by saying so in the declaration
/// ([`RepositoryInputs::covers`]), where a reader can see it.
///
/// Two things are skipped structurally. A `.git` entry is the repository's own
/// object store: where content is versioned, never content a checker reads, and
/// it changes with every commit. `tracker_written` names the worktree-relative
/// paths jit writes during a mutation
/// ([`RepositoryLayout::tracker_written_paths`](crate::repository_state::RepositoryLayout::tracker_written_paths)),
/// so recording a gate result never moves the digest it was recorded under,
/// while configuration beside those paths stays an input. A checker's own build
/// output is likewise not an input — it is a function of the inputs, produced
/// after this digest is taken — so a declaration names neither.
///
/// A symlink contributes its link target text rather than the bytes it points
/// at, so the digest describes the repository's own content, a link out of the
/// worktree neither escapes it nor fails it, and the walk cannot cycle. A
/// declared root absent from the working tree contributes nothing, which is
/// itself a change of digest.
///
/// # Errors
///
/// Returns an error when a covered file exists but cannot be read: a digest
/// standing for content nobody could read would license reuse over an input set
/// that was never examined.
pub(crate) fn digest_declared_inputs(
    worktree_root: &Path,
    inputs: &RepositoryInputs,
    tracker_written: &[RootRelativePath],
    declaration: &[u8],
) -> Result<InputsDigest> {
    let walk = InputWalk {
        inputs,
        tracker_written,
    };
    let mut paths = walk.declared_input_paths(worktree_root)?;
    paths.sort_unstable();
    paths.dedup();

    paths
        .iter()
        .try_fold(
            InputsDigestBuilder::new(declaration),
            |mut builder, path| {
                let absolute = worktree_root.join(path);
                let contents = match std::fs::symlink_metadata(&absolute) {
                    Ok(metadata) if metadata.is_symlink() => std::fs::read_link(&absolute)
                        .with_context(|| format!("Failed to read declared input link {path}"))?
                        .as_os_str()
                        .as_encoded_bytes()
                        .to_vec(),
                    Ok(_) => std::fs::read(&absolute)
                        .with_context(|| format!("Failed to read declared input {path}"))?,
                    // Walked a moment ago and gone now: contributes nothing.
                    Err(_) => return Ok(builder),
                };
                builder.push_file(path, &contents);
                Ok(builder)
            },
        )
        .map(InputsDigestBuilder::finish)
}

/// One declaration's walk, with the paths it skips structurally.
struct InputWalk<'a> {
    inputs: &'a RepositoryInputs,
    tracker_written: &'a [RootRelativePath],
}

impl InputWalk<'_> {
    /// Every repository-relative path the declaration covers, walking only the
    /// declared roots.
    ///
    /// Walking the roots rather than the whole worktree is what keeps a narrowly
    /// scoped gate cheap: a declaration naming one directory never stats a file
    /// outside it.
    fn declared_input_paths(&self, worktree_root: &Path) -> Result<Vec<String>> {
        self.inputs
            .roots()
            .iter()
            .try_fold(Vec::new(), |mut paths, root| {
                self.collect_declared_root(worktree_root, root.worktree_relative(), &mut paths)?;
                Ok(paths)
            })
    }

    /// Collect the covered paths at or beneath one declared root, which `root`
    /// names worktree-relative (empty for the worktree itself).
    fn collect_declared_root(
        &self,
        worktree_root: &Path,
        root: &str,
        paths: &mut Vec<String>,
    ) -> Result<()> {
        if self.is_skipped(root) {
            return Ok(());
        }
        let absolute = worktree_root.join(root);
        let Ok(metadata) = std::fs::symlink_metadata(&absolute) else {
            // A declared root that is not present contributes nothing.
            return Ok(());
        };
        if metadata.is_dir() {
            return self.collect_walked_paths(&absolute, root, paths);
        }
        if self.inputs.covers(root) {
            paths.push(root.to_string());
        }
        Ok(())
    }

    /// Recursively collect every covered path beneath `directory`, which the
    /// repository-relative `prefix` names.
    ///
    /// Descent stops at a directory the declaration excludes whole
    /// ([`RepositoryInputs::excludes_directory`]), so taking a large generated
    /// tree out of a gate's inputs costs one declared pattern and no traversal.
    /// A directory symlink is not followed: it is recorded as the link it is.
    fn collect_walked_paths(
        &self,
        directory: &Path,
        prefix: &str,
        paths: &mut Vec<String>,
    ) -> Result<()> {
        let entries = std::fs::read_dir(directory).with_context(|| {
            format!("Failed to walk declared inputs at {}", directory.display())
        })?;
        for entry in entries {
            let entry = entry.with_context(|| {
                format!("Failed to walk declared inputs at {}", directory.display())
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() {
                name
            } else {
                format!("{prefix}/{name}")
            };
            if self.is_skipped(&path) {
                continue;
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                if !self.inputs.excludes_directory(&path) {
                    self.collect_walked_paths(&entry.path(), &path, paths)?;
                }
            } else if self.inputs.covers(&path) {
                paths.push(path);
            }
        }
        Ok(())
    }

    /// Whether the walk skips `repository_path` whatever the declaration says:
    /// it is git plumbing or a path jit writes during a mutation.
    fn is_skipped(&self, repository_path: &str) -> bool {
        is_git_plumbing(repository_path)
            || self
                .tracker_written
                .iter()
                .any(|written| written.contains(repository_path))
    }
}

/// Whether `repository_path` names the repository's own object store, in either
/// spelling: a directory in an ordinary checkout, a file in a linked worktree.
///
/// This is where content is versioned, never content a checker reads, and it
/// moves with every commit — so the walk skips it without a declared
/// exclusion, at a declared root as much as inside one.
fn is_git_plumbing(repository_path: &str) -> bool {
    repository_path.split('/').any(|segment| segment == ".git")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{
        RepositoryLayout, RepositoryRootEvidence, VirtualPath, BOOTSTRAP_LOCK_FILE,
        EVENTS_LOCK_FILE, INDEX_LOCK_FILE, RECORD_SCRATCH_DIR, REPO_WRITE_LOCK_FILE,
    };
    use crate::test_utils::{document_reference, document_reference_typed};
    use std::collections::HashMap;

    /// Execute `checker` as the run of a fixed test id.
    fn execute_fixture_checker(
        gate_key: &GateKey,
        issue_id: &IssueId,
        stage: GateStage,
        checker: &GateChecker,
        working_dir: &Path,
        context: Option<&GateContext>,
        documents: &[DocumentReference],
    ) -> Result<GateRunResult> {
        execute_gate_checker_with_context(
            GateRunSubject {
                run_id: &crate::test_utils::gate_run_id(1),
                gate_key,
                issue_id,
                stage,
            },
            checker,
            working_dir,
            None,
            context,
            documents,
        )
    }

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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
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
    fn test_execute_gate_checker_in_memory_evidence_ignores_explicit_git_working_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path());
        repository.write("seed.txt", "seed\n").commit("seed");
        let in_memory = crate::commands::test_helpers::setup();
        let head = in_memory
            .require_worktree_paths()
            .unwrap()
            .git()
            .map(|git| git.head())
            .transpose()
            .unwrap()
            .flatten();
        assert!(head.is_none());
        let checker = GateChecker::Exec {
            command: "true".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };

        let result = execute_gate_checker_with_context(
            GateRunSubject {
                run_id: &crate::test_utils::gate_run_id(1),
                gate_key: &"g".parse().unwrap(),
                issue_id: &crate::test_utils::issue_id(0xc4e9_59a0_b8a4_228d_7550_22a2_872f_f725),
                stage: GateStage::Postcheck,
            },
            &checker,
            repository.root(),
            head.as_ref(),
            None,
            &[],
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.commit, None);
        assert_eq!(result.branch, None);
    }

    /// A worktree with `src/` and `docs/` beneath a git repository, where
    /// `src/generated/` is ignored.
    fn worktree_with_declared_inputs() -> tempfile::TempDir {
        let temp = tempfile::TempDir::new().expect("a temp worktree");
        let root = temp.path();
        std::fs::create_dir_all(root.join("src/generated")).expect("create src");
        std::fs::create_dir_all(root.join("docs")).expect("create docs");
        std::fs::write(root.join(".gitignore"), "src/generated/\n").expect("write ignore");
        std::fs::write(root.join("src/lib.rs"), "fn main() {}\n").expect("write source");
        std::fs::write(root.join("docs/guide.md"), "guide\n").expect("write doc");
        std::fs::write(root.join("seed.txt"), "seed\n").expect("write seed");
        crate::test_harness::git::GitRepository::init(root).commit("seed");
        temp
    }

    fn digest_of(root: &Path, roots: &[&str], exclude: &[&str]) -> InputsDigest {
        let inputs = RepositoryInputs::parse(roots, exclude).expect("a declarable input set");
        digest_declared_inputs(root, &inputs, &[], b"checker").expect("the declared inputs digest")
    }

    /// REQ-02: the digest is over the working tree, so a source file the
    /// repository does not track still changes it — an uncommitted `.rs` file
    /// changes what a compiler reads.
    #[test]
    fn test_digest_declared_inputs_covers_a_file_the_repository_does_not_track() {
        let temp = worktree_with_declared_inputs();
        let committed = digest_of(temp.path(), &["src"], &[]);

        std::fs::write(temp.path().join("src/extra.rs"), "fn extra() {}\n").expect("write");

        assert_ne!(
            committed,
            digest_of(temp.path(), &["src"], &[]),
            "an untracked source file must change the digest"
        );
    }

    /// A file a checker reads is in the digest even where the repository's
    /// ignore rules exclude it. What a project declines to version is a
    /// different question from what a checker reads, and a checker routinely
    /// reads ignored material — a `node_modules` tree decides what a test suite
    /// runs against. Dropping it here would leave the digest still while the
    /// checker's inputs moved.
    #[test]
    fn test_digest_declared_inputs_covers_a_file_the_ignore_rules_exclude() {
        let temp = worktree_with_declared_inputs();
        let baseline = digest_of(temp.path(), &["src"], &[]);

        std::fs::write(temp.path().join("src/generated/out.rs"), "generated\n").expect("write");

        assert_ne!(
            baseline,
            digest_of(temp.path(), &["src"], &[]),
            "an ignored file a declared root covers must still change the digest"
        );
    }

    /// Generated material leaves the digest because the declaration says so.
    /// The excluded directory is skipped whole, so taking a large generated
    /// tree out of a gate's inputs costs one pattern and no traversal.
    #[test]
    fn test_digest_declared_inputs_omits_a_directory_a_declared_exclusion_names() {
        let temp = worktree_with_declared_inputs();
        let declared: &[&str] = &["src/generated"];
        let baseline = digest_of(temp.path(), &["src"], declared);

        std::fs::write(temp.path().join("src/generated/out.rs"), "generated\n").expect("write");
        assert_eq!(
            baseline,
            digest_of(temp.path(), &["src"], declared),
            "a declared exclusion must keep the build artefact out"
        );

        // The exclusion reaches the whole subtree, not just its top level.
        std::fs::create_dir_all(temp.path().join("src/generated/deep")).expect("create");
        std::fs::write(temp.path().join("src/generated/deep/more.rs"), "deeper\n").expect("write");
        assert_eq!(
            baseline,
            digest_of(temp.path(), &["src"], declared),
            "naming a directory must exclude everything beneath it"
        );

        // And it removes only what it names.
        std::fs::write(temp.path().join("src/lib.rs"), "fn main() { }\n").expect("write");
        assert_ne!(
            baseline,
            digest_of(temp.path(), &["src"], declared),
            "an exclusion must not remove its siblings"
        );
    }

    /// REQ-06/REQ-07: the digest tracks content beneath the declared roots and
    /// nothing else — an edit inside changes it, an edit outside does not.
    #[test]
    fn test_digest_declared_inputs_tracks_only_content_beneath_the_declared_roots() {
        let temp = worktree_with_declared_inputs();
        let baseline = digest_of(temp.path(), &["src"], &[]);

        std::fs::write(temp.path().join("docs/guide.md"), "rewritten\n").expect("write");
        assert_eq!(
            baseline,
            digest_of(temp.path(), &["src"], &[]),
            "a change no declared root covers must leave the digest alone"
        );

        std::fs::write(temp.path().join("src/lib.rs"), "fn main() { }\n").expect("write");
        assert_ne!(
            baseline,
            digest_of(temp.path(), &["src"], &[]),
            "a change beneath a declared root must change the digest"
        );
    }

    /// REQ-02: a declared exclusion removes a path from the set even though a
    /// root covers it.
    #[test]
    fn test_digest_declared_inputs_drops_paths_a_declared_exclusion_matches() {
        let temp = worktree_with_declared_inputs();
        let excluded = digest_of(temp.path(), &["src", "docs"], &["docs/**"]);

        std::fs::write(temp.path().join("docs/guide.md"), "rewritten\n").expect("write");

        assert_eq!(
            excluded,
            digest_of(temp.path(), &["src", "docs"], &["docs/**"]),
            "an excluded path must not contribute to the digest"
        );
    }

    /// The digest is over content: a rewrite of identical bytes, which moves
    /// the modification time, digests to the same value.
    #[test]
    fn test_digest_declared_inputs_is_unchanged_by_a_rewrite_of_identical_bytes() {
        let temp = worktree_with_declared_inputs();
        let baseline = digest_of(temp.path(), &["src"], &[]);

        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(temp.path().join("src/lib.rs"), "fn main() {}\n").expect("rewrite");

        assert_eq!(baseline, digest_of(temp.path(), &["src"], &[]));
    }

    /// `@/charter/D-4`: the enumeration is a filesystem walk, so a worktree
    /// with no git repository digests by exactly the same rule as one with.
    #[test]
    fn test_digest_declared_inputs_walks_a_worktree_without_git() {
        let temp = tempfile::TempDir::new().expect("a temp worktree");
        std::fs::create_dir_all(temp.path().join("src/nested")).expect("create");
        std::fs::write(temp.path().join("src/lib.rs"), "one\n").expect("write");
        let baseline = digest_of(temp.path(), &["src"], &[]);

        std::fs::write(temp.path().join("src/nested/deep.rs"), "two\n").expect("write");

        assert_ne!(
            baseline,
            digest_of(temp.path(), &["src"], &[]),
            "a nested file must be walked where git cannot list it"
        );
    }

    /// The repository's own object store is never repository content: it is
    /// where content is versioned, and it moves with every commit.
    #[test]
    fn test_digest_declared_inputs_never_digests_the_git_directory() {
        let temp = worktree_with_declared_inputs();
        let baseline = digest_of(temp.path(), &["src", ".git"], &[]);

        std::fs::write(temp.path().join(".git/description"), "rewritten\n").expect("write");

        assert_eq!(
            baseline,
            digest_of(temp.path(), &["src", ".git"], &[]),
            "git plumbing must stay out even where a root names it"
        );
    }

    /// A declared root may name one file, and its absence is itself a state the
    /// digest distinguishes.
    #[test]
    fn test_digest_declared_inputs_covers_a_file_root_and_notes_its_absence() {
        let temp = worktree_with_declared_inputs();
        let present = digest_of(temp.path(), &["docs/guide.md"], &[]);

        std::fs::write(temp.path().join("docs/guide.md"), "rewritten\n").expect("write");
        let rewritten = digest_of(temp.path(), &["docs/guide.md"], &[]);
        assert_ne!(present, rewritten, "a file root's content is digested");

        std::fs::remove_file(temp.path().join("docs/guide.md")).expect("remove");
        let absent = digest_of(temp.path(), &["docs/guide.md"], &[]);
        assert_ne!(rewritten, absent, "an absent root contributes nothing");
    }

    /// The worktree root `.` digests every file beneath the worktree that no
    /// declared pattern excludes, top-level files included.
    #[test]
    fn test_digest_declared_inputs_worktree_root_covers_every_unexcluded_file() {
        let temp = worktree_with_declared_inputs();
        let declared: &[&str] = &["docs/**"];
        let baseline = digest_of(temp.path(), &["."], declared);

        std::fs::write(temp.path().join("docs/guide.md"), "rewritten\n").expect("write");
        assert_eq!(
            baseline,
            digest_of(temp.path(), &["."], declared),
            "a declared exclusion still applies under the worktree root"
        );

        std::fs::write(temp.path().join("NEW.md"), "new\n").expect("write");
        let added = digest_of(temp.path(), &["."], declared);
        assert_ne!(
            baseline, added,
            "a new top-level file must change the digest"
        );

        std::fs::write(temp.path().join("src/lib.rs"), "fn main() { }\n").expect("write");
        assert_ne!(
            added,
            digest_of(temp.path(), &["."], declared),
            "a nested edit must change the digest"
        );
    }

    /// A worktree whose data root sits at `data_offset`, holding configuration,
    /// with its layout.
    fn worktree_with_data_root(data_offset: &str) -> (tempfile::TempDir, RepositoryLayout) {
        let temp = tempfile::TempDir::new().expect("a temp worktree");
        let data = temp.path().join(data_offset);
        std::fs::create_dir_all(data.join("schemas")).expect("create data root");
        std::fs::create_dir_all(temp.path().join("src")).expect("create src");
        std::fs::write(temp.path().join("src/lib.rs"), "fn main() {}\n").expect("write");
        for configuration in [VirtualPath::CONFIG, VirtualPath::GATES, VirtualPath::RULES] {
            std::fs::write(data.join(configuration.relative().as_path()), "# seed\n")
                .expect("write configuration");
        }
        let evidence =
            |path: &Path| RepositoryRootEvidence::new(path, format!("id:{}", path.display()), true);
        let layout =
            RepositoryLayout::new(evidence(temp.path()), evidence(&data)).expect("a nested layout");
        (temp, layout)
    }

    fn layout_digest(layout: &RepositoryLayout, roots: &[&str]) -> InputsDigest {
        let inputs = RepositoryInputs::parse(roots, &[]).expect("a declarable input set");
        digest_declared_inputs(
            layout.worktree_root(),
            &inputs,
            &layout.tracker_written_paths(),
            b"checker",
        )
        .expect("the declared inputs digest")
    }

    /// Append to every path jit writes during a mutation, each named through
    /// the storage layout.
    fn write_tracker_paths(layout: &RepositoryLayout) {
        let records = [
            VirtualPath::EVENTS,
            VirtualPath::gate_run_record(
                &crate::test_utils::issue_id(1),
                &crate::test_utils::gate_run_id(1),
            ),
            VirtualPath::archive_evidence_record(
                &crate::domain::artifact_plan::PlanTarget::Document {
                    path: "doc.md".into(),
                },
            ),
            VirtualPath::INDEX,
        ];
        let written = records
            .iter()
            .map(|path| layout.resolve(path).expect("a resolvable record"))
            .chain(
                [REPO_WRITE_LOCK_FILE, INDEX_LOCK_FILE, EVENTS_LOCK_FILE]
                    .map(|file| layout.data_root().join(file)),
            )
            .chain(std::iter::once(
                layout
                    .resolve(&VirtualPath::WORKTREE_IDENTITY)
                    .expect("the worktree identity path"),
            ))
            .chain([
                layout
                    .resolve(&VirtualPath::ISSUES)
                    .expect("the issue directory")
                    .join("0f0e0d0c-0000-4000-8000-000000000000.json"),
                layout
                    .data_root()
                    .join(RECORD_SCRATCH_DIR)
                    .join(crate::storage::RECORD_FILE_NAME),
                layout.worktree_root().join(BOOTSTRAP_LOCK_FILE),
            ]);
        for path in written {
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("create parent");
            let previous = std::fs::read(&path).unwrap_or_default();
            std::fs::write(&path, [previous.as_slice(), b"written\n"].concat())
                .expect("write tracker path");
        }
    }

    /// Tracker writes never move a digest, wherever the data root sits and
    /// whether a root covers the whole worktree or the data root alone.
    #[test]
    fn test_digest_declared_inputs_ignores_every_tracker_written_path() {
        for data_offset in [".jit", "state/data"] {
            let (_temp, layout) = worktree_with_data_root(data_offset);
            let roots: [&[&str]; 2] = [&["."], &[data_offset]];
            let baselines = roots.map(|roots| layout_digest(&layout, roots));

            write_tracker_paths(&layout);
            write_tracker_paths(&layout);

            assert_eq!(
                baselines,
                roots.map(|roots| layout_digest(&layout, roots)),
                "tracker writes under {data_offset} must leave every digest unchanged"
            );
        }
    }

    /// The data root's configuration stays an input: a checker reading it must
    /// see its edits.
    #[test]
    fn test_digest_declared_inputs_covers_configuration_under_the_data_root() {
        let (_temp, layout) = worktree_with_data_root(".jit");
        for configuration in [VirtualPath::CONFIG, VirtualPath::GATES, VirtualPath::RULES] {
            let baseline = layout_digest(&layout, &["."]);
            let path = layout
                .resolve(&configuration)
                .expect("a configuration path");
            std::fs::write(&path, "# edited\n").expect("edit configuration");
            assert_ne!(
                baseline,
                layout_digest(&layout, &["."]),
                "editing {} must change the digest",
                path.display()
            );
        }
        let baseline = layout_digest(&layout, &[".jit"]);
        std::fs::write(layout.data_root().join("schemas/rule.json"), "{}").expect("add schema");
        assert_ne!(baseline, layout_digest(&layout, &[".jit"]));
    }

    /// A relocated data root moves the exclusion with it: the conventional
    /// spelling is then ordinary worktree content.
    #[test]
    fn test_digest_declared_inputs_hard_codes_no_data_root_spelling() {
        let (temp, layout) = worktree_with_data_root("state/data");
        let baseline = layout_digest(&layout, &["."]);

        std::fs::create_dir_all(temp.path().join(".jit")).expect("create");
        std::fs::write(
            temp.path()
                .join(".jit")
                .join(VirtualPath::INDEX.relative().as_path()),
            "{}",
        )
        .expect("write");

        assert_ne!(baseline, layout_digest(&layout, &["."]));
    }

    #[test]
    fn test_execute_gate_checker_records_head_commit() {
        let temp = tempfile::TempDir::new().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(temp.path());
        let expected_commit = repository.write("seed.txt", "seed\n").commit("seed");
        let head = crate::storage::WorktreePaths::detect_from(repository.root())
            .unwrap()
            .git()
            .unwrap()
            .head()
            .unwrap()
            .unwrap();
        let checker_dir = tempfile::TempDir::new().unwrap();
        let checker = GateChecker::Exec {
            command: "true".to_string(),
            timeout_seconds: 10,
            working_dir: None,
            env: HashMap::new(),
            pass_context: false,
            prompt: None,
            prompt_file: None,
        };
        let result = execute_gate_checker_with_context(
            GateRunSubject {
                run_id: &crate::test_utils::gate_run_id(1),
                gate_key: &"g".parse().unwrap(),
                issue_id: &crate::test_utils::issue_id(0xc4e9_59a0_b8a4_228d_7550_22a2_872f_f725),
                stage: GateStage::Postcheck,
            },
            &checker,
            checker_dir.path(),
            Some(&head),
            None,
            &[],
        )
        .unwrap();

        assert_eq!(result.commit.as_deref(), Some(expected_commit.as_str()));
        assert_eq!(result.branch.as_deref(), Some(head.branch_label()));
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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
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
        let result = execute_fixture_checker(
            &"my-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xdd2d_59e0_9bc4_ccb0_00c2_4a77_9225_c066),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert!(result.stdout.contains(&format!("ID={}", result.issue_id)));
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
                label: Some("Implementation Plan".to_string()),
                ..document_reference_typed("dev/active/plan.md", "design")
            },
            // A doc with no label/doc_type still serializes, as explicit nulls.
            document_reference("NOTES.md"),
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
            label: Some("Plan".to_string()),
            ..document_reference_typed("dev/active/my-plan.md", "design")
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
        let result = execute_fixture_checker(
            &"my-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xdd2d_59e0_9bc4_ccb0_00c2_4a77_9225_c066),
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
        let result = execute_fixture_checker(
            &"my-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xdd2d_59e0_9bc4_ccb0_00c2_4a77_9225_c066),
            GateStage::Postcheck,
            &checker,
            &temp_dir,
            None,
            &[],
        )
        .unwrap();

        assert_eq!(result.status, GateRunStatus::Passed);
        assert_eq!(result.stdout.trim(), "[]");
    }

    #[test]
    fn test_context_file_written_when_context_provided() {
        use crate::domain::GateContext;

        let issue_id = crate::test_utils::issue_id(0xdd2d_59e0_9bc4_ccb0_00c2_4a77_9225_c066);

        let context = GateContext {
            schema_version: 1,
            prompt: Some("Review the code".to_string()),
            issue: serde_json::json!({"id": issue_id, "title": "Test issue"}),
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
        let result = execute_fixture_checker(
            &"review".parse().unwrap(),
            &issue_id,
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
        assert_eq!(parsed["issue"]["id"], serde_json::json!(issue_id));
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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
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
        let result = execute_fixture_checker(
            &"test-gate".parse().unwrap(),
            &crate::test_utils::issue_id(0xfb9a_5d7a_9def_7971_ebc7_0df1_a8bb_16a7),
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

        let issue_id = crate::test_utils::issue_id(0xdd2d_59e0_9bc4_ccb0_00c2_4a77_9225_c066);

        let previous_run = GateRunResult {
            schema_version: 1,
            run_id: crate::test_utils::gate_run_id(2),
            gate_key: "review".parse().unwrap(),
            stage: GateStage::Postcheck,
            issue_id: issue_id.clone(),
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
            by: Some(Assignee::AUTO_EXECUTOR),
            message: None,
            findings: None,
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
            evaluation_stability: crate::domain::GateEvaluationStability::Stable,
        };

        let context = GateContext {
            schema_version: 1,
            prompt: Some("Review again".to_string()),
            issue: serde_json::json!({"id": issue_id}),
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
        let result = execute_fixture_checker(
            &"review".parse().unwrap(),
            &issue_id,
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
        assert_eq!(
            history[0]["run_id"],
            crate::test_utils::gate_run_id(2).as_str()
        );
        assert_eq!(history[0]["status"], "failed");
        assert_eq!(history[0]["stdout"], "Previous review feedback");
    }
}
