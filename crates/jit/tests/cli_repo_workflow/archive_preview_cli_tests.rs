use jit::domain::artifact_classifier::artifact_mirror_destination;
use jit::domain::artifact_plan::{ArchiveCandidates, ArtifactPlan};
use jit::output::{render_archive_candidates, render_archive_plan};
use serde_json::Value;
use std::collections::BTreeMap;
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

/// Replace the scaffolded documentation policy with a test-specific one.
///
/// `jit init` authors a complete `[documentation]` policy, so a test that needs
/// its own classification overrides that table instead of adding a second one.
fn set_documentation_policy(repo: &TempDir, policy: &str) {
    let path = repo.path().join(".jit/config.toml");
    let mut config = fs::read_to_string(&path)
        .unwrap()
        .parse::<toml::Table>()
        .unwrap();
    config.extend(policy.parse::<toml::Table>().unwrap());
    fs::write(&path, config.to_string()).unwrap();
}

fn configured_bundle(repo: &TempDir) {
    assert_success(&jit(repo, &["init", "--json"]));
    set_documentation_policy(
        repo,
        r#"
[documentation]
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
    assert_success(&jit(&container_repo, &["init", "--json"]));
    set_documentation_policy(
        &container_repo,
        "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
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
fn test_archive_execute_retains_out_of_root_source_and_publishes_mirror_without_pending_deletion() {
    // A linked file outside the configured development root classifies
    // permanent (`@/issue/8e071e18/decision/D-14`): it is archived by copy, so
    // the working-tree source is never relocated. Guard that guarantee at both
    // the plan level (REQ-03: no pending deletion is scheduled) and the
    // execution level (REQ-01: source retained, REQ-02: mirror published),
    // pinned together so the retention cannot be satisfied by silently
    // dropping the artifact from the plan.
    let repo = TempDir::new().unwrap();
    assert_success(&jit(&repo, &["init", "--json"]));
    set_documentation_policy(
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
    let expected_mirror = artifact_mirror_destination(&destination_root, "scripts/install.sh");
    let artifact = plan["artifacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|artifact| artifact["source"] == "scripts/install.sh")
        .unwrap();
    assert_eq!(artifact["action"], "copy");
    assert_eq!(artifact["destination"], expected_mirror);
    assert_eq!(
        artifact["pending_deletions"],
        serde_json::json!([]),
        "an out-of-root permanent source must not schedule a pending deletion"
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
        result["publications"]
            .as_array()
            .unwrap()
            .iter()
            .any(|publication| publication["source"] == "scripts/install.sh"
                && publication["destination"] == expected_mirror),
        "execution must publish the mirrored copy under the archive root: {result}"
    );

    let source_path = repo.path().join("scripts/install.sh");
    assert!(
        source_path.exists(),
        "source outside the development root must remain at its original path"
    );
    assert_eq!(fs::read(&source_path).unwrap(), source_bytes);

    let mirror_path = repo.path().join(&expected_mirror);
    assert!(
        mirror_path.exists(),
        "archive execution must publish a mirrored copy under the archive root"
    );
    assert_eq!(fs::read(&mirror_path).unwrap(), source_bytes);
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
                .any(|evidence| evidence == "permanent-path" || evidence == "unmanaged-path")
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
    assert_success(&jit(&repo, &["init", "--json"]));
    set_documentation_policy(
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
        fs::write(repo.path().join(".jit/config.toml"), config).unwrap();
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
    assert_success(&jit(&repo, &["init", "--json"]));
    set_documentation_policy(
        &repo,
        "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
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
