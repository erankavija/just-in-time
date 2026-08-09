use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn jit(dir: &std::path::Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("failed to run jit")
}

/// The id the checked-in fixture package declares.
const FIXTURE_PROFILE: &str = "planner-asset-only";

/// The result for the package an application named, which an application
/// applies after everything that package depends on.
pub(super) fn requested_profile(applied: &Value) -> &Value {
    applied["profiles"]
        .as_array()
        .expect("an application reports one result per applied package")
        .last()
        .expect("an application applies at least the package it named")
}

/// Whether any package of a composed application published something.
///
/// Which package publishes a coupled materialization is a property of the
/// closure rather than of the request: a target derived from the declarations
/// one package writes is repaired under that package's application, not under
/// the one an adopter happened to name.
fn published_anything(applied: &Value) -> bool {
    applied["profiles"]
        .as_array()
        .expect("an application reports one result per applied package")
        .iter()
        .any(|profile| profile["status"] == "applied")
}

/// The ids an application reports, in the order it applied them.
pub(super) fn applied_ids(applied: &Value) -> Vec<&str> {
    applied["profiles"]
        .as_array()
        .expect("an application reports one result per applied package")
        .iter()
        .map(|profile| {
            profile["id"]
                .as_str()
                .expect("each result names its package")
        })
        .collect()
}

/// Copy the fixture package tree into `repo` at the repository-relative
/// `location`, and return that location.
///
/// A package is applied from inside the repository it is applied to, because
/// the applied-profile record names its worktree-relative location.
fn package_at<'a>(repo: &Path, location: &'a str) -> &'a str {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile-packages")
        .join(FIXTURE_PROFILE);
    jit::test_utils::copy_package_tree(&source, &repo.join(location));
    location
}

/// Copy the fixture package and give it a distinct identity and target so a
/// selector-order test can observe the complete occurrence stream.
fn package_with_id<'a>(repo: &Path, location: &'a str, id: &str) -> &'a str {
    package_at(repo, location);
    let manifest = repo.join(location).join("manifest.toml");
    let declared = fs::read_to_string(&manifest)
        .unwrap()
        .replace(
            &format!("id = \"{FIXTURE_PROFILE}\""),
            &format!("id = \"{id}\""),
        )
        .replace(
            "target = \"docs/profile.txt\"",
            &format!("target = \"docs/{id}.txt\""),
        );
    fs::write(manifest, declared).unwrap();
    location
}

/// Write a package with one semantic contribution and no content targets.
fn package_with_shared_namespace<'a>(
    repo: &Path,
    location: &'a str,
    id: &str,
    description: &str,
) -> &'a str {
    let root = repo.join(location);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("manifest.toml"),
        format!(
            "[profile]\nmanifest-version = 1\nid = \"{id}\"\nversion = \"1.0.0\"\njit = \">=0.2.0, <2.0.0\"\n\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"namespaces\"\nidentity = \"shared\"\nvalue = {{ description = \"{description}\", unique = false }}\n"
        ),
    )
    .unwrap();
    location
}

fn path_selector(location: &str) -> String {
    format!("path:{location}")
}

fn id_selector(id: &str) -> String {
    format!("id:{id}")
}

/// The record this repository stores for the fixture package.
fn stored_record(repo: &Path) -> Value {
    let path = repo.join(format!(".jit/profiles/{FIXTURE_PROFILE}.json"));
    serde_json::from_slice(&fs::read(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    }))
    .expect("a stored record is JSON")
}

/// Republish the package tree at `location` under `version`.
///
/// Two copies of one fixture are byte-identical and therefore indistinguishable
/// in everything a resolution reports, so a test that asks which copy was read
/// edits one of them first. The published asset is left alone: identical target
/// bytes are what keeps the second application an ordinary re-application
/// rather than a target conflict.
fn set_package_version(repo: &Path, location: &str, version: &str) -> PathBuf {
    let manifest = repo.join(location).join("manifest.toml");
    let declared = fs::read_to_string(&manifest)
        .unwrap()
        .replace("version = \"1.0.0\"", &format!("version = \"{version}\""));
    fs::write(&manifest, declared).unwrap();
    manifest
}

/// Write a small v2 package whose templated asset makes the resolved value
/// observable without putting that value in package identity or provenance.
struct VariablePackageSpec<'a> {
    id: &'a str,
    variable: &'a str,
    default: Option<&'a str>,
    environment: Option<&'a str>,
    target: &'a str,
    body: &'a str,
    dependency: Option<&'a str>,
}

fn write_variable_package(repo: &Path, location: &str, spec: VariablePackageSpec<'_>) {
    let root = repo.join(location);
    fs::create_dir_all(root.join("assets")).unwrap();
    let dependency = spec.dependency.map_or(String::new(), |dependency| {
        format!("\n[[dependency]]\nid = \"{dependency}\"\nversion = \"*\"\n")
    });
    let default = spec
        .default
        .map_or(String::new(), |value| format!("default = \"{value}\"\n"));
    let environment = spec
        .environment
        .map_or(String::new(), |name| format!("env = \"{name}\"\n"));
    let manifest = format!(
        "[profile]\nmanifest-version = 2\nid = \"{}\"\nversion = \"1.0.0\"\ncompatible-jit = \"*\"\n{dependency}\n[[variable]]\nname = \"{}\"\n{default}{environment}\n[[asset]]\nsource = \"assets/content.txt\"\ntarget = \"{}\"\ntemplate = true\n",
        spec.id, spec.variable, spec.target
    );
    fs::write(root.join("manifest.toml"), manifest).unwrap();
    fs::write(root.join("assets/content.txt"), spec.body).unwrap();
}

