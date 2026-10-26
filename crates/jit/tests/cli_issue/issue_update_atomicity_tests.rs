//! `jit issue update` applies every requested change or none.

use jit::test_cli::JitCli;

fn repository_text(root: &std::path::Path, id: &str) -> [String; 3] {
    [
        format!(".jit/issues/{id}.json"),
        ".jit/index.json".to_string(),
        ".jit/events.jsonl".to_string(),
    ]
    .map(|path| std::fs::read_to_string(root.join(path)).unwrap())
}

#[test]
fn test_issue_update_rejected_label_leaves_gate_and_assignee_changes_unapplied() {
    let temp = JitCli::initialised().unwrap();
    let root = temp.root();
    temp.run(["gate", "define", "review", "-t", "Review", "-d", "Review"])
        .unwrap();
    let id = temp.create_issue(["--title", "Target"]).unwrap();
    let before = repository_text(root, &id);

    let update = temp
        .command([
            "issue",
            "update",
            &id,
            "--add-gate",
            "review",
            "--assignee",
            "agent:w1",
            "--label",
            "BADLABEL",
        ])
        .output()
        .unwrap();

    assert!(!update.status.success(), "{update:?}");
    assert_eq!(
        repository_text(root, &id),
        before,
        "a refused update must leave the issue file, index, and event log unchanged"
    );
}
