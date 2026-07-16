use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

struct TestRepo {
    _parent: TempDir,
    path: PathBuf,
}

impl TestRepo {
    fn new() -> Self {
        let parent = TempDir::new().expect("create repository parent");
        let path = parent.path().join("repo");
        fs::create_dir(&path).expect("create repository");
        Self {
            _parent: parent,
            path,
        }
    }
}

fn jit(repo: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("JIT_DATA_DIR")
        .env_remove("JIT_SRC")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "")
        .output()
        .unwrap_or_else(|error| panic!("failed to run jit {args:?}: {error}"))
}

fn parse_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstatus={}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn success_json(repo: &Path, args: &[&str]) -> Value {
    let output = jit(repo, args);
    assert!(
        output.status.success(),
        "jit {args:?} failed\nstatus={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output)
}

fn failed_json(repo: &Path, args: &[&str], exit_code: i32) -> Value {
    let output = jit(repo, args);
    assert_eq!(
        output.status.code(),
        Some(exit_code),
        "jit {args:?} returned the wrong status\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output)
}

fn normalized_events(repo: &Path) -> Vec<Value> {
    fs::read_to_string(repo.join(".jit/events.jsonl"))
        .expect("read event log")
        .lines()
        .map(|line| {
            let mut event: Value = serde_json::from_str(line).expect("parse event");
            let object = event.as_object_mut().expect("event must be an object");
            object.remove("id");
            object.remove("timestamp");
            event
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum SnapshotEntry {
    Directory,
    File { bytes: Vec<u8>, executable: bool },
}

fn normalized_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_machine_local_path(path: &str) -> bool {
    path == ".jit/events.jsonl"
        || path == ".jit/tmp"
        || path.starts_with(".jit/tmp/")
        || path.ends_with(".lock")
        || matches!(
            path,
            ".jit/worktree.json" | ".jit/server.log" | ".jit/server.pid.json"
        )
}

#[cfg(unix)]
fn is_executable(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(_metadata: &fs::Metadata) -> bool {
    false
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, SnapshotEntry> {
    fn visit(root: &Path, current: &Path, entries: &mut BTreeMap<String, SnapshotEntry>) {
        let mut children = fs::read_dir(current)
            .expect("read repository directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("read repository entries");
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let path = child.path();
            let relative = normalized_path(path.strip_prefix(root).expect("repository-relative"));
            if is_machine_local_path(&relative) {
                continue;
            }
            let metadata = child.metadata().expect("read repository metadata");
            if metadata.is_dir() {
                entries.insert(relative, SnapshotEntry::Directory);
                visit(root, &path, entries);
            } else {
                entries.insert(
                    relative,
                    SnapshotEntry::File {
                        bytes: fs::read(&path).expect("read repository file"),
                        executable: is_executable(&metadata),
                    },
                );
            }
        }
    }

    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries);
    entries
}

fn target<'a>(plan: &'a Value, path: &str) -> &'a Value {
    plan["targets"]
        .as_array()
        .expect("profile targets")
        .iter()
        .find(|target| target["path"] == path)
        .unwrap_or_else(|| panic!("missing profile target {path}"))
}

fn property_keys(schema: &Value, definition: &str) -> BTreeSet<String> {
    schema["definitions"][definition]["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("missing properties for schema definition {definition}"))
        .keys()
        .cloned()
        .collect()
}

fn expected_keys(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| (*key).to_string()).collect()
}

#[test]
fn test_profile_fresh_init_and_existing_apply_are_equivalent_without_git() {
    let fresh = TestRepo::new();
    let existing = TestRepo::new();
    assert!(!fresh.path.join(".git").exists());
    assert!(!existing.path.join(".git").exists());

    let fresh_init = success_json(&fresh.path, &["init", "--profile", "jit-dogfood", "--json"]);
    assert_eq!(fresh_init["profile"]["status"], "applied");

    success_json(&existing.path, &["init", "--json"]);
    let preview = success_json(
        &existing.path,
        &["profile", "apply", "jit-dogfood", "--dry-run", "--json"],
    );
    assert_eq!(preview["status"], "would_apply");
    assert!(!existing.path.join(".jit/profiles").exists());
    assert_eq!(target(&preview, "scripts/ai-review.sh")["executable"], true);

    let applied = success_json(
        &existing.path,
        &["profile", "apply", "jit-dogfood", "--json"],
    );
    assert_eq!(applied["status"], "applied");
    let no_op = success_json(
        &existing.path,
        &["profile", "apply", "jit-dogfood", "--dry-run", "--json"],
    );
    assert_eq!(no_op["status"], "unchanged");
    assert_eq!(target(&no_op, "scripts/ai-review.sh")["executable"], true);

    assert_eq!(snapshot_tree(&fresh.path), snapshot_tree(&existing.path));
    assert_eq!(
        normalized_events(&fresh.path),
        normalized_events(&existing.path)
    );
    assert!(!fresh.path.join(".git").exists());
    assert!(!existing.path.join(".git").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script_mode = fs::metadata(existing.path.join("scripts/ai-review.sh"))
            .unwrap()
            .permissions()
            .mode();
        let prompt_mode = fs::metadata(existing.path.join("scripts/code-review-prompt.md"))
            .unwrap()
            .permissions()
            .mode();
        assert_ne!(script_mode & 0o111, 0, "review checker must be executable");
        assert_eq!(
            prompt_mode & 0o111,
            0,
            "review prompt must not be executable"
        );
    }

    #[cfg(windows)]
    {
        let permissions = fs::metadata(existing.path.join("scripts/ai-review.sh"))
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Windows preserves executable intent in the public plan, not a Unix mode bit"
        );
    }
}

#[test]
fn test_offline_public_cli_profile_reaches_implementation_ready_breakdown() {
    let repo = TestRepo::new();
    success_json(&repo.path, &["init", "--profile", "jit-dogfood", "--json"]);
    assert!(!repo.path.join(".git").exists());

    let invariants = success_json(&repo.path, &["invariant", "render", "--json"]);
    assert_eq!(invariants["target"], "AGENTS.md");
    let references = success_json(&repo.path, &["reference", "render", "--json"]);
    assert_eq!(references["target"], ".jit/reference/rules-and-gates.md");
    let agents = fs::read_to_string(repo.path.join("AGENTS.md")).unwrap();
    assert!(agents.contains("<!-- jit:invariants:begin -->"));
    assert!(agents.contains("<!-- jit:invariants:end -->"));
    let rules_and_gates =
        fs::read_to_string(repo.path.join(".jit/reference/rules-and-gates.md")).unwrap();
    assert!(rules_and_gates.contains("## Gates"));
    assert!(rules_and_gates.contains("plan-review"));

    let container = success_json(
        &repo.path,
        &[
            "issue",
            "create",
            "Acceptance epic",
            "--description",
            "Profile adoption is exercised through its public workflow.\n\n## Success Criteria\n\n- [hard] REQ-01: Produces one implementation-ready task.",
            "--type",
            "epic",
            "--label",
            "epic:acceptance",
            "--json",
        ],
    );
    let container_id = container["id"].as_str().unwrap();
    let applied = success_json(&repo.path, &["apply", "plan", container_id, "--json"]);
    let planning_id = applied["created_node_ids_by_role"]["planning"]
        .as_str()
        .unwrap();
    let breakdown_id = applied["created_node_ids_by_role"]["breakdown"]
        .as_str()
        .unwrap();

    success_json(
        &repo.path,
        &["issue", "claim", planning_id, "agent:acceptance", "--json"],
    );
    let blocked_plan = failed_json(
        &repo.path,
        &["issue", "update", planning_id, "--state", "done", "--json"],
        4,
    );
    assert_eq!(blocked_plan["error"]["details"]["actual_state"], "gated");
    let plan_review = success_json(
        &repo.path,
        &["gate", "evaluate", planning_id, "plan-review", "--json"],
    );
    assert_eq!(plan_review["status"], "passed");
    assert!(plan_review["warnings"][0]
        .as_str()
        .unwrap()
        .contains("EXTERNAL REVIEW PLACEHOLDER"));
    success_json(
        &repo.path,
        &["issue", "update", planning_id, "--state", "done", "--json"],
    );

    let implementation = success_json(
        &repo.path,
        &[
            "issue",
            "create",
            "Implement acceptance criterion",
            "--description",
            "This leaf is ready for an implementation agent.\n\n## Success Criteria\n\n- [hard] REQ-01: Delivers the acceptance behavior.",
            "--type",
            "task",
            "--label",
            "epic:acceptance",
            "--label",
            "satisfies:REQ-01",
            "--json",
        ],
    );
    let implementation_id = implementation["id"].as_str().unwrap();
    success_json(
        &repo.path,
        &["dep", "add", implementation_id, breakdown_id, "--json"],
    );
    success_json(
        &repo.path,
        &[
            "dep",
            "add",
            container_id,
            implementation_id,
            "--reduce",
            "--json",
        ],
    );

    success_json(
        &repo.path,
        &["issue", "claim", breakdown_id, "agent:acceptance", "--json"],
    );
    let blocked_breakdown = failed_json(
        &repo.path,
        &["issue", "update", breakdown_id, "--state", "done", "--json"],
        4,
    );
    assert_eq!(
        blocked_breakdown["error"]["details"]["actual_state"],
        "gated"
    );
    let coverage = success_json(
        &repo.path,
        &[
            "gate",
            "evaluate",
            breakdown_id,
            "coverage-preview",
            "--json",
        ],
    );
    assert_eq!(coverage["status"], "passed");
    assert_eq!(coverage["warnings"], Value::Array(Vec::new()));
    let breakdown_review = success_json(
        &repo.path,
        &[
            "gate",
            "evaluate",
            breakdown_id,
            "breakdown-review",
            "--json",
        ],
    );
    assert_eq!(breakdown_review["status"], "passed");
    assert!(breakdown_review["warnings"][0]
        .as_str()
        .unwrap()
        .contains("EXTERNAL REVIEW PLACEHOLDER"));
    success_json(
        &repo.path,
        &["issue", "update", breakdown_id, "--state", "done", "--json"],
    );

    let statuses = success_json(
        &repo.path,
        &["issue", "status", breakdown_id, implementation_id, "--json"],
    );
    let by_id = statuses["issues"]
        .as_array()
        .unwrap()
        .iter()
        .map(|issue| {
            (
                issue["short_id"].as_str().unwrap().to_string(),
                issue["state"].as_str().unwrap().to_string(),
            )
        })
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        by_id[&breakdown_id[..8]],
        "done",
        "breakdown checkpoint must be approved"
    );
    assert_eq!(
        by_id[&implementation_id[..8]],
        "ready",
        "implementation leaf must be ready"
    );

    let validation = success_json(&repo.path, &["validate", "--json"]);
    assert_eq!(validation["valid"], true);
    assert_eq!(validation["error_count"], 0);
    assert!(validation["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["rule"] == "review-placeholder"));
    assert!(!repo.path.join(".git").exists());
}

#[test]
fn test_public_profile_schema_excludes_deferred_lifecycle_surface() {
    let repo = TestRepo::new();
    let schema = success_json(&repo.path, &["--schema"]);
    let commands = schema["commands"]["profile"]["subcommands"]
        .as_object()
        .unwrap();
    assert_eq!(
        commands.keys().cloned().collect::<BTreeSet<_>>(),
        expected_keys(&["apply", "list", "show"])
    );

    let show_schema = &commands["show"]["output"]["success_schema"];
    assert_eq!(
        property_keys(show_schema, "ProfileManifest"),
        expected_keys(&["asset", "contribution", "profile", "region"])
    );
    assert_eq!(
        property_keys(show_schema, "ProfileMetadata"),
        expected_keys(&["id", "jit", "manifest-version", "version"])
    );
    assert_eq!(
        property_keys(show_schema, "AppliedProfileRecord"),
        expected_keys(&["id", "origin", "package_hash", "target_hashes", "version"])
    );
    for definition in ["ProfileManifest", "ProfileMetadata", "AppliedProfileRecord"] {
        assert_eq!(
            show_schema["definitions"][definition]["additionalProperties"], false,
            "{definition} must reject undeclared lifecycle fields"
        );
    }
}
