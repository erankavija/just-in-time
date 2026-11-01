//! `jit issue reject` is the only command form that rejects an issue.

use jit::test_cli::JitCli;

fn assert_refused_naming_reject(cli: &JitCli, selector: &[&str]) {
    let mut args = vec!["issue", "update"];
    args.extend_from_slice(selector);
    args.extend(["--state", "rejected"]);
    let output = cli.command(args).output().unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "a usage error, not a failure"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("jit issue reject"),
        "the refusal names the surviving command, got: {stderr}"
    );
}

fn state_of(cli: &JitCli, id: &str) -> String {
    let issue = cli.run_json(["issue", "show", id]).unwrap();
    issue["state"].as_str().unwrap().to_string()
}

#[test]
fn test_issue_update_state_rejected_refused_naming_reject_command() {
    let cli = JitCli::initialised().unwrap();
    let id = cli
        .create_issue(["-t", "Doomed", "--label", "type:task"])
        .unwrap();
    let before = state_of(&cli, &id);

    assert_refused_naming_reject(&cli, &[&id]);

    assert_eq!(
        state_of(&cli, &id),
        before,
        "the refusal leaves the issue untouched"
    );
}

#[test]
fn test_issue_update_filter_state_rejected_refused_naming_reject_command() {
    let cli = JitCli::initialised().unwrap();
    let id = cli
        .create_issue(["-t", "Doomed", "--label", "type:task"])
        .unwrap();
    let before = state_of(&cli, &id);

    assert_refused_naming_reject(&cli, &["--filter", "label:type:task"]);

    assert_eq!(
        state_of(&cli, &id),
        before,
        "the refusal leaves the issue untouched"
    );
}
