//! One public profile-lifecycle journey through a repository Git never touches.

use super::profile_cli_tests::{
    applied_ids, capture_sources, id_selector, jit, json, only_profile, path_selector,
    requested_profile, write_lifecycle_package,
};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

fn assert_never_has_git(repo: &Path, phase: &str) {
    assert!(
        !repo.join(".git").exists(),
        "{phase} created or required a version-control directory"
    );
}

fn successful_lifecycle_json(repo: &Path, phase: &str, args: &[&str]) -> Value {
    assert_never_has_git(repo, &format!("before {phase}"));
    let output = jit(repo, args);
    assert!(
        output.status.success(),
        "{phase}: jit {args:?} failed\nstatus={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_no_version_control_error(&output, phase);
    assert_never_has_git(repo, &format!("after {phase}"));
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
    let repo = TempDir::new().expect("create repository without git");
    let repo = repo.path();
    assert_never_has_git(repo, "new repository");

    successful_lifecycle_json(repo, "initialize", &["init", "--json"]);

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
    let workflow_selector = path_selector("packages/gitless-workflow");
    let shown = successful_lifecycle_json(
        repo,
        "select workflow package",
        &["profile", "show", "--profile", &workflow_selector, "--json"],
    );
    assert_eq!(shown["count"], 1);
    assert_eq!(shown["profiles"][0]["manifest"]["id"], "gitless-workflow");

    let resolved = successful_lifecycle_json(
        repo,
        "resolve and apply workflow dependency closure",
        &[
            "profile",
            "apply",
            "--profile",
            &workflow_selector,
            "--json",
        ],
    );
    assert_eq!(resolved["count"], 2);
    assert_eq!(
        applied_ids(&resolved),
        vec!["gitless-base", "gitless-workflow"]
    );
    for id in ["gitless-base", "gitless-workflow"] {
        assert!(
            repo.join(format!(".jit/profiles/{id}.json")).is_file(),
            "dependency resolution did not record {id}"
        );
    }

    let lifecycle_id = "gitless-lifecycle";
    write_lifecycle_package(repo, "packages/lifecycle", lifecycle_id, "1.0.0");
    let lifecycle_selector = path_selector("packages/lifecycle");
    let lifecycle = successful_lifecycle_json(
        repo,
        "apply lifecycle package",
        &[
            "profile",
            "apply",
            "--profile",
            &lifecycle_selector,
            "--json",
        ],
    );
    assert_eq!(requested_profile(&lifecycle)["status"], "applied");

    let lifecycle_id_selector = id_selector(lifecycle_id);
    let reconfigured = successful_lifecycle_json(
        repo,
        "reconfigure lifecycle package",
        &[
            "profile",
            "reconfigure",
            "--profile",
            &lifecycle_id_selector,
            "--set",
            "GREETING=gitless",
            "--json",
        ],
    );
    assert_eq!(only_profile(&reconfigured)["status"], "applied");
    assert_eq!(
        fs::read_to_string(repo.join(format!("docs/{lifecycle_id}-templated.txt")))
            .expect("read reconfigured target"),
        "greeting=gitless\n"
    );

    write_lifecycle_package(repo, "packages/lifecycle", lifecycle_id, "2.0.0");
    let upgraded = successful_lifecycle_json(
        repo,
        "upgrade lifecycle package",
        &[
            "profile",
            "upgrade",
            "--profile",
            &lifecycle_id_selector,
            "--json",
        ],
    );
    assert_eq!(only_profile(&upgraded)["version"], "2.0.0");

    capture_sources(repo, "packages/capture-source");
    let captured = successful_lifecycle_json(
        repo,
        "capture profile package",
        &[
            "profile",
            "capture",
            "--source",
            "packages/capture-source",
            "--destination",
            "packages/captured",
            "--json",
        ],
    );
    assert_eq!(only_profile(&captured)["destination"], "packages/captured");
    assert_eq!(
        fs::read_to_string(repo.join("packages/captured/assets/live/docs/guide.md"))
            .expect("read captured live asset"),
        "# Captured guide\n"
    );

    let packed = successful_lifecycle_json(
        repo,
        "pack captured profile package",
        &[
            "profile",
            "pack",
            "--source",
            "packages/captured",
            "--output",
            "packages/captured.tar",
            "--json",
        ],
    );
    assert!(repo.join("packages/captured.tar").is_file());

    let added = successful_lifecycle_json(
        repo,
        "add packed profile package",
        &[
            "profile",
            "add",
            "--archive",
            "packages/captured.tar",
            "--destination",
            "packages/imported",
            "--json",
        ],
    );
    assert_eq!(
        only_profile(&added)["package_hash"],
        only_profile(&packed)["package_hash"]
    );
    let imported_selector = path_selector("packages/imported");
    let imported = successful_lifecycle_json(
        repo,
        "select added profile package",
        &["profile", "show", "--profile", &imported_selector, "--json"],
    );
    assert_eq!(imported["count"], 1);
    assert_eq!(
        imported["profiles"][0]["package_hash"],
        only_profile(&packed)["package_hash"]
    );

    let difference = successful_lifecycle_json(
        repo,
        "report lifecycle package difference",
        &[
            "profile",
            "diff",
            "--profile",
            &lifecycle_id_selector,
            "--json",
        ],
    );
    assert_eq!(difference["count"], 1);
    assert_eq!(difference["profiles"][0]["id"], lifecycle_id);

    let agreement = successful_lifecycle_json(
        repo,
        "check applied profiles",
        &["profile", "validate", "--json"],
    );
    assert!(agreement["profiles"]
        .as_array()
        .expect("profile check reports profiles")
        .iter()
        .all(|profile| profile["divergences"] == serde_json::json!([])));
    successful_lifecycle_json(repo, "validate repository", &["validate", "--json"]);
}
