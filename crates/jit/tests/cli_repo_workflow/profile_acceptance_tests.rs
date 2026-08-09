use super::profile_cli_tests::{applied_ids, requested_profile};
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

#[cfg(unix)]
struct CheckerPath {
    _directory: TempDir,
    value: PathBuf,
}

#[cfg(unix)]
fn checker_path_without_jq() -> CheckerPath {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new().expect("create checker PATH");
    for command in [
        "bash", "cat", "cut", "grep", "head", "mktemp", "rm", "sed", "sh", "tail", "wc",
    ] {
        let source = std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|entry| entry.join(command))
            .find(|candidate| candidate.is_file())
            .unwrap_or_else(|| panic!("required checker command is unavailable: {command}"));
        symlink(&source, directory.path().join(command))
            .unwrap_or_else(|error| panic!("link {command} into checker PATH: {error}"));
    }
    assert!(
        !directory.path().join("jq").exists(),
        "the offline checker PATH must not expose jq"
    );
    CheckerPath {
        value: directory.path().to_path_buf(),
        _directory: directory,
    }
}

fn jit_with_path(repo: &Path, args: &[&str], path: Option<&Path>) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jit"));
    command
        .args(args)
        .current_dir(repo)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("JIT_DATA_DIR")
        .env_remove("JIT_SRC")
        .env("HTTP_PROXY", "http://127.0.0.1:9")
        .env("HTTPS_PROXY", "http://127.0.0.1:9")
        .env("ALL_PROXY", "http://127.0.0.1:9")
        .env("NO_PROXY", "");
    if let Some(path) = path {
        command.env("PATH", path);
    }
    command
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
    success_json_with_path(repo, args, None)
}

