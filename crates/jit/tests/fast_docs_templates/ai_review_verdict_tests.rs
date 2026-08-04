//! Regression tests for the ai-review.sh verdict parser.
//!
//! Verifies that reviewer output whose VERDICT: PASS line is followed by
//! additional prose is still recorded as a pass. The parser lives in the one
//! packaged wrapper, contrib/gates/ai-review.sh.

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

/// Returns the packaged AI-review wrapper, the single copy in the repository.
fn wrapper() -> std::path::PathBuf {
    repo_root().join("contrib/gates/ai-review.sh")
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

/// Runs the ai-review.sh wrapper and returns its exit code.
fn run_script(script: &Path, context_file: &Path, reviewer_agent: &str) -> i32 {
    let output = Command::new("bash")
        .arg(script)
        .env("JIT_CONTEXT_FILE", context_file)
        .env("REVIEWER_AGENT", reviewer_agent)
        // Suppress diagnostic stderr from the script so test output stays clean.
        .env("AGENT_STDERR_HEAD_LINES", "0")
        .output()
        .unwrap_or_else(|e| panic!("failed to run {}: {e}", script.display()));
    output.status.code().unwrap_or(1)
}

fn run_script_output(
    script: &Path,
    context_file: &Path,
    reviewer_agent: &str,
) -> std::process::Output {
    Command::new("bash")
        .arg(script)
        .env("JIT_CONTEXT_FILE", context_file)
        .env("REVIEWER_AGENT", reviewer_agent)
        .env("AGENT_STDERR_HEAD_LINES", "0")
        .output()
        .unwrap_or_else(|error| panic!("failed to run {}: {error}", script.display()))
}

#[test]
fn test_verdict_pass_with_trailing_prose_records_a_pass() {
    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    // Reviewer emits: a finding, the verdict, then trailing prose.
    let agent = write_fake_agent(
        &temp,
        "1. No issues found.\nTotal findings: 0\nVERDICT: PASS\nThank you for the review opportunity.\n",
    );

    let script = wrapper();
    assert!(script.exists(), "script not found: {}", script.display());

    let exit_code = run_script(&script, &context, agent.to_str().unwrap());
    assert_eq!(
        exit_code, 0,
        "expected exit 0 (PASS) but got {exit_code}: \
         trailing prose after VERDICT: PASS must not flip the verdict"
    );
}

// Sanity: VERDICT: FAIL is still recorded as failure.
#[test]
fn test_verdict_fail_records_a_failure() {
    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(&temp, "1. Bug found.\nTotal findings: 1\nVERDICT: FAIL\n");

    let exit_code = run_script(&wrapper(), &context, agent.to_str().unwrap());
    assert_eq!(exit_code, 1, "expected exit 1 (FAIL) but got {exit_code}");
}

// Sanity: no verdict line at all → failure.
#[test]
fn test_verdict_unparseable_records_a_failure() {
    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(&temp, "Some prose with no verdict line.\n");

    let exit_code = run_script(&wrapper(), &context, agent.to_str().unwrap());
    assert_ne!(
        exit_code, 0,
        "expected non-zero exit for unparseable verdict"
    );
}

#[test]
fn test_missing_prompt_fails_with_a_diagnostic() {
    let temp = TempDir::new().unwrap();
    let context = temp.path().join("context-without-prompt.json");
    fs::write(&context, r#"{"prompt":null}"#).unwrap();
    let agent = write_fake_agent(&temp, "Total findings: 0\nVERDICT: PASS\n");

    let output = run_script_output(&wrapper(), &context, agent.to_str().unwrap());

    assert_eq!(output.status.code(), Some(1), "missing prompt passed");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("No prompt defined"),
        "missing-prompt diagnostic absent"
    );
}

#[test]
fn test_classified_advisory_finding_passes() {
    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(
        &temp,
        "1. Existing cleanup opportunity.\nTotal findings: 1\n<<<JIT-FINDINGS-JSON\n{\"verdict\":\"pass\",\"summary\":\"implementation is sound\",\"findings\":[{\"id\":\"F1\",\"severity\":\"low\",\"disposition\":\"advisory\",\"origin\":\"pre-existing\",\"summary\":\"existing cleanup opportunity\"}]}\nJIT-FINDINGS-JSON>>>\nVERDICT: PASS\n",
    );

    let exit_code = run_script(&wrapper(), &context, agent.to_str().unwrap());
    assert_eq!(exit_code, 0, "classified advisory output failed");
}

#[test]
fn test_classified_referenced_finding_fails_and_keeps_the_reference() {
    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(
        &temp,
        "1. Policy defect.\nTotal findings: 1\n<<<JIT-FINDINGS-JSON\n{\"verdict\":\"fail\",\"summary\":\"policy defect\",\"findings\":[{\"id\":\"F1\",\"severity\":\"high\",\"disposition\":\"blocking\",\"origin\":\"issue-impact\",\"summary\":\"policy defect\",\"references\":[\"@/inv/pid-safety\"]}]}\nJIT-FINDINGS-JSON>>>\nVERDICT: FAIL\n",
    );

    let output = run_script_output(&wrapper(), &context, agent.to_str().unwrap());

    assert_eq!(
        output.status.code(),
        Some(1),
        "wrapper passed a failing verdict"
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("@/inv/pid-safety"),
        "wrapper did not preserve a referenced finding"
    );
}

#[test]
fn test_wrapper_contract_documents_optional_references_without_requiring_them() {
    let contents = fs::read_to_string(wrapper()).unwrap();

    assert!(contents.contains("references"));
    assert!(contents.contains("optional"));
    assert!(contents.contains("\"references\":[\"<qualified-policy-id>\"]"));
    assert!(
        contents.contains("may be omitted"),
        "legacy findings without references must remain valid"
    );
}

#[test]
fn test_ai_review_wrapper_stays_tool_agnostic_and_portable() {
    let contents = fs::read_to_string(wrapper()).unwrap();

    assert!(
        !contents.contains("jq"),
        "portable wrapper must not require jq"
    );
    assert!(
        !contents.contains("codex"),
        "generic wrapper must not prescribe a reviewer tool"
    );
}

#[test]
fn test_prompt_contract_does_not_execute_markdown_as_shell_commands() {
    let temp = TempDir::new().unwrap();
    let context = write_context_file(&temp);
    let agent = write_fake_agent(
        &temp,
        "Total findings: 0\n<<<JIT-FINDINGS-JSON\n{\"verdict\":\"pass\",\"summary\":\"clear\",\"findings\":[]}\nJIT-FINDINGS-JSON>>>\nVERDICT: PASS\n",
    );

    let output = run_script_output(&wrapper(), &context, agent.to_str().unwrap());

    assert!(output.status.success(), "wrapper failed");
    assert!(
        output.stderr.is_empty(),
        "prompt construction emitted stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
