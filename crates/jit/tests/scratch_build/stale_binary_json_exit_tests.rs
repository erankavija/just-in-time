//! REQ-01/02 JSON-mode regression for jit:7446af34.
//!
//! The stale-binary refusal (`crate::errors::StaleBinaryError`) must exit `10`
//! and carry a structured, `STALE_BINARY`-coded envelope under `--json`,
//! exactly matching the exit code the non-`--json` path already produced.
//! Before that fix, `render_gate_pass_error` (main.rs) had no branch for
//! `StaleBinaryError`, so it fell through to the generic `GATE_ERROR` code,
//! whose legacy constructor explicitly preserves exit `1` — silently breaking
//! the documented exit-10 contract under `--json` only.
//!
//! Reproducing a genuine EVALUATOR-path refusal needs a binary whose own build
//! commit is a commit the repository under review knows but has advanced past.
//! Since build provenance no longer tracks ambient git (jit:5d862134), an
//! ordinary `cargo test` binary reports `git_commit == "unknown"` and could
//! never be objectively stale against any repository. This test therefore
//! builds a SECOND `jit` binary via `build.rs`'s `JIT_BUILD_GIT_*` override
//! (the intentional release-injection interface — a real, separately-compiled
//! binary, not a mock), told it was built from a real ancestor commit, and
//! runs THAT binary as the evaluator inside a scratch repo whose `HEAD` sits
//! one commit past that ancestor. The assertions — exit 10 in both modes and a
//! structured `STALE_BINARY` envelope — are unchanged; only the way a
//! genuinely-stale evaluator is obtained differs.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use super::stale_binary_child_process_tests::{
    ancestor_commit, build_stale_child_binary, workspace_root,
};

#[test]
fn test_stale_binary_error_code_resolves_to_external_error() {
    use jit::output::{ErrorCode, ExitCode};

    let code = "STALE_BINARY"
        .parse::<ErrorCode>()
        .expect("the emitted stale-binary code should be registered");

    assert_eq!(code, ErrorCode::StaleBinary);
    assert_eq!(code.exit_code(), ExitCode::ExternalError);
}

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// A `jit` invocation with the gate-context variables scrubbed. When this test
/// suite itself executes under a `cargo-ci` gate evaluation, the whole process
/// tree inherits `JIT_GATE_RUN=1` from the evaluator; left in place, it would
/// turn every spawned `jit` into a self-checking gate child (main's startup
/// precheck) and refuse before the EVALUATOR-path (`check_gate`) behavior under
/// test is ever reached.
fn scrub_gate_context(cmd: &mut Command) {
    cmd.env_remove("JIT_GATE_RUN")
        .env_remove("JIT_ISSUE_ID")
        .env_remove("JIT_GATE_KEY");
}

/// Build a scratch git repository whose `HEAD` is one commit past `ancestor`:
/// `ancestor` is a known commit here (fetched from the real workspace, with its
/// history), but no longer at `HEAD`. Entirely inside the disposable scratch
/// repo — no ref in the real workspace is read, moved, or written. Returns
/// `None` (skip) if any local git step fails.
fn scratch_repo_stale_for(workspace_root: &Path, ancestor: &str) -> Option<TempDir> {
    scratch_repo_advanced_with_change(
        workspace_root,
        ancestor,
        "crates/jit/src/main.rs",
        b"\n// build-input change for stale-binary coverage\n",
    )
}

/// Build the same scratch repository shape while changing only metadata that
/// cannot affect the compiled binary.
fn scratch_repo_metadata_only_for(workspace_root: &Path, ancestor: &str) -> Option<TempDir> {
    scratch_repo_advanced_with_change(
        workspace_root,
        ancestor,
        "docs/stale-binary-metadata.md",
        b"metadata-only change for stale-binary coverage\n",
    )
}

fn scratch_repo_advanced_with_change(
    workspace_root: &Path,
    ancestor: &str,
    changed_path: &str,
    change: &[u8],
) -> Option<TempDir> {
    let temp = TempDir::new().ok()?;
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(temp.path())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !run(&["init", "-q"]) {
        return None;
    }
    if !run(&["fetch", "-q", workspace_root.to_str()?, ancestor]) {
        return None;
    }
    if !run(&["checkout", "-q", "FETCH_HEAD"]) {
        return None;
    }
    let path = temp.path().join(changed_path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).ok()?;
    }
    std::fs::write(path, change).ok()?;
    if !run(&["add", changed_path]) {
        return None;
    }
    if !run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "-q",
        "-m",
        "advance past the build commit",
    ]) {
        return None;
    }
    // The checked-out ANCESTOR predates the repository's move to canonical
    // `.agents/skills` doc links (`.claude` was a user-local convenience symlink
    // briefly git-tracked by mistake). A session-backed `jit init` captures the
    // whole-repository closure, whose no-follow discipline must not traverse that
    // symlink, so scrub the stale repository data from the disposable scratch tree:
    // the `jit init` below then runs fresh and exercises the stale-binary refusal
    // without any `.claude` dependency (jit:49adf23b).
    let _ = Command::new("rm")
        .args(["-rf", ".jit", ".claude"])
        .current_dir(temp.path())
        .status();
    Some(temp)
}

