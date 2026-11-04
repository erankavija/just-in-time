use super::profile_cli_tests::{applied_ids, requested_profile};
use jit::test_cli::JitCli;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Output};
use tempfile::TempDir;

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

fn jit_with_path(repo: &JitCli, args: &[&str], path: Option<&Path>) -> Output {
    let mut command = repo.command_with_env(
        args,
        [
            ("HTTP_PROXY", "http://127.0.0.1:9"),
            ("HTTPS_PROXY", "http://127.0.0.1:9"),
            ("ALL_PROXY", "http://127.0.0.1:9"),
            ("NO_PROXY", ""),
        ],
    );
    command.env_remove("GIT_DIR").env_remove("GIT_WORK_TREE");
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

fn success_json(repo: &JitCli, args: &[&str]) -> Value {
    success_json_with_path(repo, args, None)
}

fn success_json_with_path(repo: &JitCli, args: &[&str], path: Option<&Path>) -> Value {
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

fn failed_json_with_path(repo: &JitCli, args: &[&str], path: Option<&Path>) -> Value {
    let output = jit_with_path(repo, args, path);
    assert!(
        !output.status.success(),
        "jit {args:?} unexpectedly succeeded\nstdout={}\nstderr={}",
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
pub(super) enum SnapshotEntry {
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
    snapshot_tree_excluding(root, is_machine_local_path)
}

/// Every directory and file under `root`, skipping relative paths `excluded`
/// names and everything below them.
pub(super) fn snapshot_tree_excluding(
    root: &Path,
    excluded: fn(&str) -> bool,
) -> BTreeMap<String, SnapshotEntry> {
    fn visit(
        root: &Path,
        current: &Path,
        excluded: fn(&str) -> bool,
        entries: &mut BTreeMap<String, SnapshotEntry>,
    ) {
        let mut children = fs::read_dir(current)
            .expect("read repository directory")
            .collect::<Result<Vec<_>, _>>()
            .expect("read repository entries");
        children.sort_by_key(|entry| entry.file_name());

        for child in children {
            let path = child.path();
            let relative = normalized_path(path.strip_prefix(root).expect("repository-relative"));
            if excluded(&relative) {
                continue;
            }
            let metadata = child.metadata().expect("read repository metadata");
            if metadata.is_dir() {
                entries.insert(relative, SnapshotEntry::Directory);
                visit(root, &path, excluded, entries);
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
    visit(root, root, excluded, &mut entries);
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

fn expected_keys(keys: &[&str]) -> BTreeSet<String> {
    keys.iter().map(|key| (*key).to_string()).collect()
}

#[test]
fn test_profile_fresh_init_and_existing_apply_are_equivalent_without_git() {
    // Both repositories share one basename, which `init` seeds into the
    // configuration the trees are compared on.
    let parents = [TempDir::new().unwrap(), TempDir::new().unwrap()];
    let [fresh, existing] = parents.each_ref().map(|parent| {
        JitCli::with_repository(
            tempfile::Builder::new()
                .prefix("repo")
                .rand_bytes(0)
                .tempdir_in(parent.path())
                .unwrap(),
        )
    });
    assert!(!fresh.root().join(".git").exists());
    assert!(!existing.root().join(".git").exists());

    // Both worktrees carry the same staged packages, so the trees compared at
    // the end differ only in how the profile got there.
    let fresh_location = crate::repository_package_at(fresh.root(), "jit-dogfood");
    let existing_location = crate::repository_package_at(existing.root(), "jit-dogfood");
    let fresh_selector = fresh_location.to_string();
    assert_eq!(fresh_location, existing_location);
    let fresh_init = success_json(&fresh, &["init", "--profile", &fresh_selector, "--json"]);
    assert_eq!(
        requested_profile(&fresh_init["profile"])["status"],
        "applied"
    );

    success_json(&existing, &["init", "--json"]);
    // A preview is derived over one package, so the self-contained one is what
    // a repository declaring nothing can be shown. It writes nothing.
    let existing_default_location = crate::repository_package_at(existing.root(), "jit-default");
    let existing_default_selector = existing_default_location.to_string();
    let preview = success_json(
        &existing,
        &[
            "profile",
            "apply",
            "--profile",
            &existing_default_selector,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(preview["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(preview["profiles"][0]["status"], "would_apply");

    let applied = success_json(
        &existing,
        &[
            "profile",
            "apply",
            "--profile",
            &existing_location,
            "--json",
        ],
    );
    assert_eq!(requested_profile(&applied)["status"], "applied");
    let no_op = success_json(
        &existing,
        &[
            "profile",
            "apply",
            "--profile",
            &existing_location,
            "--dry-run",
            "--json",
        ],
    );
    assert_eq!(no_op["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(requested_profile(&no_op)["status"], "unchanged");
    assert_eq!(
        target(&no_op, "contrib/gates/ai-review.sh")["executable"],
        true
    );

    assert_eq!(snapshot_tree(fresh.root()), snapshot_tree(existing.root()));
    let fresh_events = normalized_events(fresh.root());
    let existing_events = normalized_events(existing.root());
    assert_eq!(fresh_events.len(), 1);
    assert_eq!(existing_events.len(), 1);
    assert_eq!(fresh_events[0]["operation"], "initialize");
    assert_eq!(existing_events[0]["operation"], "apply");
    let mut fresh_event = fresh_events[0].clone();
    let mut existing_event = existing_events[0].clone();
    fresh_event.as_object_mut().unwrap().remove("operation");
    existing_event.as_object_mut().unwrap().remove("operation");
    assert_eq!(fresh_event, existing_event);
    assert!(!fresh.root().join(".git").exists());
    assert!(!existing.root().join(".git").exists());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script_mode = fs::metadata(existing.root().join("contrib/gates/ai-review.sh"))
            .unwrap()
            .permissions()
            .mode();
        let prompt_mode = fs::metadata(existing.root().join("contrib/gates/code-review-prompt.md"))
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
        let permissions = fs::metadata(existing.root().join("contrib/gates/ai-review.sh"))
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
    let repo = JitCli::uninitialised().unwrap();
    success_json(&repo, &["init", "--json"]);
    // An obtained set of packages, side by side inside the worktree.
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.root().join("packages/base"),
        "base",
        &[],
    );
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.root().join("packages/workflow"),
        "workflow",
        &["base"],
    );

    let applied = success_json(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            "packages/workflow",
            "--json",
        ],
    );

    // One result per applied package, the dependency first.
    assert_eq!(applied_ids(&applied), vec!["base", "workflow"]);
    assert_eq!(applied["profiles"].as_array().unwrap().len(), 2);
    assert_eq!(requested_profile(&applied)["status"], "applied");
    // The selection has one aggregate event.
    let events = normalized_events(repo.root());
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
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            "packages/workflow",
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

#[test]
fn test_init_profile_reports_a_dependency_that_cannot_be_resolved() {
    let repo = JitCli::uninitialised().unwrap();
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.root().join("packages/workflow"),
        "workflow",
        &["absent-base"],
    );

    let failure = failed_json_with_path(
        &repo,
        &["init", "--profile", "packages/workflow", "--json"],
        None,
    );

    assert_eq!(failure["error"]["code"], "PROFILE_ERROR");
    let message = failure["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("a failure carries a message: {failure}"));
    assert!(message.contains("workflow"), "{message}");
    assert!(message.contains("absent-base"), "{message}");
    assert!(
        !repo.root().join(".jit").exists(),
        "the set is resolved before a repository is created"
    );
}

#[test]
fn test_profile_apply_reports_a_dependency_that_cannot_be_resolved() {
    let repo = JitCli::uninitialised().unwrap();
    success_json(&repo, &["init", "--json"]);
    jit::test_utils::write_package_declaring(
        &composition_package(),
        &repo.root().join("packages/workflow"),
        "workflow",
        &["absent-base"],
    );

    let failure = failed_json_with_path(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            "packages/workflow",
            "--json",
        ],
        None,
    );

    assert_eq!(failure["error"]["code"], "PROFILE_ERROR", "{failure}");
    // Both packages are named: the one the adopter asked for, and the one it
    // declared that could not be found.
    let message = failure["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("a failure carries a message: {failure}"));
    assert!(message.contains("workflow"), "{message}");
    assert!(message.contains("absent-base"), "{message}");
    assert!(normalized_events(repo.root()).is_empty());
}

#[test]
fn test_profile_application_contributes_workflow_invariants_to_scaffolded_registry() {
    let repo = JitCli::uninitialised().unwrap();

    success_json(&repo, &["init", "--json"]);
    let dogfood_location = crate::repository_package_at(repo.root(), "jit-dogfood");
    let dogfood_selector = dogfood_location.to_string();
    let applied = success_json(
        &repo,
        &["profile", "apply", "--profile", &dogfood_selector, "--json"],
    );
    assert_eq!(requested_profile(&applied)["status"], "applied");
    assert_eq!(success_json(&repo, &["validate", "--json"])["valid"], true);

    let registry: toml::Value = fs::read_to_string(repo.root().join(".jit/invariants.toml"))
        .expect("read contributed invariant registry")
        .parse()
        .expect("parse contributed invariant registry");
    let ids = registry["invariants"]
        .as_array()
        .expect("invariant registry array")
        .iter()
        .map(|entry| entry["id"].as_str().expect("invariant id").to_string())
        .collect::<BTreeSet<_>>();

    // Set equality against the package's declarations is owned by the unit-level
    // round-trip; this layer adds only that the CLI path reaches the registry.
    assert!(!ids.is_empty());
    assert!(ids.contains("label-format"));
    assert!(ids.contains("convention-convergence"));
}

#[test]
#[cfg(unix)]
fn test_offline_public_cli_profile_reaches_implementation_ready_breakdown() {
    let repo = JitCli::uninitialised().unwrap();
    let checker_path = checker_path_without_jq();
    let path = Some(checker_path.value.as_path());
    let dogfood_location = crate::repository_package_at(repo.root(), "jit-dogfood");
    let dogfood_selector = dogfood_location.to_string();
    success_json_with_path(
        &repo,
        &["init", "--profile", &dogfood_selector, "--json"],
        path,
    );
    assert!(!repo.root().join(".git").exists());

    let invariants = success_json_with_path(
        &repo,
        &["project", "render", "--name", "invariants", "--json"],
        path,
    );
    assert_eq!(invariants["projections"][0]["target"], "AGENTS.md");
    let references = success_json_with_path(
        &repo,
        &["project", "render", "--name", "rules-and-gates", "--json"],
        path,
    );
    assert_eq!(
        references["projections"][0]["target"],
        ".jit/reference/rules-and-gates.md"
    );
    let agents = fs::read_to_string(repo.root().join("AGENTS.md")).unwrap();
    assert!(agents.contains("<!-- jit:invariants:begin -->"));
    assert!(agents.contains("<!-- jit:invariants:end -->"));
    let rules_and_gates =
        fs::read_to_string(repo.root().join(".jit/reference/rules-and-gates.md")).unwrap();
    assert!(rules_and_gates.contains("## Gates"));
    assert!(rules_and_gates.contains("plan-review"));

    let container = success_json_with_path(
        &repo,
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
    let applied = success_json_with_path(&repo, &["apply", "plan", container_id, "--json"], path);
    let planning_id = applied["created_node_ids_by_role"]["planning"]
        .as_str()
        .unwrap();
    let breakdown_id = applied["created_node_ids_by_role"]["breakdown"]
        .as_str()
        .unwrap();

    success_json_with_path(
        &repo,
        &["issue", "claim", planning_id, "agent:acceptance", "--json"],
        path,
    );
    let blocked_plan = failed_json_with_path(
        &repo,
        &["issue", "update", planning_id, "--state", "done", "--json"],
        path,
    );
    assert_eq!(blocked_plan["error"]["details"]["actual_state"], "gated");
    let plan_review = success_json_with_path(
        &repo,
        &["gate", "evaluate", planning_id, "plan-review", "--json"],
        path,
    );
    assert_eq!(plan_review["status"], "passed");
    assert!(plan_review["warnings"][0]
        .as_str()
        .unwrap()
        .contains("EXTERNAL REVIEW PLACEHOLDER"));
    success_json_with_path(
        &repo,
        &["issue", "update", planning_id, "--state", "done", "--json"],
        path,
    );

    let credit = format!("satisfies:{}/REQ-01", &container_id[..8]);
    let implementation = success_json_with_path(
        &repo,
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
            &credit,
            "--json",
        ],
        path,
    );
    let implementation_id = implementation["id"].as_str().unwrap();
    success_json_with_path(
        &repo,
        &["dep", "add", implementation_id, breakdown_id, "--json"],
        path,
    );
    success_json_with_path(
        &repo,
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
        &repo,
        &["issue", "claim", breakdown_id, "agent:acceptance", "--json"],
        path,
    );
    let blocked_breakdown = failed_json_with_path(
        &repo,
        &["issue", "update", breakdown_id, "--state", "done", "--json"],
        path,
    );
    assert_eq!(
        blocked_breakdown["error"]["details"]["actual_state"],
        "gated"
    );
    let coverage = success_json_with_path(
        &repo,
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
    assert!(coverage.get("warnings").is_none(), "{coverage}");
    let breakdown_review = success_json_with_path(
        &repo,
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
        &repo,
        &["issue", "update", breakdown_id, "--state", "done", "--json"],
        path,
    );

    let statuses = success_json_with_path(
        &repo,
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

    let validation = success_json_with_path(&repo, &["validate", "--json"], path);
    assert_eq!(validation["valid"], true);
    assert_eq!(validation["error_count"], 0);
    assert!(validation["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["rule"] == "review-placeholder"));
    assert!(!repo.root().join(".git").exists());
}

#[test]
fn test_public_profile_schema_states_the_shipped_lifecycle_surface() {
    let repo = JitCli::uninitialised().unwrap();
    let schema = success_json(&repo, &["--schema"]);
    let commands = schema["commands"]["profile"]["subcommands"]
        .as_object()
        .unwrap();
    assert!(commands["apply"]["description"]
        .as_str()
        .unwrap()
        .contains("multi-target redo record"));
    assert_eq!(
        commands.keys().cloned().collect::<BTreeSet<_>>(),
        expected_keys(&["add", "apply", "diff", "pack", "show"])
    );
    // Pack and add move a package between repositories, so each takes the
    // package location it works from and the archive it works through.
    for (command, flags) in [
        ("pack", ["dry-run", "output", "source"]),
        ("add", ["archive", "destination", "dry-run"]),
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
    // Apply takes an ordered selector stream, the value channels, and the
    // rehearsal that precedes publication.
    assert_eq!(
        commands["apply"]["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|argument| argument["name"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        expected_keys(&[]),
        "apply takes its selection through flags"
    );
    assert_eq!(
        commands["apply"]["flags"]
            .as_array()
            .unwrap()
            .iter()
            .map(|flag| flag["name"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>(),
        expected_keys(&["dry-run", "profile", "set", "values-file"]),
        "apply exposes one lifecycle input surface"
    );

    let show_schema = &commands["show"]["output"]["success_schema"];
    assert_eq!(
        show_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles", "warnings"])
    );
    assert_eq!(
        property_keys(show_schema, "ProfileShowEntry"),
        expected_keys(&[
            "byte_size",
            "file_count",
            "id",
            "manifest",
            "origin",
            "package_hash",
            "targets",
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
        show_schema["definitions"]["ProfilePackageModel"]["additionalProperties"], false,
        "ProfilePackageModel must reject undeclared lifecycle fields"
    );

    let apply_schema = &commands["apply"]["output"]["success_schema"];
    assert_eq!(
        apply_schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        expected_keys(&["count", "profiles", "warnings"])
    );
    assert_eq!(
        property_keys(apply_schema, "ProfileApplyResult"),
        expected_keys(&[
            "contributions",
            "id",
            "origin",
            "refusal",
            "status",
            "targets",
            "version",
        ])
    );
    assert_eq!(
        property_keys(apply_schema, "ProfileTargetChange"),
        expected_keys(&["action", "executable", "path", "reason"])
    );
    assert_eq!(
        property_keys(apply_schema, "ProfileContributionChange"),
        expected_keys(&["action", "identity", "reason"])
    );
}

#[test]
fn test_release_profile_dry_run_smoke_uses_count_wrapped_result_contract() {
    let workflow_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.github/workflows/release-artifacts.yml");
    let workflow = fs::read_to_string(&workflow_path)
        .unwrap_or_else(|error| panic!("read {workflow_path:?}: {error}"));

    assert!(
        workflow.contains("jit init --profile packages/jit-dogfood --json > init.json"),
        "release smoke must use the repeatable selector syntax for init"
    );
    assert!(
        workflow.contains(
            "jit profile apply --profile packages/jit-dogfood --dry-run --json > plan.json"
        ),
        "release smoke must use the repeatable selector syntax for dry-run"
    );
    assert!(
        workflow.contains("assert plan[\"profiles\"], plan"),
        "release smoke must validate a non-empty dry-run result"
    );
    assert!(
        workflow.contains(
            "assert all(profile[\"status\"] == \"unchanged\" for profile in plan[\"profiles\"]), plan"
        ),
        "release smoke must validate every aggregate profile observation"
    );
    for removed in ["list", "validate", "reconfigure", "upgrade"] {
        assert!(
            !workflow.contains(&format!("jit profile {removed}")),
            "release smoke must not invoke the removed `profile {removed}` subcommand"
        );
    }
    assert!(
        !workflow.contains("assert plan[\"status\"] == \"unchanged\", plan"),
        "release smoke must not read the removed top-level dry-run status"
    );
}

/// A fresh repository can obtain the shipped workflow from archives made by a
/// repository holding it, then initialize through the documented path selector.
#[test]
fn test_shipped_profile_archive_bootstraps_a_repository_without_jit_state() {
    let source = JitCli::uninitialised().unwrap();
    let dogfood = crate::repository_package_at(source.root(), "jit-dogfood");
    let holding =
        super::profile_cli_tests::jit(&source, &["init", "--profile", &dogfood, "--json"]);
    assert!(holding.status.success(), "{holding:?}");
    let exchange = TempDir::new().unwrap();
    let repo = JitCli::uninitialised().unwrap();
    assert!(!repo.root().join(".jit").exists());

    for id in ["jit-default", "jit-dogfood"] {
        let archive = exchange.path().join(format!("{id}.tar"));
        let archive = archive.to_str().unwrap();
        let package = crate::repository_package_at(source.root(), id);
        let packed = super::profile_cli_tests::jit(
            &source,
            &[
                "profile", "pack", "--source", &package, "--output", archive, "--json",
            ],
        );
        assert!(packed.status.success(), "packing {id} failed: {packed:?}");

        let initialized = super::profile_cli_tests::jit(&repo, &["init", "--json"]);
        assert!(
            initialized.status.success(),
            "plain init failed: {initialized:?}"
        );
        let added = super::profile_cli_tests::jit(
            &repo,
            &[
                "profile",
                "add",
                "--archive",
                archive,
                "--destination",
                &format!("packages/{id}"),
                "--json",
            ],
        );
        assert!(added.status.success(), "adding {id} failed: {added:?}");
    }

    let profiled = super::profile_cli_tests::jit(
        &repo,
        &["init", "--profile", "packages/jit-dogfood", "--json"],
    );
    assert!(
        profiled.status.success(),
        "profiled init failed: {profiled:?}"
    );
    let lifecycle = normalized_events(repo.root())
        .into_iter()
        .find(|event| event["type"] == "profile_lifecycle")
        .expect("the profiled init audits its application");
    assert_eq!(
        lifecycle["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["jit-default", "jit-dogfood"]
    );
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
fn profile_difference(repo: &JitCli, selectors: &[&str]) -> (ExitStatus, Value) {
    let args = ["profile", "diff"]
        .into_iter()
        .chain(
            selectors
                .iter()
                .flat_map(|selector| ["--profile", *selector]),
        )
        .chain(["--json"])
        .collect::<Vec<_>>();
    let output = jit_with_path(repo, &args, None);
    let parsed = parse_json(&output);
    let report = parsed
        .pointer("/error/details")
        .cloned()
        .unwrap_or_else(|| parsed.clone());
    (output.status, report)
}

/// The decisions the report carries for profile `id`, each as its action beside
/// the target it names.
fn decisions(report: &Value, id: &str) -> Vec<(String, String)> {
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
            )
        })
        .collect()
}

/// Seed `repo` with nothing applied, holding the asset-only fixture
/// package at `packages/planner` beside the target that package declares.
fn seed_unapplied_planner(repo: &JitCli) -> String {
    const LOCATION: &str = "packages/planner";

    let directory =
        jit::test_utils::copy_package_tree(&composition_package(), &repo.root().join(LOCATION));
    let target = jit::profile::ProfilePackage::from_directory(&directory)
        .expect("a valid package tree")
        .model()
        .assets
        .first()
        .expect("the fixture package declares one asset")
        .target
        .clone();
    success_json(repo, &["init", "--json"]);
    target
}

/// A package this repository has not applied is reported from the location it
/// was selected at, and reporting it writes nothing (REQ-01, REQ-02, REQ-04).
#[test]
fn test_profile_diff_reports_an_unapplied_package_without_writing() {
    let repo = JitCli::uninitialised().unwrap();
    let target = seed_unapplied_planner(&repo);
    let before = snapshot_tree(repo.root());
    let events = normalized_events(repo.root());

    let (status, report) = profile_difference(&repo, &["packages/planner"]);

    assert!(status.success(), "a publishable selection is a clean check");
    assert!(
        decisions(&report, "planner-asset-only").contains(&("create".to_string(), target.clone())),
        "the report states the target the package would publish: {report}"
    );
    assert_eq!(
        snapshot_tree(repo.root()),
        before,
        "the report writes nothing"
    );
    assert_eq!(normalized_events(repo.root()), events);
}

/// A target two selected packages define differently cannot be published, and
/// stating that keeps the report as the answer while carrying the
/// repository-check exit status (REQ-01, REQ-02).
#[test]
fn test_profile_diff_exits_on_an_unpublishable_target_it_still_reports() {
    let repo = JitCli::uninitialised().unwrap();
    let target = seed_unapplied_planner(&repo);
    let rival = repo.root().join("packages/rival");
    jit::test_utils::write_package_declaring(&composition_package(), &rival, "rival", &[]);
    let declared = jit::profile::ProfilePackage::from_directory(&rival)
        .expect("a valid package tree")
        .model()
        .assets
        .first()
        .expect("the fixture package declares one asset")
        .clone();
    let manifest = rival.join(jit::profile::MANIFEST_FILE_NAME);
    let authored = fs::read_to_string(&manifest).expect("read the rival manifest");
    fs::write(
        &manifest,
        authored.replace(
            &format!("target = \"{}\"", declared.target),
            &format!("target = \"{target}\""),
        ),
    )
    .expect("point the rival asset at the planner's target");
    fs::write(rival.join(&declared.source), b"the rival package's bytes\n")
        .expect("rewrite the asset");
    let before = snapshot_tree(repo.root());

    let (status, report) = profile_difference(&repo, &["packages/planner", "packages/rival"]);

    assert!(
        !status.success(),
        "a selection that cannot be published fails the check: {report}"
    );
    assert!(
        decisions(&report, "planner-asset-only")
            .contains(&("conflict".to_string(), target.clone())),
        "the report states the target it cannot publish: {report}"
    );
    assert_eq!(
        snapshot_tree(repo.root()),
        before,
        "the report writes nothing"
    );
}

/// A final-validation refusal fails the repository check and reaches the
/// report's reader, in both output forms.
#[test]
fn test_profile_diff_exits_on_a_final_validation_refusal_it_still_reports() {
    let repo = JitCli::uninitialised().unwrap();
    seed_unapplied_planner(&repo);
    let config = repo.root().join(".jit/config.toml");
    let authored = fs::read_to_string(&config).expect("read the configuration");
    fs::write(
        &config,
        format!("{authored}{}", jit::test_utils::GATE_PROJECTION_CONFIG),
    )
    .expect("configure a gate projection");
    success_json(&repo, &["project", "render", "--json"]);
    let manifest = repo
        .root()
        .join("packages/planner")
        .join(jit::profile::MANIFEST_FILE_NAME);
    let declared = fs::read_to_string(&manifest).expect("read the package manifest");
    fs::write(
        &manifest,
        format!("{declared}{}", jit::test_utils::NEW_GATE_CONTRIBUTION),
    )
    .expect("declare a gate the projection would have to render");

    let (status, report) = profile_difference(&repo, &["packages/planner"]);
    let human = jit_with_path(
        &repo,
        &["profile", "diff", "--profile", "packages/planner"],
        None,
    );

    assert!(!status.success(), "{report}");
    let refusal = report["profiles"][0]["refusal"]
        .as_str()
        .unwrap_or_else(|| panic!("the report states the refusal: {report}"));
    assert!(refusal.contains("docs/gates.md"), "{refusal}");
    assert!(!human.status.success());
    assert!(
        String::from_utf8_lossy(&human.stdout).contains(refusal.lines().last().unwrap()),
        "{human:?}"
    );
}

/// The declaration decisions the report carries for profile `id`, each as its
/// action beside the semantic identity it names.
fn declarations(report: &Value, id: &str) -> Vec<(String, String)> {
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
fn package_contributing(repo: &JitCli, location: &str, id: &str, description: &str) {
    let directory = repo.root().join(location);
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
/// targets: one the registry lacks is created, and one two selected packages
/// define differently is stated as the conflict it is (REQ-01, REQ-02).
#[test]
fn test_profile_diff_states_the_declarations_a_selection_would_publish() {
    let repo = JitCli::uninitialised().unwrap();
    success_json(&repo, &["init", "--json"]);
    package_contributing(&repo, "packages/base", "base", "The base meaning.");
    package_contributing(
        &repo,
        "packages/workflow",
        "workflow",
        "The workflow meaning.",
    );

    let (status, report) = profile_difference(&repo, &["packages/base"]);

    assert!(status.success(), "a publishable selection is a clean check");
    assert!(
        declarations(&report, "base").contains(&("create".to_string(), contributed_identity())),
        "the report states the declaration the package would publish: {report}"
    );

    let before = snapshot_tree(repo.root());

    let (status, report) = profile_difference(&repo, &["packages/base", "packages/workflow"]);

    assert!(
        !status.success(),
        "a selection that cannot be published fails the check: {report}"
    );
    assert!(
        declarations(&report, "workflow")
            .contains(&("conflict".to_string(), contributed_identity())),
        "the report states the declaration it cannot publish: {report}"
    );
    assert_eq!(
        snapshot_tree(repo.root()),
        before,
        "the report writes nothing"
    );
}