/// Rewrite the package tree at `location` to declare a dependency on `id`.
///
/// No package this repository ships declares a dependency, so a composition
/// scenario is authored over the fixture rather than borrowed from one.
fn declare_dependency(repo: &Path, location: &str, id: &str) {
    let manifest = repo.join(location).join("manifest.toml");
    let declared = fs::read_to_string(&manifest).unwrap().replace(
        "[profile]",
        &format!("dependencies = [\"{id}\"]\n\n[profile]"),
    );
    fs::write(&manifest, declared).unwrap();
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn test_profile_list_and_show_work_without_repository() {
    let repo = TempDir::new().unwrap();

    // A directory that is not a repository has recorded no profile, so it names
    // none.
    let list = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(list.status.success(), "{list:?}");
    let list = json(&list);
    assert_eq!(list["count"], 0);
    assert_eq!(list["profiles"].as_array().unwrap(), &Vec::<Value>::new());

    // Inspection still resolves a package a location names, which is the only
    // route left when nothing has been recorded.
    let location = crate::repository_package_at(repo.path(), "jit-dogfood");
    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &format!("path:{location}"),
            "--json",
        ],
    );
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["count"], 1);
    assert_eq!(show["profiles"][0]["manifest"]["id"], "jit-dogfood");
    assert_eq!(
        show["profiles"][0]["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );
    assert!(show["profiles"][0]["package_hash"].as_str().unwrap().len() >= 64);
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_profile_apply_preserves_interleaved_selector_order() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    package_with_id(repo.path(), "packages/first", "first");
    package_with_id(repo.path(), "packages/second", "second");
    let seeded = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/first",
            "--json",
        ],
    );
    assert!(seeded.status.success(), "{seeded:?}");
    let output = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            "path:packages/first",
            "--profile",
            "id:first",
            "--profile",
            "path:packages/second",
            "--json",
        ],
    );

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        applied_ids(&json(&output)),
        vec!["first", "first", "second"],
        "the selected path/id/path occurrence order must reach runtime results"
    );
}

#[test]
fn test_profile_apply_rejects_a_path_that_shadows_a_selected_profile_id() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let recorded = package_at(repo.path(), "packages/recorded");
    let apply = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &format!("path:{recorded}"),
            "--json",
        ],
    );
    assert!(apply.status.success(), "{apply:?}");

    let shadow = package_at(repo.path(), "packages/shadow");
    let output = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &format!("path:{shadow}"),
            "--profile",
            &format!("id:{FIXTURE_PROFILE}"),
            "--json",
        ],
    );
    assert!(!output.status.success(), "{output:?}");
    let error_json = json(&output);
    let message = error_json["error"]["message"].as_str().unwrap_or_default();
    assert!(message.contains(FIXTURE_PROFILE), "{message}");
}

#[test]
fn test_profile_apply_preflights_conflicting_repeatable_selectors_before_publication() {
    for (first, second) in [
        ("packages/base", "packages/workflow"),
        ("packages/workflow", "packages/base"),
    ] {
        let repo = TempDir::new().unwrap();
        assert!(jit(repo.path(), &["init"]).status.success());
        package_with_shared_namespace(repo.path(), "packages/base", "base", "Base definition");
        package_with_shared_namespace(
            repo.path(),
            "packages/workflow",
            "workflow",
            "Workflow definition",
        );

        let config_path = repo.path().join(".jit/config.toml");
        let events_path = repo.path().join(".jit/events.jsonl");
        let config_before = fs::read(&config_path).unwrap();
        let events_before = fs::read(&events_path).unwrap();
        let output = jit(
            repo.path(),
            &[
                "profile",
                "apply",
                "--profile",
                &path_selector(first),
                "--profile",
                &path_selector(second),
                "--json",
            ],
        );

        assert_eq!(output.status.code(), Some(4), "{output:?}");
        assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
        assert_eq!(fs::read(&config_path).unwrap(), config_before, "{output:?}");
        assert_eq!(fs::read(&events_path).unwrap(), events_before);
        assert!(!repo.path().join(".jit/profiles/base.json").exists());
        assert!(!repo.path().join(".jit/profiles/workflow.json").exists());
    }
}

#[test]
fn test_profile_apply_records_complete_shared_ownership_on_first_apply() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let base =
        package_with_shared_namespace(repo.path(), "packages/base", "base", "Shared definition");
    let workflow = package_with_shared_namespace(
        repo.path(),
        "packages/workflow",
        "workflow",
        "Shared definition",
    );

    let selectors = [path_selector(base), path_selector(workflow)];
    let args = [
        "profile",
        "apply",
        "--profile",
        selectors[0].as_str(),
        "--profile",
        selectors[1].as_str(),
        "--json",
    ];
    let first = jit(repo.path(), &args);
    assert!(first.status.success(), "{first:?}");

    for id in ["base", "workflow"] {
        let record: Value = serde_json::from_slice(
            &fs::read(repo.path().join(format!(".jit/profiles/{id}.json"))).unwrap(),
        )
        .unwrap();
        let owners = record["contributions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|contribution| contribution["identity"]["target"]["name"] == "shared")
            .unwrap()["owners"]
            .as_array()
            .unwrap()
            .iter()
            .map(|owner| owner.as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(owners, vec!["base", "workflow"]);
    }
}

#[test]
fn test_profile_commands_reject_the_superseded_positional_and_from_surface() {
    let repo = TempDir::new().unwrap();
    let show = jit(repo.path(), &["profile", "show", FIXTURE_PROFILE, "--json"]);
    assert!(!show.status.success());
    assert!(String::from_utf8_lossy(&show.stderr).contains("unexpected argument"));

    let apply = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            "id:missing",
            "--from",
            "packages",
        ],
    );
    assert!(!apply.status.success());
    assert!(String::from_utf8_lossy(&apply.stderr).contains("unexpected argument"));
}