/// `jit init` + one automated gate (key `g`, always-passing checker) + one
/// issue requiring it, all inside `repo_root`, using the real (fresh) test
/// binary with gate context scrubbed (its own build commit is `unknown`, so it
/// is never objectively stale). Returns the issue id.
fn setup_gated_issue(repo_root: &Path) -> String {
    let mut cmd = Command::new(jit_binary());
    scrub_gate_context(&mut cmd);
    let status = cmd.current_dir(repo_root).arg("init").status().unwrap();
    assert!(status.success());

    let mut cmd = Command::new(jit_binary());
    scrub_gate_context(&mut cmd);
    let status = cmd
        .current_dir(repo_root)
        .args([
            "gate",
            "define",
            "g",
            "--title",
            "G",
            "--description",
            "G",
            "--mode",
            "auto",
            "--checker-command",
            "echo ran",
        ])
        .status()
        .unwrap();
    assert!(status.success());

    let mut cmd = Command::new(jit_binary());
    scrub_gate_context(&mut cmd);
    let output = cmd
        .current_dir(repo_root)
        .args(["issue", "create", "--title", "Test", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let created: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = created["id"].as_str().unwrap().to_string();

    let mut cmd = Command::new(jit_binary());
    scrub_gate_context(&mut cmd);
    let status = cmd
        .current_dir(repo_root)
        .args(["gate", "add", &issue_id, "g"])
        .status()
        .unwrap();
    assert!(status.success());

    issue_id
}

/// REQ-01/02: text mode and `--json` mode agree on exit code 10 for the
/// EVALUATOR-path stale-binary refusal, and the JSON envelope is a structured,
/// `STALE_BINARY` error carrying the reinstall hint — not the generic fallback
/// `GATE_ERROR` (which would exit 1).
#[test]
fn test_gate_evaluate_stale_binary_exits_10_in_text_and_json_modes() {
    let workspace_root = workspace_root();
    let Some(ancestor) = ancestor_commit(&workspace_root) else {
        eprintln!("SKIP: workspace does not have 9+ commits to pick a safe ancestor from");
        return;
    };
    let Some(stale_binary) = build_stale_child_binary(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not build the stale binary (cargo unavailable?)");
        return;
    };
    let Some(scratch) = scratch_repo_stale_for(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not construct the scratch repo (git unavailable?)");
        return;
    };

    let issue_id = setup_gated_issue(scratch.path());

    // Text mode: the stale binary is the evaluator; its own build commit is
    // known in `scratch` but no longer at HEAD, so `check_gate` refuses.
    let mut cmd = Command::new(&stale_binary);
    scrub_gate_context(&mut cmd);
    let text_output = cmd
        .current_dir(scratch.path())
        .args(["gate", "evaluate", &issue_id, "g"])
        .output()
        .unwrap();
    assert_eq!(
        text_output.status.code(),
        Some(10),
        "text mode: stderr={}",
        String::from_utf8_lossy(&text_output.stderr)
    );
    let text_stderr = String::from_utf8_lossy(&text_output.stderr);
    assert!(
        text_stderr.contains("predates the tree under review"),
        "text mode stderr should explain the refusal: {text_stderr}"
    );

    // JSON mode: same exit code, plus a structured envelope.
    let mut cmd = Command::new(&stale_binary);
    scrub_gate_context(&mut cmd);
    let json_output = cmd
        .current_dir(scratch.path())
        .args(["gate", "evaluate", &issue_id, "g", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        json_output.status.code(),
        Some(10),
        "json mode must exit 10, matching text mode and docs/reference/exit-codes.md; \
         got stdout={} stderr={}",
        String::from_utf8_lossy(&json_output.stdout),
        String::from_utf8_lossy(&json_output.stderr)
    );

    let body: serde_json::Value = serde_json::from_slice(&json_output.stdout).unwrap_or_else(|e| {
        panic!(
            "stdout should be valid JSON: {e}\nstdout={}",
            String::from_utf8_lossy(&json_output.stdout)
        )
    });
    assert_eq!(body["error"]["code"], "STALE_BINARY");
    assert_eq!(body["error"]["details"]["issue_id"], issue_id);
    assert_eq!(body["error"]["details"]["key"], "g");
    assert_eq!(body["error"]["details"]["built_from"], ancestor);
    // Pre-verdict: no `verdict` field, unlike a completed (post-verdict)
    // GATE_FAILED/checker-error envelope.
    assert!(
        body["error"]["details"]["verdict"].is_null(),
        "a refused, never-run checker must not carry a verdict: {body}"
    );
    let suggestions = body["error"]["suggestions"]
        .as_array()
        .expect("suggestions should be an array");
    assert!(
        suggestions.iter().any(|s| s
            .as_str()
            .unwrap_or_default()
            .contains("cargo install --path crates/jit")),
        "suggestions should include the reinstall hint: {suggestions:?}"
    );
}

#[test]
fn test_gate_evaluate_metadata_only_commit_does_not_refuse_stale_binary() {
    let workspace_root = workspace_root();
    let Some(ancestor) = ancestor_commit(&workspace_root) else {
        eprintln!("SKIP: workspace does not have 9+ commits to pick a safe ancestor from");
        return;
    };
    let Some(stale_binary) = build_stale_child_binary(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not build the stale binary (cargo unavailable?)");
        return;
    };
    let Some(scratch) = scratch_repo_metadata_only_for(&workspace_root, &ancestor) else {
        eprintln!("SKIP: could not construct the scratch repo (git unavailable?)");
        return;
    };
    let issue_id = setup_gated_issue(scratch.path());

    let mut cmd = Command::new(&stale_binary);
    scrub_gate_context(&mut cmd);
    let output = cmd
        .current_dir(scratch.path())
        .args(["gate", "evaluate", &issue_id, "g"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "metadata-only changes must not refuse the gate: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
