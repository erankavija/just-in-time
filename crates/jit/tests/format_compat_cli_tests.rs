//! CLI-level startup guard for a too-new repository format (jit:def64ac4 REQ-02).
//!
//! A binary older than the repository's `index.json` `schema_version` must
//! refuse to operate at startup for EVERY non-init command — including ones
//! that never load the index themselves (e.g. `gate list`, which reads
//! `gates.toml` directly) — with a single-line, nonzero error naming both
//! versions, rather than misreading newer data.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn jit_binary() -> &'static str {
    env!("CARGO_BIN_EXE_jit")
}

/// Init a repo, then rewrite its `index.json` to a format version far newer than
/// any binary supports, simulating a repo written by a future `jit`.
fn setup_too_new_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    let out = Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "init failed");
    let index = temp.path().join(".jit/index.json");
    fs::write(
        &index,
        r#"{"schema_version": 99, "all_ids": [], "deleted_ids": []}"#,
    )
    .unwrap();
    temp
}

fn assert_too_new_refusal(out: &std::process::Output, cmd: &str) {
    assert_eq!(
        out.status.code(),
        Some(10),
        "{cmd} on a too-new repo must exit 10 (ExternalError); stderr={}",
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
    let temp = setup_too_new_repo();
    let out = Command::new(jit_binary())
        .args(["status"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_too_new_refusal(&out, "status");
}

#[test]
fn test_gate_list_refuses_too_new_repo_format_despite_bypassing_index() {
    // `gate list` reads gates.toml directly and never loads index.json, so this
    // is the exact bypass the reviewer flagged: the startup guard in validate()
    // must still refuse it.
    let temp = setup_too_new_repo();
    let out = Command::new(jit_binary())
        .args(["gate", "list"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_too_new_refusal(&out, "gate list");
}

#[test]
fn test_init_refuses_reinitializing_too_new_repo_format() {
    // `jit init` is the one command that bypasses the non-init startup
    // validate() gate, so it must run the format guard itself: re-initializing
    // over an existing too-new repository must refuse rather than write into it.
    let temp = setup_too_new_repo();
    let out = Command::new(jit_binary())
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    assert_too_new_refusal(&out, "init");
}
