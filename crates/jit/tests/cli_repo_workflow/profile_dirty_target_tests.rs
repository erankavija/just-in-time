//! The refusal to publish a profile over uncommitted target changes.

use super::profile_acceptance_tests::snapshot_tree_excluding;
use super::profile_cli_tests::{
    jit, json, package_with_id, package_with_shared_namespace, requested_profile,
};
use jit::test_cli::JitCli;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;

const PACKAGE: &str = "packages/first";
const TARGET: &str = "docs/first.txt";
const NAMESPACE_SCHEMA: &str = ".jit/schemas/default-namespace-registry.json";
const LOCAL_EDIT: &str = "a local edit\n";

fn apply_args(locations: &[&'static str]) -> Vec<&'static str> {
    let selectors = locations
        .iter()
        .flat_map(|location| ["--profile", *location]);
    ["profile", "apply"]
        .into_iter()
        .chain(selectors)
        .chain(["--json"])
        .collect()
}

fn apply(cli: &JitCli) -> Output {
    jit(cli, &apply_args(&[PACKAGE]))
}

/// A repository with a committed package, optionally without git.
fn repository(with_git: bool) -> JitCli {
    let cli = JitCli::uninitialised().unwrap();
    if with_git {
        cli.init_git().unwrap();
    }
    cli.run(["init"]).unwrap();
    package_with_id(cli.root(), PACKAGE, "first");
    if with_git {
        cli.commit("track the package").unwrap();
    }
    cli
}

/// A git repository whose committed tree holds the package's applied target.
fn applied_repository() -> JitCli {
    let cli = repository(true);
    assert!(apply(&cli).status.success());
    cli.commit("apply the package").unwrap();
    cli
}

fn write(cli: &JitCli, path: &str, contents: &str) {
    let path = cli.root().join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn read(root: &Path, path: &str) -> String {
    fs::read_to_string(root.join(path)).unwrap()
}

/// The error of a refused application, proven to have published nothing.
fn refused(cli: &JitCli, events_root: &Path, args: &[&str]) -> Value {
    let events_before = read(events_root, "events.jsonl");
    let output = jit(cli, args);
    assert!(!output.status.success(), "{output:?}");
    assert_eq!(read(events_root, "events.jsonl"), events_before);
    json(&output)["error"].clone()
}

fn refused_run(output: &Output) -> Value {
    assert!(!output.status.success(), "{output:?}");
    json(output)["error"].clone()
}

fn refused_paths(error: &Value) -> Vec<&str> {
    assert_eq!(error["code"], "PROFILE_UNCOMMITTED_TARGETS");
    error["details"]["paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap())
        .collect()
}

#[test]
fn test_profile_apply_refuses_a_modified_target_keeping_the_edit() {
    let cli = applied_repository();
    write(&cli, TARGET, LOCAL_EDIT);

    let error = refused(&cli, &cli.root().join(".jit"), &apply_args(&[PACKAGE]));

    assert_eq!(refused_paths(&error), [TARGET]);
    assert_eq!(read(cli.root(), TARGET), LOCAL_EDIT);
}

#[test]
fn test_profile_apply_refusal_leaves_the_repository_byte_identical() {
    let cli = applied_repository();
    write(&cli, TARGET, LOCAL_EDIT);
    write(&cli, "UNTRACKED.md", LOCAL_EDIT);
    // Git owns its directory; `git status` may refresh its index.
    let tree = || snapshot_tree_excluding(cli.root(), |path| path == ".git");
    let before = tree();

    let output = apply(&cli);

    assert!(!output.status.success(), "{output:?}");
    assert_eq!(tree(), before);
}

#[test]
fn test_profile_apply_refuses_a_staged_target() {
    let cli = applied_repository();
    write(&cli, TARGET, LOCAL_EDIT);
    cli.git(["add", TARGET]).unwrap();

    let error = refused(&cli, &cli.root().join(".jit"), &apply_args(&[PACKAGE]));

    assert_eq!(refused_paths(&error), [TARGET]);
}

#[test]
fn test_profile_apply_refuses_an_untracked_target() {
    let cli = repository(true);
    write(&cli, TARGET, LOCAL_EDIT);

    let error = refused(&cli, &cli.root().join(".jit"), &apply_args(&[PACKAGE]));

    assert_eq!(refused_paths(&error), [TARGET]);
}

#[test]
fn test_profile_apply_refusal_names_each_path_and_how_to_proceed() {
    const SECOND_PACKAGE: &str = "packages/second";
    const SECOND_TARGET: &str = "docs/second.txt";
    let cli = repository(true);
    package_with_id(cli.root(), SECOND_PACKAGE, "second");
    cli.commit("track the second package").unwrap();
    write(&cli, TARGET, LOCAL_EDIT);
    write(&cli, SECOND_TARGET, LOCAL_EDIT);

    let error = refused(
        &cli,
        &cli.root().join(".jit"),
        &apply_args(&[PACKAGE, SECOND_PACKAGE]),
    );

    assert_eq!(refused_paths(&error), [TARGET, SECOND_TARGET]);
    let message = error["message"].as_str().unwrap();
    let resolution = error["details"]["resolution"].as_str().unwrap();
    assert!(message.contains(TARGET) && message.contains(SECOND_TARGET));
    assert!(message.contains(resolution));
    assert!(resolution.contains("commit") && resolution.contains("discard"));
}

#[test]
fn test_profile_apply_refuses_a_dirty_derived_schema_it_rewrites() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    cli.run(["init"]).unwrap();
    let package = package_with_shared_namespace(cli.root(), PACKAGE, "first", "shared");
    cli.commit("track the package").unwrap();
    write(&cli, NAMESPACE_SCHEMA, "{}\n");

    let args = ["profile", "apply", "--profile", package, "--json"];
    let error = refused(&cli, &cli.root().join(".jit"), &args);

    assert_eq!(refused_paths(&error), [NAMESPACE_SCHEMA]);
}

#[test]
fn test_profile_apply_proceeds_over_dirty_unrelated_paths_and_event_log() {
    let cli = repository(true);
    write(&cli, "UNRELATED.md", LOCAL_EDIT);
    cli.create_issue(["--title", "dirties the event log"])
        .unwrap();

    let output = apply(&cli);

    assert!(output.status.success(), "{output:?}");
    assert!(cli.root().join(TARGET).is_file());
    assert_eq!(read(cli.root(), "UNRELATED.md"), LOCAL_EDIT);
}

#[test]
fn test_profile_apply_over_its_own_uncommitted_rendering_writes_nothing_and_proceeds() {
    let cli = repository(true);
    assert!(apply(&cli).status.success());

    let output = apply(&cli);

    assert!(output.status.success(), "{output:?}");
    assert_eq!(requested_profile(&json(&output))["status"], "unchanged");
}

#[test]
fn test_profile_apply_without_git_overwrites_an_edited_target() {
    let cli = repository(false);
    assert!(apply(&cli).status.success());
    write(&cli, TARGET, LOCAL_EDIT);

    let output = apply(&cli);

    assert!(output.status.success(), "{output:?}");
    assert_ne!(read(cli.root(), TARGET), LOCAL_EDIT);
}

#[test]
fn test_init_profile_with_absent_data_root_in_a_dirty_checkout_is_not_refused() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    package_with_id(cli.root(), PACKAGE, "first");
    write(&cli, "UNRELATED.md", LOCAL_EDIT);

    let output = jit(&cli, &["init", "--profile", PACKAGE, "--json"]);

    assert!(output.status.success(), "{output:?}");
    assert!(cli.root().join(TARGET).is_file());
}

#[test]
fn test_profile_apply_reports_a_relocated_data_root_target_at_its_checkout_path() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    let data_root = cli.root().join("state");
    let env = [("JIT_DATA_DIR", &data_root)];
    let run = |args: &[&str]| cli.command_with_env(args, env).output().unwrap();
    assert!(run(&["init"]).status.success());
    let package = package_with_shared_namespace(cli.root(), PACKAGE, "first", "shared");
    cli.commit("track the relocated store").unwrap();
    let config = data_root.join("config.toml");
    fs::write(
        &config,
        format!("{}\n# a local edit\n", fs::read_to_string(&config).unwrap()),
    )
    .unwrap();

    let error = refused_run(&run(&["profile", "apply", "--profile", package, "--json"]));

    assert_eq!(refused_paths(&error), ["state/config.toml"]);
}

#[test]
fn test_profile_apply_refuses_a_worktree_target_when_the_data_root_is_outside_every_checkout() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    let store = tempfile::TempDir::new().unwrap();
    let data_root = store.path().join("store");
    let env = [("JIT_DATA_DIR", &data_root)];
    let run = |args: &[&str]| cli.command_with_env(args, env).output().unwrap();
    assert!(run(&["init"]).status.success());
    package_with_id(cli.root(), PACKAGE, "first");
    cli.commit("track the package").unwrap();
    write(&cli, TARGET, LOCAL_EDIT);

    let error = refused_run(&run(&apply_args(&[PACKAGE])));

    assert_eq!(refused_paths(&error), [TARGET]);
    assert_eq!(read(cli.root(), TARGET), LOCAL_EDIT);
}

