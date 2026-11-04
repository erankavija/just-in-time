//! A list flag takes one value per repetition: a comma stays inside its value
//! and fails that value's own validation, so no list flag writes a split list.

use assert_cmd::prelude::*;
use jit::test_cli::JitCli;

fn repository_snapshot(cli: &JitCli) -> serde_json::Value {
    cli.run_json(["issue", "list"]).unwrap()
}

#[test]
fn test_every_list_flag_refuses_a_comma_joined_value_and_writes_nothing() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Target"]).unwrap();
    cli.command([
        "gate",
        "define",
        "tests",
        "--title",
        "Tests",
        "--description",
        "manual",
    ])
    .assert()
    .success();
    cli.command(["gate", "add", &id, "tests"])
        .assert()
        .success();
    cli.command(["gate", "preset", "create", &id, "ci"])
        .assert()
        .success();
    let invocations: [&[&str]; 8] = [
        &[
            "issue",
            "create",
            "--title",
            "New",
            "--gate",
            "tests,clippy",
        ],
        &["issue", "create", "--title", "New", "--label", "a:b,c:d"],
        &["issue", "show", &id, "--fields", "title,state"],
        &["issue", "update", &id, "--label", "a:b,c:d"],
        &["issue", "update", &id, "--remove-label", "a:b,c:d"],
        &["issue", "update", &id, "--add-gate", "tests,clippy"],
        &["issue", "update", &id, "--remove-gate", "tests,clippy"],
        &[
            "gate",
            "preset",
            "apply",
            "ci",
            &id,
            "--except",
            "tests,clippy",
        ],
    ];
    let before = repository_snapshot(&cli);

    for args in invocations {
        let output = cli.command(args).output().unwrap();

        assert!(!output.status.success(), "{args:?} accepted a comma value");
        assert_eq!(repository_snapshot(&cli), before, "{args:?} wrote");
    }
}

#[test]
fn test_remove_label_removes_a_stored_label_containing_a_comma() {
    let cli = JitCli::initialised().unwrap();
    let id = cli.create_issue(["--title", "Prose label"]).unwrap();
    let label = "resolution:not needed, see x";
    let path = cli.root().join(format!(".jit/issues/{id}.json"));
    let mut record: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    record["labels"] = serde_json::json!([label, "component:core"]);
    std::fs::write(&path, serde_json::to_string(&record).unwrap()).unwrap();

    cli.run(["issue", "update", &id, "--remove-label", label])
        .unwrap();

    let issue = cli.run_json(["issue", "show", &id]).unwrap();
    assert_eq!(issue["labels"], serde_json::json!(["component:core"]));
}
