//! REQ-01/02 JSON-mode regression for jit:7446af34.
//!
//! The stale-binary refusal (`crate::errors::StaleBinaryError`) must exit `10`
//! and carry a structured, `STALE_BINARY`-coded envelope under `--json`,
//! exactly matching the exit code the non-`--json` path already produced.
//! Before this fix, `render_gate_pass_error` (main.rs) had no branch for
//! `StaleBinaryError`, so it fell through to the generic `GATE_ERROR` code,
//! which `ErrorCode::to_exit_code` maps to exit `1` — silently breaking the
//! documented exit-10 contract under `--json` only.
//!
//! Reproducing a genuine stale-binary refusal needs a repository whose `HEAD`
//! is a commit the running binary's own build commit is an ancestor of, but
//! not equal to (see `commands::gate_check`'s wiring tests for the same
//! technique at the library level). This test does the equivalent through the
//! actual `jit` binary: it fetches the *test binary's* own build commit from
//! the real jit workspace into a disposable scratch repo, checks it out, then
//! advances `HEAD` past it — entirely inside the scratch repo; nothing in the
//! real workspace is read, moved, or written.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Build a scratch git repository whose `HEAD` is one commit past the running
/// test binary's own build commit. Returns `None` (the caller should skip)
/// when the binary was built without git, or any local git step fails for
/// environment reasons — mirrors
/// `commands::gate_check::tests::scratch_repo_stale_against_own_build`.
fn scratch_repo_stale_against_own_build() -> Option<(TempDir, String)> {
    let info = jit::build_info::version_info();
    if info.git_commit == "unknown" {
        eprintln!("SKIP: this binary was built without git; no build commit to compare");
        return None;
    }
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)?
        .to_str()?
        .to_string();

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
        eprintln!("SKIP: git init failed in scratch repo");
        return None;
    }
    if !run(&["fetch", "-q", &workspace_root, info.git_commit]) {
        eprintln!("SKIP: git fetch of the running binary's build commit failed");
        return None;
    }
    if !run(&["checkout", "-q", "FETCH_HEAD"]) {
        eprintln!("SKIP: checkout of the fetched build commit failed");
        return None;
    }
    if !run(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@example.com",
        "commit",
        "--allow-empty",
        "-q",
        "-m",
        "advance past the build commit",
    ]) {
        eprintln!("SKIP: advancing HEAD past the build commit failed");
        return None;
    }
    Some((temp, info.git_commit.to_string()))
}

/// `jit init` + one automated gate (key `g`, always-passing checker) + one
/// issue requiring it, all inside `repo_root`. Returns the issue id.
fn setup_gated_issue(repo_root: &Path) -> String {
    let status = Command::new(jit_binary())
        .current_dir(repo_root)
        .arg("init")
        .status()
        .unwrap();
    assert!(status.success());

    let status = Command::new(jit_binary())
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

    let output = Command::new(jit_binary())
        .current_dir(repo_root)
        .args(["issue", "create", "--title", "Test", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let created: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let issue_id = created["id"].as_str().unwrap().to_string();

    let status = Command::new(jit_binary())
        .current_dir(repo_root)
        .args(["gate", "add", &issue_id, "g"])
        .status()
        .unwrap();
    assert!(status.success());

    issue_id
}

/// REQ-01/02: text mode and `--json` mode agree on exit code 10 for the
/// stale-binary refusal, and the JSON envelope is a structured, `STALE_BINARY`
/// error carrying the reinstall hint — not the generic fallback `GATE_ERROR`
/// (which would exit 1).
#[test]
fn test_gate_evaluate_stale_binary_exits_10_in_text_and_json_modes() {
    let Some((temp, built_from)) = scratch_repo_stale_against_own_build() else {
        return;
    };
    let issue_id = setup_gated_issue(temp.path());

    // Text mode.
    let text_output = Command::new(jit_binary())
        .current_dir(temp.path())
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
    let json_output = Command::new(jit_binary())
        .current_dir(temp.path())
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
    assert_eq!(body["error"]["details"]["built_from"], built_from);
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
