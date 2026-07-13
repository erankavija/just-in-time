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
