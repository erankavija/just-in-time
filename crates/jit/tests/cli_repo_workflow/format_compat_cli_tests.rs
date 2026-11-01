//! CLI-level startup guard for a too-new repository format (jit:def64ac4 REQ-02).
//!
//! A binary older than the repository's `index.json` `schema_version` must
//! refuse to operate at startup for EVERY non-init command — including ones
//! that never load the index themselves (e.g. `gate list`, which reads
//! `gates.toml` directly) — with a single-line, nonzero error naming both
//! versions, rather than misreading newer data.

use jit::test_cli::JitCli;
use std::fs;

/// Rewrite the repository's `index.json` to a format version far newer than
/// any binary supports, simulating a repo written by a future `jit`.
fn make_repository_too_new(cli: &JitCli) {
    fs::write(
        cli.root().join(".jit/index.json"),
        r#"{"schema_version": 99, "all_ids": [], "deleted_ids": []}"#,
    )
    .unwrap();
}

fn assert_too_new_refusal(out: &std::process::Output, cmd: &str) {
    assert!(
        !out.status.success(),
        "{cmd} on a too-new repo must fail; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("99") && stderr.contains("2"),
        "{cmd} error must name both the repo version (99) and the binary's supported version (2); got: {stderr}"
    );
    // Single-line message (one trailing newline at most).
    let msg = stderr.trim_end();
    assert!(
        !msg.is_empty() && !msg.trim().contains('\n'),
        "{cmd} error must be a single line; got: {stderr}"
    );
}

#[test]
fn test_status_refuses_too_new_repo_format() {
    let cli = JitCli::initialised().unwrap();
    make_repository_too_new(&cli);
    let out = cli.command(["status"]).output().unwrap();
    assert_too_new_refusal(&out, "status");
}

#[test]
fn test_gate_list_refuses_too_new_repo_format_despite_bypassing_index() {
    // `gate list` reads gates.toml directly and never loads index.json, so this
    // is the exact bypass the reviewer flagged: the startup guard in validate()
    // must still refuse it.
    let cli = JitCli::initialised().unwrap();
    make_repository_too_new(&cli);
    let out = cli.command(["gate", "list"]).output().unwrap();
    assert_too_new_refusal(&out, "gate list");
}

#[test]
fn test_init_refuses_reinitializing_too_new_repo_format() {
    // `jit init` is the one command that bypasses the non-init startup
    // validate() gate, so it must run the format guard itself: re-initializing
    // over an existing too-new repository must refuse rather than write into it.
    let cli = JitCli::initialised().unwrap();
    make_repository_too_new(&cli);
    let out = cli.command(["init"]).output().unwrap();
    assert_too_new_refusal(&out, "init");
}
