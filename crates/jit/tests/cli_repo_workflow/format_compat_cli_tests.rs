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
        stderr.contains("99") && stderr.contains("3"),
        "{cmd} error must name both the repo version (99) and the binary's supported version (3); got: {stderr}"
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
fn test_format_2_repository_reads_and_writes_from_its_issue_files() {
    let cli = JitCli::initialised().unwrap();
    let root = cli.create_issue(["-t", "Root"]).unwrap();
    let listed_only_in_index = cli.create_issue(["-t", "Indexed"]).unwrap();
    let unindexed = cli.create_issue(["-t", "Unindexed"]).unwrap();
    for dependent in [&listed_only_in_index, &unindexed] {
        cli.run(["dep", "add", dependent, &root]).unwrap();
    }
    let index_path = cli.root().join(".jit/index.json");
    let format_2 = serde_json::to_vec_pretty(&serde_json::json!({
        "all_ids": [root, listed_only_in_index],
        "deleted_ids": [],
        "schema_version": 2,
    }))
    .unwrap();
    fs::write(&index_path, &format_2).unwrap();

    assert_eq!(
        listed_ids(&cli),
        [
            root.clone(),
            listed_only_in_index.clone(),
            unindexed.clone()
        ]
        .into_iter()
        .collect()
    );

    let created = cli.create_issue(["-t", "Created"]).unwrap();
    assert!(listed_ids(&cli).contains(&created));
    cli.run(["dep", "add", &created, &root]).unwrap();
    assert_eq!(
        fs::read(&index_path).unwrap(),
        format_2,
        "a create must leave a format 2 index as it stands"
    );
    let validation = cli.command(["validate"]).output().unwrap();
    assert!(validation.status.success(), "{validation:?}");

    let deleted = cli
        .command_with_env(
            ["issue", "delete", &unindexed],
            [("JIT_ALLOW_DELETION", "1")],
        )
        .output()
        .unwrap();
    assert!(deleted.status.success(), "{deleted:?}");
    assert!(!listed_ids(&cli).contains(&unindexed));
    let index: serde_json::Value = serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
    assert_eq!(index["schema_version"], 2, "a write must keep the marker");
    assert_eq!(index["deleted_ids"], serde_json::json!([unindexed]));
    assert!(index.get("all_ids").is_none());
    let validation = cli.command(["validate"]).output().unwrap();
    assert!(validation.status.success(), "{validation:?}");
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
