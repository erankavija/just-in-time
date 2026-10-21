//! `jit issue update` applies every requested change or none.

use assert_cmd::prelude::*;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit(root: &Path, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(root)
        .args(args)
        .output()
        .unwrap()
}

fn repository_text(root: &Path, id: &str) -> [String; 3] {
    [
        format!(".jit/issues/{id}.json"),
        ".jit/index.json".to_string(),
        ".jit/events.jsonl".to_string(),
    ]
    .map(|path| std::fs::read_to_string(root.join(path)).unwrap())
}

#[test]
fn test_issue_update_rejected_label_leaves_gate_and_assignee_changes_unapplied() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(root)
        .arg("init")
        .assert()
        .success();
    let define = jit(
        root,
        &["gate", "define", "review", "-t", "Review", "-d", "Review"],
    );
    assert!(define.status.success(), "{define:?}");
    let created = jit(root, &["issue", "create", "--title", "Target", "--json"]);
    let created: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    let before = repository_text(root, &id);

    let update = jit(
        root,
        &[
            "issue",
            "update",
            &id,
            "--add-gate",
            "review",
            "--assignee",
            "agent:w1",
            "--label",
            "BADLABEL",
        ],
    );

    assert!(!update.status.success(), "{update:?}");
    assert_eq!(
        repository_text(root, &id),
        before,
        "a refused update must leave the issue file, index, and event log unchanged"
    );
}
