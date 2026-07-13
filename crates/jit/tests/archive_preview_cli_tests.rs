use jit::domain::artifact_plan::ArtifactPlan;
use jit::output::render_archive_plan;
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

fn assert_success(output: &std::process::Output) {
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configured_bundle(repo: &TempDir) {
    assert_success(&jit(repo, &["init", "--json"]));
    let shipped_config = fs::read_to_string(repo.path().join(".jit/config.toml")).unwrap();
    fs::write(
        repo.path().join(".jit/config.toml"),
        format!(
            r#"
[documentation]
managed_paths = ["fixtures"]
permanent_paths = []
archive_root = "archive"

{shipped_config}"#
        ),
    )
    .unwrap();
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
    let before = snapshot_files(repo.path());

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

    assert_eq!(snapshot_files(repo.path()), before);
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
    assert!(document_repo
        .path()
        .join("archive/fixtures/root.md")
        .exists());

    let container_repo = TempDir::new().unwrap();
    assert_success(&jit(&container_repo, &["init", "--json"]));
    let shipped = fs::read_to_string(container_repo.path().join(".jit/config.toml")).unwrap();
    fs::write(
        container_repo.path().join(".jit/config.toml"),
        format!(
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n\n{shipped}"
        ),
    )
    .unwrap();
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