#[test]
fn test_profile_commands_require_a_tagged_selector() {
    let repo = TempDir::new().unwrap();
    let invalid = jit(
        repo.path(),
        &["profile", "show", "--profile", "profile:planner"],
    );

    assert_eq!(invalid.status.code(), Some(2), "{invalid:?}");
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("expected id:ID or path:DIR"),
        "{}",
        String::from_utf8_lossy(&invalid.stderr)
    );
}

#[test]
fn test_profile_show_reads_the_package_a_supplied_location_holds() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &path_selector(location),
            "--json",
        ],
    );

    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["count"], 1);
    // Nothing but the supplied location holds a package declaring this
    // profile, so reporting it at all is that location having been read.
    assert_eq!(show["profiles"][0]["manifest"]["id"], FIXTURE_PROFILE);
    assert_eq!(
        show["profiles"][0]["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );
}

#[test]
fn test_profile_show_json_preserves_repeated_selectors_in_order() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");
    let selector = path_selector(location);

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &selector,
            "--profile",
            &selector,
            "--json",
        ],
    );

    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["count"], 2);
    assert_eq!(
        show["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["manifest"]["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec![FIXTURE_PROFILE, FIXTURE_PROFILE]
    );
}

#[test]
fn test_profile_show_human_renders_each_selected_occurrence() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");
    let selector = path_selector(location);

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &selector,
            "--profile",
            &selector,
        ],
    );

    assert!(show.status.success(), "{show:?}");
    let stdout = String::from_utf8_lossy(&show.stdout);
    assert_eq!(
        stdout.matches("Profile: planner-asset-only").count(),
        2,
        "human rendering must include every selected occurrence"
    );
}

#[test]
fn test_profile_show_refuses_a_supplied_location_outside_the_repository() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    // A package beside the repository rather than inside it. The location a
    // record would have to name is worktree-relative, so there is none.
    let elsewhere = TempDir::new().unwrap();
    package_at(elsewhere.path(), "planner");

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &path_selector(elsewhere.path().join("planner").to_str().unwrap()),
            "--json",
        ],
    );

    assert!(!show.status.success(), "{show:?}");
    let show = json(&show);
    assert!(
        show["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("is not inside the repository worktree"),
        "{show}"
    );
}

#[test]
fn test_profile_show_refuses_a_supplied_location_under_the_data_root() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), ".jit/vendored");

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &path_selector(location),
            "--json",
        ],
    );

    assert!(!show.status.success(), "{show:?}");
    let show = json(&show);
    assert!(
        show["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("is not inside the repository worktree"),
        "{show}"
    );
}

#[test]
fn test_profile_apply_dry_run_reads_the_package_a_supplied_location_holds() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");

    let preview = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(location),
            "--dry-run",
            "--json",
        ],
    );

    assert!(preview.status.success(), "{preview:?}");
    let preview = json(&preview);
    assert_eq!(preview["profiles"][0]["id"], FIXTURE_PROFILE);
    assert_eq!(preview["count"], 1);
    assert_eq!(preview["profiles"][0]["status"], "would_apply");
    assert!(
        preview["profiles"][0]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["path"] == "docs/profile.txt"),
        "{preview}"
    );
    // A preview writes nothing, the record included.
    assert!(!repo.path().join(".jit/profiles").exists());
    assert!(!repo.path().join("docs/profile.txt").exists());
}