#[cfg(unix)]
#[test]
fn test_profile_apply_starts_no_git_probe_beyond_what_profile_diff_starts() {
    use jit::test_harness::git::GitStandIn;

    let cli = repository(true);
    let probes_of = |args: &[&str]| {
        let stand_in = GitStandIn::new();
        let env = [("PATH", stand_in.search_path())];
        let output = cli.command_with_env(args, env).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        stand_in
            .invocations()
            .iter()
            .filter(|invocation| invocation.args().iter().any(|arg| arg == "rev-parse"))
            .count()
    };

    let diff_probes = probes_of(&["profile", "diff", "--profile", PACKAGE, "--json"]);
    let apply_probes = probes_of(&apply_args(&[PACKAGE]));

    assert_eq!(apply_probes, diff_probes);
}

#[cfg(unix)]
#[test]
fn test_profile_commands_probe_no_second_root_when_the_data_root_is_not_relocated() {
    use jit::test_harness::git::GitStandIn;

    let cli = repository(false);
    let stand_in = GitStandIn::new();
    let env = [("PATH", stand_in.search_path())];

    let output = cli
        .command_with_env(apply_args(&[PACKAGE]), env)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert!(stand_in
        .invocations()
        .iter()
        .all(|invocation| invocation.directory() != Some(cli.root())));
}

#[test]
fn test_profile_apply_dry_run_refuses_what_the_apply_would_refuse() {
    let cli = applied_repository();
    write(&cli, TARGET, LOCAL_EDIT);

    let args = [
        "profile",
        "apply",
        "--profile",
        PACKAGE,
        "--dry-run",
        "--json",
    ];
    let error = refused(&cli, &cli.root().join(".jit"), &args);

    assert_eq!(refused_paths(&error), [TARGET]);
}

#[test]
fn test_profile_diff_reports_rather_than_refuses_a_dirty_target() {
    let cli = applied_repository();
    write(&cli, TARGET, LOCAL_EDIT);

    let output = jit(&cli, &["profile", "diff", "--profile", PACKAGE, "--json"]);

    assert!(output.status.success(), "{output:?}");
}
