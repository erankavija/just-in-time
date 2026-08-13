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
    plan["profiles"]
        .as_array()
        .expect("profile collection")
        .iter()
        .flat_map(|profile| {
            profile["targets"]
                .as_array()
                .expect("profile targets")
                .iter()
        })
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
    assert_eq!(no_op["count"], 2);
    assert_eq!(requested_profile(&no_op)["status"], "unchanged");
    assert_eq!(
        target(&no_op, "contrib/gates/ai-review.sh")["executable"],
        true
    );

    assert_eq!(snapshot_tree(&fresh.path), snapshot_tree(&existing.path));
    let fresh_events = normalized_events(&fresh.path);
    let existing_events = normalized_events(&existing.path);
    assert_eq!(fresh_events.len(), 1);
    assert_eq!(existing_events.len(), 1);
    assert_eq!(fresh_events[0]["operation"], "initialize");
    assert_eq!(existing_events[0]["operation"], "apply");
    let mut fresh_event = fresh_events[0].clone();
    let mut existing_event = existing_events[0].clone();
    fresh_event.as_object_mut().unwrap().remove("operation");
    existing_event.as_object_mut().unwrap().remove("operation");
    assert_eq!(fresh_event, existing_event);
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
    // Each package has its own provenance record; the selection has one aggregate event.
    assert!(repo.path.join(".jit/profiles/base.json").is_file());
    assert!(repo.path.join(".jit/profiles/workflow.json").is_file());
    let events = normalized_events(&repo.path);
    assert_eq!(events.len(), 1, "one aggregate lifecycle event is appended");
    assert_eq!(events[0]["type"], "profile_lifecycle");
    assert_eq!(events[0]["operation"], "apply");
    assert_eq!(
        events[0]["profiles"]
            .as_array()
            .expect("lifecycle event lists its profile outcomes")
            .iter()
            .map(|profile| profile["id"].as_str().unwrap().to_string())
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
fn test_public_profile_schema_states_the_shipped_lifecycle_surface() {
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
        expected_keys(&[
            "add",
            "apply",
            "capture",
            "diff",
            "list",
            "pack",
            "reconfigure",
            "show",
            "upgrade",
            "validate",
        ])
    );
    // Capture authors a package rather than selecting an applied one, so it
    // takes the two directories it works between instead of the selector
    // stream and value channels the lifecycle commands share. Pack and add
    // move a package between repositories, so each takes the package location
    // it works from and the archive it works through.
    for (command, flags) in [
        ("capture", ["destination", "dry-run", "json", "source"]),
        ("pack", ["dry-run", "json", "output", "source"]),
        ("add", ["archive", "destination", "dry-run", "json"]),
    ] {
        assert_eq!(
            commands[command]["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|flag| flag["name"].as_str().unwrap().to_string())
                .collect::<BTreeSet<_>>(),
            expected_keys(&flags),
            "{command} exposes the inputs it works between"
        );
    }
    // The three commands that change what a profile publishes take one input
    // surface: an ordered selector stream, the same value channels, and the
    // rehearsal that precedes publication.
    for lifecycle in ["apply", "reconfigure", "upgrade"] {
        assert_eq!(
            commands[lifecycle]["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|argument| argument["name"].as_str().unwrap().to_string())
                .collect::<BTreeSet<_>>(),
            expected_keys(&[]),
            "{lifecycle} takes its selection through flags"
        );
        assert_eq!(
            commands[lifecycle]["flags"]
                .as_array()
                .unwrap()
                .iter()
                .map(|flag| flag["name"].as_str().unwrap().to_string())
                .collect::<BTreeSet<_>>(),
            expected_keys(&["dry-run", "json", "profile", "set", "values-file"]),
            "{lifecycle} exposes one lifecycle input surface"
        );
    }

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
            "id",
            "manifest",
            "origin",
            "package_hash",
            "target_hashes",
            "version",
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
        expected_keys(&[
            "claims",
            "compatible_jit",
            "id",
            "origin",
            "package_hash",
            "record_version",
            "variables",
            "version",
        ])
    );
    assert_eq!(
        property_keys(show_schema, "ResolvedVariable"),
        expected_keys(&["source", "value"])
    );
    assert_eq!(
        show_schema["definitions"]["AppliedProfileRecord"]["properties"]["variables"]
            ["additionalProperties"]["$ref"],
        "#/definitions/ResolvedVariable"
    );
    assert_eq!(
        show_schema["definitions"]["AppliedProfileRecord"]["properties"]["claims"]["items"]["$ref"],
        "#/definitions/AppliedProfileClaim"
    );
    for definition in [
        "ProfilePackageModel",
        "AppliedProfileRecord",
        "AppliedProfileClaim",
        "ResolvedVariable",
    ] {
        assert_eq!(
            show_schema["definitions"][definition]["additionalProperties"], false,
            "{definition} must reject undeclared lifecycle fields"
        );
    }

    let apply_schemas = commands["apply"]["output"]["success_schema"]["oneOf"]
        .as_array()
        .unwrap();
    assert_eq!(apply_schemas.len(), 2);
    let apply_schema = apply_schemas
        .iter()
        .find(|schema| schema["definitions"].get("ProfileApplyResult").is_some())
        .expect("apply output schema");
    let plan_schema = apply_schemas
        .iter()
        .find(|schema| schema["definitions"].get("ProfilePlanEntry").is_some())
        .expect("rehearsal output schema");
    assert_eq!(
        apply_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles"])
    );
    assert_eq!(
        property_keys(apply_schema, "ProfileApplyResult"),
        expected_keys(&[
            "contributions",
            "id",
            "origin",
            "plan_hash",
            "status",
            "targets",
            "transaction_id",
            "version",
            "warnings",
        ])
    );
    assert_eq!(
        plan_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles"])
    );
    assert_eq!(
        property_keys(plan_schema, "ProfilePlanEntry"),
        expected_keys(&[
            "contributions",
            "id",
            "origin",
            "plan_hash",
            "status",
            "targets",
            "version",
        ])
    );
    assert_eq!(
        variant_property_keys(apply_schema, "ProfileApplicationWarning"),
        expected_keys(&["kind", "reason", "transaction_id"])
    );
    assert_eq!(
        property_keys(plan_schema, "ProfileTargetChange"),
        expected_keys(&["action", "executable", "owners", "path", "reason"])
    );
    assert_eq!(
        property_keys(plan_schema, "ProfileContributionChange"),
        expected_keys(&["action", "identity", "owners", "reason"])
    );
}

#[test]
fn test_release_profile_dry_run_smoke_uses_count_wrapped_result_contract() {
    let workflow_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/release-artifacts.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {workflow_path:?}: {error}"));

    assert!(
        workflow.contains("jit init --profile path:packages/jit-dogfood --json > init.json"),
        "release smoke must use the repeatable selector syntax for init"
    );
    assert!(
        workflow
            .contains("jit profile apply --profile id:jit-dogfood --dry-run --json > plan.json"),
        "release smoke must use the repeatable selector syntax for dry-run"
    );
    assert!(
        workflow.contains("assert plan[\"count\"] == len(plan[\"profiles\"]) >= 1, plan"),
        "release smoke must validate the count-wrapped dry-run envelope"
    );
    assert!(
        workflow.contains(
            "assert all(profile[\"status\"] == \"unchanged\" for profile in plan[\"profiles\"]), plan"
        ),
        "release smoke must validate every aggregate profile observation"
    );
    assert!(
        !workflow.contains("assert plan[\"status\"] == \"unchanged\", plan"),
        "release smoke must not read the removed top-level dry-run status"
    );
}

// ===========================================================================
// jit:f2389b18 — `jit profile validate` checks the profiles this repository
// records against the packages they came from and the content they own.
// ===========================================================================

/// The agreement report `jit profile validate --json` produced, beside the
/// status it exited with.
///
/// The report is the answer whether or not anything diverged, so a divergent
/// run's report is unwrapped from the shared typed error envelope rather than
/// read as a different shape.
fn profile_agreement(repo: &Path) -> (Option<i32>, Value) {
    let output = jit_with_path(repo, &["profile", "validate", "--json"], None);
    let parsed = parse_json(&output);
    let report = parsed
        .pointer("/error/details")
        .cloned()
        .unwrap_or_else(|| parsed.clone());
    assert_eq!(
        report["count"].as_u64().map(|count| count as usize),
        report["profiles"].as_array().map(Vec::len),
        "the report uses the count-wrapped list envelope: {parsed}"
    );
    (output.status.code(), report)
}

/// Every divergence the report carries for profile `id`, each as its kind
/// beside the target it names when it names one.
fn divergences(report: &Value, id: &str) -> Vec<(String, Option<String>)> {
    report["profiles"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries a profile collection: {report}"))
        .iter()
        .find(|profile| profile["id"] == id)
        .unwrap_or_else(|| panic!("the report names profile {id}: {report}"))["divergences"]
        .as_array()
        .unwrap_or_else(|| panic!("a profile entry carries its divergences: {report}"))
        .iter()
        .map(|divergence| {
            (
                divergence["kind"]
                    .as_str()
                    .expect("a divergence names its kind")
                    .to_string(),
                divergence["target"].as_str().map(str::to_string),
            )
        })
        .collect()
}

/// A repository holding the asset-only fixture package at `packages/planner`,
/// applied, with the target that package owns.
fn repository_with_applied_planner() -> (TestRepo, PathBuf) {
    const LOCATION: &str = "packages/planner";

    let repo = TestRepo::new();
    let directory =
        jit::test_utils::copy_package_tree(&composition_package(), &repo.path.join(LOCATION));
    let target = jit::profile::ProfilePackage::from_directory(&directory)
        .expect("a valid package tree")
        .model()
        .assets
        .first()
        .expect("the fixture package declares one asset")
        .target
        .clone();
    success_json(
        &repo.path,
        &["init", "--profile", &format!("path:{LOCATION}"), "--json"],
    );
    let target = repo.path.join(target);
    (repo, target)
}

/// A repository that just applied a package agrees with it, and saying so
/// leaves the repository byte-for-byte as it was (REQ-01, REQ-03).
#[test]
fn test_profile_validate_agrees_with_a_freshly_applied_package_without_writing() {
    let (repo, _) = repository_with_applied_planner();
    let before = snapshot_tree(&repo.path);
    let events = normalized_events(&repo.path);

    let (status, report) = profile_agreement(&repo.path);

    assert_eq!(status, Some(0), "an agreeing repository is a clean check");
    assert_eq!(
        report["count"], 1,
        "the recorded profile is the whole inventory: {report}"
    );
    assert!(
        divergences(&report, "planner-asset-only").is_empty(),
        "{report}"
    );
    assert_eq!(
        snapshot_tree(&repo.path),
        before,
        "the check writes nothing"
    );
    assert_eq!(normalized_events(&repo.path), events);
}

/// An owned target edited in place no longer holds what the profile published,
/// and reporting that writes nothing (REQ-01, REQ-03).
#[test]
fn test_profile_validate_reports_an_owned_target_edited_in_place_without_writing() {
    let (repo, target) = repository_with_applied_planner();
    fs::write(&target, b"edited by hand\n").expect("edit the profile-owned target in place");
    let before = snapshot_tree(&repo.path);
    let events = normalized_events(&repo.path);

    let (status, report) = profile_agreement(&repo.path);

    assert_eq!(status, Some(4), "a diverged profile fails the check");
    assert_eq!(
        divergences(&report, "planner-asset-only"),
        vec![(
            "changed_target".to_string(),
            Some(
                target
                    .strip_prefix(&repo.path)
                    .expect("the target is repository-relative")
                    .to_string_lossy()
                    .into_owned()
            )
        )],
        "{report}"
    );
    assert_eq!(
        snapshot_tree(&repo.path),
        before,
        "a divergent check writes nothing"
    );
    assert_eq!(normalized_events(&repo.path), events);
}

/// An ownership claim whose target is gone is reported rather than ignored
/// (REQ-02).
#[test]
fn test_profile_validate_reports_an_ownership_claim_whose_target_no_longer_exists() {
    let (repo, target) = repository_with_applied_planner();
    fs::remove_file(&target).expect("delete the profile-owned target");

    let (status, report) = profile_agreement(&repo.path);

    assert_eq!(status, Some(4));
    assert!(
        divergences(&report, "planner-asset-only")
            .iter()
            .any(|(kind, named)| kind == "absent_target"
                && named.as_deref()
                    == target
                        .strip_prefix(&repo.path)
                        .expect("the target is repository-relative")
                        .to_str()),
        "{report}"
    );
}

/// A record whose package is gone reports the unreadable package and every
/// target it still claims, rather than one standing for the other (REQ-01,
/// REQ-02).
#[test]
fn test_profile_validate_reports_a_record_whose_package_is_no_longer_readable() {
    let (repo, target) = repository_with_applied_planner();
    fs::remove_dir_all(repo.path.join("packages/planner")).expect("delete the recorded package");

    let (status, report) = profile_agreement(&repo.path);

    assert_eq!(status, Some(4));
    let reported = divergences(&report, "planner-asset-only");
    assert!(
        reported
            .iter()
            .any(|(kind, _)| kind == "unreadable_package"),
        "{report}"
    );
    assert!(
        reported.iter().any(|(kind, named)| kind == "unowned_target"
            && named.as_deref()
                == target
                    .strip_prefix(&repo.path)
                    .expect("the target is repository-relative")
                    .to_str()),
        "the targets a vanished package still claims are reported: {report}"
    );
    // The target itself is untouched, so nothing but the missing package
    // accounts for the report.
    assert!(target.is_file());
}

/// A recorded location holding a different package than the record identifies
/// is reported with both identities, read from the record and the package
/// rather than pinned in the test (REQ-01).
#[test]
fn test_profile_validate_reports_a_package_that_no_longer_matches_the_recorded_identity() {
    let (repo, _) = repository_with_applied_planner();
    let manifest = repo.path.join("packages/planner/manifest.toml");
    let authored = fs::read_to_string(&manifest).expect("read the recorded package manifest");
    fs::write(
        &manifest,
        authored.replace("version = \"1.0.0\"", "version = \"2.0.0\""),
    )
    .expect("re-version the recorded package in place");

    let recorded: Value = serde_json::from_slice(
        &fs::read(repo.path.join(".jit/profiles/planner-asset-only.json"))
            .expect("read the applied-profile record"),
    )
    .expect("the record parses");
    let current = success_json(
        &repo.path,
        &[
            "profile",
            "show",
            "--profile",
            "path:packages/planner",
            "--json",
        ],
    );
    let (status, report) = profile_agreement(&repo.path);

    assert_eq!(status, Some(4));
    let entry = report["profiles"]
        .as_array()
        .expect("a profile collection")
        .iter()
        .find(|profile| profile["id"] == "planner-asset-only")
        .expect("the recorded profile is reported");
    let reported = entry["divergences"]
        .as_array()
        .expect("divergences")
        .iter()
        .find(|divergence| divergence["kind"] == "changed_package_identity")
        .unwrap_or_else(|| panic!("the changed identity is reported: {report}"));
    assert_eq!(reported["recorded_version"], recorded["version"]);
    assert_eq!(reported["recorded_package_hash"], recorded["package_hash"]);
    assert_eq!(
        reported["current_version"],
        current["profiles"][0]["manifest"]["version"]
    );
    assert_eq!(
        reported["current_package_hash"],
        current["profiles"][0]["package_hash"]
    );
}

/// The declarations and managed regions a package contributes are owned as
/// precisely as its files: an edited registry declaration is named by its own
/// semantic identity, a removed region by its document and region id, and each
/// under the profile that published it rather than under the selection as a
/// whole (REQ-01, REQ-02).
#[test]
fn test_profile_validate_names_the_owned_declaration_and_region_a_hand_edit_changed() {
    let repo = TestRepo::new();
    let location = crate::repository_package_at(&repo.path, "jit-dogfood");
    success_json(
        &repo.path,
        &["init", "--profile", &format!("path:{location}"), "--json"],
    );
    assert_eq!(
        profile_agreement(&repo.path).0,
        Some(0),
        "the repository this package just configured agrees with it, \
         declarations, assets and regions alike"
    );

    // One owned registry declaration and one owned managed region, both changed
    // the way an adopter would change them: in the repository file that carries
    // them.
    let config = repo.path.join(".jit/config.toml");
    let authored = fs::read_to_string(&config).expect("read the contributed configuration");
    fs::write(
        &config,
        authored.replace("development_root = \"", "development_root = \"edited/"),
    )
    .expect("edit an owned declaration in place");
    let agents = repo.path.join("AGENTS.md");
    let document = fs::read_to_string(&agents).expect("read the contributed document");
    let (begin, end) = (
        "<!-- jit:dogfood-guidance:begin -->",
        "<!-- jit:dogfood-guidance:end -->",
    );
    let region_at = document.find(begin).expect("the region was published");
    let region_to = document.find(end).expect("the region was published") + end.len();
    fs::write(
        &agents,
        format!("{}{}", &document[..region_at], &document[region_to..]),
    )
    .expect("remove the owned region from the document");

    let (status, report) = profile_agreement(&repo.path);

    assert_eq!(status, Some(4));
    // The scalar belongs to the package `jit-dogfood` depends on, and the region
    // to `jit-dogfood` itself, so each divergence is reported under the record
    // that claims it.
    assert_eq!(
        divergences(&report, "jit-default"),
        vec![(
            "changed_target".to_string(),
            Some(".jit/config.toml:scalar:documentation-development-root".to_string())
        )],
        "the edited declaration is named by its own identity: {report}"
    );
    assert_eq!(
        divergences(&report, "jit-dogfood"),
        vec![(
            "absent_target".to_string(),
            Some("AGENTS.md#dogfood-guidance".to_string())
        )],
        "the removed region is named by its document and region id: {report}"
    );
}

/// Repository-wide validation reports the same owned-target divergence the
/// profile check reports, because both read it from the same comparison
/// (REQ-04).
#[test]
fn test_repository_validation_reports_the_owned_target_divergence_the_profile_check_reports() {
    let (repo, target) = repository_with_applied_planner();
    assert_eq!(
        success_json(&repo.path, &["validate", "--json"])["valid"],
        true,
        "an agreeing repository validates"
    );
    fs::write(&target, b"edited by hand\n").expect("edit the profile-owned target in place");

    let owned = divergences(&profile_agreement(&repo.path).1, "planner-asset-only");
    let repository = failed_json_with_path(&repo.path, &["validate", "--json"], 4, None);
    let findings = repository["error"]["details"]["rule_findings"]
        .as_array()
        .unwrap_or_else(|| panic!("validation reports its findings: {repository}"))
        .iter()
        .filter(|finding| finding["rule"] == "profile-ownership")
        .map(|finding| {
            finding["message"]
                .as_str()
                .expect("a finding carries a message")
                .to_string()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        findings.len(),
        owned.len(),
        "one finding per divergence the profile check reports: {repository}"
    );
    for (_, named) in &owned {
        let named = named
            .as_deref()
            .expect("an owned-target divergence names it");
        assert!(
            findings
                .iter()
                .any(|finding| finding.contains(named) && finding.contains("planner-asset-only")),
            "validation names the same profile and target: {findings:?}"
        );
    }
}

// ===========================================================================
// jit:96268a98 — `jit profile diff` states what a selection would change
// before anything is published.
// ===========================================================================

/// The difference report `jit profile diff --json` produced for `selector`,
/// beside the status it exited with.
///
/// The report is the answer whether or not the selection can be published, so
/// an unpublishable run's report is unwrapped from the shared typed error
/// envelope rather than read as a different shape.
fn profile_difference(repo: &Path, selector: &str) -> (Option<i32>, Value) {
    let output = jit_with_path(
        repo,
        &["profile", "diff", "--profile", selector, "--json"],
        None,
    );
    let parsed = parse_json(&output);
    let report = parsed
        .pointer("/error/details")
        .cloned()
        .unwrap_or_else(|| parsed.clone());
    assert_eq!(
        report["count"].as_u64().map(|count| count as usize),
        report["profiles"].as_array().map(Vec::len),
        "the report uses the count-wrapped list envelope: {parsed}"
    );
    (output.status.code(), report)
}

/// The decisions the report carries for profile `id`, each as its action beside
/// the target it names and the packages that claim it.
fn decisions(report: &Value, id: &str) -> Vec<(String, String, Vec<String>)> {
    report["profiles"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries a profile collection: {report}"))
        .iter()
        .find(|profile| profile["id"] == id)
        .unwrap_or_else(|| panic!("the report names profile {id}: {report}"))["targets"]
        .as_array()
        .unwrap_or_else(|| panic!("a profile entry carries its targets: {report}"))
        .iter()
        .map(|target| {
            (
                target["action"]
                    .as_str()
                    .expect("a decision names its action")
                    .to_string(),
                target["path"]
                    .as_str()
                    .expect("a decision names its target")
                    .to_string(),
                target["owners"]
                    .as_array()
                    .expect("a decision names the packages that claim its target")
                    .iter()
                    .map(|owner| {
                        owner
                            .as_str()
                            .expect("an owner is a package id")
                            .to_string()
                    })
                    .collect(),
            )
        })
        .collect()
}

/// A repository that has applied nothing, holding the asset-only fixture
/// package at `packages/planner` beside the target that package declares.
fn repository_with_unapplied_planner() -> (TestRepo, String) {
    const LOCATION: &str = "packages/planner";

    let repo = TestRepo::new();
    let directory =
        jit::test_utils::copy_package_tree(&composition_package(), &repo.path.join(LOCATION));
    let target = jit::profile::ProfilePackage::from_directory(&directory)
        .expect("a valid package tree")
        .model()
        .assets
        .first()
        .expect("the fixture package declares one asset")
        .target
        .clone();
    success_json(&repo.path, &["init", "--json"]);
    (repo, target)
}

/// A package this repository has not applied is reported from the location it
/// was selected at, and reporting it writes nothing (REQ-01, REQ-02, REQ-04).
#[test]
fn test_profile_diff_reports_an_unapplied_package_without_writing() {
    let (repo, target) = repository_with_unapplied_planner();
    let before = snapshot_tree(&repo.path);
    let events = normalized_events(&repo.path);

    let (status, report) = profile_difference(&repo.path, "path:packages/planner");

    assert_eq!(status, Some(0), "a publishable selection is a clean check");
    assert!(
        decisions(&report, "planner-asset-only").contains(&(
            "create".to_string(),
            target.clone(),
            Vec::new()
        )),
        "the report states the target the package would publish, claimed by no package: {report}"
    );
    assert_eq!(
        snapshot_tree(&repo.path),
        before,
        "the report writes nothing"
    );
    assert_eq!(normalized_events(&repo.path), events);
}

/// A target the repository authored cannot be published, and stating that keeps
/// the report as the answer while carrying the repository-check exit status
/// (REQ-01, REQ-02).
#[test]
fn test_profile_diff_exits_on_an_unpublishable_target_it_still_reports() {
    let (repo, target) = repository_with_unapplied_planner();
    let authored = repo.path.join(&target);
    fs::create_dir_all(
        authored
            .parent()
            .expect("the target sits below the worktree"),
    )
    .expect("create the authored target's directory");
    fs::write(&authored, b"authored by hand\n").expect("author the target by hand");
    let before = snapshot_tree(&repo.path);

    let (status, report) = profile_difference(&repo.path, "path:packages/planner");

    assert_eq!(
        status,
        Some(4),
        "a selection that cannot be published fails the check: {report}"
    );
    assert!(
        decisions(&report, "planner-asset-only").contains(&(
            "conflict".to_string(),
            target.clone(),
            Vec::new()
        )),
        "the report states the target it cannot publish, claimed by no package: {report}"
    );
    assert_eq!(
        snapshot_tree(&repo.path),
        before,
        "the report writes nothing"
    );
}

/// The declaration decisions the report carries for profile `id`, each as its
/// action beside the semantic identity it names and the packages that claim it.
fn declarations(report: &Value, id: &str) -> Vec<(String, String, Vec<String>)> {
    report["profiles"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries a profile collection: {report}"))
        .iter()
        .find(|profile| profile["id"] == id)
        .unwrap_or_else(|| panic!("the report names profile {id}: {report}"))["contributions"]
        .as_array()
        .unwrap_or_else(|| panic!("a profile entry carries its declarations: {report}"))
        .iter()
        .map(|declaration| {
            (
                declaration["action"]
                    .as_str()
                    .expect("a decision names its action")
                    .to_string(),
                declaration["identity"]
                    .as_str()
                    .expect("a decision names the declaration it is about")
                    .to_string(),
                declaration["owners"]
                    .as_array()
                    .expect("a decision names the packages that claim its declaration")
                    .iter()
                    .map(|owner| {
                        owner
                            .as_str()
                            .expect("an owner is a package id")
                            .to_string()
                    })
                    .collect(),
            )
        })
        .collect()
}

/// The package at `location`, declaring `id` and contributing the label
/// namespace `namespace` described as `description`.
///
/// Contributions are what a package states about a registry rather than about a
/// file, so a report over one is authored from the same fixture tree the target
/// cases stage.
fn package_contributing(repo: &TestRepo, location: &str, id: &str, description: &str) {
    let directory = repo.path.join(location);
    jit::test_utils::write_package_declaring(&composition_package(), &directory, id, &[]);
    let manifest = directory.join(jit::profile::MANIFEST_FILE_NAME);
    let authored = fs::read_to_string(&manifest).expect("read the staged package manifest");
    fs::write(
        &manifest,
        format!(
            "{authored}\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"namespaces\"\n\
             identity = \"{CONTRIBUTED_NAMESPACE}\"\n\
             value = {{ description = \"{description}\", unique = false }}\n"
        ),
    )
    .expect("declare the package's contribution");
}

/// The label namespace every declaration case below contributes.
const CONTRIBUTED_NAMESPACE: &str = "reported-vocabulary";

/// The canonical identity of the contributed namespace declaration, read from
/// the engine's own identity vocabulary rather than spelled here.
fn contributed_identity() -> String {
    jit::repository_state::Contribution::MapEntry {
        target: jit::repository_state::MapEntryTarget::Namespaces,
        identity: CONTRIBUTED_NAMESPACE.to_string(),
        value: serde_json::json!({}),
    }
    .semantic_identity()
    .to_string()
}

/// A report states the declarations a selection would publish beside its file
/// targets: one no package claims is created and unowned, and one two packages
/// define differently is stated as the conflict it is, named by the package
/// that claims it (REQ-01, REQ-02).
#[test]
fn test_profile_diff_states_the_declarations_a_selection_would_publish() {
    let repo = TestRepo::new();
    success_json(&repo.path, &["init", "--json"]);
    package_contributing(&repo, "packages/base", "base", "The base meaning.");
    package_contributing(
        &repo,
        "packages/workflow",
        "workflow",
        "The workflow meaning.",
    );

    let (status, report) = profile_difference(&repo.path, "path:packages/base");

    assert_eq!(status, Some(0), "a publishable selection is a clean check");
    assert!(
        declarations(&report, "base").contains(&(
            "create".to_string(),
            contributed_identity(),
            Vec::new()
        )),
        "the report states the declaration the package would publish, claimed by no package: \
         {report}"
    );

    success_json(
        &repo.path,
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/base",
            "--json",
        ],
    );
    let before = snapshot_tree(&repo.path);

    let (status, report) = profile_difference(&repo.path, "path:packages/workflow");

    assert_eq!(
        status,
        Some(4),
        "a selection that cannot be published fails the check: {report}"
    );
    assert!(
        declarations(&report, "workflow").contains(&(
            "conflict".to_string(),
            contributed_identity(),
            vec!["base".to_string()]
        )),
        "the report states the declaration it cannot publish, claimed by the package that \
         declared it first: {report}"
    );
    assert_eq!(
        snapshot_tree(&repo.path),
        before,
        "the report writes nothing"
    );
}
