use jit::test_cli::JitCli;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

pub(super) fn jit(cli: &JitCli, args: &[&str]) -> Output {
    cli.command(args).output().expect("failed to run jit")
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
/// application refuses a package outside the worktree.
fn package_at<'a>(repo: &Path, location: &'a str) -> &'a str {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile-packages")
        .join(FIXTURE_PROFILE);
    jit::test_utils::copy_package_tree(&source, &repo.join(location));
    location
}

/// Copy the fixture package and give it a distinct identity and target so a
/// selector-order test can observe the complete occurrence stream.
pub(super) fn package_with_id<'a>(repo: &Path, location: &'a str, id: &str) -> &'a str {
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
pub(super) fn package_with_shared_namespace<'a>(
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
            "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"1.0.0\"\ncompatible-jit = \">=0.2.0, <2.0.0\"\n\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"namespaces\"\nidentity = \"shared\"\nvalue = {{ description = \"{description}\", unique = false }}\n"
        ),
    )
    .unwrap();
    location
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
    let declared = format!(
        "{}\n[[dependency]]\nid = \"{id}\"\nversion = \"*\"\n",
        fs::read_to_string(&manifest).unwrap()
    );
    fs::write(&manifest, declared).unwrap();
}

pub(super) fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn test_profile_show_works_without_repository() {
    let repo = JitCli::uninitialised().unwrap();

    let location = super::repository_package_at(repo.root(), "jit-dogfood");
    let show = jit(
        &repo,
        &["profile", "show", "--profile", &location, "--json"],
    );
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(show["profiles"][0]["manifest"]["id"], "jit-dogfood");
    assert!(show["profiles"][0].get("applied").is_none());
    assert_eq!(show["profiles"][0]["origin"], serde_json::json!(location));
    assert!(show["profiles"][0]["package_hash"].as_str().unwrap().len() >= 64);
    assert!(!repo.root().join(".jit").exists());
}

#[test]
fn test_profile_apply_preserves_interleaved_selector_order() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    package_with_id(repo.root(), "packages/first", "first");
    package_with_id(repo.root(), "packages/second", "second");
    let seeded = jit(
        &repo,
        &["profile", "apply", "--profile", "packages/first", "--json"],
    );
    assert!(seeded.status.success(), "{seeded:?}");
    let output = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            "packages/first",
            "--profile",
            "packages/first",
            "--profile",
            "packages/second",
            "--json",
        ],
    );

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        applied_ids(&json(&output)),
        vec!["first", "first", "second"],
        "the selected occurrence order, repeats included, must reach runtime results"
    );
}

#[test]
fn test_profile_apply_preflights_conflicting_repeatable_selectors_before_publication() {
    for (first, second) in [
        ("packages/base", "packages/workflow"),
        ("packages/workflow", "packages/base"),
    ] {
        let repo = JitCli::uninitialised().unwrap();
        assert!(jit(&repo, &["init"]).status.success());
        package_with_shared_namespace(repo.root(), "packages/base", "base", "Base definition");
        package_with_shared_namespace(
            repo.root(),
            "packages/workflow",
            "workflow",
            "Workflow definition",
        );

        let config_path = repo.root().join(".jit/config.toml");
        let events_path = repo.root().join(".jit/events.jsonl");
        let config_before = fs::read(&config_path).unwrap();
        let events_before = fs::read(&events_path).unwrap();
        let output = jit(
            &repo,
            &[
                "profile",
                "apply",
                "--profile",
                first,
                "--profile",
                second,
                "--json",
            ],
        );

        assert!(!output.status.success(), "{output:?}");
        assert_eq!(json(&output)["error"]["code"], "PROFILE_CONFLICT");
        assert_eq!(fs::read(&config_path).unwrap(), config_before, "{output:?}");
        assert_eq!(fs::read(&events_path).unwrap(), events_before);
    }
}

#[test]
fn test_profile_lifecycle_commands_reject_the_superseded_positional_and_from_surface() {
    let repo = JitCli::uninitialised().unwrap();

    let positional = jit(
        &repo,
        &[
            "profile",
            "apply",
            FIXTURE_PROFILE,
            "--profile",
            "packages/missing",
        ],
    );
    assert!(!positional.status.success());
    assert!(String::from_utf8_lossy(&positional.stderr).contains("unexpected argument"));

    let apply = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            "packages/missing",
            "--from",
            "packages",
        ],
    );
    assert!(!apply.status.success());
    assert!(String::from_utf8_lossy(&apply.stderr).contains("unexpected argument"));

    let show = jit(&repo, &["profile", "show", FIXTURE_PROFILE]);
    assert!(!show.status.success());
    assert!(String::from_utf8_lossy(&show.stderr).contains("unexpected argument"));
}

#[test]
fn test_profile_commands_refuse_a_tagged_selector_naming_the_bare_path_form() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    for command in ["show", "diff", "apply"] {
        for selector in ["id:planner", "path:packages/planner"] {
            let refused = jit(
                &repo,
                &["profile", command, "--profile", selector, "--json"],
            );

            assert!(!refused.status.success(), "{refused:?}");
            let error = json(&refused);
            assert_eq!(error["error"]["code"], "INVALID_ARGUMENT");
            let message = error["error"]["message"].as_str().unwrap();
            assert!(message.contains(selector), "{message}");
            assert!(message.contains("bare package directory path"), "{message}");
        }
    }
}

#[test]
fn test_profile_show_reads_the_package_a_supplied_location_holds() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let location = package_at(repo.root(), "vendor/planner");

    let show = jit(&repo, &["profile", "show", "--profile", location, "--json"]);

    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["profiles"].as_array().unwrap().len(), 1);
    // Nothing but the supplied location holds a package declaring this
    // profile, so reporting it at all is that location having been read.
    assert_eq!(show["profiles"][0]["manifest"]["id"], FIXTURE_PROFILE);
    assert_eq!(show["profiles"][0]["origin"], serde_json::json!(location));
}

