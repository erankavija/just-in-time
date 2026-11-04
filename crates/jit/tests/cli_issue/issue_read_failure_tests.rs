//! Failures of the issue read commands report the classifier's code, with the
//! same exit status under `--json` and in human mode.

use jit::test_cli::JitCli;
use std::fs;

const READ_COMMANDS: [&str; 4] = ["show", "status", "children", "progress"];

fn exit_codes(repository: &JitCli, args: &[&str]) -> (i32, i32, serde_json::Value) {
    let json = repository
        .command(args.iter().copied().chain(["--json"]))
        .output()
        .unwrap();
    let human = repository.command(args).output().unwrap();
    let envelope = serde_json::from_slice(&json.stdout).expect("failure envelope on stdout");
    (
        json.status.code().unwrap(),
        human.status.code().unwrap(),
        envelope,
    )
}

#[test]
fn test_issue_read_unreadable_issue_file_reports_classified_code_not_issue_not_found() {
    let repository = JitCli::initialised().unwrap();
    let id = repository.create_issue(["--title", "Broken"]).unwrap();
    fs::write(
        repository.root().join(format!(".jit/issues/{id}.json")),
        b"{",
    )
    .unwrap();

    for command in READ_COMMANDS {
        let (json_exit, human_exit, envelope) = exit_codes(&repository, &["issue", command, &id]);

        assert_ne!(envelope["error"]["code"], "ISSUE_NOT_FOUND", "{command}");
        assert_ne!(json_exit, 3, "{command}");
        assert_eq!(json_exit, human_exit, "{command}");
    }
}

#[test]
fn test_issue_read_missing_issue_reports_issue_not_found_with_exit_3() {
    let repository = JitCli::initialised().unwrap();

    for command in READ_COMMANDS {
        let (json_exit, human_exit, envelope) =
            exit_codes(&repository, &["issue", command, "deadbeef"]);

        assert_eq!(envelope["error"]["code"], "ISSUE_NOT_FOUND", "{command}");
        assert_eq!((json_exit, human_exit), (3, 3), "{command}");
    }
}

#[test]
fn test_issue_show_missing_item_reports_item_not_found() {
    let repository = JitCli::initialised().unwrap();
    let id = repository.create_issue(["--title", "Owner"]).unwrap();
    let address = format!("{}/REQ-99", &id[..8]);

    let (json_exit, human_exit, envelope) = exit_codes(&repository, &["issue", "show", &address]);

    assert_eq!(envelope["error"]["code"], "ITEM_NOT_FOUND");
    assert_eq!(json_exit, human_exit);
}
