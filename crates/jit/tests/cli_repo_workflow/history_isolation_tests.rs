use assert_cmd::prelude::*;
use jit::domain::identity::IssueId;
use jit::repository_state::VirtualPath;
use jit::test_cli::JitCli;
use serde_json::Value;
use std::fs;

const GATE: &str = "history-isolation";

struct HistoryRepo {
    cli: JitCli,
    target: String,
    other: String,
}

fn history_repo() -> HistoryRepo {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    cli.run(["init"]).unwrap();
    cli.command([
        "gate",
        "define",
        GATE,
        "--title",
        "History isolation",
        "--description",
        "Test gate",
        "--mode",
        "auto",
        "--checker-command",
        "true",
        "--timeout",
        "10",
    ])
    .assert()
    .success();

    let target = cli
        .create_issue(["--title", "Target", "--gate", GATE])
        .unwrap();
    let other = cli
        .create_issue(["--title", "Other", "--gate", GATE])
        .unwrap();
    cli.command(["gate", "evaluate", &other, GATE])
        .assert()
        .success();
    cli.command_with_env(
        ["claim", "acquire", &other, "--ttl", "600"],
        [("JIT_AGENT_ID", "agent:history-fixture")],
    )
    .assert()
    .success();
    cli.command_with_env(
        ["claim", "release", &other],
        [("JIT_AGENT_ID", "agent:history-fixture")],
    )
    .assert()
    .success();

    HistoryRepo { cli, target, other }
}

fn corrupt_history(repo: &HistoryRepo) {
    fs::write(
        repo.cli.root().join(".jit/events.jsonl"),
        b"not-json-event-one\nnot-json-event-two",
    )
    .unwrap();
    fs::write(
        repo.cli.root().join(".git/jit/claims.jsonl"),
        b"not-json-claim-one\nnot-json-claim-two\n",
    )
    .unwrap();

    let other: IssueId = repo.other.parse().unwrap();
    let run_directory = repo.cli.root().join(".jit").join(
        VirtualPath::gate_run_issue_directory(&other)
            .relative()
            .as_path(),
    );
    let records = fs::read_dir(run_directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert!(!records.is_empty(), "the fixture recorded a gate run");
    records.iter().for_each(|path| {
        fs::write(path, b"not-json-gate-run\n").unwrap();
    });
}

#[test]
fn test_issue_reads_succeed_with_unparseable_history() {
    let repo = history_repo();
    corrupt_history(&repo);

    repo.cli.run_json(["issue", "show", &repo.target]).unwrap();
    repo.cli
        .run_json(["issue", "status", &repo.target])
        .unwrap();
    repo.cli.run_json(["query", "available"]).unwrap();
}

#[test]
fn test_gate_evaluate_succeeds_with_unparseable_other_issue_history() {
    let repo = history_repo();
    corrupt_history(&repo);

    repo.cli
        .command(["gate", "evaluate", &repo.target, GATE])
        .assert()
        .success();
}

#[test]
fn test_claim_acquire_appends_event_with_unparseable_history() {
    let repo = history_repo();
    corrupt_history(&repo);

    repo.cli
        .command_with_env(
            ["claim", "acquire", &repo.target, "--ttl", "600"],
            [("JIT_AGENT_ID", "agent:history-claim")],
        )
        .assert()
        .success();

    let events = fs::read(repo.cli.root().join(".jit/events.jsonl")).unwrap();
    assert!(events.ends_with(b"\n"));
    let last_line = events
        .split(|byte| *byte == b'\n')
        .rfind(|line| !line.is_empty())
        .unwrap();
    let event: Value = serde_json::from_slice(last_line).unwrap();
    assert_eq!(event["type"], "issue_claimed");
    assert_eq!(event["issue_id"], repo.target);
}

#[test]
fn test_lease_acquire_renew_release_succeed_with_unparseable_history() {
    let repo = history_repo();
    corrupt_history(&repo);

    repo.cli
        .command_with_env(
            ["claim", "acquire", &repo.other, "--ttl", "600"],
            [("JIT_AGENT_ID", "agent:history-lease")],
        )
        .assert()
        .success();
    repo.cli
        .command_with_env(
            ["claim", "renew", &repo.other, "--extension", "600"],
            [("JIT_AGENT_ID", "agent:history-lease")],
        )
        .assert()
        .success();
    repo.cli
        .command_with_env(
            ["claim", "release", &repo.other],
            [("JIT_AGENT_ID", "agent:history-lease")],
        )
        .assert()
        .success();
}