#[test]
fn test_profile_variable_inputs_cover_init_apply_dry_run_and_dependency_precedence() {
    let init_repo = TempDir::new().unwrap();
    write_variable_package(
        init_repo.path(),
        "packages/init-vars",
        VariablePackageSpec {
            id: "init-vars",
            variable: "NAME",
            default: Some("default"),
            environment: None,
            target: "docs/init-vars.txt",
            body: "NAME={{jit:var:NAME}}\n",
            dependency: None,
        },
    );
    let init_values = init_repo.path().join("values.toml");
    fs::write(&init_values, "[variables]\nNAME = \"from-file\"\n").unwrap();
    let init_values = init_values.to_str().unwrap();
    let init = jit(
        init_repo.path(),
        &[
            "init",
            "--profile",
            "path:packages/init-vars",
            "--values-file",
            init_values,
            "--set",
            "NAME=first",
            "--set",
            "NAME=last",
            "--json",
        ],
    );
    assert!(init.status.success(), "{init:?}");
    assert_eq!(
        fs::read_to_string(init_repo.path().join("docs/init-vars.txt")).unwrap(),
        "NAME=last\n"
    );

    let apply_repo = TempDir::new().unwrap();
    write_variable_package(
        apply_repo.path(),
        "packages/dependency-vars",
        VariablePackageSpec {
            id: "dependency-vars",
            variable: "TOKEN",
            default: Some("dependency-default"),
            environment: None,
            target: "docs/dependency-vars.txt",
            body: "TOKEN={{jit:var:TOKEN}}\n",
            dependency: None,
        },
    );
    write_variable_package(
        apply_repo.path(),
        "packages/root-vars",
        VariablePackageSpec {
            id: "root-vars",
            variable: "NAME",
            default: Some("root-default"),
            environment: None,
            target: "docs/root-vars.txt",
            body: "NAME={{jit:var:NAME}}\n",
            dependency: Some("dependency-vars"),
        },
    );
    assert!(jit(apply_repo.path(), &["init"]).status.success());
    let values = apply_repo.path().join("values.toml");
    fs::write(&values, "[variables]\nNAME = \"from-file\"\n").unwrap();
    let values = values.to_str().unwrap();
    let selector = "path:packages/root-vars";
    let preview = jit(
        apply_repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            selector,
            "--values-file",
            values,
            "--set",
            "NAME=first",
            "--set",
            "NAME=last",
            "--set",
            "TOKEN=dependency-value",
            "--dry-run",
            "--json",
        ],
    );
    assert!(preview.status.success(), "{preview:?}");
    let preview = json(&preview);
    assert_eq!(preview["profiles"][0]["status"], "would_apply");
    assert!(!apply_repo.path().join("docs/root-vars.txt").exists());
    assert!(!apply_repo.path().join("docs/dependency-vars.txt").exists());

    let alternate = jit(
        apply_repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            selector,
            "--set",
            "NAME=alternate",
            "--set",
            "TOKEN=dependency-value",
            "--dry-run",
            "--json",
        ],
    );
    assert!(alternate.status.success(), "{alternate:?}");
    assert_ne!(
        preview["profiles"][0]["plan_hash"],
        json(&alternate)["profiles"][0]["plan_hash"],
        "resolved plan identity must cover resolved values"
    );

    let applied = jit(
        apply_repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            selector,
            "--values-file",
            values,
            "--set",
            "NAME=first",
            "--set",
            "NAME=last",
            "--set",
            "TOKEN=dependency-value",
            "--json",
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(
        fs::read_to_string(apply_repo.path().join("docs/root-vars.txt")).unwrap(),
        "NAME=last\n"
    );
    assert_eq!(
        fs::read_to_string(apply_repo.path().join("docs/dependency-vars.txt")).unwrap(),
        "TOKEN=dependency-value\n"
    );
}

#[test]
fn test_profile_variable_record_drives_validation_repair_and_provenance_checks() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    write_variable_package(
        repo.path(),
        "packages/recorded-vars",
        VariablePackageSpec {
            id: "recorded-vars",
            variable: "NAME",
            default: None,
            environment: Some("JIT_PROFILE_RECORDED_NAME"),
            target: "docs/recorded-vars.txt",
            body: "NAME={{jit:var:NAME}}\n",
            dependency: None,
        },
    );
    let resolved_value = "persisted-public-value";
    let applied = Command::new(env!("CARGO_BIN_EXE_jit"))
        .args([
            "profile",
            "apply",
            "--profile",
            "path:packages/recorded-vars",
            "--set",
            &format!("NAME={resolved_value}"),
            "--json",
        ])
        .env("JIT_PROFILE_RECORDED_NAME", "environment-at-apply")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(
        fs::read_to_string(repo.path().join("docs/recorded-vars.txt")).unwrap(),
        format!("NAME={resolved_value}\n")
    );

    let record_path = repo.path().join(".jit/profiles/recorded-vars.json");
    let record_bytes = fs::read(&record_path).unwrap();
    let record: Value = serde_json::from_slice(&record_bytes).unwrap();
    assert_eq!(record["variables"]["NAME"]["value"], resolved_value);
    assert_eq!(record["variables"]["NAME"]["source"], "set");
    let events = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();
    assert!(!events
        .windows(resolved_value.len())
        .any(|window| window == resolved_value.as_bytes()));

    let validate = Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(["validate", "--json"])
        .env("JIT_PROFILE_RECORDED_NAME", "different-current-environment")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);

    fs::write(repo.path().join("docs/recorded-vars.txt"), "DRIFTED\n").unwrap();
    let repaired = Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(["validate", "--fix", "--json"])
        .env_remove("JIT_PROFILE_RECORDED_NAME")
        .current_dir(repo.path())
        .output()
        .unwrap();
    assert!(repaired.status.success(), "{repaired:?}");
    assert_eq!(
        fs::read_to_string(repo.path().join("docs/recorded-vars.txt")).unwrap(),
        format!("NAME={resolved_value}\n")
    );

    let assert_not_applied = |mutated: Value| {
        fs::write(&record_path, serde_json::to_vec_pretty(&mutated).unwrap()).unwrap();
        let listed = jit(repo.path(), &["profile", "list", "--json"]);
        assert!(listed.status.success(), "{listed:?}");
        assert_eq!(json(&listed)["profiles"][0]["applied"], false);
    };
    let mut value_tamper = record.clone();
    value_tamper["variables"]["NAME"]["value"] = Value::String("tampered".to_string());
    assert_not_applied(value_tamper);
    let mut source_tamper = record.clone();
    source_tamper["variables"]["NAME"]["source"] = Value::String("values_file".to_string());
    assert_not_applied(source_tamper);
    let mut hash_tamper = record.clone();
    let target_hash = hash_tamper["target_hashes"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    *target_hash = Value::String("0".repeat(64));
    assert_not_applied(hash_tamper);
    fs::write(record_path, record_bytes).unwrap();
}

