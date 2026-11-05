//! Git-optional commands keep working when a checkout's child process cannot
//! find Git; claim commands report the typed Git requirement.

use jit::output::ErrorCode;
use jit::test_cli::JitCli;
use serde_json::Value;
use std::process::Output;
use std::str::FromStr;
use tempfile::TempDir;

fn run_without_git(cli: &JitCli, path: &TempDir, args: &[&str]) -> Output {
    cli.command_with_env(
        args.iter().copied().chain(["--json"]),
        [("PATH", path.path().as_os_str())],
    )
    .output()
    .expect("jit should start without Git on PATH")
}

fn assert_success(output: Output, command: &str) -> Value {
    assert!(
        output.status.success(),
        "{command} should succeed without Git on PATH: status={}, stdout={}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("successful --json command emits JSON")
}

fn assert_requires_git(output: Output, command: &str) {
    assert_eq!(
        output.status.code(),
        Some(ErrorCode::RequiresGit.exit_code().code()),
        "{command} should fail with the RequiresGit exit code: stdout={}, stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json: Value = serde_json::from_slice(&output.stdout)
        .expect("failed --json command emits an error envelope");
    let actual = ErrorCode::from_str(
        json["error"]["code"]
            .as_str()
            .expect("error envelope carries a code"),
    )
    .expect("error code is registered");
    assert_eq!(actual, ErrorCode::RequiresGit, "{command} error code");
}

#[test]
fn test_core_repository_commands_succeed_without_git_executable() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    let empty_path = TempDir::new().expect("empty PATH directory should be created");

    assert_success(run_without_git(&cli, &empty_path, &["init"]), "jit init");

    let prerequisite = assert_success(
        run_without_git(
            &cli,
            &empty_path,
            &["issue", "create", "--title", "Prerequisite"],
        ),
        "jit issue create prerequisite",
    )["id"]
        .as_str()
        .expect("created issue has an id")
        .to_owned();
    let dependent = assert_success(
        run_without_git(
            &cli,
            &empty_path,
            &["issue", "create", "--title", "Dependent"],
        ),
        "jit issue create dependent",
    )["id"]
        .as_str()
        .expect("created issue has an id")
        .to_owned();

    assert_success(
        run_without_git(&cli, &empty_path, &["issue", "show", &dependent]),
        "jit issue show",
    );
    assert_success(
        run_without_git(&cli, &empty_path, &["query", "available"]),
        "jit query available",
    );
    assert_success(
        run_without_git(
            &cli,
            &empty_path,
            &["dep", "add", &dependent, &prerequisite],
        ),
        "jit dep add",
    );
}

#[test]
fn test_claim_commands_require_git_executable() {
    let cli = JitCli::initialised().unwrap();
    cli.init_git().unwrap().commit("Initial commit").unwrap();
    let issue_id = cli.create_issue(["--title", "Claim target"]).unwrap();
    let empty_path = TempDir::new().expect("empty PATH directory should be created");

    [
        (
            "jit claim acquire",
            vec![
                "claim",
                "acquire",
                &issue_id,
                "--ttl",
                "600",
                "--agent-id",
                "agent:no-git",
            ],
        ),
        ("jit claim release", vec!["claim", "release", &issue_id]),
        (
            "jit claim renew",
            vec!["claim", "renew", &issue_id, "--extension", "300"],
        ),
        ("jit claim heartbeat", vec!["claim", "heartbeat", &issue_id]),
        (
            "jit claim status",
            vec!["claim", "status", "--issue", &issue_id],
        ),
        ("jit claim list", vec!["claim", "list"]),
        (
            "jit claim force-evict",
            vec!["claim", "force-evict", &issue_id, "--reason", "test"],
        ),
    ]
    .into_iter()
    .for_each(|(name, args)| assert_requires_git(run_without_git(&cli, &empty_path, &args), name));
}
