//! Integration tests for type hierarchy auto-fix functionality

use jit::test_cli::JitCli;
use jit::test_utils::profiled_repository_fixture;

#[test]
fn test_fix_unknown_type_with_suggestion() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    // A typo in the type label: should be "task".
    let issue_id = cli
        .create_issue(["--title", "Fix bug", "--label", "type:taks"])
        .unwrap();

    let output = cli.command(["validate"]).output().unwrap();
    assert!(
        !output.status.success(),
        "Validation should fail with unknown type"
    );

    let stdout = cli
        .run(["validate", "--fix", "--dry-run"])
        .expect("dry run succeeds");
    assert!(stdout.contains("taks"));
    assert!(stdout.contains("task"));
    assert!(stdout.contains("Would replace"));

    let stdout = cli.run(["validate", "--fix"]).expect("fix succeeds");
    assert!(stdout.contains("Applied") || stdout.contains("fix"));

    let json = cli.run_json(["issue", "show", &issue_id]).unwrap();
    let labels = json["labels"].as_array().unwrap();
    assert!(labels.iter().any(|l| l.as_str() == Some("type:task")));
    assert!(!labels.iter().any(|l| l.as_str() == Some("type:taks")));

    cli.run(["validate"])
        .expect("validation passes after the fix");
}

#[test]
fn test_fix_unknown_type_no_suggestion() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    cli.create_issue([
        "--title",
        "Unknown work",
        "--label",
        "type:completely_unknown_xyz",
    ])
    .unwrap();

    // Does not crash, but finds nothing to fix.
    let output = cli.command(["validate", "--fix"]).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no automatic fixes") || stdout.contains("No fixes"));
}

#[test]
fn test_fix_multiple_issues() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    let id1 = cli
        .create_issue(["--title", "Issue 1", "--label", "type:taks"])
        .unwrap();
    let id2 = cli
        .create_issue(["--title", "Issue 2", "--label", "type:epik"])
        .unwrap();
    // Connect the issues to avoid an isolated-node error.
    cli.run(["dep", "add", &id2, &id1]).unwrap();

    let stdout = cli.run(["validate", "--fix"]).unwrap();
    assert!(stdout.contains("2") || stdout.contains("Applied"));

    cli.run(["validate"]).unwrap();
}

#[test]
fn test_dry_run_doesnt_modify() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    let issue_id = cli
        .create_issue(["--title", "Test", "--label", "type:taks"])
        .unwrap();

    cli.run(["validate", "--fix", "--dry-run"]).unwrap();

    let json = cli.run_json(["issue", "show", &issue_id]).unwrap();
    let labels = json["labels"].as_array().unwrap();
    assert!(labels.iter().any(|l| l.as_str() == Some("type:taks")));
    assert!(!labels.iter().any(|l| l.as_str() == Some("type:task")));

    let output = cli.command(["validate"]).output().unwrap();
    assert!(!output.status.success());
}

#[test]
fn test_fix_json_output() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    cli.create_issue(["--title", "Test", "--label", "type:taks"])
        .unwrap();

    let json = cli.run_json(["validate", "--fix"]).unwrap();

    assert!(json["fixes_applied"].as_u64().unwrap() > 0);
    assert_eq!(json["dry_run"], false);
}

#[test]
fn test_dry_run_requires_fix_flag() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );

    let output = cli.command(["validate", "--dry-run"]).output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("--dry-run requires --fix"));
}

#[test]
fn test_fix_with_valid_repository() {
    let cli = JitCli::with_repository(
        profiled_repository_fixture(crate::DEFAULT_PACKAGE, "packages").unwrap(),
    );
    cli.create_issue(["--title", "Valid issue", "--label", "type:task"])
        .unwrap();

    let stdout = cli.run(["validate", "--fix"]).unwrap();
    assert!(stdout.contains("No fixes") || stdout.contains("valid"));
}
