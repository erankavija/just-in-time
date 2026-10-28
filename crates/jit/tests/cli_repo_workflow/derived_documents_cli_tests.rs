//! Integration tests for documents an issue owns by location: the files under
//! the directory it owns in a declared issue-scoped area, with no explicit
//! link. The repository, issue, and area fixtures come from the sibling
//! `doc_show_tests` module, and each owned directory is whatever `jit doc dir`
//! resolves, so no expectation here restates the naming rule.

use crate::doc_show_tests::{declared_area, declared_config};
use crate::linked_checkout_write_policy_tests::store_bytes;
use jit::domain::identity::IssueIdStaging;
use jit::test_cli::JitCli;
use jit::test_utils::declaration_dir;
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Write `content` at the repository-relative `path`, creating its parents.
fn write_file(repo: &Path, path: &str, content: &str) {
    let absolute = repo.join(path);
    fs::create_dir_all(absolute.parent().expect("a file sits in a directory")).unwrap();
    fs::write(absolute, content).unwrap();
}

/// Run `args` with `--json` and parse stdout, whatever the exit status.
fn json_output(cli: &JitCli, args: &[&str]) -> Value {
    let args = args.iter().copied().chain(["--json"]).collect::<Vec<_>>();
    let output = cli.command(&args).output().unwrap();
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "jit {args:?} prints JSON ({error}): {} {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// The paths `doc list --json` reports for `id`.
fn listed_paths(cli: &JitCli, id: &str) -> Vec<String> {
    let listing = json_output(cli, &["doc", "list", id]);
    listing["documents"]
        .as_array()
        .unwrap_or_else(|| panic!("doc list wraps a documents collection: {listing}"))
        .iter()
        .map(|document| document["path"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn test_doc_list_includes_every_file_under_the_issue_artifact_directory_without_a_link() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let issue = cli
        .create_issue(["--title", "Owner of derived documents"])
        .unwrap();
    let directory = cli
        .run(["doc", "dir", issue.short_id(), declared_area()])
        .unwrap()
        .trim()
        .to_string();
    let owned = [
        format!("{directory}/plan.md"),
        format!("{directory}/notes/sketch.txt"),
    ];
    let flat = format!("{}/{}-plan.md", declared_area(), issue.short_id());
    owned
        .iter()
        .chain([&flat])
        .for_each(|path| write_file(cli.root(), path, "# artifact\n"));

    let listed = listed_paths(&cli, issue.short_id());

    owned.iter().for_each(|path| {
        assert!(listed.contains(path), "doc list names {path}: {listed:?}");
    });
    assert!(
        !listed.contains(&flat),
        "a file outside the owned directory needs a link: {listed:?}"
    );
}

#[test]
fn test_doc_check_links_checks_and_tracks_a_derived_document() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let issue = cli
        .create_issue(["--title", "Owner of derived documents"])
        .unwrap();
    let directory = cli
        .run(["doc", "dir", issue.short_id(), declared_area()])
        .unwrap()
        .trim()
        .to_string();
    let plan = format!("{directory}/plan.md");
    let design = format!("{directory}/design.md");
    write_file(
        cli.root(),
        &plan,
        "[design](design.md)\n[gone](missing.md)\n",
    );
    write_file(cli.root(), &design, "# design\n");

    let scope = format!("issue:{}", issue.short_id());
    let report = json_output(&cli, &["doc", "check-links", "--scope", &scope]);
    let findings = |kind: &str| -> Vec<Value> {
        report[kind]
            .as_array()
            .unwrap_or_else(|| panic!("check-links reports {kind}: {report}"))
            .clone()
    };

    assert!(
        findings("errors")
            .iter()
            .any(|error| error["document"] == plan.as_str() && error["link"] == "missing.md"),
        "the derived document's links are checked: {report}"
    );
    assert!(
        findings("warnings")
            .iter()
            .all(|warning| warning["link"] != "design.md"),
        "a link to a derived document targets a tracked document: {report}"
    );
}

#[test]
fn test_doc_show_displays_a_derived_document() {
    let cli = JitCli::with_repository(
        declaration_dir(&[(".jit/config.toml", &declared_config())]).unwrap(),
    );
    cli.run(["init", "--json"]).unwrap();
    let issue = cli
        .create_issue(["--title", "Owner of derived documents"])
        .unwrap();
    let directory = cli
        .run(["doc", "dir", issue.short_id(), declared_area()])
        .unwrap()
        .trim()
        .to_string();
    let plan = format!("{directory}/plan.md");
    write_file(cli.root(), &plan, "derived plan body\n");

    let output = cli
        .command(["doc", "show", issue.short_id(), &plan])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "doc show {plan}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("derived plan body"));
}

#[test]
fn test_doc_list_includes_artifacts_merged_from_a_linked_worktree_without_a_tracker_write() {
    let (primary, linked) = JitCli::linked_checkouts().unwrap();
    let area = "workspace/drafts";
    let config = primary.root().join(".jit/config.toml");
    let declared = fs::read_to_string(&config).unwrap()
        + &format!("\n[documentation]\nissue_scoped_areas = [\"{area}\"]\n");
    fs::write(&config, declared).unwrap();
    let issue = primary
        .create_issue(["--title", "Owner of merged artifacts"])
        .unwrap();
    let artifact = format!(
        "{}/report.md",
        primary
            .run(["doc", "dir", issue.short_id(), area])
            .unwrap()
            .trim()
    );

    write_file(linked.root(), &artifact, "# worker report\n");
    linked.git(["add", artifact.as_str()]).unwrap();
    linked.git(["commit", "-qm", "worker artifact"]).unwrap();
    let branch = linked.git(["branch", "--show-current"]).unwrap();

    assert!(!listed_paths(&primary, issue.short_id()).contains(&artifact));
    let store_before_merge = store_bytes(primary.root());
    primary
        .git(["merge", "-q", "--ff-only", branch.trim()])
        .unwrap();

    let listed = listed_paths(&primary, issue.short_id());
    assert!(
        listed.contains(&artifact),
        "the merged artifact is listed: {listed:?}"
    );
    assert_eq!(
        store_bytes(primary.root()),
        store_before_merge,
        "listing the merged artifact writes nothing to the tracker"
    );
}