#[test]
fn test_profile_apply_dry_run_preserves_order_and_duplicates_without_mutation() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let first = package_with_id(repo.path(), "packages/first", "first");
    let second = package_with_id(repo.path(), "packages/second", "second");

    let seeded = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(first),
            "--json",
        ],
    );
    assert!(seeded.status.success(), "{seeded:?}");
    let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let second_selector = path_selector(second);
    let first_selector = path_selector(first);
    let output = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &second_selector,
            "--profile",
            "id:first",
            "--profile",
            &first_selector,
            "--profile",
            &second_selector,
            "--dry-run",
            "--json",
        ],
    );

    assert!(output.status.success(), "{output:?}");
    let preview = json(&output);
    assert_eq!(preview["count"], 4);
    assert_eq!(
        preview["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["second", "first", "first", "second"]
    );
    assert_eq!(
        preview["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["would_apply", "unchanged", "unchanged", "would_apply"]
    );
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_before,
        "dry-run must not append events"
    );
    assert!(!repo.path().join(".jit/profiles/second.json").exists());
    assert!(!repo.path().join("docs/second.txt").exists());

    let human = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &second_selector,
            "--profile",
            "id:first",
            "--profile",
            &first_selector,
            "--profile",
            &second_selector,
            "--dry-run",
        ],
    );
    assert!(human.status.success(), "{human:?}");
    let human_stdout = String::from_utf8_lossy(&human.stdout);
    let human_ids = human_stdout
        .lines()
        .filter_map(|line| line.strip_prefix("Profile "))
        .filter_map(|line| line.split_whitespace().next())
        .collect::<Vec<_>>();
    assert_eq!(human_ids, vec!["second", "first", "first", "second"]);
}

#[test]
fn test_profile_apply_from_a_supplied_location_is_re_read_from_the_record() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");

    let applied = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(location),
            "--json",
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(requested_profile(&json(&applied))["status"], "applied");
    assert_eq!(
        stored_record(repo.path())["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );

    // No location is named this time: the record is the repository's own
    // statement of where the bytes are, and reading them back from there is
    // what makes the second application an exact no-op.
    let reapplied = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &id_selector(FIXTURE_PROFILE),
            "--json",
        ],
    );
    assert!(reapplied.status.success(), "{reapplied:?}");
    assert_eq!(requested_profile(&json(&reapplied))["status"], "unchanged");

    let listed = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(listed.status.success(), "{listed:?}");
    let listed = json(&listed);
    assert_eq!(listed["count"], 1);
    assert_eq!(listed["profiles"][0]["id"], FIXTURE_PROFILE);
    assert_eq!(listed["profiles"][0]["applied"], true);
    assert_eq!(
        listed["profiles"][0]["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );
}

#[test]
fn test_profile_show_prefers_a_supplied_location_over_the_recorded_one() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let recorded = package_at(repo.path(), "vendor/recorded");
    assert!(jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(recorded),
            "--json"
        ]
    )
    .status
    .success());

    // A second copy of the same profile, distinguishable from the recorded one
    // by the version it declares.
    let supplied = package_at(repo.path(), "vendor/supplied");
    set_package_version(repo.path(), supplied, "2.0.0");

    let shown = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &path_selector(supplied),
            "--json",
        ],
    );
    assert!(shown.status.success(), "{shown:?}");
    let shown = json(&shown);
    assert_eq!(shown["count"], 1);
    assert_eq!(shown["profiles"][0]["manifest"]["version"], "2.0.0");
    assert_eq!(
        shown["profiles"][0]["origin"],
        serde_json::json!({ "source": "directory", "location": supplied })
    );

    // Supplying nothing falls back to the record, which still names the copy it
    // was applied from.
    let recorded_show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &id_selector(FIXTURE_PROFILE),
            "--json",
        ],
    );
    assert!(recorded_show.status.success(), "{recorded_show:?}");
    let recorded_show = json(&recorded_show);
    assert_eq!(recorded_show["count"], 1);
    assert_eq!(recorded_show["profiles"][0]["manifest"]["version"], "1.0.0");
    assert_eq!(
        recorded_show["profiles"][0]["origin"],
        serde_json::json!({ "source": "directory", "location": recorded })
    );
}

#[test]
fn test_profile_list_reports_a_recorded_location_that_no_longer_resolves() {
    let repo = TempDir::new().unwrap();
    assert!(jit(repo.path(), &["init"]).status.success());
    let location = package_at(repo.path(), "vendor/planner");
    assert!(jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(location),
            "--json"
        ]
    )
    .status
    .success());
    fs::remove_dir_all(repo.path().join(location)).unwrap();

    let listed = jit(repo.path(), &["profile", "list", "--json"]);

    assert!(!listed.status.success(), "{listed:?}");
    let listed = json(&listed);
    let message = listed["error"]["message"].as_str().unwrap_or_default();
    assert!(
        message.contains(&format!(".jit/profiles/{FIXTURE_PROFILE}.json"))
            && message.contains(location),
        "the failure must name the record and the location: {listed}"
    );
    assert_ne!(
        listed["error"]["code"], "PROFILE_NOT_FOUND",
        "a record whose package moved is not a profile this repository never applied"
    );

    // A location that is simply gone carries a NotFound cause, which the human
    // path would otherwise classify as a missing repository. Both renderings
    // report one failure, so they carry one exit status.
    let plain = jit(repo.path(), &["profile", "list"]);
    assert_eq!(
        plain.status.code(),
        jit(repo.path(), &["profile", "list", "--json"])
            .status
            .code(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&plain.stdout),
        String::from_utf8_lossy(&plain.stderr)
    );
    assert_ne!(plain.status.code(), Some(3), "{plain:?}");
}

