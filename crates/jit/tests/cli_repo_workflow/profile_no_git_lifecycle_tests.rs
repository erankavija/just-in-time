//! One public profile-lifecycle journey through a repository Git never touches.

use super::profile_cli_tests::{
    applied_ids, jit, json, live_asset_package, only_profile, requested_profile,
    write_lifecycle_package,
};
use jit::test_cli::JitCli;
use serde_json::Value;
use std::path::Path;
use std::process::Output;

fn assert_never_has_git(repo: &Path, phase: &str) {
    assert!(
        !repo.join(".git").exists(),
        "{phase} created or required a version-control directory"
    );
}

fn successful_lifecycle_json(cli: &JitCli, phase: &str, args: &[&str]) -> Value {
    assert_never_has_git(cli.root(), &format!("before {phase}"));
    let output = jit(cli, args);
    assert!(
        output.status.success(),
        "{phase}: jit {args:?} failed\nstatus={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_no_version_control_error(&output, phase);
    assert_never_has_git(cli.root(), &format!("after {phase}"));
    json(&output)
}

fn assert_no_version_control_error(output: &Output, phase: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    assert!(
        !stderr.contains("version-control") && !stderr.contains("version control"),
        "{phase} emitted a version-control error: {stderr}"
    );
}

/// The public CLI lifecycle completes its full package journey in a repository
/// created without `.git` (jit:8021d507).
#[test]
fn test_profile_lifecycle_completes_without_git() {
    let cli = JitCli::uninitialised().unwrap();
    let repo = cli.root();
    assert_never_has_git(repo, "new repository");

    successful_lifecycle_json(&cli, "initialize", &["init", "--json"]);

    let fixture = jit::test_utils::profile_package_fixture("planner-asset-only");
    jit::test_utils::write_package_declaring(
        &fixture,
        &repo.join("packages/gitless-base"),
        "gitless-base",
        &[],
    );
    jit::test_utils::write_package_declaring(
        &fixture,
        &repo.join("packages/gitless-workflow"),
        "gitless-workflow",
        &["gitless-base"],
    );
    let workflow_selector = "packages/gitless-workflow";
    let shown = successful_lifecycle_json(
        &cli,
        "select workflow package",
        &["profile", "show", "--profile", workflow_selector, "--json"],
    );
    assert_eq!(shown["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(shown["profiles"][0]["manifest"]["id"], "gitless-workflow");

    let resolved = successful_lifecycle_json(
        &cli,
        "resolve and apply workflow dependency closure",
        &["profile", "apply", "--profile", workflow_selector, "--json"],
    );
    assert_eq!(resolved["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(
        applied_ids(&resolved),
        vec!["gitless-base", "gitless-workflow"]
    );

    let lifecycle_id = "gitless-lifecycle";
    write_lifecycle_package(repo, "packages/lifecycle", lifecycle_id, "1.0.0");
    let lifecycle_selector = "packages/lifecycle";
    let lifecycle = successful_lifecycle_json(
        &cli,
        "apply lifecycle package",
        &[
            "profile",
            "apply",
            "--profile",
            lifecycle_selector,
            "--json",
        ],
    );
    assert_eq!(requested_profile(&lifecycle)["status"], "applied");

    live_asset_package(repo, "packages/capture-source");
    let packed = successful_lifecycle_json(
        &cli,
        "pack profile package",
        &[
            "profile",
            "pack",
            "--source",
            "packages/capture-source",
            "--output",
            "packages/packed.tar",
            "--json",
        ],
    );
    assert!(repo.join("packages/packed.tar").is_file());

    let added = successful_lifecycle_json(
        &cli,
        "add packed profile package",
        &[
            "profile",
            "add",
            "--archive",
            "packages/packed.tar",
            "--destination",
            "packages/imported",
            "--json",
        ],
    );
    assert_eq!(
        only_profile(&added)["package_hash"],
        only_profile(&packed)["package_hash"]
    );
    let imported_selector = "packages/imported";
    let imported = successful_lifecycle_json(
        &cli,
        "select added profile package",
        &["profile", "show", "--profile", imported_selector, "--json"],
    );
    assert_eq!(imported["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(
        imported["profiles"][0]["package_hash"],
        only_profile(&packed)["package_hash"]
    );

    let difference = successful_lifecycle_json(
        &cli,
        "report lifecycle package difference",
        &["profile", "diff", "--profile", lifecycle_selector, "--json"],
    );
    assert_eq!(difference["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(difference["profiles"][0]["id"], lifecycle_id);

    successful_lifecycle_json(&cli, "validate repository", &["validate", "--json"]);
}
