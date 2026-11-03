//! CLI-level startup guard for an unsupported repository format (jit:def64ac4 REQ-02).
//!
//! A binary must refuse a repository in any format but its own at startup for
//! EVERY non-init command, including ones that never load the index themselves
//! (e.g. `gate list`, which reads `gates.toml` directly), with a single-line,
//! nonzero error naming both versions.

use jit::test_cli::JitCli;
use std::fs;

fn supported_version() -> u64 {
    serde_json::from_slice::<serde_json::Value>(
        &jit::repository_state::fresh_index_bytes().unwrap(),
    )
    .unwrap()["schema_version"]
        .as_u64()
        .unwrap()
}

/// Rewrite the repository's `index.json` to a format version far newer than
/// any binary supports, simulating a repo written by a future `jit`.
fn make_repository_too_new(cli: &JitCli) {
    fs::write(
        cli.root().join(".jit/index.json"),
        r#"{"schema_version": 99, "deleted_ids": []}"#,
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
        stderr.contains("99") && stderr.contains(&supported_version().to_string()),
        "{cmd} error must name both the repo version (99) and the binary's supported version; got: {stderr}"
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

fn listed_ids(cli: &JitCli) -> std::collections::BTreeSet<String> {
    cli.run_json(["query", "all"]).unwrap()["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| issue["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn test_every_other_format_exits_10_naming_both_versions() {
    for found in [supported_version() - 1, supported_version() + 1] {
        let cli = JitCli::initialised().unwrap();
        fs::write(
            cli.root().join(".jit/index.json"),
            format!(r#"{{"schema_version": {found}, "all_ids": [], "deleted_ids": []}}"#),
        )
        .unwrap();

        let out = cli.command(["query", "all"]).output().unwrap();

        assert_eq!(out.status.code(), Some(10), "{out:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr)
                .trim_end()
                .lines()
                .count(),
            1
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(&found.to_string())
                && stderr.contains(&supported_version().to_string()),
            "{stderr}"
        );
    }
}

#[test]
fn test_validate_fails_on_a_format_4_index_carrying_all_ids() {
    let cli = JitCli::initialised().unwrap();
    fs::write(
        cli.root().join(".jit/index.json"),
        r#"{"schema_version": 4, "all_ids": [], "deleted_ids": []}"#,
    )
    .unwrap();

    let out = cli.command(["validate"]).output().unwrap();

    assert!(!out.status.success(), "{out:?}");
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(output.contains("all_ids"), "{output}");
}

#[test]
fn test_missing_issues_directory_lists_as_empty_through_capture() {
    let cli = JitCli::initialised().unwrap();
    fs::remove_dir_all(cli.root().join(".jit/issues")).unwrap();

    let validation = cli.command(["validate"]).output().unwrap();
    assert!(validation.status.success(), "{validation:?}");
    let created = cli.create_issue(["-t", "First"]).unwrap();
    assert_eq!(listed_ids(&cli), [created].into_iter().collect());
}
