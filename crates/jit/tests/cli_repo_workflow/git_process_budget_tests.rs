use jit::test_cli::JitCli;
use jit::test_harness::git::{GitRepository, GitStandIn};
use serde_json::Value;

fn assert_read_with_git_budget(
    cli: &JitCli,
    args: &[&str],
    reads_issue: impl FnOnce(&Value) -> bool,
) {
    let stand_in = GitStandIn::new();
    let output = cli
        .command_with_env(args, [("PATH", stand_in.search_path())])
        .output()
        .unwrap();
    let invocations = stand_in.invocations();

    assert!(
        invocations.len() <= 1,
        "jit {args:?} exceeded one git process: {invocations:#?}"
    );
    assert!(
        output.status.success(),
        "jit {args:?} failed: {output:?}; git invocations: {invocations:#?}"
    );
    let json: Value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("jit {args:?} returned invalid JSON: {error}; git invocations: {invocations:#?}")
    });
    assert!(
        reads_issue(&json),
        "jit {args:?} did not read the issue: {json}; git invocations: {invocations:#?}"
    );
}

fn assert_issue_read_budget(cli: &JitCli, id: &str) {
    assert_read_with_git_budget(cli, &["query", "available", "--json"], |json| {
        json["issues"]
            .as_array()
            .is_some_and(|issues| issues.iter().any(|issue| issue["id"] == id))
    });
    assert_read_with_git_budget(cli, &["issue", "show", id, "--json"], |json| {
        json["id"] == id
    });
}

fn create_ready_issue(cli: &JitCli) -> String {
    let id = cli.create_issue(["--title", "Available issue"]).unwrap();
    cli.run(["issue", "update", &id, "--state", "ready"])
        .unwrap();
    id
}

#[test]
fn test_issue_reads_start_at_most_one_git_process_in_primary_checkout() {
    let cli = JitCli::uninitialised().unwrap();
    GitRepository::init(cli.root());
    cli.run(["init"]).unwrap();
    let id = create_ready_issue(&cli);

    assert_issue_read_budget(&cli, &id);
}

#[test]
fn test_issue_reads_start_at_most_one_git_process_for_primary_only_issue_in_linked_checkout() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let id = create_ready_issue(&primary);
    let issue_path = format!(".jit/issues/{id}.json");
    assert!(primary.root().join(&issue_path).is_file());
    assert!(!linked.root().join(&issue_path).exists());

    assert_issue_read_budget(&linked, &id);
}