#[test]
fn test_profile_commands_report_an_absent_package_directory_as_one_error_naming_the_path() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());

    for command in ["show", "diff", "apply"] {
        let output = jit(
            &repo,
            &["profile", command, "--profile", "vendor/absent", "--json"],
        );

        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = &json(&output)["error"];
        assert_eq!(error["code"], "PROFILE_ERROR");
        let message = error["message"].as_str().unwrap();
        assert!(message.contains("vendor/absent"), "{message}");
        assert!(message.contains("manifest.toml"), "{message}");
    }
}

#[test]
fn test_profile_show_prints_target_paths_without_hash_values() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let location = package_at(repo.root(), "vendor/planner");

    let shown = json(&jit(
        &repo,
        &["profile", "show", "--profile", location, "--json"],
    ));
    let targets = shown["profiles"][0]["targets"].as_array().unwrap();
    assert!(!targets.is_empty());

    let text = jit(&repo, &["profile", "show", "--profile", location]);
    let text = String::from_utf8_lossy(&text.stdout);
    for target in targets {
        let target = target.as_str().unwrap();
        assert!(
            text.lines().any(|line| line.trim() == target),
            "'{target}' is not its own line in: {text}"
        );
    }
}

#[test]
fn test_profile_show_json_preserves_repeated_selectors_in_order() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let location = package_at(repo.root(), "vendor/planner");
    let selector = location;

    let show = jit(
        &repo,
        &[
            "profile",
            "show",
            "--profile",
            selector,
            "--profile",
            selector,
            "--json",
        ],
    );

    assert!(show.status.success(), "{show:?}");
    let show = json(&show);
    assert_eq!(show["profiles"].as_array().unwrap().len(), 2);
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
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let location = package_at(repo.root(), "vendor/planner");
    let selector = location;

    let show = jit(
        &repo,
        &[
            "profile",
            "show",
            "--profile",
            selector,
            "--profile",
            selector,
        ],
    );

    assert!(show.status.success(), "{show:?}");
    let stdout = String::from_utf8_lossy(&show.stdout);
    assert_eq!(
        stdout.matches("Profile planner-asset-only 1.0.0").count(),
        2,
        "human rendering must include every selected occurrence"
    );
}

#[test]
fn test_profile_show_refuses_a_supplied_location_outside_the_repository() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    // A package beside the repository rather than inside it, so it has no
    // worktree-relative location.
    let elsewhere = TempDir::new().unwrap();
    package_at(elsewhere.path(), "planner");

    let show = jit(
        &repo,
        &[
            "profile",
            "show",
            "--profile",
            elsewhere.path().join("planner").to_str().unwrap(),
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
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let location = package_at(repo.root(), ".jit/vendored");

    let show = jit(&repo, &["profile", "show", "--profile", location, "--json"]);

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
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let location = package_at(repo.root(), "vendor/planner");

    let preview = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            location,
            "--dry-run",
            "--json",
        ],
    );

    assert!(preview.status.success(), "{preview:?}");
    let preview = json(&preview);
    assert_eq!(preview["profiles"][0]["id"], FIXTURE_PROFILE);
    assert_eq!(preview["profiles"].as_array().unwrap().len(), 1);
    assert_eq!(preview["profiles"][0]["status"], "would_apply");
    assert!(
        preview["profiles"][0]["targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["path"] == "docs/profile.txt"),
        "{preview}"
    );
    // A preview writes nothing.
    assert!(!repo.root().join("docs/profile.txt").exists());
}