fn success_json_with_path(repo: &Path, args: &[&str], path: Option<&Path>) -> Value {
    let output = jit_with_path(repo, args, path);
    assert!(
        output.status.success(),
        "jit {args:?} failed\nstatus={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    parse_json(&output)
}

fn failed_json_with_path(repo: &Path, args: &[&str], exit_code: i32, path: Option<&Path>) -> Value {
    let output = jit_with_path(repo, args, path);
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
    plan["profiles"][0]["targets"]
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

fn variant_property_keys(schema: &Value, definition: &str) -> BTreeSet<String> {
    schema["definitions"][definition]["oneOf"][0]["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("missing variant properties for schema definition {definition}"))
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

    // Both worktrees carry the same staged packages, so the trees compared at
    // the end differ only in how the profile got there.
    let fresh_location = crate::repository_package_at(&fresh.path, "jit-dogfood");
    let existing_location = crate::repository_package_at(&existing.path, "jit-dogfood");
    let fresh_selector = format!("path:{fresh_location}");
    assert_eq!(fresh_location, existing_location);
    let fresh_init = success_json(
        &fresh.path,
        &["init", "--profile", &fresh_selector, "--json"],
    );
    assert_eq!(
        requested_profile(&fresh_init["profile"])["status"],
        "applied"
    );

    success_json(&existing.path, &["init", "--json"]);
    // A preview is derived over one package, so the self-contained one is what
    // a repository declaring nothing can be shown. It writes nothing.
    let existing_default_location = crate::repository_package_at(&existing.path, "jit-default");
    let existing_default_selector = format!("path:{existing_default_location}");
    let preview = success_json(
        &existing.path,
        &[
            "profile",
            "apply",
            "--profile",
            &existing_default_selector,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["count"], 1);
    assert_eq!(preview["profiles"][0]["status"], "would_apply");
    assert!(!existing.path.join(".jit/profiles").exists());

    let applied = success_json(
        &existing.path,
        &[
            "profile",
            "apply",
            "--profile",
            &format!("path:{existing_location}"),
            "--json",
        ],
    );
    assert_eq!(requested_profile(&applied)["status"], "applied");
    let no_op = success_json(
        &existing.path,
        &[
            "profile",
            "apply",
            "--profile",
            "id:jit-dogfood",
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(no_op["count"], 1);
    assert_eq!(no_op["profiles"][0]["status"], "unchanged");
    assert_eq!(
        target(&no_op, "contrib/gates/ai-review.sh")["executable"],
        true
    );

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
        let script_mode = fs::metadata(existing.path.join("contrib/gates/ai-review.sh"))
            .unwrap()
            .permissions()
            .mode();
        let prompt_mode = fs::metadata(existing.path.join("contrib/gates/code-review-prompt.md"))
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
        let permissions = fs::metadata(existing.path.join("contrib/gates/ai-review.sh"))
            .unwrap()
            .permissions();
        assert!(
            !permissions.readonly(),
            "Windows preserves executable intent in the public plan, not a Unix mode bit"
        );
    }
}

/// The checked-in fixture tree every composition case below stages copies of,
/// each under a rewritten manifest declaring its own id.
fn composition_package() -> std::path::PathBuf {
    jit::test_utils::profile_package_fixture("planner-asset-only")
}

#[test]
fn test_profile_apply_applies_the_packages_the_named_one_depends_on() {
    let repo = TestRepo::new();
    success_json(&repo.path, &["init", "--json"]);
    // An obtained set of packages, side by side inside the worktree.
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.path.join("packages/base"),
        "base",
        &[],
    );
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.path.join("packages/workflow"),
        "workflow",
        &["base"],
    );

    let applied = success_json(
        &repo.path,
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/workflow",
            "--json",
        ],
    );

    // One result per applied package, the dependency first, under the standard
    // count-wrapped shape.
    assert_eq!(applied_ids(&applied), vec!["base", "workflow"]);
    assert_eq!(applied["count"], 2);
    assert_eq!(requested_profile(&applied)["status"], "applied");
    // Each package's own provenance record and its own audit event.
    assert!(repo.path.join(".jit/profiles/base.json").is_file());
    assert!(repo.path.join(".jit/profiles/workflow.json").is_file());
    assert_eq!(
        normalized_events(&repo.path)
            .iter()
            .filter_map(|event| event["profile_id"].as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        vec!["base".to_string(), "workflow".to_string()]
    );

    // Re-applying an applied set is no work anywhere in it.
    let reapplied = success_json(
        &repo.path,
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/workflow",
            "--json",
        ],
    );
    assert_eq!(applied_ids(&reapplied), vec!["base", "workflow"]);
    assert!(reapplied["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .all(|profile| profile["status"] == "unchanged"));
}

/// A repository that applied a directory package validates, and repairs the
/// targets that package owns from the location its record names.
///
/// The recorded location is the only place the package's bytes exist, so every
/// fact validation states about it — that the repository is coherent, and the
/// bytes it restores to a deleted target — can only have come from reading that
/// location again.
#[test]
fn test_validate_repairs_a_profile_applied_from_a_directory() {
    const LOCATION: &str = "packages/planner";

    let repo = TestRepo::new();
    let directory =
        jit::test_utils::copy_package_tree(&composition_package(), &repo.path.join(LOCATION));
    let package = jit::profile::ProfilePackage::from_directory(&directory)
        .expect("a valid package tree")
        .model()
        .clone();
    let declared = package
        .assets
        .first()
        .expect("the fixture package declares one asset");
    let asset = fs::read_to_string(directory.join(&declared.source)).expect("read package asset");

    let selector = format!("path:{LOCATION}");
    success_json(&repo.path, &["init", "--profile", &selector, "--json"]);

    assert_eq!(
        success_json(&repo.path, &["validate", "--json"])["valid"],
        true,
        "a repository that correctly applied a directory package is coherent"
    );

    let target = repo.path.join(&declared.target);
    fs::remove_file(&target).expect("delete the profile-owned target");
    let repaired = success_json(&repo.path, &["validate", "--fix", "--json"]);

    assert!(
        repaired["fixes_applied"].as_u64().unwrap_or_default() > 0,
        "the deleted profile-owned target is repairable: {repaired}"
    );
    assert_eq!(
        fs::read_to_string(&target).expect("the deleted target is restored"),
        asset,
        "repair restores the bytes the recorded location holds"
    );
    assert_eq!(
        success_json(&repo.path, &["validate", "--json"])["valid"],
        true
    );
}

#[test]
fn test_init_profile_reports_a_dependency_that_cannot_be_resolved() {
    let repo = TestRepo::new();
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.path.join("packages/workflow"),
        "workflow",
        &["absent-base"],
    );

    let failure = failed_json_with_path(
        &repo.path,
        &["init", "--profile", "path:packages/workflow", "--json"],
        1,
        None,
    );

    assert_eq!(failure["error"]["code"], "PROFILE_ERROR");
    let message = failure["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("a failure carries a message: {failure}"));
    assert!(message.contains("workflow"), "{message}");
    assert!(message.contains("absent-base"), "{message}");
    assert!(
        !repo.path.join(".jit").exists(),
        "the set is resolved before a repository is created"
    );
}

#[test]
fn test_profile_apply_reports_a_dependency_that_cannot_be_resolved() {
    let repo = TestRepo::new();
    success_json(&repo.path, &["init", "--json"]);
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.path.join("packages/workflow"),
        "workflow",
        &["absent-base"],
    );

    let failure = failed_json_with_path(
        &repo.path,
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/workflow",
            "--json",
        ],
        1,
        None,
    );

    // Both packages are named: the one the adopter asked for, and the one it
    // declared that could not be found.
    let message = failure["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("a failure carries a message: {failure}"));
    assert!(message.contains("workflow"), "{message}");
    assert!(message.contains("absent-base"), "{message}");
    assert!(!repo.path.join(".jit/profiles").exists());
    assert!(normalized_events(&repo.path).is_empty());
}

#[test]
fn test_profile_application_contributes_workflow_invariants_to_scaffolded_registry() {
    let repo = TestRepo::new();

    success_json(&repo.path, &["init", "--json"]);
    let dogfood_location = crate::repository_package_at(&repo.path, "jit-dogfood");
    let dogfood_selector = format!("path:{dogfood_location}");
    let applied = success_json(
        &repo.path,
        &["profile", "apply", "--profile", &dogfood_selector, "--json"],
    );
    assert_eq!(requested_profile(&applied)["status"], "applied");
    assert_eq!(
        success_json(&repo.path, &["validate", "--json"])["valid"],
        true
    );

    let registry: toml::Value = fs::read_to_string(repo.path.join(".jit/invariants.toml"))
        .expect("read contributed invariant registry")
        .parse()
        .expect("parse contributed invariant registry");
    let ids = registry["invariants"]
        .as_array()
        .expect("invariant registry array")
        .iter()
        .map(|entry| entry["id"].as_str().expect("invariant id").to_string())
        .collect::<BTreeSet<_>>();

    assert_eq!(ids.len(), 18);
    assert!(ids.contains("label-format"));
    assert!(ids.contains("convention-convergence"));
}

#[test]
#[cfg(unix)]
fn test_offline_public_cli_profile_reaches_implementation_ready_breakdown() {
    let repo = TestRepo::new();
    let checker_path = checker_path_without_jq();
    let path = Some(checker_path.value.as_path());
    let dogfood_location = crate::repository_package_at(&repo.path, "jit-dogfood");
    let dogfood_selector = format!("path:{dogfood_location}");
    success_json_with_path(
        &repo.path,
        &["init", "--profile", &dogfood_selector, "--json"],
        path,
    );
    assert!(!repo.path.join(".git").exists());

    let invariants = success_json_with_path(
        &repo.path,
        &["project", "render", "--name", "invariants", "--json"],
        path,
    );
    assert_eq!(invariants["projections"][0]["target"], "AGENTS.md");
    let references = success_json_with_path(
        &repo.path,
        &["project", "render", "--name", "rules-and-gates", "--json"],
        path,
    );
    assert_eq!(
        references["projections"][0]["target"],
        ".jit/reference/rules-and-gates.md"
    );
    let agents = fs::read_to_string(repo.path.join("AGENTS.md")).unwrap();
    assert!(agents.contains("<!-- jit:invariants:begin -->"));
    assert!(agents.contains("<!-- jit:invariants:end -->"));
    let rules_and_gates =
        fs::read_to_string(repo.path.join(".jit/reference/rules-and-gates.md")).unwrap();
    assert!(rules_and_gates.contains("## Gates"));
    assert!(rules_and_gates.contains("plan-review"));

    let container = success_json_with_path(
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
        path,
    );
    let container_id = container["id"].as_str().unwrap();
    let applied =
        success_json_with_path(&repo.path, &["apply", "plan", container_id, "--json"], path);
    let planning_id = applied["created_node_ids_by_role"]["planning"]
        .as_str()
        .unwrap();
    let breakdown_id = applied["created_node_ids_by_role"]["breakdown"]
        .as_str()
        .unwrap();

    success_json_with_path(
        &repo.path,
        &["issue", "claim", planning_id, "agent:acceptance", "--json"],
        path,
    );
    let blocked_plan = failed_json_with_path(
        &repo.path,
        &["issue", "update", planning_id, "--state", "done", "--json"],
        4,
        path,
    );
    assert_eq!(blocked_plan["error"]["details"]["actual_state"], "gated");
    let plan_review = success_json_with_path(
        &repo.path,
        &["gate", "evaluate", planning_id, "plan-review", "--json"],
        path,
    );
    assert_eq!(plan_review["status"], "passed");
    assert!(plan_review["warnings"][0]
        .as_str()
        .unwrap()
        .contains("EXTERNAL REVIEW PLACEHOLDER"));
    success_json_with_path(
        &repo.path,
        &["issue", "update", planning_id, "--state", "done", "--json"],
        path,
    );

    let implementation = success_json_with_path(
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
        path,
    );
    let implementation_id = implementation["id"].as_str().unwrap();
    success_json_with_path(
        &repo.path,
        &["dep", "add", implementation_id, breakdown_id, "--json"],
        path,
    );
    success_json_with_path(
        &repo.path,
        &[
            "dep",
            "add",
            container_id,
            implementation_id,
            "--reduce",
            "--json",
        ],
        path,
    );

    success_json_with_path(
        &repo.path,
        &["issue", "claim", breakdown_id, "agent:acceptance", "--json"],
        path,
    );
    let blocked_breakdown = failed_json_with_path(
        &repo.path,
        &["issue", "update", breakdown_id, "--state", "done", "--json"],
        4,
        path,
    );
    assert_eq!(
        blocked_breakdown["error"]["details"]["actual_state"],
        "gated"
    );
    let coverage = success_json_with_path(
        &repo.path,
        &[
            "gate",
            "evaluate",
            breakdown_id,
            "coverage-preview",
            "--json",
        ],
        path,
    );
    assert_eq!(coverage["status"], "passed");
    assert_eq!(coverage["warnings"], Value::Array(Vec::new()));
    let breakdown_review = success_json_with_path(
        &repo.path,
        &[
            "gate",
            "evaluate",
            breakdown_id,
            "breakdown-review",
            "--json",
        ],
        path,
    );
    assert_eq!(breakdown_review["status"], "passed");
    assert!(breakdown_review["warnings"][0]
        .as_str()
        .unwrap()
        .contains("EXTERNAL REVIEW PLACEHOLDER"));
    success_json_with_path(
        &repo.path,
        &["issue", "update", breakdown_id, "--state", "done", "--json"],
        path,
    );

    let statuses = success_json_with_path(
        &repo.path,
        &["issue", "status", breakdown_id, implementation_id, "--json"],
        path,
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

    let validation = success_json_with_path(&repo.path, &["validate", "--json"], path);
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
    assert!(commands["apply"]["description"]
        .as_str()
        .unwrap()
        .contains("recoverable multi-target transaction"));
    assert_eq!(
        commands.keys().cloned().collect::<BTreeSet<_>>(),
        expected_keys(&["apply", "list", "show"])
    );
    assert_eq!(
        commands["apply"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument["name"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        expected_keys(&[])
    );
    assert_eq!(
        commands["apply"]["flags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|flag| flag["name"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        expected_keys(&["dry-run", "json", "profile"])
    );

    let list_schema = &commands["list"]["output"]["success_schema"];
    assert_eq!(
        list_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles"])
    );
    assert_eq!(
        property_keys(list_schema, "ProfileSummary"),
        expected_keys(&["applied", "id", "jit", "origin", "version"])
    );

    let show_schema = &commands["show"]["output"]["success_schema"];
    assert_eq!(
        show_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles"])
    );
    assert_eq!(
        property_keys(show_schema, "ProfileShowEntry"),
        expected_keys(&[
            "applied",
            "byte_size",
            "file_count",
            "manifest",
            "origin",
            "package_hash",
            "target_hashes",
        ])
    );
    assert_eq!(
        property_keys(show_schema, "ProfilePackageModel"),
        expected_keys(&[
            "asset",
            "compatible-jit",
            "contribution",
            "dependency",
            "id",
            "incompatibility",
            "live-source",
            "region",
            "variable",
            "version",
        ])
    );
    assert_eq!(
        property_keys(show_schema, "AppliedProfileRecord"),
        expected_keys(&["id", "origin", "package_hash", "target_hashes", "version"])
    );
    for definition in ["ProfilePackageModel", "AppliedProfileRecord"] {
        assert_eq!(
            show_schema["definitions"][definition]["additionalProperties"], false,
            "{definition} must reject undeclared lifecycle fields"
        );
    }

    let apply_schemas = commands["apply"]["output"]["success_schema"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(apply_schemas.len(), 2);
    let by_title = apply_schemas
        .iter()
        .map(|schema| (schema["title"].as_str().unwrap(), schema))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        by_title["ProfileComposedApplyResult"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles"])
    );
    assert_eq!(
        property_keys(by_title["ProfileComposedApplyResult"], "ProfileApplyResult"),
        expected_keys(&[
            "id",
            "plan_hash",
            "status",
            "transaction_id",
            "version",
            "warnings",
        ])
    );
    assert_eq!(
        by_title["ProfilePlanResult"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles"])
    );
    assert_eq!(
        property_keys(by_title["ProfilePlanResult"], "ProfilePlanEntry"),
        expected_keys(&["id", "plan_hash", "status", "targets", "version"])
    );
    assert_eq!(
        variant_property_keys(
            by_title["ProfileComposedApplyResult"],
            "ProfileApplicationWarning"
        ),
        expected_keys(&["kind", "reason", "transaction_id"])
    );
    assert_eq!(
        property_keys(by_title["ProfilePlanResult"], "ProfileTargetChange"),
        expected_keys(&["action", "executable", "path"])
    );
}
