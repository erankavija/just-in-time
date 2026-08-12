use jit::domain::artifact_plan::{ArchiveCandidates, ArtifactPlan};
use jit::output::{render_archive_candidates, render_archive_plan};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn jit(repo: &TempDir, args: &[&str]) -> std::process::Output {
    Command::new(assert_cmd::cargo::cargo_bin!("jit"))
        .current_dir(repo.path())
        .args(args)
        .output()
        .unwrap()
}

fn snapshot_files(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<String, Vec<u8>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path.is_dir() {
                visit(root, &path, result);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "lock")
            {
                // Advisory machine-local lock files are not repository artifacts.
                continue;
            } else {
                result.insert(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .to_string(),
                    fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, Option<Vec<u8>>> {
    fn visit(root: &Path, current: &Path, result: &mut BTreeMap<String, Option<Vec<u8>>>) {
        let mut entries = fs::read_dir(current)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.path());
        for entry in entries {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|extension| extension == "lock")
            {
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            if path.is_dir() {
                result.insert(relative, None);
                visit(root, &path, result);
            } else {
                result.insert(relative, Some(fs::read(path).unwrap()));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// The type vocabulary these cases name: a container type carrying a
/// membership namespace, and the leaf beneath it.
///
/// A repository declares its own vocabulary, and these cases address containers
/// by their membership label, so the fixture declares the types and the
/// association that label rides on.
const TAXONOMY: &str = r#"
[type_hierarchy]
types = { epic = 1, task = 2 }
strategic_types = ["epic"]

[type_hierarchy.label_associations]
epic = "epic"

[namespaces.type]
description = "Issue type"
unique = true

[namespaces.epic]
description = "Epic membership"
unique = false
"#;

/// Replace an initialized repository's `[documentation]` table.
///
/// Only the classification changes, and no rule or schema is derived from it,
/// so a case whose subject is reclassifying an existing repository rewrites the
/// table in place.
fn set_documentation_policy(repo: &TempDir, policy: &str) {
    let path = repo.path().join(".jit/config.toml");
    let mut config = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    config.extend(policy.parse::<toml::Table>().unwrap());
    fs::write(&path, config.to_string()).unwrap();
}

/// Initialize `repo` declaring the shared taxonomy and `policy`.
///
/// The configuration is written before initialization, which preserves it and
/// derives the coupled rules and schemas from the registry it declares.
fn initialize_with_policy(repo: &TempDir, policy: &str) {
    let jit_dir = repo.path().join(".jit");
    fs::create_dir_all(&jit_dir).unwrap();
    fs::write(jit_dir.join("config.toml"), format!("{TAXONOMY}\n{policy}")).unwrap();
    assert_success(&jit(repo, &["init", "--json"]));
}

/// A bundle repository that declares no development-root boundary, so its
/// `fixtures` area classifies by the managed/permanent split alone.
///
/// The empty `development_root` is deliberate: an omitted key falls back to the
/// shipped `dev` root, which would place this whole fixture outside the
/// boundary and retain every artifact, leaving nothing to archive. The tests
/// whose subject is that boundary configure a real root of their own.
fn configured_bundle(repo: &TempDir) {
    initialize_with_policy(
        repo,
        r#"
[documentation]
development_root = ""
managed_paths = ["fixtures"]
permanent_paths = []
archive_root = "archive"
"#,
    );
    fs::create_dir_all(repo.path().join("fixtures/bundle/theme")).unwrap();
    fs::write(
        repo.path().join("fixtures/root.md"),
        concat!(
            "[page](bundle/index.html) ",
            "[data](data.csv) ",
            "![png](bundle/image.png) ",
            "![svg](bundle/image.svg)"
        ),
    )
    .unwrap();
    fs::write(
        repo.path().join("fixtures/bundle/index.html"),
        r#"<link href="theme/base.css"><img src="image.svg">"#,
    )
    .unwrap();
    fs::write(
        repo.path().join("fixtures/bundle/theme/base.css"),
        r#"@import "nested.css"; body { background: url("../image.png") }"#,
    )
    .unwrap();
    fs::write(
        repo.path().join("fixtures/bundle/theme/nested.css"),
        "a { color: black }",
    )
    .unwrap();
    fs::write(repo.path().join("fixtures/data.csv"), "name,value\na,1\n").unwrap();
    fs::write(repo.path().join("fixtures/bundle/image.png"), b"png-bytes").unwrap();
    fs::write(
        repo.path().join("fixtures/bundle/image.svg"),
        "<svg viewBox=\"0 0 1 1\"/>",
    )
    .unwrap();
}

fn expected_bundle_decisions(
    target_kind: &str,
) -> [(
    &'static str,
    Option<&'static str>,
    &'static str,
    &'static str,
); 7] {
    let (png, svg, base, nested) = if target_kind == "document" {
        ("copy", "copy", "copy", "copy")
    } else {
        ("move", "move", "move", "move")
    };
    let explicit_bundle_member = if target_kind == "container" {
        "explicit"
    } else {
        "embedded"
    };
    [
        (
            "fixtures/bundle/image.png",
            None,
            png,
            explicit_bundle_member,
        ),
        (
            "fixtures/bundle/image.svg",
            None,
            svg,
            explicit_bundle_member,
        ),
        (
            "fixtures/bundle/index.html",
            Some("html"),
            "move",
            explicit_bundle_member,
        ),
        (
            "fixtures/bundle/theme/base.css",
            Some("css"),
            base,
            explicit_bundle_member,
        ),
        (
            "fixtures/bundle/theme/nested.css",
            Some("css"),
            nested,
            "embedded",
        ),
        ("fixtures/data.csv", None, "move", explicit_bundle_member),
        ("fixtures/root.md", Some("markdown"), "move", "explicit"),
    ]
}

fn assert_exact_bundle_plan(plan: &Value, expected_target_kind: &str) {
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["target"]["kind"], expected_target_kind);
    assert_eq!(plan["policy_status"], "configured");
    assert_eq!(plan["eligible"], true);
    assert_eq!(plan["count"], 7);
    assert_eq!(plan["blockers"], serde_json::json!([]));
    assert_eq!(plan["warnings"], serde_json::json!([]));
    let (moves, copies, pending_deletions) = if expected_target_kind == "document" {
        (3, 4, 3)
    } else {
        (7, 0, 7)
    };
    assert_eq!(
        plan["action_counts"],
        serde_json::json!({
            "move": moves,
            "copy": copies,
            "retain": 0,
            "block": 0,
            "already_archived": 0,
            "pending_deletions": pending_deletions
        })
    );
    let decisions = plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|artifact| {
            (
                artifact["source"].as_str().unwrap(),
                artifact["format"].as_str(),
                artifact["action"].as_str().unwrap(),
                artifact["provenance"].clone(),
                artifact["blockers"].clone(),
                artifact["warnings"].clone(),
            )
        })
        .collect::<Vec<_>>();
    let expected = expected_bundle_decisions(expected_target_kind)
        .into_iter()
        .map(|(source, format, action, provenance)| {
            (
                source,
                format,
                action,
                serde_json::json!([provenance]),
                serde_json::json!([]),
                serde_json::json!([]),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(decisions, expected);
}

fn assert_human_parity(plan: &Value, human: &str) {
    assert!(human.contains(&format!(
        "Policy: {}",
        plan["policy_status"].as_str().unwrap()
    )));
    assert!(human.contains(&format!(
        "Eligible for execution: {}",
        plan["eligible"].as_bool().unwrap()
    )));
    assert!(human.contains(&format!(
        "Destination root: {}",
        plan["destination_root"].as_str().unwrap()
    )));
    for artifact in plan["artifacts"].as_array().unwrap() {
        let source = artifact["source"].as_str().unwrap();
        let action = artifact["action"].as_str().unwrap();
        let version = artifact["version"].as_str().unwrap();
        let destination = artifact["destination"].as_str().unwrap_or("-");
        let format = artifact["format"].as_str().unwrap_or("opaque");
        assert!(
            human.contains(&format!("- [{action}] {source} @ {version}")),
            "human preview omitted JSON action for {source}:\n{human}"
        );
        assert!(human.contains(&format!("destination: {destination}")));
        assert!(human.contains(&format!("format: {format}")));
        for blocker in artifact["blockers"].as_array().unwrap() {
            assert!(human.contains(blocker["code"].as_str().unwrap()));
        }
        for warning in artifact["warnings"].as_array().unwrap() {
            assert!(human.contains(warning["code"].as_str().unwrap()));
        }
    }
}

#[test]
fn test_archive_document_cli_json_is_exact_plan_and_blocked_preview_exits_zero_without_mutation() {
    let repo = TempDir::new().unwrap();
    assert!(jit(&repo, &["init", "--json"]).status.success());
    fs::write(repo.path().join(".jit/config.toml"), "").unwrap();
    fs::write(repo.path().join("root.csv"), "name,value\na,1\n").unwrap();
    let before = snapshot_tree(repo.path());

    let json_output = jit(&repo, &["archive", "document", "root.csv", "--json"]);
    assert!(
        json_output.status.success(),
        "{}",
        String::from_utf8_lossy(&json_output.stderr)
    );
    let plan: Value = serde_json::from_slice(&json_output.stdout).unwrap();
    assert_eq!(plan["schema_version"], 1);
    assert_eq!(plan["target"]["kind"], "document");
    assert_eq!(plan["target"]["path"], "root.csv");
    assert_eq!(plan["policy_status"], "unconfigured");
    assert_eq!(plan["eligible"], false);
    assert!(plan.get("message").is_none());
    let keys = plan
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "action_counts",
            "artifacts",
            "blockers",
            "count",
            "destination_root",
            "eligible",
            "policy_status",
            "schema_version",
            "target",
            "warnings",
        ]
    );

    let human_output = jit(&repo, &["archive", "document", "root.csv"]);
    assert!(human_output.status.success());
    let human = String::from_utf8(human_output.stdout).unwrap();
    assert!(human.contains("Archive preview: document root.csv"));
    assert!(human.contains("Policy: unconfigured"));
    assert!(human.contains("policy-unconfigured: target"));
    let action = plan["artifacts"][0]["action"].as_str().unwrap();
    assert!(human.contains(&format!("[{action}] root.csv @ working-tree")));
    assert!(human.contains("Archival execution is disabled"));

    assert_eq!(snapshot_tree(repo.path()), before);
    assert!(!repo.path().join("archive").exists());
}

#[test]
fn test_configured_archive_document_and_container_cli_are_exact_deterministic_and_non_mutating() {
    let repo = TempDir::new().unwrap();
    configured_bundle(&repo);

    let create = jit(
        &repo,
        &[
            "issue",
            "create",
            "--title",
            "Archived fixture",
            "--type",
            "epic",
            "--json",
        ],
    );
    assert_success(&create);
    let created: Value = serde_json::from_slice(&create.stdout).unwrap();
    let id = created["id"].as_str().unwrap().to_string();
    for path in [
        "fixtures/root.md",
        "fixtures/bundle/index.html",
        "fixtures/bundle/theme/base.css",
        "fixtures/data.csv",
        "fixtures/bundle/image.png",
        "fixtures/bundle/image.svg",
    ] {
        assert_success(&jit(
            &repo,
            &["doc", "add", &id, path, "--skip-scan", "--json"],
        ));
    }
    assert_success(&jit(
        &repo,
        &["issue", "update", &id, "--state", "rejected", "--json"],
    ));
    let before = snapshot_files(repo.path());

    for (args, target_kind) in [
        (
            vec!["archive", "document", "fixtures/root.md", "--json"],
            "document",
        ),
        (
            vec!["archive", "container", id.as_str(), "--json"],
            "container",
        ),
    ] {
        let first = jit(&repo, &args);
        let second = jit(&repo, &args);
        assert_success(&first);
        assert_success(&second);
        assert_eq!(
            first.stdout, second.stdout,
            "JSON preview must be byte stable"
        );
        let plan: Value = serde_json::from_slice(&first.stdout).unwrap();
        assert_exact_bundle_plan(&plan, target_kind);

        let human_args = &args[..args.len() - 1];
        let human_output = jit(&repo, human_args);
        assert_success(&human_output);
        let human = String::from_utf8(human_output.stdout).unwrap();
        assert_human_parity(&plan, &human);
        let typed_plan: ArtifactPlan = serde_json::from_slice(&first.stdout).unwrap();
        assert_eq!(human, render_archive_plan(&typed_plan));
    }

    assert_eq!(snapshot_files(repo.path()), before);
    assert!(!repo.path().join("archive").exists());
}

#[test]
fn test_archive_execute_is_explicit_and_available_for_document_and_container_targets() {
    let document_repo = TempDir::new().unwrap();
    configured_bundle(&document_repo);
    let document = jit(
        &document_repo,
        &[
            "archive",
            "document",
            "fixtures/root.md",
            "--execute",
            "--json",
        ],
    );
    assert_success(&document);
    let result: Value = serde_json::from_slice(&document.stdout).unwrap();
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["target"]["kind"], "document");
    assert_eq!(result["event_appended"], true);
    assert_eq!(result["reconciling"], false);
    assert!(result["publications"].is_array());
    assert!(result["reference_changes"].is_array());
    assert!(result["planned_deletions"].is_array());
    assert!(result["deleted_sources"].is_array());
    assert!(result["warnings"].is_array());
    assert_eq!(
        result
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "deleted_sources",
            "destination_root",
            "event_appended",
            "planned_deletions",
            "publications",
            "reconciling",
            "reference_changes",
            "schema_version",
            "target",
            "warnings",
        ]
    );
    assert!(document_repo
        .path()
        .join("archive/fixtures/root.md")
        .exists());

    let container_repo = TempDir::new().unwrap();
    initialize_with_policy(
        &container_repo,
        "[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
    );
    fs::create_dir(container_repo.path().join("fixtures")).unwrap();
    fs::write(container_repo.path().join("fixtures/root.md"), "container").unwrap();
    let created = jit(
        &container_repo,
        &[
            "issue",
            "create",
            "--title",
            "Container",
            "--type",
            "epic",
            "--json",
        ],
    );
    assert_success(&created);
    let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_success(&jit(
        &container_repo,
        &[
            "doc",
            "add",
            &id,
            "fixtures/root.md",
            "--skip-scan",
            "--json",
        ],
    ));
    assert_success(&jit(
        &container_repo,
        &["issue", "update", &id, "--state", "rejected", "--json"],
    ));
    let container = jit(
        &container_repo,
        &["archive", "container", &id, "--execute", "--json"],
    );
    assert_success(&container);
    let result: Value = serde_json::from_slice(&container.stdout).unwrap();
    assert_eq!(result["target"]["kind"], "container");
    assert_eq!(result["event_appended"], true);
    let destination_root = format!("archive/{}", &id[..8]);
    assert_eq!(
        fs::read_to_string(
            container_repo
                .path()
                .join(&destination_root)
                .join(".jit-container")
        )
        .unwrap(),
        format!("{id}\n")
    );
    assert!(container_repo
        .path()
        .join(destination_root)
        .join("fixtures/root.md")
        .exists());
}

#[test]
fn test_archive_execute_leaves_an_out_of_root_source_where_it_is_without_scheduling_a_destination()
{
    // A linked file outside the configured development root is retained where
    // it is (`@/issue/8e071e18/decision/D-14`): the plan schedules no
    // destination for it and execution neither relocates nor duplicates it.
    // Guard that at both the plan level and the execution level, pinned
    // together so the retention cannot be satisfied by silently dropping the
    // artifact from the plan.
    let repo = TempDir::new().unwrap();
    initialize_with_policy(
        &repo,
        concat!(
            "[documentation]\n",
            "development_root = \"workspace\"\n",
            "managed_paths = [\"workspace/active\"]\n",
            "permanent_paths = []\n",
            "archive_root = \"workspace/archive\"\n",
        ),
    );
    fs::create_dir_all(repo.path().join("scripts")).unwrap();
    let source_bytes = b"#!/bin/sh\necho archived\n".to_vec();
    fs::write(repo.path().join("scripts/install.sh"), &source_bytes).unwrap();

    // The criterion is scoped to a *linked* artifact, so give the script a
    // terminal owner. Without a document reference the terminal-owner check is
    // vacuously true and the scenario REQ-01 names is never exercised.
    let owner = jit(
        &repo,
        &[
            "issue",
            "create",
            "--title",
            "Owner of the installer script",
            "--json",
        ],
    );
    assert_success(&owner);
    let owner_id = serde_json::from_slice::<Value>(&owner.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_success(&jit(
        &repo,
        &[
            "doc",
            "add",
            &owner_id,
            "scripts/install.sh",
            "--skip-scan",
            "--json",
        ],
    ));
    assert_success(&jit(
        &repo,
        &["issue", "update", &owner_id, "--state", "done", "--json"],
    ));

    let preview = jit(
        &repo,
        &["archive", "document", "scripts/install.sh", "--json"],
    );
    assert_success(&preview);
    let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(plan["eligible"], true);
    let destination_root = plan["destination_root"].as_str().unwrap().to_string();
    let expected_mirror = format!("{destination_root}/scripts/install.sh");
    let artifact = plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["source"] == "scripts/install.sh")
        .unwrap();
    assert_eq!(artifact["action"], "retain");
    assert_eq!(artifact["destination"], Value::Null);
    assert_eq!(
        artifact["pending_deletions"],
        serde_json::json!([]),
        "an out-of-root source must not schedule a pending deletion"
    );

    let executed = jit(
        &repo,
        &[
            "archive",
            "document",
            "scripts/install.sh",
            "--execute",
            "--json",
        ],
    );
    assert_success(&executed);
    let result: Value = serde_json::from_slice(&executed.stdout).unwrap();
    assert!(result["planned_deletions"].as_array().unwrap().is_empty());
    assert!(result["deleted_sources"].as_array().unwrap().is_empty());
    assert!(
        result["publications"].as_array().unwrap().is_empty(),
        "execution must publish nothing for a retained out-of-root source: {result}"
    );

    let source_path = repo.path().join("scripts/install.sh");
    assert!(
        source_path.exists(),
        "source outside the development root must remain at its original path"
    );
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes);
    assert!(
        !repo.path().join(&expected_mirror).exists(),
        "execution must not duplicate a retained out-of-root source into the archive"
    );
}

#[test]
fn test_archive_execute_blocked_is_nonzero_nonmutating_and_human_reports_warnings() {
    let blocked_repo = TempDir::new().unwrap();
    assert_success(&jit(&blocked_repo, &["init", "--json"]));
    fs::write(blocked_repo.path().join(".jit/config.toml"), "").unwrap();
    fs::write(blocked_repo.path().join("root.md"), "blocked").unwrap();
    let before = snapshot_files(blocked_repo.path());
    let blocked = jit(
        &blocked_repo,
        &["archive", "document", "root.md", "--execute", "--json"],
    );
    assert!(!blocked.status.success());
    assert_eq!(snapshot_files(blocked_repo.path()), before);
    assert!(!blocked_repo.path().join("archive").exists());

    let warning_repo = TempDir::new().unwrap();
    configured_bundle(&warning_repo);
    let human = jit(
        &warning_repo,
        &["archive", "document", "fixtures/root.md", "--execute"],
    );
    assert_success(&human);
    let stdout = String::from_utf8(human.stdout).unwrap();
    assert!(stdout.contains("Archive execution complete:"));
    assert!(stdout.contains("warning: no-owner (fixtures/root.md)"));
}

#[test]
fn test_archive_candidates_cli_returns_complete_deterministic_plans_with_human_parity() {
    let repo = TempDir::new().unwrap();
    configured_bundle(&repo);
    let create = |title: &str, issue_type: &str| {
        let output = jit(
            &repo,
            &[
                "issue", "create", "--title", title, "--type", issue_type, "--json",
            ],
        );
        assert_success(&output);
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let evaluated = create("Evaluated container", "epic");
    assert_success(&jit(
        &repo,
        &[
            "doc",
            "add",
            &evaluated,
            "fixtures/root.md",
            "--skip-scan",
            "--json",
        ],
    ));
    assert_success(&jit(
        &repo,
        &[
            "issue", "update", &evaluated, "--state", "rejected", "--json",
        ],
    ));
    let empty = create("Zero-document container", "epic");
    assert_success(&jit(
        &repo,
        &["issue", "update", &empty, "--state", "rejected", "--json"],
    ));
    let _active = create("Active container", "epic");
    let leaf = create("Terminal leaf", "task");
    assert_success(&jit(
        &repo,
        &["issue", "update", &leaf, "--state", "rejected", "--json"],
    ));

    set_documentation_policy(
        &repo,
        "[documentation]\nmanaged_paths = [\"fixtures/root.md\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
    );
    let occupied = repo.path().join("archive").join(&evaluated[..8]);
    fs::create_dir_all(&occupied).unwrap();
    fs::write(occupied.join("foreign.txt"), "unaccounted").unwrap();
    let before = snapshot_tree(repo.path());

    let first = jit(&repo, &["archive", "candidates", "--json"]);
    let second = jit(&repo, &["archive", "candidates", "--json"]);
    assert_success(&first);
    assert_success(&second);
    assert_eq!(first.stdout, second.stdout);
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(
        report
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<Vec<_>>(),
        ["candidates", "count", "schema_version"]
    );
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["count"], 2);
    let candidates = report["candidates"].as_array().unwrap();
    let ids = candidates
        .iter()
        .map(|candidate| candidate["target"]["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    let mut expected_ids = vec![empty.as_str(), evaluated.as_str()];
    expected_ids.sort();
    assert_eq!(ids, expected_ids);
    for candidate in candidates {
        assert_eq!(
            candidate
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "action_counts",
                "artifacts",
                "blockers",
                "count",
                "destination_root",
                "eligible",
                "policy_status",
                "schema_version",
                "target",
                "warnings",
            ]
        );
    }
    let evaluated_plan = candidates
        .iter()
        .find(|candidate| candidate["target"]["id"] == evaluated)
        .unwrap();
    assert!(evaluated_plan["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker["code"] == "destination-conflict"));
    let source_retaining = evaluated_plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|artifact| {
            artifact["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|evidence| {
                    evidence == "permanent-path"
                        || evidence == "outside-development-root"
                        || evidence == "unmanaged-path"
                })
        })
        .collect::<Vec<_>>();
    assert!(!source_retaining.is_empty());
    assert!(source_retaining
        .iter()
        .all(|artifact| matches!(artifact["action"].as_str(), Some("copy" | "retain"))));
    assert!(evaluated_plan["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|blocker| blocker["code"] != "unmanaged-selected-root"));

    let human_output = jit(&repo, &["archive", "candidates"]);
    assert_success(&human_output);
    let human = String::from_utf8(human_output.stdout).unwrap();
    let typed: ArchiveCandidates = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(human, render_archive_candidates(&typed));
    for candidate in candidates {
        let id = candidate["target"]["id"].as_str().unwrap();
        assert!(human.contains(&id[..8]));
        for blocker in candidate["blockers"].as_array().unwrap() {
            assert!(human.contains(blocker["code"].as_str().unwrap()));
        }
    }
    assert_eq!(snapshot_tree(repo.path()), before);
}

#[test]
fn test_archive_candidates_cli_reports_directory_root_but_skips_directory_link_targets() {
    let repo = TempDir::new().unwrap();
    initialize_with_policy(
        &repo,
        "[documentation]\nmanaged_paths = [\"dev/active\"]\npermanent_paths = []\narchive_root = \"dev/archive\"\n",
    );
    fs::create_dir_all(repo.path().join("dev/active")).unwrap();
    fs::write(repo.path().join("dev/index.md"), "[active](active/)").unwrap();
    fs::write(repo.path().join("dev/active/regular.md"), "regular").unwrap();

    let create = |title: &str| {
        let created = jit(
            &repo,
            &[
                "issue", "create", "--title", title, "--type", "epic", "--json",
            ],
        );
        assert_success(&created);
        serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let directory_candidate = create("Directory candidate");
    let regular_candidate = create("Regular candidate");
    for (id, path) in [
        (&directory_candidate, "dev/index.md"),
        (&regular_candidate, "dev/active/regular.md"),
    ] {
        assert_success(&jit(
            &repo,
            &["doc", "add", id, path, "--skip-scan", "--json"],
        ));
        assert_success(&jit(
            &repo,
            &["issue", "update", id, "--state", "rejected", "--json"],
        ));
    }
    let before = snapshot_tree(repo.path());

    let first = jit(&repo, &["archive", "candidates", "--json"]);
    let second = jit(&repo, &["archive", "candidates", "--json"]);
    assert_success(&first);
    assert_success(&second);
    assert_eq!(first.stdout, second.stdout);
    let report: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(report["count"], 2);
    let directory_plan = report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["target"]["id"] == directory_candidate)
        .unwrap();
    assert_eq!(directory_plan["eligible"], false);
    assert!(directory_plan["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| blocker["code"] == "unmanaged-selected-root"));
    let directory_artifacts = directory_plan["artifacts"].as_array().unwrap();
    assert!(directory_artifacts
        .iter()
        .all(|artifact| artifact["source"] != "dev/active"));
    assert!(directory_artifacts
        .iter()
        .flat_map(|artifact| artifact["blockers"].as_array().unwrap())
        .chain(directory_plan["blockers"].as_array().unwrap())
        .all(|blocker| blocker["code"] != "unsupported-artifact-type"));
    let parent_artifact = directory_artifacts
        .iter()
        .find(|artifact| artifact["source"] == "dev/index.md")
        .unwrap();
    assert!(parent_artifact["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .all(|warning| warning["code"] != "unsupported-edge-target"));
    assert!(parent_artifact["edges"]
        .as_array()
        .unwrap()
        .iter()
        .all(|edge| edge["target"] != "dev/active"));

    let preview = jit(
        &repo,
        &["archive", "container", &directory_candidate, "--json"],
    );
    assert_success(&preview);
    let preview_plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(preview_plan, *directory_plan);
    let document_preview = jit(&repo, &["archive", "document", "dev/active", "--json"]);
    assert_success(&document_preview);
    let document_plan: Value = serde_json::from_slice(&document_preview.stdout).unwrap();
    assert!(document_plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["source"] == "dev/active")
        .unwrap()["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .any(|blocker| {
            blocker["code"] == "unsupported-artifact-type" && blocker["path"] == "dev/active"
        }));
    let execute = jit(
        &repo,
        &[
            "archive",
            "container",
            &directory_candidate,
            "--execute",
            "--json",
        ],
    );
    assert!(!execute.status.success());
    let document_execute = jit(
        &repo,
        &["archive", "document", "dev/active", "--execute", "--json"],
    );
    assert!(!document_execute.status.success());
    assert_eq!(snapshot_tree(repo.path()), before);
}

#[test]
fn test_archive_candidates_cli_preserves_all_three_policy_states_without_mutation() {
    for (config, expected) in [
        ("", "unconfigured"),
        ("[documentation]\nmanaged_paths = []\n", "incomplete"),
        (
            "[documentation]\nmanaged_paths = []\npermanent_paths = []\narchive_root = \"archive\"\n",
            "configured",
        ),
    ] {
        let repo = TempDir::new().unwrap();
        assert_success(&jit(&repo, &["init", "--json"]));
        let created = jit(
            &repo,
            &[
                "issue", "create", "--title", "Candidate", "--type", "epic", "--json",
            ],
        );
        assert_success(&created);
        let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_success(&jit(
            &repo,
            &["issue", "update", &id, "--state", "rejected", "--json"],
        ));
        // Each variant is a complete configuration: the declared hierarchy that
        // makes the rejected epic a container, plus the documentation-policy
        // state under test.
        fs::write(
            repo.path().join(".jit/config.toml"),
            format!("[type_hierarchy]\ntypes = {{ epic = 1, task = 2 }}\n\n{config}"),
        )
        .unwrap();
        let before = snapshot_tree(repo.path());

        let output = jit(&repo, &["archive", "candidates", "--json"]);
        assert_success(&output);
        let report: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["count"], 1);
        assert_eq!(report["candidates"][0]["policy_status"], expected);
        let blocker = match expected {
            "unconfigured" => Some("policy-unconfigured"),
            "incomplete" => Some("policy-incomplete"),
            _ => None,
        };
        if let Some(blocker) = blocker {
            assert!(report["candidates"][0]["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| entry["code"] == blocker));
        }
        assert_eq!(snapshot_tree(repo.path()), before);
    }
}

#[test]
fn test_container_archive_slug_is_consistent_and_frozen_by_marker() {
    let repo = TempDir::new().unwrap();
    initialize_with_policy(
        &repo,
        "[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
    );
    fs::create_dir(repo.path().join("fixtures")).unwrap();
    fs::write(repo.path().join("fixtures/root.md"), "archive me").unwrap();

    let created = jit(
        &repo,
        &[
            "issue",
            "create",
            "--title",
            "Mutable title",
            "--type",
            "epic",
            "--label",
            "epic:artifact-archival",
            "--json",
        ],
    );
    assert_success(&created);
    let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_success(&jit(
        &repo,
        &[
            "doc",
            "add",
            &id,
            "fixtures/root.md",
            "--skip-scan",
            "--json",
        ],
    ));
    assert_success(&jit(
        &repo,
        &["issue", "update", &id, "--state", "rejected", "--json"],
    ));

    let expected_root = format!("archive/{}-artifact-archival", &id[..8]);
    let before_preview = snapshot_tree(repo.path());
    let preview = jit(&repo, &["archive", "container", &id, "--json"]);
    assert_success(&preview);
    let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
    assert_eq!(plan["destination_root"], expected_root);
    let human = jit(&repo, &["archive", "container", &id]);
    assert_success(&human);
    assert!(String::from_utf8(human.stdout)
        .unwrap()
        .contains(&format!("Destination root: {expected_root}")));
    let candidates = jit(&repo, &["archive", "candidates", "--json"]);
    assert_success(&candidates);
    let report: Value = serde_json::from_slice(&candidates.stdout).unwrap();
    let candidate = report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["target"]["id"] == id)
        .unwrap();
    assert_eq!(candidate["destination_root"], expected_root);
    assert_eq!(snapshot_tree(repo.path()), before_preview);

    let executed = jit(&repo, &["archive", "container", &id, "--execute", "--json"]);
    assert_success(&executed);
    let execution: Value = serde_json::from_slice(&executed.stdout).unwrap();
    assert_eq!(execution["destination_root"], expected_root);
    assert_eq!(
        fs::read_to_string(repo.path().join(&expected_root).join(".jit-container")).unwrap(),
        format!("{id}\n")
    );

    assert_success(&jit(
        &repo,
        &[
            "issue",
            "update",
            &id,
            "--title",
            "Entirely renamed",
            "--remove-label",
            "epic:artifact-archival",
            "--label",
            "epic:new-strategic-slug",
            "--json",
        ],
    ));
    let frozen = jit(&repo, &["archive", "container", &id, "--json"]);
    assert_success(&frozen);
    assert_eq!(
        serde_json::from_slice::<Value>(&frozen.stdout).unwrap()["destination_root"],
        expected_root
    );
    assert!(!repo
        .path()
        .join(format!("archive/{}-new-strategic-slug", &id[..8]))
        .exists());
}

/// A minimal configured repository whose citation scan universe is exactly
/// `notes/`: `fixtures/root.md` is the moving artifact, and `notes/mention.md`
/// carries `citing_text` verbatim. Archival is a core command (`@/charter/D-4`),
/// so version control is opt-in here: `vcs` runs `git init` before `jit init`,
/// mirroring `test_doc_show_with_git`'s setup, while its absence leaves the
/// directory as bare as every other fixture in this suite.
fn citation_scan_repo(vcs: bool, citing_text: &str) -> TempDir {
    let repo = TempDir::new().unwrap();
    if vcs {
        for args in [
            vec!["init"],
            vec!["config", "user.name", "Test User"],
            vec!["config", "user.email", "test@example.com"],
        ] {
            assert!(Command::new("git")
                .current_dir(repo.path())
                .args(&args)
                .status()
                .unwrap()
                .success());
        }
    }
    initialize_with_policy(
        &repo,
        concat!(
            "[documentation]\n",
            "development_root = \"\"\n",
            "managed_paths = [\"fixtures\"]\n",
            "permanent_paths = []\n",
            "archive_root = \"archive\"\n",
            "citation_scan_roots = [\"notes\"]\n",
        ),
    );
    fs::create_dir_all(repo.path().join("fixtures")).unwrap();
    fs::create_dir_all(repo.path().join("notes")).unwrap();
    fs::write(repo.path().join("fixtures/root.md"), "moving artifact\n").unwrap();
    fs::write(repo.path().join("notes/mention.md"), citing_text).unwrap();
    repo
}

/// Every `moving-path-citation` warning path recorded against `source` in `plan`.
fn citation_warning_paths(plan: &Value, source: &str) -> Vec<String> {
    plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["source"] == source)
        .expect("plan carries the moving artifact")["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|warning| warning["code"] == "moving-path-citation")
        .map(|warning| warning["path"].as_str().unwrap().to_string())
        .collect()
}

/// The complete warning set from either an archive plan or an execution result.
/// Plans partition warnings between target and artifact records; executions
/// flatten that same semantic set into their result envelope.
fn archive_warning_set(report: &Value) -> BTreeSet<(String, Option<String>)> {
    report["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .chain(
            report["artifacts"]
                .as_array()
                .into_iter()
                .flatten()
                .flat_map(|artifact| artifact["warnings"].as_array().into_iter().flatten()),
        )
        .map(|warning| {
            (
                warning["code"].as_str().unwrap().to_string(),
                warning["path"].as_str().map(str::to_string),
            )
        })
        .collect()
}

/// Blank out every warnings collection in `plan`, in place, leaving actions,
/// blockers, and eligibility as the only remaining variables.
fn clear_warnings(plan: &mut Value) {
    plan["warnings"] = serde_json::json!([]);
    for artifact in plan["artifacts"].as_array_mut().unwrap() {
        artifact["warnings"] = serde_json::json!([]);
    }
}

/// REQ-01 (`jit:9ca124f9`): the citation scan reads a declared root set, not
/// tracking data, so a preview over a citing fixture reports the moving
/// artifact's citation even when the working tree is under no version control
/// at all (`@/charter/D-4` — archival is a core command).
#[test]
fn test_archive_document_preview_reports_moving_path_citation_without_version_control() {
    let citing_text = "See fixtures/root.md for the source of truth.\n";
    let repo = citation_scan_repo(false, citing_text);
    assert!(
        !repo.path().join(".git").exists(),
        "fixture must genuinely lack version control to exercise REQ-01"
    );

    let preview = jit(
        &repo,
        &["archive", "document", "fixtures/root.md", "--json"],
    );
    assert_success(&preview);
    let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();

    let column = citing_text.find("fixtures/root.md").unwrap() + 1;
    assert_eq!(
        citation_warning_paths(&plan, "fixtures/root.md"),
        vec![format!("notes/mention.md:1:{column}")]
    );
}

/// REQ-02 (`jit:9ca124f9`): the identical fixture inside an initialized git
/// repository reports the identical citation warnings as its untracked twin —
/// the scan is indifferent to whether the tree is under version control.
#[test]
fn test_archive_document_preview_reports_same_moving_path_citation_inside_version_control() {
    let citing_text = "See fixtures/root.md for the source of truth.\n";
    let untracked = citation_scan_repo(false, citing_text);
    let tracked = citation_scan_repo(true, citing_text);
    assert!(
        tracked.path().join(".git").is_dir(),
        "fixture must genuinely sit under version control to exercise REQ-02"
    );

    let untracked_preview = jit(
        &untracked,
        &["archive", "document", "fixtures/root.md", "--json"],
    );
    let tracked_preview = jit(
        &tracked,
        &["archive", "document", "fixtures/root.md", "--json"],
    );
    assert_success(&untracked_preview);
    assert_success(&tracked_preview);
    let untracked_plan: Value = serde_json::from_slice(&untracked_preview.stdout).unwrap();
    let tracked_plan: Value = serde_json::from_slice(&tracked_preview.stdout).unwrap();

    let untracked_citations = citation_warning_paths(&untracked_plan, "fixtures/root.md");
    assert!(
        !untracked_citations.is_empty(),
        "fixture must actually carry a citation for the comparison to be meaningful"
    );
    assert_eq!(
        untracked_citations,
        citation_warning_paths(&tracked_plan, "fixtures/root.md")
    );
}

/// REQ-03 (`jit:9ca124f9`): a citation warning is purely advisory. A plan whose
/// scan universe carries a citation of the moving artifact's path reports the
/// same actions, blockers, and eligibility as the same plan whose scan universe
/// holds no citation at all — the fixture pair differs only in the text of the
/// citing file.
#[test]
fn test_archive_document_preview_actions_blockers_and_eligibility_match_regardless_of_citation() {
    let with_citation =
        citation_scan_repo(false, "See fixtures/root.md for the source of truth.\n");
    let without_citation = citation_scan_repo(
        false,
        "See fixtures/unrelated.md for the source of truth.\n",
    );

    let with_output = jit(
        &with_citation,
        &["archive", "document", "fixtures/root.md", "--json"],
    );
    let without_output = jit(
        &without_citation,
        &["archive", "document", "fixtures/root.md", "--json"],
    );
    assert_success(&with_output);
    assert_success(&without_output);
    let mut with_plan: Value = serde_json::from_slice(&with_output.stdout).unwrap();
    let mut without_plan: Value = serde_json::from_slice(&without_output.stdout).unwrap();

    // Guard: the scan universe genuinely differs in whether it carries a citation.
    assert!(!citation_warning_paths(&with_plan, "fixtures/root.md").is_empty());
    assert!(citation_warning_paths(&without_plan, "fixtures/root.md").is_empty());

    assert_eq!(with_plan["eligible"], without_plan["eligible"]);
    assert_eq!(with_plan["blockers"], without_plan["blockers"]);
    assert_eq!(with_plan["action_counts"], without_plan["action_counts"]);
    clear_warnings(&mut with_plan);
    clear_warnings(&mut without_plan);
    assert_eq!(
        with_plan, without_plan,
        "a citation warning must change nothing but the plan's warnings"
    );
}

/// A repository that classifies its development areas explicitly, per
/// `@/issue/8e071e18/decision/D-6`, with `dev/presentations` as the one
/// configured managed area.
///
/// `managed` selects whether the deck sits inside that configured area or in
/// an adjacent development-root directory that no configured entry claims,
/// reproducing the predecessor deck's actual gap. `terminal` selects whether
/// the linked owner reaches a terminal state before the preview runs. The
/// deck is always linked with `jit doc add`: an unlinked artifact makes the
/// terminal-owner check vacuously true (`jit:1f80212b`'s code-review finding
/// against a sibling test), so every caller here exercises a genuine
/// ownership edge, not an absent one.
fn presentation_deck_repo(managed: bool, terminal: bool) -> (TempDir, String, String) {
    let repo = TempDir::new().unwrap();
    initialize_with_policy(
        &repo,
        concat!(
            "[documentation]\n",
            "development_root = \"dev\"\n",
            "managed_paths = [\"dev/presentations\"]\n",
            "permanent_paths = []\n",
            "archive_root = \"dev/archive\"\n",
        ),
    );
    let area = if managed {
        "dev/presentations"
    } else {
        "dev/unclassified"
    };
    let deck_path = format!("{area}/talk.html");
    fs::create_dir_all(repo.path().join(area)).unwrap();
    fs::write(repo.path().join(&deck_path), "<html>deck</html>").unwrap();

    let created = jit(
        &repo,
        &[
            "issue",
            "create",
            "--title",
            "Presentation deck owner",
            "--type",
            "epic",
            "--json",
        ],
    );
    assert_success(&created);
    let id = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_success(&jit(
        &repo,
        &["doc", "add", &id, &deck_path, "--skip-scan", "--json"],
    ));
    if terminal {
        assert_success(&jit(
            &repo,
            &["issue", "update", &id, "--state", "rejected", "--json"],
        ));
    }
    (repo, id, deck_path)
}

/// The plan artifact recorded for `source`, panicking if the fixture never produced one.
fn find_artifact<'a>(plan: &'a Value, source: &str) -> &'a Value {
    plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["source"] == source)
        .unwrap_or_else(|| panic!("plan carries no artifact for {source}: {plan}"))
}

/// REQ-01/REQ-03 (`jit:daf4fa46`): a deck asset under the configured
/// `dev/presentations` managed area, linked to a terminal-state owner,
/// previews as an eligible document-target plan reporting no target-level
/// `unmanaged-selected-root` blocker.
///
/// The counterfactual at the end pins the terminal-owner check against the
/// vacuous pass a sibling issue's test fell into: an artifact with a linked
/// but non-terminal owner must stay `retain`, not `move`, so the `move`
/// asserted above is genuine evidence the owner's state was consulted rather
/// than an artifact of previewing an unlinked path.
#[test]
fn test_archive_document_preview_reports_eligible_plan_for_deck_owned_by_terminal_issue() {
    let (repo, _owner, deck_path) = presentation_deck_repo(true, true);

    let preview = jit(&repo, &["archive", "document", &deck_path, "--json"]);
    assert_success(&preview);
    let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();

    assert_eq!(plan["eligible"], true);
    assert!(plan["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|blocker| blocker["code"] != "unmanaged-selected-root"));
    assert_eq!(
        find_artifact(&plan, &deck_path)["action"],
        "move",
        "the deck must genuinely be selected for relocation, not vacuously eligible: {plan}"
    );

    let (non_terminal_repo, _owner, deck_path) = presentation_deck_repo(true, false);
    let blocked = jit(
        &non_terminal_repo,
        &["archive", "document", &deck_path, "--json"],
    );
    assert_success(&blocked);
    let blocked_plan: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_ne!(
        find_artifact(&blocked_plan, &deck_path)["action"],
        "move",
        "a non-terminal owner must not be selected for relocation: {blocked_plan}"
    );
}

/// REQ-02/REQ-03 (`jit:daf4fa46`): a container-target preview whose subtree
/// contains a deck under the configured managed area, owned (as the
/// container itself) by a terminal issue, reaches the same eligible outcome
/// and the same absence of a target-level `unmanaged-selected-root` blocker
/// as the document-target form.
///
/// The counterfactual mirrors the document-target test: a non-terminal
/// container owner leaves the deck `retain`, proving the container form
/// consults the same terminal-owner state rather than defaulting to eligible.
#[test]
fn test_archive_container_preview_reports_eligible_plan_for_subtree_deck_owned_by_terminal_issue() {
    let (repo, owner, deck_path) = presentation_deck_repo(true, true);

    let preview = jit(&repo, &["archive", "container", &owner, "--json"]);
    assert_success(&preview);
    let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();

    assert_eq!(plan["eligible"], true);
    assert!(plan["blockers"]
        .as_array()
        .unwrap()
        .iter()
        .all(|blocker| blocker["code"] != "unmanaged-selected-root"));
    assert_eq!(
        find_artifact(&plan, &deck_path)["action"],
        "move",
        "the deck must genuinely be selected for relocation, not vacuously eligible: {plan}"
    );

    let (non_terminal_repo, non_terminal_owner, deck_path) = presentation_deck_repo(true, false);
    let blocked = jit(
        &non_terminal_repo,
        &["archive", "container", &non_terminal_owner, "--json"],
    );
    assert_success(&blocked);
    let blocked_plan: Value = serde_json::from_slice(&blocked.stdout).unwrap();
    assert_ne!(
        find_artifact(&blocked_plan, &deck_path)["action"],
        "move",
        "a non-terminal owner must not be selected for relocation: {blocked_plan}"
    );
}

/// REQ-03 negative control (`jit:daf4fa46`): the identical deck and terminal
/// owner, relocated one directory over into a development-root area that no
/// configured entry claims, reports `unmanaged-selected-root` in the
/// target-level blocker array through both target forms. This is what proves
/// the absence asserted in the two tests above is meaningful: these fixtures
/// can and do raise the blocker when the area genuinely is unmanaged.
#[test]
fn test_archive_preview_reports_unmanaged_selected_root_for_deck_outside_configured_area() {
    let (repo, owner, deck_path) = presentation_deck_repo(false, true);

    for args in [
        vec!["archive", "document", deck_path.as_str(), "--json"],
        vec!["archive", "container", owner.as_str(), "--json"],
    ] {
        let preview = jit(&repo, &args);
        assert_success(&preview);
        let plan: Value = serde_json::from_slice(&preview.stdout).unwrap();
        assert_eq!(plan["eligible"], false, "{args:?} plan: {plan}");
        assert!(
            plan["blockers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|blocker| blocker["code"] == "unmanaged-selected-root"),
            "{args:?} plan: {plan}"
        );
    }
}

/// The one file of the citation-scan fixture's declared scan universe.
const CITING_FILE: &str = "notes/mention.md";

/// The moving artifact whose path [`CITING_FILE`] names in its text.
const CITED_ARTIFACT: &str = "fixtures/root.md";

/// Every repository document, keyed by its normalized relative path.
///
/// `.jit/` records jit's own state rather than document content, and relinking
/// an issue's document list rewrites it by design (`jit:3be32cad` REQ-03), so an
/// inventory that answers "which document bytes did execution synthesize"
/// leaves it out. Everything else [`snapshot_files`] reaches stays in scope.
fn document_contents(root: &Path) -> BTreeMap<String, Vec<u8>> {
    snapshot_files(root)
        .into_iter()
        .filter(|(path, _)| !path.starts_with(".jit/"))
        .collect()
}

/// What one execution did to every document in the tree.
///
/// The four buckets partition the post-execution tree, and `lost` reports the
/// pre-execution content that survived nowhere. Classification is by content,
/// not by path: a file that kept its bytes under a new name is `relocated`,
/// while `synthesized` holds exactly those post-execution paths whose bytes no
/// pre-execution file held. That is what turns "the marker is synthesized" into
/// "the marker is the *only* thing synthesized".
#[derive(Debug)]
struct ExecutionEffect {
    /// Paths present before and after, with identical bytes.
    unchanged: BTreeSet<String>,
    /// Paths present before and after, with differing bytes.
    rewritten: BTreeSet<String>,
    /// New paths, mapped to a pre-execution path that held the same bytes.
    relocated: BTreeMap<String, String>,
    /// New paths whose bytes no pre-execution path held.
    synthesized: BTreeSet<String>,
    /// Pre-execution paths whose bytes are held by no post-execution path.
    lost: BTreeSet<String>,
}

/// Classify every document in `after` against the tree captured in `before`.
fn classify_execution_effect(
    before: &BTreeMap<String, Vec<u8>>,
    after: &BTreeMap<String, Vec<u8>>,
) -> ExecutionEffect {
    let origin_of = |bytes: &[u8]| {
        before
            .iter()
            .find(|(_, held)| held.as_slice() == bytes)
            .map(|(path, _)| path.clone())
    };
    let arrivals = || after.iter().filter(|(path, _)| !before.contains_key(*path));
    ExecutionEffect {
        unchanged: after
            .iter()
            .filter(|(path, bytes)| before.get(*path) == Some(*bytes))
            .map(|(path, _)| path.clone())
            .collect(),
        rewritten: after
            .iter()
            .filter(|(path, bytes)| before.get(*path).is_some_and(|held| held != *bytes))
            .map(|(path, _)| path.clone())
            .collect(),
        relocated: arrivals()
            .filter_map(|(path, bytes)| Some((path.clone(), origin_of(bytes)?)))
            .collect(),
        synthesized: arrivals()
            .filter(|(_, bytes)| origin_of(bytes).is_none())
            .map(|(path, _)| path.clone())
            .collect(),
        lost: before
            .iter()
            .filter(|(_, bytes)| !after.values().any(|held| held == *bytes))
            .map(|(path, _)| path.clone())
            .collect(),
    }
}

/// Sources the plan selects for relocation.
fn planned_moves(plan: &Value) -> Vec<String> {
    plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|artifact| artifact["action"] == "move")
        .map(|artifact| artifact["source"].as_str().unwrap().to_string())
        .collect()
}

/// Destination the plan records for the artifact sourced at `source`.
fn planned_destination(plan: &Value, source: &str) -> String {
    find_artifact(plan, source)["destination"]
        .as_str()
        .unwrap_or_else(|| panic!("plan records no destination for {source}: {plan}"))
        .to_string()
}

/// One executed container archival over the citation-scan fixture, captured
/// either side of the run.
struct ArchivedCitation {
    repo: TempDir,
    container: String,
    destination_root: String,
    preview: Value,
    execution: Value,
    before: BTreeMap<String, Vec<u8>>,
    after: BTreeMap<String, Vec<u8>>,
}

/// Archive a terminal container holding two managed artifacts, one of which
/// [`CITING_FILE`] names in its text, and capture the document tree either side.
///
/// Every guarantee `jit:3be32cad` locks in is a claim about this single
/// execution, so the preconditions that keep those claims from passing
/// vacuously are established here rather than in each test: both linked
/// artifacts are genuinely selected for relocation, and the planner genuinely
/// sees the citation. A run that relocated nothing, or one whose scan universe
/// never reached the citing file, would satisfy immutability trivially.
///
/// The citing file sits outside `managed_paths` and is linked to no issue, so
/// it is not itself an artifact of the plan: it stays put, and the only thing
/// execution could do to it is rewrite the now-stale path in its text.
fn archived_citation() -> ArchivedCitation {
    archived_citation_with_scan_directories(0)
}

/// Add empty scan directories to exercise the transaction's complete-listing
/// closure without changing the one citation the fixture reports.
fn archived_citation_with_scan_directories(directory_count: usize) -> ArchivedCitation {
    let repo = citation_scan_repo(false, "See fixtures/root.md for the source of truth.\n");
    (0..directory_count).for_each(|index| {
        fs::create_dir_all(repo.path().join(format!("notes/fanout-{index:03}"))).unwrap();
    });
    fs::write(repo.path().join("fixtures/appendix.md"), "appendix body\n").unwrap();

    let container = terminal_citation_container(&repo);

    let previewed = jit(&repo, &["archive", "container", &container, "--json"]);
    assert_success(&previewed);
    let preview: Value = serde_json::from_slice(&previewed.stdout).unwrap();
    assert_eq!(
        planned_moves(&preview).into_iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            CITED_ARTIFACT.to_string(),
            "fixtures/appendix.md".to_string()
        ]),
        "both linked artifacts must genuinely relocate, or immutability holds trivially: {preview}"
    );
    assert!(
        citation_warning_paths(&preview, CITED_ARTIFACT)
            .iter()
            .any(|occurrence| occurrence.starts_with(CITING_FILE)),
        "the scan must genuinely see the citation, or nothing observes the rewrite it declines to make: {preview}"
    );

    let before = document_contents(repo.path());
    let executed = jit(
        &repo,
        &["archive", "container", &container, "--execute", "--json"],
    );
    assert_success(&executed);
    let execution: Value = serde_json::from_slice(&executed.stdout).unwrap();
    let after = document_contents(repo.path());
    let destination_root = execution["destination_root"].as_str().unwrap().to_string();

    ArchivedCitation {
        repo,
        container,
        destination_root,
        preview,
        execution,
        before,
        after,
    }
}

/// Create the terminal container shared by citation preview/execution tests.
fn terminal_citation_container(repo: &TempDir) -> String {
    terminal_container_with_documents(repo, &[CITED_ARTIFACT, "fixtures/appendix.md"])
}

fn terminal_container_with_documents(repo: &TempDir, documents: &[&str]) -> String {
    let created = jit(
        repo,
        &[
            "issue",
            "create",
            "--title",
            "Citing container",
            "--type",
            "epic",
            "--json",
        ],
    );
    assert_success(&created);
    let container = serde_json::from_slice::<Value>(&created.stdout).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    for path in documents {
        assert_success(&jit(
            repo,
            &["doc", "add", &container, *path, "--skip-scan", "--json"],
        ));
    }
    assert_success(&jit(
        repo,
        &[
            "issue", "update", &container, "--state", "rejected", "--json",
        ],
    ));
    container
}

/// REQ-01 (`jit:3be32cad`): execution relocates artifacts, so every artifact
/// the plan selects for `move` vacates its source path and arrives at its
/// planned destination holding byte-identical content.
///
/// The closing comparison against `deleted_sources` pins the vacated paths to
/// execution's own record: the sources are gone because execution removed them
/// after publishing, not because the plan quietly dropped them.
#[test]
fn test_archive_container_execution_relocates_every_artifact_byte_identically() {
    let run = archived_citation();

    let moves = planned_moves(&run.preview);
    for source in &moves {
        let destination = planned_destination(&run.preview, source);
        let original = run
            .before
            .get(source)
            .unwrap_or_else(|| panic!("fixture must hold {source} before execution"));
        assert!(
            !run.after.contains_key(source),
            "a relocated artifact must vacate its source path: {source}"
        );
        assert_eq!(
            run.after.get(&destination),
            Some(original),
            "relocation must preserve every byte: {source} -> {destination}"
        );
    }
    assert_eq!(
        run.execution["deleted_sources"]
            .as_array()
            .unwrap()
            .iter()
            .map(|source| source.as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        moves.into_iter().collect::<BTreeSet<_>>(),
        "execution must report removing exactly the sources it relocated: {}",
        run.execution
    );
}

/// REQ-02 (`jit:3be32cad`): the warning channel is the whole response to an
/// in-content citation. The scanned file that names the moving artifact's path
/// survives execution byte for byte, so the citation it carries is left naming
/// a path the tree no longer holds (`@/issue/8e071e18/decision/D-8`).
#[test]
fn test_archive_container_execution_leaves_a_citing_scanned_file_byte_identical() {
    let run = archived_citation();

    let original = run
        .before
        .get(CITING_FILE)
        .unwrap_or_else(|| panic!("fixture must hold {CITING_FILE} before execution"));
    assert_eq!(
        run.after.get(CITING_FILE),
        Some(original),
        "a scanned citing file must survive execution byte for byte"
    );
    assert!(
        !run.after.contains_key(CITED_ARTIFACT),
        "the cited path must genuinely stop resolving, or the citation never broke"
    );
    assert!(
        String::from_utf8(original.clone())
            .unwrap()
            .contains(CITED_ARTIFACT),
        "the surviving text must still carry the stale citation verbatim"
    );
}

/// REQ-03 (`jit:ef118aea`): execution derives the same advisory warnings as a
/// preview of the unchanged target, including citations it intentionally leaves
/// for the adopter to repair after relocation.
#[test]
fn test_archive_container_execution_reports_the_preview_warning_set() {
    let run = archived_citation();

    assert_eq!(
        archive_warning_set(&run.execution),
        archive_warning_set(&run.preview),
        "execution must report the warning set preview computed for the same archive target"
    );
}

#[cfg(unix)]
#[test]
fn test_archive_container_preview_and_execution_preserve_citations_beside_unreadable_subtree() {
    use std::os::unix::fs::PermissionsExt as _;

    let repo = TempDir::new().unwrap();
    initialize_with_policy(
        &repo,
        concat!(
            "[documentation]\n",
            "development_root = \"dev\"\n",
            "managed_paths = [\"dev/active\"]\n",
            "permanent_paths = []\n",
            "archive_root = \"dev/archive\"\n",
            "citation_scan_roots = [\"dev/scan\"]\n",
        ),
    );
    fs::create_dir_all(repo.path().join("dev/active")).unwrap();
    fs::create_dir_all(repo.path().join("dev/scan")).unwrap();
    fs::write(repo.path().join("dev/active/design.md"), "moving design\n").unwrap();
    fs::write(
        repo.path().join("dev/scan/readable.md"),
        concat!(
            "First: dev/active/design.md\n",
            "Second: dev/active/design.md\n",
            "Third: dev/active/design.md\n",
        ),
    )
    .unwrap();
    let secret = repo.path().join("dev/scan/secret");
    fs::create_dir(&secret).unwrap();
    fs::write(secret.join("hidden.md"), "dev/active/design.md\n").unwrap();
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o000)).unwrap();
    let container = terminal_container_with_documents(&repo, &["dev/active/design.md"]);

    let previewed = jit(&repo, &["archive", "container", &container, "--json"]);
    let executed = jit(
        &repo,
        &["archive", "container", &container, "--execute", "--json"],
    );
    fs::set_permissions(&secret, fs::Permissions::from_mode(0o700)).unwrap();
    assert_success(&previewed);
    assert_success(&executed);
    let preview: Value = serde_json::from_slice(&previewed.stdout).unwrap();
    let execution: Value = serde_json::from_slice(&executed.stdout).unwrap();
    let expected = BTreeSet::from([
        "dev/scan/readable.md:1:8".to_string(),
        "dev/scan/readable.md:2:9".to_string(),
        "dev/scan/readable.md:3:8".to_string(),
    ]);
    let preview_citations = citation_warning_paths(&preview, "dev/active/design.md")
        .into_iter()
        .collect::<BTreeSet<_>>();
    let execution_citations = archive_warning_set(&execution)
        .into_iter()
        .filter_map(|(code, path)| (code == "moving-path-citation").then_some(path).flatten())
        .collect::<BTreeSet<_>>();

    assert_eq!(preview_citations, expected);
    assert_eq!(execution_citations, expected);
    assert_eq!(
        archive_warning_set(&execution),
        archive_warning_set(&preview)
    );
}

/// REQ-03 (`jit:ef118aea`): a declared scan root may contain more directories
/// than the ordinary validation closure lists; execution still captures and
/// revalidates that complete configured universe rather than imposing a
/// smaller repository-size cap than its listing budget.
#[test]
fn test_archive_container_execution_reports_citations_with_more_than_256_scan_directories() {
    let run = archived_citation_with_scan_directories(261);

    assert_eq!(
        archive_warning_set(&run.execution),
        archive_warning_set(&run.preview),
        "a large declared scan root must preserve preview/execution warning parity"
    );
}

/// REQ-03 (`jit:3be32cad`): immutability is not achieved by doing nothing.
/// Every relocated artifact's document link record is relinked to the archived
/// location, and each relinked path names a file the tree actually holds
/// (`@/invariant/derived-state-coherence`).
#[test]
fn test_archive_container_execution_relinks_document_records_to_the_archived_locations() {
    let run = archived_citation();

    let shown = jit(&run.repo, &["issue", "show", &run.container, "--json"]);
    assert_success(&shown);
    let recorded = serde_json::from_slice::<Value>(&shown.stdout).unwrap()["documents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|document| document["path"].as_str().unwrap().to_string())
        .collect::<BTreeSet<_>>();

    let changes = run.execution["reference_changes"].as_array().unwrap();
    assert!(
        !changes.is_empty(),
        "execution must relink the records of the artifacts it relocated: {}",
        run.execution
    );
    for change in changes {
        let from = change["from_path"].as_str().unwrap();
        let to = change["to_path"].as_str().unwrap();
        assert_ne!(from, to, "a relink must move the record: {change}");
        assert!(
            recorded.contains(to) && !recorded.contains(from),
            "the stored record must name {to} and drop {from}: {recorded:?}"
        );
        assert!(
            run.after.contains_key(to),
            "a relinked record must name a file the tree holds: {to}"
        );
    }
    assert_eq!(
        recorded,
        planned_moves(&run.preview)
            .iter()
            .map(|source| planned_destination(&run.preview, source))
            .collect::<BTreeSet<_>>(),
        "every document record must point at its planned destination"
    );
}

/// REQ-04 (`jit:3be32cad`): the container marker is the only file whose bytes
/// execution synthesizes.
///
/// The claim is about the whole document tree, so it is asserted from a
/// before/after inventory rather than from named paths. The buckets partition
/// the post-execution tree — the totality check below is what makes
/// `synthesized == {marker}` mean *only* the marker — and each of the three
/// outcomes the criterion distinguishes is separately shown to be non-empty
/// where it should be: files relocated, files stayed, and one file is new.
#[test]
fn test_archive_container_execution_synthesizes_no_document_bytes_beyond_the_container_marker() {
    let run = archived_citation();
    let marker = format!("{}/.jit-container", run.destination_root);
    let effect = classify_execution_effect(&run.before, &run.after);

    assert_eq!(
        effect
            .unchanged
            .iter()
            .chain(&effect.rewritten)
            .chain(effect.relocated.keys())
            .chain(&effect.synthesized)
            .cloned()
            .collect::<BTreeSet<_>>(),
        run.after.keys().cloned().collect::<BTreeSet<_>>(),
        "every post-execution document must be classified, or the claim below covers only part of the tree"
    );
    assert_eq!(
        effect.synthesized,
        BTreeSet::from([marker.clone()]),
        "the container marker must be the only synthesized content: {effect:#?}"
    );
    assert!(
        effect.rewritten.is_empty() && effect.lost.is_empty(),
        "execution must rewrite and lose no document content: {effect:#?}"
    );
    assert!(
        !effect.relocated.is_empty() && effect.unchanged.contains(CITING_FILE),
        "the inventory must observe both a relocated and a stationary document: {effect:#?}"
    );

    let unsourced = run.execution["publications"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|publication| publication["source"].is_null())
        .map(|publication| publication["destination"].as_str().unwrap().to_string())
        .collect::<Vec<_>>();
    assert_eq!(
        unsourced,
        vec![marker],
        "the marker must be the one publication execution attributes to no source: {}",
        run.execution
    );
}