#[test]
fn test_init_profile_applies_the_package_a_supplied_location_holds() {
    let repo = TempDir::new().unwrap();
    let location = package_at(repo.path(), "vendor/planner");

    // A repository being created has no record to read, so the location is the
    // only statement of where its package is.
    let init = jit(
        repo.path(),
        &["init", "--profile", &path_selector(location), "--json"],
    );

    assert!(init.status.success(), "{init:?}");
    assert_eq!(
        requested_profile(&json(&init)["profile"])["status"],
        "applied"
    );
    assert!(repo.path().join("docs/profile.txt").is_file());
    assert_eq!(
        stored_record(repo.path())["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );

    // The record initialization wrote is readable as the location it names: a
    // later application that supplies nothing resolves the same package.
    let reapplied = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &id_selector(FIXTURE_PROFILE),
            "--json",
        ],
    );
    assert!(reapplied.status.success(), "{reapplied:?}");
    assert_eq!(requested_profile(&json(&reapplied))["status"], "unchanged");
}

#[test]
fn test_init_profile_json_preserves_repeated_root_occurrences_in_order() {
    let repo = TempDir::new().unwrap();
    let location = package_at(repo.path(), "vendor/planner");
    let selector = path_selector(location);

    let init = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &selector,
            "--profile",
            &selector,
            "--json",
        ],
    );

    assert!(init.status.success(), "{init:?}");
    let init_json = json(&init);
    let profiles = init_json["profile"]["profiles"]
        .as_array()
        .expect("init reports one result per root occurrence");
    assert_eq!(
        profiles
            .iter()
            .map(|profile| {
                (
                    profile["id"].as_str().unwrap(),
                    profile["status"].as_str().unwrap(),
                )
            })
            .collect::<Vec<_>>(),
        vec![(FIXTURE_PROFILE, "applied"), (FIXTURE_PROFILE, "unchanged"),]
    );
}

#[test]
fn test_init_profile_resolves_a_declared_dependency_beside_the_declaring_package() {
    let repo = TempDir::new().unwrap();
    let location = package_at(repo.path(), "vendor/planner");
    declare_dependency(repo.path(), location, "jit-default");
    // The dependency is named and no location is supplied for it, so it is
    // resolved from the directory named after it beside the package declaring
    // it — the shape an obtained set of packages arrives in.
    let dependency = "vendor/jit-default";
    jit::test_utils::assemble_repository_package("jit-default", &repo.path().join(dependency))
        .expect("this repository's jit-default package assembles");

    let init = jit(
        repo.path(),
        &["init", "--profile", &path_selector(location), "--json"],
    );

    assert!(
        init.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );
    let applied = &json(&init)["profile"];
    assert_eq!(applied_ids(applied), vec!["jit-default", FIXTURE_PROFILE]);
    assert_eq!(requested_profile(applied)["status"], "applied");

    // Both packages reached the created repository: each has its provenance
    // record naming the directory it was read from, and the named package
    // published its asset.
    assert_eq!(
        serde_json::from_slice::<Value>(
            &fs::read(repo.path().join(".jit/profiles/jit-default.json")).unwrap()
        )
        .unwrap()["origin"],
        serde_json::json!({ "source": "directory", "location": dependency })
    );
    assert_eq!(
        stored_record(repo.path())["origin"],
        serde_json::json!({ "source": "directory", "location": location })
    );
    assert!(repo.path().join("docs/profile.txt").is_file());

    // Validation compares every record against the repository state it
    // describes, so a clean report is both applications having landed whole.
    let validate = jit(repo.path(), &["validate", "--json"]);
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);
    let listed = jit(repo.path(), &["profile", "list", "--json"]);
    assert!(listed.status.success(), "{listed:?}");
    let listed = json(&listed);
    let carried: Vec<(&str, &Value)> = listed["profiles"]
        .as_array()
        .expect("the repository names the profiles it carries")
        .iter()
        .map(|profile| {
            (
                profile["id"].as_str().expect("a named profile"),
                &profile["applied"],
            )
        })
        .collect();
    assert!(
        ["jit-default", FIXTURE_PROFILE]
            .iter()
            .all(|id| carried.contains(&(*id, &Value::Bool(true)))),
        "the repository carries both packages as applied: {listed}"
    );
}

#[test]
fn test_init_from_without_a_profile_is_a_usage_error() {
    let repo = TempDir::new().unwrap();
    let location = package_at(repo.path(), "vendor/planner");

    let init = jit(repo.path(), &["init", "--from", location, "--json"]);

    assert_eq!(init.status.code(), Some(2), "{init:?}");
    assert!(!repo.path().join(".jit").exists());
}