#[test]
fn test_profile_variable_inputs_cover_init_apply_dry_run_and_dependency_precedence() {
    let init_repo = JitCli::uninitialised().unwrap();
    write_variable_package(
        init_repo.root(),
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
    let init_values = init_repo.root().join("values.toml");
    fs::write(&init_values, "[variables]\nNAME = \"from-file\"\n").unwrap();
    let init_values = init_values.to_str().unwrap();
    let init = jit(
        &init_repo,
        &[
            "init",
            "--profile",
            "packages/init-vars",
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
        fs::read_to_string(init_repo.root().join("docs/init-vars.txt")).unwrap(),
        "NAME=last\n"
    );

    let apply_repo = JitCli::uninitialised().unwrap();
    write_variable_package(
        apply_repo.root(),
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
        apply_repo.root(),
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
    assert!(jit(&apply_repo, &["init"]).status.success());
    let values = apply_repo.root().join("values.toml");
    fs::write(&values, "[variables]\nNAME = \"from-file\"\n").unwrap();
    let values = values.to_str().unwrap();
    let selector = "packages/root-vars";
    let preview = jit(
        &apply_repo,
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
    assert!(!apply_repo.root().join("docs/root-vars.txt").exists());
    assert!(!apply_repo.root().join("docs/dependency-vars.txt").exists());

    let applied = jit(
        &apply_repo,
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
        fs::read_to_string(apply_repo.root().join("docs/root-vars.txt")).unwrap(),
        "NAME=last\n"
    );
    assert_eq!(
        fs::read_to_string(apply_repo.root().join("docs/dependency-vars.txt")).unwrap(),
        "TOKEN=dependency-value\n"
    );
}

#[test]
fn test_profile_variable_records_the_resolved_value_in_the_event_and_validates() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    write_variable_package(
        repo.root(),
        "packages/resolved-vars",
        VariablePackageSpec {
            id: "resolved-vars",
            variable: "NAME",
            default: None,
            environment: Some("JIT_PROFILE_RESOLVED_NAME"),
            target: "docs/resolved-vars.txt",
            body: "NAME={{jit:var:NAME}}\n",
            dependency: None,
        },
    );
    let resolved_value = "persisted-public-value";
    let applied = repo
        .command_with_env(
            [
                "profile",
                "apply",
                "--profile",
                "packages/resolved-vars",
                "--set",
                &format!("NAME={resolved_value}"),
                "--json",
            ],
            [("JIT_PROFILE_RESOLVED_NAME", "environment-at-apply")],
        )
        .output()
        .unwrap();
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(
        fs::read_to_string(repo.root().join("docs/resolved-vars.txt")).unwrap(),
        format!("NAME={resolved_value}\n")
    );

    let events = fs::read_to_string(repo.root().join(".jit/events.jsonl")).unwrap();
    let lifecycle = events
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .find(|event| event["type"] == "profile_lifecycle")
        .expect("the apply is audited");
    let recorded = &lifecycle["profiles"][0];
    assert_eq!(recorded["id"], "resolved-vars");
    assert_eq!(recorded["version"], "1.0.0");
    assert_eq!(recorded["variables"]["NAME"], resolved_value);

    let validate = repo
        .command_with_env(
            ["validate", "--json"],
            [("JIT_PROFILE_RESOLVED_NAME", "different-current-environment")],
        )
        .output()
        .unwrap();
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);
}

#[test]
fn test_profile_apply_dry_run_preserves_order_and_duplicates_without_mutation() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let first = package_with_id(repo.root(), "packages/first", "first");
    let second = package_with_id(repo.root(), "packages/second", "second");

    let seeded = jit(&repo, &["profile", "apply", "--profile", first, "--json"]);
    assert!(seeded.status.success(), "{seeded:?}");
    let events_before = fs::read(repo.root().join(".jit/events.jsonl")).unwrap();

    let second_selector = second;
    let first_selector = first;
    let output = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            second_selector,
            "--profile",
            first_selector,
            "--profile",
            first_selector,
            "--profile",
            second_selector,
            "--dry-run",
            "--json",
        ],
    );

    assert!(output.status.success(), "{output:?}");
    let preview = json(&output);
    assert_eq!(preview["profiles"].as_array().unwrap().len(), 4);
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
        vec!["would_apply", "unchanged", "unchanged", "unchanged"]
    );
    assert_eq!(
        fs::read(repo.root().join(".jit/events.jsonl")).unwrap(),
        events_before,
        "dry-run must not append events"
    );
    assert!(!repo.root().join("docs/second.txt").exists());

    let human = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            second_selector,
            "--profile",
            first_selector,
            "--profile",
            first_selector,
            "--profile",
            second_selector,
            "--dry-run",
        ],
    );
    assert!(human.status.success(), "{human:?}");
    let human_stdout = String::from_utf8_lossy(&human.stdout);
    let expected = preview["profiles"]
        .as_array()
        .unwrap()
        .iter()
        .map(|profile| format!("Profile {} ", profile["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    let headings = human_stdout
        .lines()
        .filter(|line| line.starts_with("Profile "))
        .collect::<Vec<_>>();
    assert!(
        headings.len() == expected.len()
            && headings
                .iter()
                .zip(&expected)
                .all(|(heading, prefix)| heading.starts_with(prefix.as_str())),
        "human headings must render the JSON decisions exactly, in order: {expected:?} vs {headings:?}"
    );
    assert!(human_stdout.contains("  create:"), "{human_stdout}");
}

#[test]
fn test_init_profile_applies_the_package_a_supplied_location_holds() {
    let repo = JitCli::uninitialised().unwrap();
    let location = package_at(repo.root(), "vendor/planner");

    let init = jit(&repo, &["init", "--profile", location, "--json"]);

    assert!(init.status.success(), "{init:?}");
    assert_eq!(
        requested_profile(&json(&init)["profile"])["status"],
        "applied"
    );
    assert!(repo.root().join("docs/profile.txt").is_file());
    assert_eq!(
        requested_profile(&json(&init)["profile"])["origin"],
        serde_json::json!(location)
    );
}

#[test]
fn test_init_profile_json_preserves_repeated_root_occurrences_in_order() {
    let repo = JitCli::uninitialised().unwrap();
    let location = package_at(repo.root(), "vendor/planner");
    let selector = location;

    let init = jit(
        &repo,
        &[
            "init",
            "--profile",
            selector,
            "--profile",
            selector,
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
    let repo = JitCli::uninitialised().unwrap();
    let location = package_at(repo.root(), "vendor/planner");
    declare_dependency(repo.root(), location, "jit-default");
    // The dependency is named and no location is supplied for it, so it is
    // resolved from the directory named after it beside the package declaring
    // it — the shape an obtained set of packages arrives in.
    let dependency = "vendor/jit-default";
    jit::test_utils::capture_repository_package("jit-default", &repo.root().join(dependency))
        .expect("this repository's jit-default package captures");

    let init = jit(&repo, &["init", "--profile", location, "--json"]);

    assert!(
        init.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&init.stdout),
        String::from_utf8_lossy(&init.stderr)
    );
    let applied = &json(&init)["profile"];
    assert_eq!(applied_ids(applied), vec!["jit-default", FIXTURE_PROFILE]);
    assert_eq!(requested_profile(applied)["status"], "applied");

    // Both packages reached the created repository: each reports the
    // directory it was read from, and the named package published its asset.
    assert_eq!(
        applied["profiles"][0]["origin"],
        serde_json::json!(dependency)
    );
    assert_eq!(
        requested_profile(applied)["origin"],
        serde_json::json!(location)
    );
    assert!(repo.root().join("docs/profile.txt").is_file());

    // A clean report is both applications having landed whole.
    let validate = jit(&repo, &["validate", "--json"]);
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);
}

#[test]
fn test_init_from_without_a_profile_is_a_usage_error() {
    let repo = JitCli::uninitialised().unwrap();
    let location = package_at(repo.root(), "vendor/planner");

    let init = jit(&repo, &["init", "--from", location, "--json"]);

    assert!(!init.status.success(), "{init:?}");
    assert!(String::from_utf8_lossy(&init.stderr).contains("--from"));
    assert!(!repo.root().join(".jit").exists());
}

/// The inspection output carries the roots the package's live assets are drawn
/// from, and carries them usably: the declaration and the assets it bounds are
/// read out of the same document, so a consumer needs nothing else to tell
/// which root any packaged live asset came from.
#[test]
fn test_profile_show_json_reports_the_roots_its_live_assets_are_drawn_from() {
    let repo = JitCli::uninitialised().unwrap();

    let show = jit(
        &repo,
        &[
            "profile",
            "show",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(show.status.success(), "{show:?}");
    let show = json(&show);

    assert_eq!(show["profiles"].as_array().unwrap().len(), 1);
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
fn test_profile_missing_package_directory_has_typed_json_error_without_mutation() {
    let repo = JitCli::uninitialised().unwrap();
    let output = jit(
        &repo,
        &["init", "--profile", "packages/missing-profile", "--json"],
    );

    assert!(!output.status.success(), "{output:?}");
    assert_eq!(json(&output)["error"]["code"], "PROFILE_ERROR");
    assert!(!repo.root().join(".jit").exists());
}

#[test]
fn test_profiled_init_publishes_a_valid_repo() {
    let repo = JitCli::uninitialised().unwrap();
    let init = jit(
        &repo,
        &[
            "init",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
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
    assert!(repo
        .root()
        .join(".agents/skills/jit-manage/SKILL.md")
        .is_file());
    let events = fs::read_to_string(repo.root().join(".jit/events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        events.len(),
        1,
        "initialization appends one aggregate event"
    );
    assert_eq!(events[0]["type"], "profile_lifecycle");
    assert_eq!(events[0]["operation"], "initialize");

    let validate = jit(&repo, &["validate", "--json"]);
    assert!(validate.status.success(), "{validate:?}");
    assert_eq!(json(&validate)["valid"], true);
}

#[test]
fn test_profile_apply_dry_run_is_read_only_then_apply_is_exact_no_op() {
    let repo = JitCli::uninitialised().unwrap();
    // A preview is derived over one package against the repository in front of
    // it, so a delta package is previewed with its declared dependency already
    // applied — the order the composed application uses.
    assert!(jit(
        &repo,
        &[
            "init",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-default"),
        ]
    )
    .status
    .success());
    let events_before = fs::read(repo.root().join(".jit/events.jsonl")).unwrap();

    let preview = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--dry-run",
            "--json",
        ],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["profiles"].as_array().unwrap().len(), 2);
    assert!(
        json(&preview)["profiles"]
            .as_array()
            .expect("a rehearsal reports the composed profile collection")
            .iter()
            .any(|profile| profile["status"] == "would_apply"),
        "the rehearsal attributes the repair to the package that owns it"
    );
    assert_eq!(
        fs::read(repo.root().join(".jit/events.jsonl")).unwrap(),
        events_before
    );

    let applied = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(applied.status.success(), "{applied:?}");
    assert_eq!(requested_profile(&json(&applied))["status"], "applied");
    let events_after = fs::read(repo.root().join(".jit/events.jsonl")).unwrap();

    let unchanged = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert_eq!(requested_profile(&json(&unchanged))["status"], "unchanged");
    assert_eq!(
        fs::read(repo.root().join(".jit/events.jsonl")).unwrap(),
        events_after
    );
}

#[test]
fn test_profile_reapply_repairs_missing_and_stale_default_schemas_before_no_op() {
    let repo = JitCli::uninitialised().unwrap();
    let initialized = jit(
        &repo,
        &[
            "init",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(initialized.status.success(), "{initialized:?}");

    let namespace_schema = repo
        .root()
        .join(".jit/schemas/default-namespace-registry.json");
    let type_schema = repo
        .root()
        .join(".jit/schemas/default-type-hierarchy-known.json");
    let expected_namespace = fs::read(&namespace_schema).unwrap();
    let expected_types = fs::read(&type_schema).unwrap();
    let events_path = repo.root().join(".jit/events.jsonl");
    let event_count_before = fs::read_to_string(&events_path).unwrap().lines().count();

    fs::remove_file(&namespace_schema).unwrap();
    let preview = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--dry-run",
            "--json",
        ],
    );
    assert!(preview.status.success(), "{preview:?}");
    assert_eq!(json(&preview)["profiles"].as_array().unwrap().len(), 2);
    assert!(
        json(&preview)["profiles"]
            .as_array()
            .expect("a rehearsal reports the composed profile collection")
            .iter()
            .any(|profile| profile["status"] == "would_apply"),
        "the rehearsal attributes the repair to the package that owns it"
    );
    assert!(!namespace_schema.exists(), "dry-run must remain read-only");

    let repaired_missing = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(repaired_missing.status.success(), "{repaired_missing:?}");
    assert!(published_anything(&json(&repaired_missing)));
    assert_eq!(
        json(&repaired_missing)["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["applied", "unchanged"],
        "the default-profile candidate owns the coupled schema repair"
    );
    assert_eq!(fs::read(&namespace_schema).unwrap(), expected_namespace);

    fs::write(&type_schema, b"stale\n").unwrap();
    let repaired_stale = jit(
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(repaired_stale.status.success(), "{repaired_stale:?}");
    assert!(published_anything(&json(&repaired_stale)));
    assert_eq!(
        json(&repaired_stale)["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["status"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["applied", "unchanged"],
        "the default-profile candidate owns the coupled schema repair"
    );
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
        &repo,
        &[
            "profile",
            "apply",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
            "--json",
        ],
    );
    assert!(unchanged.status.success(), "{unchanged:?}");
    assert!(!published_anything(&json(&unchanged)));
    assert_eq!(fs::read(events_path).unwrap(), events_after_repairs);
}

#[test]
fn test_existing_partial_profiled_init_atomically_completes_neutral_scaffold() {
    let repo = JitCli::uninitialised().unwrap();
    fs::create_dir_all(repo.root().join(".jit")).unwrap();
    let index = jit::repository_state::fresh_index_bytes().unwrap();
    fs::write(repo.root().join(".jit/index.json"), &index).unwrap();

    let output = jit(
        &repo,
        &[
            "init",
            "--profile",
            &super::repository_package_at(repo.root(), "jit-dogfood"),
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
    ] {
        assert!(
            repo.root().join(".jit").join(path).exists(),
            "missing {path}"
        );
    }
    assert_eq!(
        fs::read(repo.root().join(".jit/index.json")).unwrap(),
        index,
        "existing neutral bytes must be preserved"
    );
    let events = fs::read_to_string(repo.root().join(".jit/events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(events.len(), 1, "completion appends one aggregate event");
    assert_eq!(events[0]["type"], "profile_lifecycle");
    assert_eq!(events[0]["operation"], "initialize");
    assert!(jit(&repo, &["validate", "--json"]).status.success());
}

/// Write a versioned package whose one templated asset and one fixed asset
/// separate what a supplied value reaches from what it does not.
pub(super) fn write_lifecycle_package(repo: &Path, location: &str, id: &str, version: &str) {
    let root = repo.join(location);
    fs::create_dir_all(root.join("assets")).unwrap();
    fs::write(
        root.join("manifest.toml"),
        format!(
            "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"{version}\"\n\
             compatible-jit = \"*\"\n\n\
             [[variable]]\nname = \"GREETING\"\ndefault = \"authored\"\n\n\
             [[asset]]\nsource = \"assets/templated.txt\"\n\
             target = \"docs/{id}-templated.txt\"\ntemplate = true\n\n\
             [[asset]]\nsource = \"assets/fixed.txt\"\ntarget = \"docs/{id}-fixed.txt\"\n"
        ),
    )
    .unwrap();
    fs::write(
        root.join("assets/templated.txt"),
        "greeting={{jit:var:GREETING}}\n",
    )
    .unwrap();
    fs::write(root.join("assets/fixed.txt"), "no variable reaches this\n").unwrap();
}

/// The single profile entry returned by one-package authoring and exchange
/// operations.
pub(super) fn only_profile(response: &Value) -> &Value {
    let [profile] = response["profiles"].as_array().unwrap().as_slice() else {
        panic!("one profile entry: {response}");
    };
    profile
}

fn assert_human_profile_decision(
    output: &Output,
    id: &str,
    version: &str,
    origin: &str,
    status: &str,
) {
    assert!(output.status.success(), "{output:?}");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(&format!(
            "Profile {id} {version} (directory {origin}): {status}"
        )),
        "{stdout}"
    );
    assert!(stdout.contains("  create:"), "{stdout}");
}

/// A package directory whose live assets are absent from it and present in the
/// repository, so a packed live asset's bytes can only have come from the
/// repository file its declaration targets.
pub(super) fn live_asset_package(repo: &Path, location: &str) -> String {
    let root = repo.join(location);
    fs::create_dir_all(root.join("assets/install")).unwrap();
    fs::write(
        root.join("manifest.toml"),
        "[profile]\nmanifest-version = 2\nid = \"captured\"\nversion = \"1.0.0\"\ncompatible-jit = \">=0.2.0, <2.0.0\"\n\n[[live-source]]\nroot = \"docs\"\nexclude = []\n\n[[asset]]\nsource = \"assets/live/docs/guide.md\"\ntarget = \"docs/guide.md\"\n\n[[asset]]\nsource = \"assets/install/settings.toml\"\ntarget = \"settings.toml\"\n",
    )
    .unwrap();
    fs::write(
        root.join("assets/install/settings.toml"),
        "installed = true\n",
    )
    .unwrap();
    fs::create_dir_all(repo.join("docs")).unwrap();
    fs::write(repo.join("docs/guide.md"), "# Captured guide\n").unwrap();
    location.to_string()
}

#[test]
fn test_profile_authoring_and_exchange_rehearsals_report_decisions_without_writing() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let external = TempDir::new().unwrap();
    let external_archive = external.path().join("synthetic.tar");
    let pack = jit(
        &repo,
        &[
            "profile",
            "pack",
            "--source",
            package,
            "--output",
            external_archive.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );
    assert!(pack.status.success(), "{pack:?}");
    let pack = json(&pack);
    let packed = only_profile(&pack);
    assert_eq!(packed["status"], "would_apply");
    assert_eq!(packed["targets"][0]["action"], "create");
    assert!(!external_archive.exists());
    let pack_human = jit(
        &repo,
        &[
            "profile",
            "pack",
            "--source",
            package,
            "--output",
            external_archive.to_str().unwrap(),
            "--dry-run",
        ],
    );
    assert_human_profile_decision(
        &pack_human,
        "synthetic-workflow",
        "1.2.3",
        "packages/synthetic",
        "would pack",
    );

    let packed = jit(
        &repo,
        &[
            "profile",
            "pack",
            "--source",
            package,
            "--output",
            "synthetic.tar",
            "--json",
        ],
    );
    assert!(packed.status.success(), "{packed:?}");
    let add = jit(
        &repo,
        &[
            "profile",
            "add",
            "--archive",
            "synthetic.tar",
            "--destination",
            "packages/added",
            "--dry-run",
            "--json",
        ],
    );
    assert!(add.status.success(), "{add:?}");
    let add = json(&add);
    let added = only_profile(&add);
    assert_eq!(added["status"], "would_apply");
    assert_eq!(added["origin"], "packages/added");
    assert!(added["targets"]
        .as_array()
        .is_some_and(|targets| !targets.is_empty()
            && targets.iter().all(|target| target["action"] == "create")));
    assert!(!repo.root().join("packages/added").exists());
    let add_human = jit(
        &repo,
        &[
            "profile",
            "add",
            "--archive",
            "synthetic.tar",
            "--destination",
            "packages/added",
            "--dry-run",
        ],
    );
    assert_human_profile_decision(
        &add_human,
        "synthetic-workflow",
        "1.2.3",
        "packages/added",
        "would add",
    );
}

/// Copy the checked-in synthetic fixture into `repo` at the repository-relative
/// `location`, and return that location.
///
/// This fixture is the one that exercises a nested source and a
/// declared-executable asset, which is what makes a round trip through an
/// archive say something about directories and modes as well as bytes.
fn synthetic_package_at<'a>(repo: &Path, location: &'a str) -> &'a str {
    let source = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile-packages/synthetic-valid");
    jit::test_utils::copy_package_tree(&source, &repo.join(location));
    location
}

/// Every worktree file under `root`, repository-relative, excluding the
/// repository's own state.
///
/// What an operation left in the worktree is the whole answer to "was anything
/// written outside the destination", so it is read rather than reasoned about.
fn worktree_files(root: &Path) -> std::collections::BTreeSet<String> {
    fn visit(root: &Path, directory: &Path, found: &mut std::collections::BTreeSet<String>) {
        for entry in fs::read_dir(directory).expect("read worktree directory") {
            let path = entry.expect("read worktree entry").path();
            let relative = path
                .strip_prefix(root)
                .expect("worktree entry is under the root")
                .to_string_lossy()
                .into_owned();
            if relative == ".jit" || relative == ".git" {
                continue;
            }
            if path.is_dir() {
                visit(root, &path, found);
            } else {
                found.insert(relative);
            }
        }
    }

    let mut found = std::collections::BTreeSet::new();
    visit(root, root, &mut found);
    found
}

/// Every directory under `root`, root-relative.
///
/// A publication enumerates directories as well as files, so a test that says
/// something about which of the two drives its cost reads both.
fn worktree_directories(root: &Path) -> std::collections::BTreeSet<String> {
    fn visit(root: &Path, directory: &Path, found: &mut std::collections::BTreeSet<String>) {
        for entry in fs::read_dir(directory).expect("read worktree directory") {
            let path = entry.expect("read worktree entry").path();
            if path.is_dir() {
                found.insert(
                    path.strip_prefix(root)
                        .expect("worktree entry is under the root")
                        .to_string_lossy()
                        .into_owned(),
                );
                visit(root, &path, found);
            }
        }
    }

    let mut found = std::collections::BTreeSet::new();
    visit(root, root, &mut found);
    found
}

/// The archive is a function of the package it carries: two packs of one
/// package directory write the same file.
#[test]
fn test_profile_pack_writes_identical_archives_for_one_package_directory() {
    let repo = JitCli::uninitialised().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");

    for output in ["first.tar", "second.tar"] {
        let packed = jit(
            &repo,
            &[
                "profile", "pack", "--source", package, "--output", output, "--json",
            ],
        );
        assert!(packed.status.success(), "{packed:?}");
    }

    let first = fs::read(repo.root().join("first.tar")).unwrap();
    assert!(!first.is_empty(), "a packed archive has content");
    assert_eq!(
        first,
        fs::read(repo.root().join("second.tar")).unwrap(),
        "two packs of one package directory wrote different archives"
    );
}

/// Every shipped package is self-contained at the archive boundary, including
/// its live consumers, which a pack draws from the repository holding them.
#[test]
fn test_profile_pack_packs_every_shipped_package_from_a_repository_holding_it() {
    for id in jit::test_utils::published_package_ids() {
        let exchange = TempDir::new().unwrap();
        let repo = JitCli::uninitialised().unwrap();
        let package = super::repository_package_at(repo.root(), &id);
        let initialized = jit(&repo, &["init", "--profile", &package, "--json"]);
        assert!(initialized.status.success(), "{initialized:?}");
        let archive = exchange.path().join(format!("{id}.tar"));

        let packed = jit(
            &repo,
            &[
                "profile",
                "pack",
                "--source",
                &package,
                "--output",
                archive.to_str().unwrap(),
                "--json",
            ],
        );

        assert!(packed.status.success(), "packing {id} failed: {packed:?}");
        let tree = jit::profile::read_package_archive(&fs::read(&archive).unwrap())
            .expect("the shipped archive reads back");
        if id == "jit-dogfood" {
            assert!(
                tree.source_bytes("assets/live/.agents/skills/jit-breakdown/SKILL.md")
                    .is_some(),
                "the dogfood archive omitted a declared live consumer"
            );
        }
    }
}

/// One package reaches another repository through the archive alone: the
/// identity the pack reported is the identity the add recomputes, the tree
/// arrives whole, and the added package applies.
#[test]
fn test_profile_pack_and_add_carry_one_package_between_repositories() {
    let source_repo = JitCli::uninitialised().unwrap();
    let target_repo = JitCli::uninitialised().unwrap();
    let exchange = TempDir::new().unwrap();
    assert!(jit(&source_repo, &["init"]).status.success());
    assert!(jit(&target_repo, &["init"]).status.success());
    let package = synthetic_package_at(source_repo.root(), "packages/synthetic");
    let archive = exchange.path().join("synthetic.tar");
    let archive = archive.to_str().unwrap();
    let before = worktree_files(target_repo.root());

    let packed = jit(
        &source_repo,
        &[
            "profile", "pack", "--source", package, "--output", archive, "--json",
        ],
    );
    assert!(packed.status.success(), "{packed:?}");
    let packed = json(&packed);
    let packed = only_profile(&packed);

    let added = jit(
        &target_repo,
        &[
            "profile",
            "add",
            "--archive",
            archive,
            "--destination",
            "packages/synthetic",
            "--json",
        ],
    );
    assert!(added.status.success(), "{added:?}");
    let added = json(&added);
    let added = only_profile(&added);

    assert_eq!(added["id"], packed["id"]);
    assert_eq!(added["version"], packed["version"]);
    assert_eq!(
        added["package_hash"], packed["package_hash"],
        "the identity recomputed from the archive differs from the packed package's own"
    );
    assert_eq!(added["destination"], "packages/synthetic");

    // The archive carried the package and nothing else: the target worktree
    // gained exactly the files the source package directory holds, at the
    // destination the invocation named and nowhere else.
    let published = worktree_files(target_repo.root())
        .difference(&before)
        .cloned()
        .collect::<Vec<_>>();
    let expected = worktree_files(&source_repo.root().join(package))
        .into_iter()
        .map(|relative| format!("{package}/{relative}"))
        .collect::<Vec<_>>();
    assert!(!expected.is_empty(), "the source package has files");
    assert_eq!(published, expected, "the add published a different tree");
    for relative in &expected {
        assert_eq!(
            fs::read(target_repo.root().join(relative)).unwrap(),
            fs::read(source_repo.root().join(relative)).unwrap(),
            "{relative} arrived with different bytes"
        );
    }

    // The published tree is a package the receiving repository reads as one,
    // at the identity the archive travelled under.
    let shown = jit(
        &target_repo,
        &[
            "profile",
            "show",
            "--profile",
            "packages/synthetic",
            "--json",
        ],
    );
    assert!(shown.status.success(), "{shown:?}");
    assert_eq!(
        json(&shown)["profiles"][0]["package_hash"],
        packed["package_hash"],
        "the published tree does not read back as the package that was packed"
    );
}

/// An archive whose content no longer reproduces the digest it carries is
/// refused, and the destination is left absent.
#[test]
fn test_profile_add_refuses_a_damaged_archive_without_publishing_a_package() {
    let repo = JitCli::uninitialised().unwrap();
    let exchange = TempDir::new().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let archive = exchange.path().join("synthetic.tar");
    let archive = archive.to_str().unwrap();
    assert!(jit(
        &repo,
        &["profile", "pack", "--source", package, "--output", archive, "--json",],
    )
    .status
    .success());

    // Damage one byte of the packaged content, which the metadata's digest no
    // longer accounts for.
    let mut bytes = fs::read(archive).unwrap();
    let workflow = bytes
        .windows(b"Synthetic workflow".len())
        .position(|window| window == b"Synthetic workflow")
        .expect("the archive carries the fixture asset's bytes");
    bytes[workflow] = b'X';
    fs::write(archive, bytes).unwrap();

    let added = jit(
        &repo,
        &[
            "profile",
            "add",
            "--archive",
            archive,
            "--destination",
            "packages/arrived",
            "--json",
        ],
    );

    assert!(!added.status.success(), "{added:?}");
    assert!(
        json(&added)["error"]["message"]
            .as_str()
            .expect("a typed failure carries an error message")
            .contains("hashes to"),
        "{added:?}"
    );
    assert!(
        !repo.root().join("packages/arrived").exists(),
        "a refused add left a partially extracted package behind"
    );
}

/// An occupied destination is refused rather than published over, and what it
/// held survives untouched.
#[test]
fn test_profile_add_refuses_an_occupied_destination_without_replacing_it() {
    let repo = JitCli::uninitialised().unwrap();
    let exchange = TempDir::new().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let archive = exchange.path().join("synthetic.tar");
    let archive = archive.to_str().unwrap();
    assert!(jit(
        &repo,
        &["profile", "pack", "--source", package, "--output", archive, "--json",],
    )
    .status
    .success());
    fs::create_dir_all(repo.root().join("packages/occupied")).unwrap();
    fs::write(repo.root().join("packages/occupied/mine.txt"), "mine\n").unwrap();

    let added = jit(
        &repo,
        &[
            "profile",
            "add",
            "--archive",
            archive,
            "--destination",
            "packages/occupied",
            "--json",
        ],
    );

    assert!(!added.status.success(), "{added:?}");
    assert!(
        json(&added)["error"]["message"]
            .as_str()
            .expect("a typed failure carries an error message")
            .contains("already exists"),
        "{added:?}"
    );
    assert_eq!(
        fs::read_dir(repo.root().join("packages/occupied"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        vec!["mine.txt".to_string()],
        "a refused add published into an occupied destination"
    );
}

/// A destination outside the repository worktree is refused before the archive
/// is even read, and nothing is published there.
#[test]
fn test_profile_add_refuses_a_destination_outside_the_worktree_without_publishing() {
    let repo = JitCli::uninitialised().unwrap();
    let outside = TempDir::new().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let archive = outside.path().join("synthetic.tar");
    let archive = archive.to_str().unwrap();
    assert!(jit(
        &repo,
        &["profile", "pack", "--source", package, "--output", archive, "--json",],
    )
    .status
    .success());
    let destination = outside.path().join("arrived");

    let added = jit(
        &repo,
        &[
            "profile",
            "add",
            "--archive",
            archive,
            "--destination",
            destination.to_str().unwrap(),
            "--json",
        ],
    );

    assert!(!added.status.success(), "{added:?}");
    assert!(
        json(&added)["error"]["message"]
            .as_str()
            .expect("a typed failure carries an error message")
            .contains("worktree"),
        "{added:?}"
    );
    assert!(!destination.exists(), "a refused add published a tree");
}

/// An occupied output path is refused rather than overwritten, whether it lies
/// inside the repository or outside it.
#[test]
fn test_profile_pack_refuses_an_occupied_output_path_without_replacing_it() {
    let repo = JitCli::uninitialised().unwrap();
    let outside = TempDir::new().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let inside = repo.root().join("taken.tar");
    let external = outside.path().join("taken.tar");
    fs::write(&inside, "not an archive\n").unwrap();
    fs::write(&external, "not an archive\n").unwrap();

    for output in [&inside, &external] {
        let packed = jit(
            &repo,
            &[
                "profile",
                "pack",
                "--source",
                package,
                "--output",
                output.to_str().unwrap(),
                "--json",
            ],
        );

        assert!(!packed.status.success(), "{packed:?}");
        assert_eq!(
            fs::read_to_string(output).unwrap(),
            "not an archive\n",
            "a refused pack replaced what the output path held"
        );
    }
}

#[test]
fn test_profile_pack_external_dry_run_refuses_a_missing_parent_without_writing() {
    let repo = JitCli::uninitialised().unwrap();
    let outside = TempDir::new().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let missing_parent = outside.path().join("missing");
    let output = missing_parent.join("synthetic.tar");

    let packed = jit(
        &repo,
        &[
            "profile",
            "pack",
            "--source",
            package,
            "--output",
            output.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );

    assert!(!packed.status.success(), "{packed:?}");
    assert!(
        !missing_parent.exists(),
        "external rehearsal must not create its missing parent"
    );
}

#[cfg(unix)]
#[test]
fn test_profile_pack_external_dry_run_refuses_a_symlinked_parent_without_writing() {
    let repo = JitCli::uninitialised().unwrap();
    let outside = TempDir::new().unwrap();
    assert!(jit(&repo, &["init"]).status.success());
    let package = synthetic_package_at(repo.root(), "packages/synthetic");
    let actual_parent = outside.path().join("actual");
    let linked_parent = outside.path().join("linked");
    fs::create_dir(&actual_parent).unwrap();
    std::os::unix::fs::symlink(&actual_parent, &linked_parent).unwrap();
    let output = linked_parent.join("synthetic.tar");

    let packed = jit(
        &repo,
        &[
            "profile",
            "pack",
            "--source",
            package,
            "--output",
            output.to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
    );

    assert!(!packed.status.success(), "{packed:?}");
    assert!(
        !actual_parent.join("synthetic.tar").exists(),
        "external rehearsal must not follow a symlinked parent or publish through it"
    );
}

/// A package at the limits the model permits survives the exchange whole.
///
/// The round trip is where a bound that does not hold shows up as an archive
/// `pack` writes and `add` refuses, with the operator left holding a file
/// nothing accepts. The package this uses sits at the file-count bound, fills
/// the byte budget, and gives every source a path too long for a tar header
/// field and a directory of its own — the shape that costs an archive the most
/// per file — so the exchange is exercised where its bounds bind rather than
/// where a small fixture leaves them slack.
#[test]
fn test_profile_pack_and_add_carry_a_package_at_the_model_limits() {
    let source_repo = JitCli::uninitialised().unwrap();
    let target_repo = JitCli::uninitialised().unwrap();
    let exchange = TempDir::new().unwrap();
    assert!(jit(&source_repo, &["init"]).status.success());
    assert!(jit(&target_repo, &["init"]).status.success());
    jit::test_utils::write_package_tree_at_model_limits(
        &source_repo.root().join("packages/bounded"),
    );
    let archive = exchange.path().join("bounded.tar");
    let archive = archive.to_str().unwrap();

    let packed = jit(
        &source_repo,
        &[
            "profile",
            "pack",
            "--source",
            "packages/bounded",
            "--output",
            archive,
            "--json",
        ],
    );
    assert!(packed.status.success(), "{packed:?}");
    let packed = json(&packed);
    let packed = only_profile(&packed);
    assert_eq!(
        packed["file_count"],
        jit::profile::MAX_PROFILE_PACKAGE_FILES,
        "the package under test must sit at the file bound"
    );
    assert!(
        fs::metadata(archive).unwrap().len()
            <= jit::profile::MAX_PROFILE_PACKAGE_ARCHIVE_BYTES as u64,
        "packing a package at the model limits wrote an archive past the bound the read admits"
    );

    let added = jit(
        &target_repo,
        &[
            "profile",
            "add",
            "--archive",
            archive,
            "--destination",
            "packages/bounded",
            "--json",
        ],
    );

    assert!(added.status.success(), "{added:?}");
    let added = json(&added);
    let added = only_profile(&added);
    assert_eq!(added["package_hash"], packed["package_hash"]);
    assert_eq!(added["file_count"], packed["file_count"]);
    assert_eq!(
        worktree_files(&target_repo.root().join("packages/bounded")).len(),
        jit::profile::MAX_PROFILE_PACKAGE_FILES,
        "the published tree does not hold every file the package carried"
    );
}

/// A package at the model limits packs and adds.
///
/// A package the model accepts and `pack` writes an archive for has to be one
/// `add` publishes, or the operator is left holding an archive nothing takes.
/// The package used here sits at the file bound, fills the byte budget, and
/// gives every asset a directory chain no other asset shares, so what a
/// publication enumerates is driven by the package's directories rather than by
/// its files — the shape the package model leaves unconstrained.
#[test]
fn test_profile_add_publishes_a_package_whose_directories_outnumber_its_files() {
    let source_repo = JitCli::uninitialised().unwrap();
    let target_repo = JitCli::uninitialised().unwrap();
    let exchange = TempDir::new().unwrap();
    assert!(jit(&source_repo, &["init"]).status.success());
    assert!(jit(&target_repo, &["init"]).status.success());
    jit::test_utils::write_package_tree_at_model_limits(
        &source_repo.root().join("packages/bounded"),
    );
    let archive = exchange.path().join("bounded.tar");
    let archive = archive.to_str().unwrap();

    let packed = jit(
        &source_repo,
        &[
            "profile",
            "pack",
            "--source",
            "packages/bounded",
            "--output",
            archive,
            "--json",
        ],
    );
    assert!(packed.status.success(), "{packed:?}");
    let packed = json(&packed);
    let packed = only_profile(&packed);

    // The package under test binds every dimension the model constrains, so a
    // publication that survives it has no headroom left in any of them.
    assert_eq!(
        packed["file_count"],
        jit::profile::MAX_PROFILE_PACKAGE_FILES,
        "the package under test must sit at the file bound"
    );
    let byte_size = packed["byte_size"]
        .as_u64()
        .expect("a pack reports the package's byte size");
    assert!(
        byte_size <= jit::profile::MAX_PROFILE_PACKAGE_BYTES as u64
            && byte_size
                > (jit::profile::MAX_PROFILE_PACKAGE_BYTES
                    - jit::profile::MAX_PROFILE_PACKAGE_BYTES / 8) as u64,
        "the package under test must fill the byte budget, not sit well below it: {byte_size}"
    );

    let added = jit(
        &target_repo,
        &[
            "profile",
            "add",
            "--archive",
            archive,
            "--destination",
            "packages/bounded",
            "--json",
        ],
    );
    assert!(added.status.success(), "{added:?}");
    let added = json(&added);
    let added = only_profile(&added);
    assert_eq!(added["package_hash"], packed["package_hash"]);

    // The directories the publication had to enumerate outnumber the files it
    // published, which is the shape the model does not bound.
    let published = target_repo.root().join("packages/bounded");
    assert_eq!(
        worktree_files(&published).len(),
        jit::profile::MAX_PROFILE_PACKAGE_FILES,
        "the published tree does not hold every file the package carried"
    );
    assert!(
        worktree_directories(&published).len() > worktree_files(&published).len(),
        "the package under test must publish more directories than files"
    );
}
