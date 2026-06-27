//! Regression tests for the ai-review.sh verdict parser (REQ-11).
//!
//! Verifies that reviewer output whose VERDICT: PASS line is followed by
//! additional prose is still recorded as a pass. Both parser copies are
//! exercised: scripts/ai-review.sh and contrib/gates/ai-review.sh.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// Returns the repository root (two levels above `crates/jit/`).
fn repo_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Creates a minimal JIT_CONTEXT_FILE that ai-review.sh accepts.
fn write_context_file(dir: &TempDir) -> std::path::PathBuf {
    let path = dir.path().join("context.json");
    fs::write(
        &path,
        r#"{"prompt":"Review the implementation for correctness."}"#,
    )
    .unwrap();
    path
}

/// Creates an executable fake-agent script that emits the given text to stdout,
/// ignoring whatever is fed to its stdin.
fn write_fake_agent(dir: &TempDir, output: &str) -> std::path::PathBuf {
    let path = dir.path().join("fake_agent.sh");
    // Use printf with escaped newlines so the script is a single line.
    let escaped = output.replace('\\', "\\\\").replace('\n', "\\n");
    fs::write(&path, format!("#!/usr/bin/env bash\nprintf '{escaped}'\n")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Runs one ai-review.sh script and returns its exit code.
fn run_script(script: &Path, context_file: &Path, reviewer_agent: &str) -> i32 {
    let output = Command::new("bash")
        .arg(script)
        .env("JIT_CONTEXT_FILE", context_file)
        .env("REVIEWER_AGENT", reviewer_agent)
        // Suppress diagnostic stderr from the scripts so test output stays clean.
        .env("AGENT_STDERR_HEAD_LINES", "0")
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", script.display()));
    output.status.code().unwrap_or(1)
}

/// Skips the test if jq is not on PATH (required by the ai-review.sh scripts).
fn require_jq() -> bool {
    Command::new("jq")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

// REQ-11: VERDICT: PASS followed by trailing prose must still record PASS.
// Covered for scripts/ai-review.sh.
#[test]
fn test_verdict_pass_with_trailing_prose_scripts_copy() {
    if !require_jq() {
        eprintln!("SKIP: jq not found on PATH");
        return;
    }

    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    // Reviewer emits: a finding, the verdict, then trailing prose.
    let agent = write_fake_agent(
        &temp,
        "1. No issues found.\nTotal findings: 0\nVERDICT: PASS\nThank you for the review opportunity.\n",
    );

    let script = repo_root().join("scripts").join("ai-review.sh");
    assert!(script.exists(), "script not found: {}", script.display());

    let exit_code = run_script(&script, &context, agent.to_str().unwrap());
    assert_eq!(
        exit_code, 0,
        "expected exit 0 (PASS) but got {exit_code}: \
         trailing prose after VERDICT: PASS must not flip the verdict"
    );
}

// REQ-11: VERDICT: PASS followed by trailing prose must still record PASS.
// Covered for contrib/gates/ai-review.sh.
#[test]
fn test_verdict_pass_with_trailing_prose_contrib_copy() {
    if !require_jq() {
        eprintln!("SKIP: jq not found on PATH");
        return;
    }

    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(
        &temp,
        "1. No issues found.\nTotal findings: 0\nVERDICT: PASS\nThank you for the review opportunity.\n",
    );

    let script = repo_root()
        .join("contrib")
        .join("gates")
        .join("ai-review.sh");
    assert!(script.exists(), "script not found: {}", script.display());

    let exit_code = run_script(&script, &context, agent.to_str().unwrap());
    assert_eq!(
        exit_code, 0,
        "expected exit 0 (PASS) but got {exit_code}: \
         trailing prose after VERDICT: PASS must not flip the verdict"
    );
}

// Sanity: VERDICT: FAIL is still recorded as failure (scripts copy).
#[test]
fn test_verdict_fail_scripts_copy() {
    if !require_jq() {
        eprintln!("SKIP: jq not found on PATH");
        return;
    }

    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(&temp, "1. Bug found.\nTotal findings: 1\nVERDICT: FAIL\n");

    let script = repo_root().join("scripts").join("ai-review.sh");
    let exit_code = run_script(&script, &context, agent.to_str().unwrap());
    assert_eq!(exit_code, 1, "expected exit 1 (FAIL) but got {exit_code}");
}

// Sanity: no verdict line at all → failure (scripts copy).
#[test]
fn test_verdict_unparseable_scripts_copy() {
    if !require_jq() {
        eprintln!("SKIP: jq not found on PATH");
        return;
    }

    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(&temp, "Some prose with no verdict line.\n");

    let script = repo_root().join("scripts").join("ai-review.sh");
    let exit_code = run_script(&script, &context, agent.to_str().unwrap());
    assert_ne!(
        exit_code, 0,
        "expected non-zero exit for unparseable verdict"
    );
}