/// The inspection output carries the roots the package's live assets are drawn
/// from, and carries them usably: the declaration and the assets it bounds are
/// read out of the same document, so a consumer needs nothing else to tell
/// which root any packaged live asset came from.
#[test]
fn test_profile_show_json_reports_the_roots_its_live_assets_are_drawn_from() {
    let repo = TempDir::new().unwrap();

    let show = jit(
        repo.path(),
        &[
            "profile",
            "show",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);

    assert_eq!(show["count"], 1);
    let profile = &show["profiles"][0];
    let roots: Vec<&str> = profile["manifest"]["live-source"]
        .as_array()
        .expect("the reported manifest declares its live-source roots")
        .iter()
        .map(|declaration| {
            assert!(
                declaration["exclude"].is_array(),
                "each declared root carries its own exclusion list: {declaration}"
            );
            declaration["root"]
                .as_str()
                .unwrap_or_else(|| panic!("a declared root is a path string: {declaration}"))
        })
        .collect();
    assert!(!roots.is_empty(), "the package declares at least one root");

    let live_targets: Vec<&str> = profile["manifest"]["asset"]
        .as_array()
        .expect("the reported manifest declares assets")
        .iter()
        .filter(|asset| {
            asset["source"]
                .as_str()
                .is_some_and(|source| source.starts_with("assets/live/"))
        })
        .map(|asset| {
            asset["target"]
                .as_str()
                .expect("an asset target is a string")
        })
        .collect();
    assert!(
        !live_targets.is_empty(),
        "the package declares live assets for the roots to bound"
    );

    let unclaimed: Vec<(&str, usize)> = live_targets
        .iter()
        .map(|target| {
            (
                *target,
                roots
                    .iter()
                    .filter(|root| {
                        target
                            .strip_prefix(*root)
                            .is_some_and(|remainder| remainder.starts_with('/'))
                    })
                    .count(),
            )
        })
        .filter(|(_, claiming)| *claiming != 1)
        .collect();
    assert_eq!(
        unclaimed,
        Vec::new(),
        "each entry names a reported live asset target and the number of reported roots claiming it"
    );
}

#[test]
fn test_profile_unknown_id_has_typed_json_error_without_mutation() {
    let repo = TempDir::new().unwrap();
    let output = jit(
        repo.path(),
        &["init", "--profile", "id:missing-profile", "--json"],
    );

    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_NOT_FOUND");
    assert!(!repo.path().join(".jit").exists());
}

#[test]
fn test_profiled_init_publishes_valid_repo_and_applied_inventory() {
    let repo = TempDir::new().unwrap();
    let init = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(
        init.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );
    let init = json(&init);
    assert_eq!(requested_profile(&init["profile"])["status"], "applied");
    assert!(repo.path().join(".jit/profiles/jit-dogfood.json").is_file());
    assert!(repo
        .path()
        .join(".agents/skills/jit-manage/SKILL.md")
        .is_file());
    let applied = init["profile"]["profiles"]
        .as_array()
        .expect("one result per applied package");
    assert_eq!(
        fs::read_to_string(repo.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        applied.len(),
        "initialization appends one event per package it applied and nothing else"
    );

    let validate = jit(repo.path(), &["validate", "--json"]);
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);

    // Every package the initialization applied is enumerated as applied.
    // Enumeration follows the records, whose order is theirs rather than the
    // closure's, so the two are compared as sets.
    let list = json(&jit(repo.path(), &["profile", "list", "--json"]));
    let listed = list["profiles"]
        .as_array()
        .expect("a listed profile per record")
        .iter()
        .map(|profile| (profile["id"].to_string(), profile["applied"].clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        listed,
        applied
            .iter()
            .map(|profile| (profile["id"].to_string(), Value::Bool(true)))
            .collect::<std::collections::BTreeMap<_, _>>()
    );
}

#[test]
fn test_validate_plain_and_json_report_installed_profile_drift() {
    let repo = TempDir::new().unwrap();
    assert!(jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    )
    .status
    .success());
    fs::write(
        repo.path().join(".agents/skills/jit-manage/SKILL.md"),
        "STALE\n",
    )
    .unwrap();

    let plain = jit(repo.path(), &["validate"]);
    assert!(!plain.status.success(), "{plain:?}");
    assert!(
        String::from_utf8_lossy(&plain.stdout).contains("jit-manage/SKILL.md"),
        "stdout={}",
        String::from_utf8_lossy(&plain.stdout)
    );

    let structured = jit(repo.path(), &["validate", "--json"]);
    assert!(!structured.status.success(), "{structured:?}");
    let structured = json(&structured);
    let details = &structured["error"]["details"];
    assert_eq!(details["valid"], false);
    assert!(details["integrity_error"]
        .as_str()
        .is_some_and(|message| message.contains("jit-manage/SKILL.md")));
}

#[test]
fn test_profile_apply_dry_run_is_read_only_then_apply_is_exact_no_op() {
    let repo = TempDir::new().unwrap();
    // A preview is derived over one package against the repository in front of
    // it, so a delta package is previewed with its declared dependency already
    // applied — the order the composed application uses.
    assert!(jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-default")),
        ]
    )
    .status
    .success());
    let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let preview = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--dry-run",
            "--json",
        ],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["count"], 1);
    assert_eq!(json(&preview)["profiles"][0]["status"], "would_apply");
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_before
    );
    assert!(!repo.path().join(".jit/profiles/jit-dogfood.json").exists());

    let applied = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(requested_profile(&json(&applied))["status"], "applied");
    let events_after = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();

    let unchanged = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(requested_profile(&json(&unchanged))["status"], "unchanged");
    assert_eq!(
        fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
        events_after
    );
}

#[test]
fn test_profile_reapply_repairs_missing_and_stale_default_schemas_before_no_op() {
    let repo = TempDir::new().unwrap();
    let initialized = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(initialized.status.success(), "{initialized:?}");

    let namespace_schema = repo
        .path()
        .join(".jit/schemas/default-namespace-registry.json");
    let type_schema = repo
        .path()
        .join(".jit/schemas/default-type-hierarchy-known.json");
    let expected_namespace = fs::read(&namespace_schema).unwrap();
    let expected_types = fs::read(&type_schema).unwrap();
    let events_path = repo.path().join(".jit/events.jsonl");
    let event_count_before = fs::read_to_string(&events_path).unwrap().lines().count();

    fs::remove_file(&namespace_schema).unwrap();
    let preview = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--dry-run",
            "--json",
        ],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["count"], 1);
    assert_eq!(json(&preview)["profiles"][0]["status"], "would_apply");
    assert!(!namespace_schema.exists(), "dry-run must remain read-only");

    let repaired_missing = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(repaired_missing.status.success(), "{repaired_missing:?}");
    assert!(published_anything(&json(&repaired_missing)));
    assert_eq!(fs::read(&namespace_schema).unwrap(), expected_namespace);

    fs::write(&type_schema, b"stale\n").unwrap();
    let repaired_stale = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(repaired_stale.status.success(), "{repaired_stale:?}");
    assert!(published_anything(&json(&repaired_stale)));
    assert_eq!(fs::read(&type_schema).unwrap(), expected_types);

    let events_after_repairs = fs::read(&events_path).unwrap();
    assert_eq!(
        std::str::from_utf8(&events_after_repairs)
            .unwrap()
            .lines()
            .count(),
        event_count_before + 2
    );
    let unchanged = jit(
        repo.path(),
        &[
            "profile",
            "apply",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert!(!published_anything(&json(&unchanged)));
    assert_eq!(fs::read(events_path).unwrap(), events_after_repairs);
}

#[test]
fn test_profiled_init_conflict_preserves_the_occupant_and_publishes_no_conflicting_package() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join("contrib/gates")).unwrap();
    let occupant = repo.path().join("contrib/gates/ai-review.sh");
    fs::write(&occupant, b"local script\n").unwrap();

    let output = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(fs::read(&occupant).unwrap(), b"local script\n");
    // The package that collided published none of its own state: no provenance
    // record, and no asset beside the one it refused to overwrite.
    assert!(!repo.path().join(".jit/profiles/jit-dogfood.json").exists());
    assert!(!repo
        .path()
        .join(".agents/skills/jit-manage/SKILL.md")
        .exists());
}

#[test]
fn test_existing_partial_profiled_init_conflict_preserves_authored_bytes_and_the_occupant() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".jit")).unwrap();
    let index = b"{\n  \"schema_version\": 2,\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}";
    fs::write(repo.path().join(".jit/index.json"), index).unwrap();
    fs::create_dir_all(repo.path().join("contrib/gates")).unwrap();
    let occupant = repo.path().join("contrib/gates/ai-review.sh");
    fs::write(&occupant, b"local script\n").unwrap();

    let output = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );

    assert_eq!(output.status.code(), Some(4));
    assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
    assert_eq!(fs::read(&occupant).unwrap(), b"local script\n");
    assert_eq!(
        fs::read(repo.path().join(".jit/index.json")).unwrap(),
        index,
        "authored neutral bytes must survive a refused application"
    );
    assert!(!repo.path().join(".jit/profiles/jit-dogfood.json").exists());
}

#[test]
fn test_existing_partial_profiled_init_atomically_completes_neutral_scaffold() {
    let repo = TempDir::new().unwrap();
    fs::create_dir_all(repo.path().join(".jit")).unwrap();
    let index = b"{\n  \"schema_version\": 2,\n  \"all_ids\": [],\n  \"deleted_ids\": []\n}";
    fs::write(repo.path().join(".jit/index.json"), index).unwrap();

    let output = jit(
        repo.path(),
        &[
            "init",
            "--profile",
            &path_selector(&crate::repository_package_at(repo.path(), "jit-dogfood")),
            "--json",
        ],
    );

    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    for path in [
        "gates.toml",
        "events.jsonl",
        "config.toml",
        "rules.toml",
        "issues",
        "profiles/jit-dogfood.json",
    ] {
        assert!(
            repo.path().join(".jit").join(path).exists(),
            "missing {path}"
        );
    }
    assert_eq!(
        fs::read(repo.path().join(".jit/index.json")).unwrap(),
        index,
        "existing neutral bytes must be preserved"
    );
    assert_eq!(
        fs::read_to_string(repo.path().join(".jit/events.jsonl"))
            .unwrap()
            .lines()
            .count(),
        json(&output)["profile"]["profiles"]
            .as_array()
            .expect("one result per applied package")
            .len(),
        "completion appends one event per package it applied and nothing else"
    );
    assert!(jit(repo.path(), &["validate", "--json"]).status.success());
}
