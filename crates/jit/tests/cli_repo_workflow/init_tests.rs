//! Integration tests for `jit init`

use jit::config::DocumentationConfig;
use jit::declarations::assemble_configuration;
use jit::domain::artifact_classifier::contains_path;
use jit::profile::ProfilePackageModel;
use jit::repository_state::{Contribution, ScalarTarget, SetStringTarget};
use jit::storage::worktree_paths::WorktreePaths;
use jit::test_cli::JitCli;
use jit::test_lease_store::history_path;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

/// The package carrying the generic domain vocabulary a repository needs to be
/// usable, including the development-area classification.
const DEFAULT_PACKAGE: &str = "jit-default";

// ---------------------------------------------------------------------------
// Basic init
// ---------------------------------------------------------------------------

#[test]
fn test_init_creates_config_toml_carrying_only_repository_derived_declarations() {
    let cli = JitCli::uninitialised().unwrap();
    let out = cli.command(["init"]).output().unwrap();
    assert!(out.status.success(), "jit init failed: {:?}", out);

    let config = cli.root().join(".jit/config.toml");
    assert!(
        config.exists(),
        ".jit/config.toml should be created by init"
    );

    let content = fs::read_to_string(&config).unwrap();
    let parsed: jit::config::JitConfig =
        toml::from_str(&content).expect("the scaffolded config.toml should parse");
    // What the engine derives from the repository: the schema version, and the
    // project name slugged from the directory it was created in.
    assert!(parsed.version.is_some(), "the schema version is declared");
    assert_eq!(
        parsed
            .project
            .as_ref()
            .and_then(|project| project.name.as_ref())
            .map(|name| name.as_str().to_string()),
        cli.root()
            .file_name()
            .and_then(|name| name.to_str())
            .map(jit::config::slugify_project_name),
        "the project name is the slugged directory name"
    );

    // A vocabulary is a declaration, and this repository made none: no types,
    // no namespaces, no item kinds, no validation defaults, no area
    // classification reaches it until it applies a package that declares them.
    assert!(parsed.type_hierarchy.is_none(), "no type hierarchy");
    assert!(parsed.namespaces.is_none(), "no namespace registry");
    assert!(parsed.item_kinds.is_none(), "no item kinds");
    assert!(parsed.validation.is_none(), "no validation defaults");
    assert!(parsed.documentation.is_none(), "no area classification");
}

#[test]
fn test_init_writes_the_rule_set_the_declared_registry_supports() {
    // REQ-01: the rule set is derived from the repository's own registry, so a
    // repository declaring none receives the label grammar alone — the one rule
    // whose assertion names no namespace and no type — and exactly the
    // projection that rule references.
    let cli = JitCli::uninitialised().unwrap();
    assert!(cli.command(["init"]).output().unwrap().status.success());

    let rules: toml::Value =
        toml::from_str(&fs::read_to_string(cli.root().join(".jit/rules.toml")).unwrap()).unwrap();
    let rules = rules["rules"].as_array().unwrap();
    let references = rules
        .iter()
        .filter_map(|rule| {
            let reference = rule["assert"].get("json-schema")?.as_str()?;
            Some(reference.rsplit('/').next().unwrap().to_string())
        })
        .collect::<BTreeSet<_>>();
    let published = fs::read_dir(cli.root().join(".jit/schemas"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<BTreeSet<_>>();

    assert_eq!(
        rules
            .iter()
            .map(|rule| rule["name"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["label-format"],
    );
    assert_eq!(
        published, references,
        "every published projection is one an emitted rule references"
    );

    // The scaffold is a usable repository even so: it validates, and an issue
    // can be created in it.
    let validate = cli.command(["validate", "--json"]).output().unwrap();
    assert!(
        validate.status.success(),
        "a repository declaring no vocabulary validates: {validate:?}"
    );
    let created = cli
        .command(["issue", "create", "--title", "First", "--json"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "a repository declaring no vocabulary accepts an issue: {created:?}"
    );
}

#[test]
fn test_init_creates_required_files() {
    let cli = JitCli::uninitialised().unwrap();
    let out = cli.command(["init"]).output().unwrap();
    assert!(out.status.success());

    let jit = cli.root().join(".jit");
    assert!(jit.join("index.json").exists(), "index.json missing");
    assert!(jit.join("gates.toml").exists(), "gates.toml missing");
    assert!(jit.join("events.jsonl").exists(), "events.jsonl missing");
    assert!(jit.join("config.toml").exists(), "config.toml missing");
}

#[test]
fn test_init_outside_git_scaffolds_rules_without_creating_dot_git() {
    // `jit init` in a directory that is NOT a git repo must still scaffold the
    // default rules.toml, but it must NOT create a bogus `.git` control plane:
    // the scaffold write lock (.git/jit/locks/rules.lock) is git-only, and the
    // non-git path is intentionally lockless. Successful layout discovery alone
    // does not imply Git authority for the scaffold lock.
    // Hermetic by construction: the control plane (and thus the scaffold lock) is
    // derived from the working tree that owns this `.jit` -- i.e. whether
    // `<temp>/.git` exists -- not from the ambient cwd. A fresh TempDir has no
    // `.git` of its own, so the non-git lockless path is taken even when $TMPDIR
    // itself lives inside an unrelated git worktree (as in some CI sandboxes).
    let cli = JitCli::uninitialised().unwrap();
    let out = cli.command(["init"]).output().unwrap();
    assert!(out.status.success(), "jit init failed: {:?}", out);

    assert!(
        cli.root().join(".jit/rules.toml").exists(),
        "init should scaffold .jit/rules.toml even outside a git repo"
    );
    assert!(
        !cli.root().join(".git").exists(),
        "init outside a git repo must not create a .git directory"
    );
}

// ---------------------------------------------------------------------------
// Idempotency
// ---------------------------------------------------------------------------

#[test]
fn test_init_idempotent_does_not_overwrite_config() {
    let cli = JitCli::uninitialised().unwrap();

    // First init
    let out = cli.command(["init"]).output().unwrap();
    assert!(out.status.success());

    // Mark the scaffolded config with a sentinel, keeping its declarations: a
    // second init must leave the whole authored file alone.
    let config = cli.root().join(".jit/config.toml");
    let authored = format!(
        "# CUSTOM SENTINEL\n{}",
        fs::read_to_string(&config).unwrap()
    );
    fs::write(&config, &authored).unwrap();

    // Second init — should succeed and leave config untouched
    let out = cli.command(["init"]).output().unwrap();
    assert!(out.status.success(), "second jit init failed");

    let content = fs::read_to_string(&config).unwrap();
    assert!(
        content.contains("CUSTOM SENTINEL"),
        "init should not overwrite existing config.toml"
    );
}

#[test]
fn test_init_idempotent_does_not_overwrite_index() {
    let cli = JitCli::uninitialised().unwrap();
    cli.command(["init"]).output().unwrap();

    // Create an issue so the store has real data
    cli.command(["issue", "create", "-t", "My issue"])
        .output()
        .unwrap();

    let index_before = fs::read_to_string(cli.root().join(".jit/index.json")).unwrap();

    // Second init
    let out = cli.command(["init"]).output().unwrap();
    assert!(out.status.success());

    let index_after = fs::read_to_string(cli.root().join(".jit/index.json")).unwrap();
    assert_eq!(
        index_before, index_after,
        "second init should not reset index.json"
    );
}

#[test]
fn test_init_json_reports_repository_id_and_created_paths_outside_git() {
    let cli = JitCli::uninitialised().unwrap();
    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "repository_root": cli.root().to_string_lossy(),
            "data_dir": cli.root().join(".jit").to_string_lossy(),
            "repository_id": null,
            "gitattributes_status": "not_applicable",
            "created_paths": [
                ".jit/index.json",
                ".jit/gates.toml",
                ".jit/invariants.toml",
                ".jit/events.jsonl",
                ".jit/config.toml",
                ".jit/rules.toml"
            ],
            "modified_paths": [],
            "profile": null,
            "message": "Initialized jit repository"
        })
    );
}

#[test]
fn test_init_json_reports_repository_id_inside_git() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let repository_id = json["repository_id"]
        .as_str()
        .expect("repository_id should be a string inside a git repository");
    assert!(
        repository_id.starts_with("wt:"),
        "repository_id should be a worktree id, got: {repository_id}"
    );
}

#[test]
fn test_init_json_inside_git_reports_gitattributes_created() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "created");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        created.contains(&".gitattributes"),
        "created_paths should report .gitattributes on a fresh init inside git, got: {created:?}"
    );
    let modified = json["modified_paths"]
        .as_array()
        .expect("modified_paths should be an array");
    assert!(
        modified.is_empty(),
        "a fresh .gitattributes is created, not modified, got: {modified:?}"
    );

    let content = fs::read_to_string(cli.root().join(".gitattributes")).unwrap();
    assert!(content.contains("# JIT merge drivers"));
}

#[test]
fn test_init_nested_glob_data_dir_claims_only_literal_events_path() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    let data_dir = r#""lead space/glob*query?[set]"#;
    fs::create_dir(cli.root().join(r#""lead space"#)).unwrap();
    let out = cli
        .command_with_env(["init"], [("JIT_DATA_DIR", data_dir)])
        .output()
        .unwrap();
    assert!(out.status.success(), "jit init failed: {out:?}");

    let attributes = fs::read_to_string(cli.root().join(".gitattributes")).unwrap();
    assert!(
        attributes.contains(
            "\"\\\"lead space/glob\\\\*query\\\\?\\\\[set\\\\]/events.jsonl\" merge=union"
        ),
        "the literal nested data root must be Git-escaped: {attributes:?}"
    );

    let literal = Command::new("git")
        .args(["check-attr", "merge", "--"])
        .arg(format!("{data_dir}/events.jsonl"))
        .current_dir(cli.root())
        .output()
        .unwrap();
    assert!(literal.status.success());
    assert!(String::from_utf8(literal.stdout)
        .unwrap()
        .ends_with("merge: union\n"));

    let wildcard_neighbor = Command::new("git")
        .args(["check-attr", "merge", "--"])
        .arg(r#""lead space/glob-neighborqueryXs/events.jsonl"#)
        .current_dir(cli.root())
        .output()
        .unwrap();
    assert!(wildcard_neighbor.status.success());
    assert!(String::from_utf8(wildcard_neighbor.stdout)
        .unwrap()
        .ends_with("merge: unspecified\n"));
}

// The claim log lives under `.git/jit/`, outside the versioned `.jit/` paths
// covered by `.gitattributes`.
#[test]
fn test_init_json_gitattributes_has_no_stale_claims_jsonl_entry() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let content = fs::read_to_string(cli.root().join(".gitattributes")).unwrap();
    let paths = WorktreePaths::detect_for_data_root(&cli.root().join(".jit"), cli.root()).unwrap();
    let history = history_path(&paths.shared_jit);
    assert!(
        !content.contains(history.file_name().unwrap().to_str().unwrap()),
        "claim coordination writes its log under .git/jit/, not .jit/, so \
         .gitattributes should have no merge-driver entry for the claims log, got: {content:?}"
    );
    // Positive control: the events.jsonl entry stays — events.jsonl is
    // actually persisted under the versioned .jit/ data plane.
    assert!(
        content.contains(".jit/events.jsonl merge=union"),
        "the events.jsonl merge-driver entry should still be present, got: {content:?}"
    );
}

// Re-init reports an existing merge-driver claim as unchanged and excludes
// `.gitattributes` from both created and modified paths.
#[test]
fn test_init_json_reinit_reports_gitattributes_absent_from_created_paths() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    let first = cli.command(["init", "--json"]).output().unwrap();
    assert!(first.status.success());

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success());

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "unchanged");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let modified: Vec<&str> = json["modified_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !created.contains(&".gitattributes"),
        "an already-configured .gitattributes should not be reported as created, got: {created:?}"
    );
    assert!(
        !modified.contains(&".gitattributes"),
        "an already-configured .gitattributes should not be reported as modified, got: {modified:?}"
    );
}

// Plain re-init restores a deleted `.gitattributes` merge-driver claim.
#[test]
fn test_init_json_reinit_reclaims_deleted_gitattributes() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    let first = cli.command(["init", "--json"]).output().unwrap();
    assert!(first.status.success());
    let gitattributes = cli.root().join(".gitattributes");
    assert!(gitattributes.is_file(), "fresh init creates .gitattributes");
    fs::remove_file(&gitattributes).unwrap();

    // A plain re-init (no template, no profile) over the existing root.
    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "re-init failed: {out:?}");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "created");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        created.contains(&".gitattributes"),
        "a plain re-init must re-create the deleted .gitattributes, got: {created:?}"
    );
    let content = fs::read_to_string(&gitattributes).unwrap();
    assert!(
        content.contains(".jit/events.jsonl merge=union"),
        "the re-created .gitattributes carries the events.jsonl merge driver, got: {content:?}"
    );
}

#[test]
fn test_init_json_appends_to_existing_gitattributes_reports_modified() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();

    // Pre-existing .gitattributes without the jit merge-driver block.
    fs::write(cli.root().join(".gitattributes"), "*.txt text\n").unwrap();

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "jit init --json failed: {:?}", out);

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["gitattributes_status"], "modified");
    let created: Vec<&str> = json["created_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let modified: Vec<&str> = json["modified_paths"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(
        !created.contains(&".gitattributes"),
        "an appended .gitattributes should not be reported as created, got: {created:?}"
    );
    assert!(
        modified.contains(&".gitattributes"),
        "an appended .gitattributes should be reported as modified, got: {modified:?}"
    );

    let content = fs::read_to_string(cli.root().join(".gitattributes")).unwrap();
    assert!(content.contains("*.txt text"), "original content preserved");
    assert!(
        content.contains("# JIT merge drivers"),
        "jit block appended"
    );

    // Atomic-write regression: no stray temp sibling left behind.
    let stray: Vec<_> = fs::read_dir(cli.root())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".gitattributes.") && name.ends_with(".tmp"))
        .collect();
    assert!(
        stray.is_empty(),
        "no .gitattributes.*.tmp sibling should remain, got: {stray:?}"
    );
}

#[test]
fn test_init_json_idempotent_reports_empty_created_paths() {
    let cli = JitCli::uninitialised().unwrap();
    let first = cli.command(["init", "--json"]).output().unwrap();
    assert!(first.status.success());

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(
        out.status.success(),
        "second jit init --json failed: {:?}",
        out
    );

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["created_paths"], serde_json::json!([]));
    assert_eq!(json["modified_paths"], serde_json::json!([]));
}

#[test]
fn test_init_json_disjoint_data_root_reports_canonical_plan_paths() {
    let cli = JitCli::uninitialised().unwrap();
    let external = TempDir::new().unwrap();
    let data_dir = external.path().join("repository-data");
    cli.init_git().unwrap();

    let out = cli
        .command_with_env(["init", "--json"], [("JIT_DATA_DIR", &data_dir)])
        .output()
        .unwrap();
    assert!(out.status.success(), "disjoint init failed: {out:?}");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json["data_dir"], data_dir.to_string_lossy().as_ref());
    assert_eq!(json["gitattributes_status"], "not_applicable");
    assert_eq!(
        json["created_paths"],
        serde_json::json!([
            ".jit/index.json",
            ".jit/gates.toml",
            ".jit/invariants.toml",
            ".jit/events.jsonl",
            ".jit/config.toml",
            ".jit/rules.toml"
        ])
    );
    assert!(!cli.root().join(".gitattributes").exists());
    assert!(json["modified_paths"].as_array().unwrap().is_empty());
}

#[test]
fn test_init_json_reinit_reports_only_file_restored_by_plan() {
    let cli = JitCli::uninitialised().unwrap();
    let first = cli.command(["init", "--json"]).output().unwrap();
    assert!(first.status.success());
    fs::remove_file(cli.root().join(".jit/gates.toml")).unwrap();

    let out = cli.command(["init", "--json"]).output().unwrap();
    assert!(out.status.success(), "re-init failed: {out:?}");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        json["created_paths"],
        serde_json::json!([".jit/gates.toml"])
    );
    assert_eq!(json["modified_paths"], serde_json::json!([]));
}

// ---------------------------------------------------------------------------
// Generated config is valid TOML
// ---------------------------------------------------------------------------

#[test]
fn test_init_config_is_valid_toml() {
    let cli = JitCli::uninitialised().unwrap();
    cli.command(["init"]).output().unwrap();

    let content = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
    // toml crate can parse it — use the library directly
    let parsed: Result<toml::Value, _> = toml::from_str(&content);
    assert!(
        parsed.is_ok(),
        "generated config.toml is not valid TOML: {:?}",
        parsed.err()
    );
}

// ---------------------------------------------------------------------------
// The development-area classification the default package declares
// ---------------------------------------------------------------------------

/// The `[documentation]` policy a repository carries after applying the default
/// package.
///
/// Read through the configuration assembler, so a commented-out block resolves to
/// no policy at all rather than to a table this helper could inspect.
fn packaged_documentation_policy(dir: &Path) -> DocumentationConfig {
    let bytes = fs::read(dir.join(".jit/config.toml")).expect("init should write config.toml");
    assemble_configuration(Some(&bytes), None, None)
        .expect("the written config.toml should assemble")
        .documentation
        .expect("applying the default package should author a [documentation] policy")
}

/// Every area the applied policy classifies, managed and permanent alike.
fn classified_areas(policy: &DocumentationConfig) -> Vec<String> {
    policy
        .managed_paths
        .iter()
        .chain(policy.permanent_paths.iter())
        .flatten()
        .cloned()
        .collect()
}

#[test]
fn test_init_with_the_default_package_classifies_every_development_area() {
    let cli = JitCli::uninitialised().unwrap();
    let package = jit::test_utils::stage_repository_packages(cli.root(), DEFAULT_PACKAGE);
    let out = cli
        .command(["init", "--profile"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(out.status.success(), "profiled init failed: {out:?}");

    let policy = packaged_documentation_policy(cli.root());
    let development_root = policy.development_root();
    let managed = policy
        .managed_paths
        .clone()
        .expect("the applied policy should author its managed areas");
    let permanent = policy
        .permanent_paths
        .clone()
        .expect("the applied policy should author its permanent areas");
    assert!(
        policy.archive_root.is_some(),
        "the applied policy should author its archive root"
    );

    // Each classified entry is a distinct area beneath the development root. A
    // bare root entry would match every other area by prefix.
    for area in classified_areas(&policy) {
        assert!(
            contains_path(&development_root, &area) && area != development_root,
            "{area} should be a distinct area under {development_root}"
        );
    }

    // The two areas the criterion names, observed through the matcher the
    // classifier itself applies.
    let deck = format!("{development_root}/presentations/showcase.md");
    let architecture_note = format!("{development_root}/architecture/overview.md");
    let matches = |areas: &[String], path: &str| areas.iter().any(|area| contains_path(area, path));
    assert!(
        matches(&managed, &deck) && !matches(&permanent, &deck),
        "the presentation area should be managed, not permanent"
    );
    assert!(
        matches(&permanent, &architecture_note) && !matches(&managed, &architecture_note),
        "the architecture area should be permanent, not managed"
    );
}

#[test]
fn test_init_documentation_block_is_active_configuration_naming_only_typed_keys() {
    let cli = JitCli::uninitialised().unwrap();
    let package = jit::test_utils::stage_repository_packages(cli.root(), DEFAULT_PACKAGE);
    let out = cli
        .command(["init", "--profile"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(out.status.success(), "profiled init failed: {out:?}");

    let content = fs::read_to_string(cli.root().join(".jit/config.toml")).unwrap();
    let scaffolded: toml::Table = content.parse().expect("config.toml should be valid TOML");
    let emitted = scaffolded
        .get("documentation")
        .and_then(toml::Value::as_table)
        .expect("the documentation block should be active configuration, not commented guidance");

    // Every key the type carries, derived from the type rather than restated.
    let typed = toml::Value::try_from(DocumentationConfig {
        development_root: Some(String::new()),
        managed_paths: Some(Vec::new()),
        archive_root: Some(String::new()),
        permanent_paths: Some(Vec::new()),
        issue_scoped_areas: Some(Vec::new()),
        citation_scan_roots: Some(Vec::new()),
    })
    .unwrap();
    let carried = typed
        .as_table()
        .unwrap()
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    for key in emitted.keys() {
        assert!(
            carried.contains(key),
            "[documentation].{key} is not a key DocumentationConfig carries"
        );
    }
}

#[test]
fn test_init_with_the_default_package_classifies_root_files_as_exact_path_entries() {
    let cli = JitCli::uninitialised().unwrap();
    let package = jit::test_utils::stage_repository_packages(cli.root(), DEFAULT_PACKAGE);
    let out = cli
        .command(["init", "--profile"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(out.status.success(), "profiled init failed: {out:?}");

    let policy = packaged_documentation_policy(cli.root());
    let development_root = policy.development_root();
    let areas = classified_areas(&policy);
    let root_files = areas
        .iter()
        .filter(|area| Path::new(area).parent() == Some(Path::new(&development_root)))
        .filter(|area| Path::new(area).extension().is_some())
        .collect::<Vec<_>>();
    assert!(
        !root_files.is_empty(),
        "development-root files should be classified individually"
    );

    // An exact entry classifies its own file and nothing else: no entry reaches
    // it by prefix.
    for file in root_files {
        let matching = areas
            .iter()
            .filter(|area| contains_path(area, file))
            .collect::<Vec<_>>();
        assert_eq!(
            matching,
            vec![file],
            "{file} should be classified by its own exact entry alone"
        );
    }

    // A development-root file that belongs to no area is classified by nothing,
    // which a prefix entry over the root would contradict.
    let unclassified = format!("{development_root}/unclassified-note.md");
    assert!(
        !areas.iter().any(|area| contains_path(area, &unclassified)),
        "no entry should sweep in development-root files by prefix"
    );
}

#[test]
fn test_init_documentation_policy_is_the_default_package_declaration() {
    // REQ-06: the classification a repository receives is the one the default
    // package declares, compared against that manifest rather than against a
    // second copy of the lists.
    let cli = JitCli::uninitialised().unwrap();
    let package = jit::test_utils::stage_repository_packages(cli.root(), DEFAULT_PACKAGE);
    let out = cli
        .command(["init", "--profile"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(out.status.success(), "profiled init failed: {out:?}");

    let policy = packaged_documentation_policy(cli.root());
    let (_workspace, package) = jit::test_utils::temporary_repository_package(DEFAULT_PACKAGE);
    let manifest = package.model();

    assert_eq!(
        policy.development_root(),
        declared_scalar(manifest, ScalarTarget::DocumentationDevelopmentRoot),
    );
    assert_eq!(
        policy.archive_root(),
        declared_scalar(manifest, ScalarTarget::DocumentationArchiveRoot),
    );
    assert_eq!(
        policy.managed_paths(),
        declared_set(manifest, SetStringTarget::DocumentationManagedPaths),
    );
    assert_eq!(
        policy.permanent_paths(),
        declared_set(manifest, SetStringTarget::DocumentationPermanentPaths),
    );
    assert_eq!(
        policy.issue_scoped_areas(),
        declared_set(manifest, SetStringTarget::DocumentationIssueScopedAreas),
    );
}

/// The one scalar value a manifest contributes to `target`.
fn declared_scalar(manifest: &ProfilePackageModel, target: ScalarTarget) -> String {
    manifest
        .contributions
        .iter()
        .find_map(|contribution| match contribution {
            Contribution::Scalar {
                target: declared,
                value,
            } if declared == &target => Some(value.clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("the package declares {target:?}"))
}

/// Every value a manifest contributes to `target`, in declaration order.
fn declared_set(manifest: &ProfilePackageModel, target: SetStringTarget) -> Vec<String> {
    let declared = manifest
        .contributions
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::SetString {
                target: declared,
                value,
            } if declared == &target => Some(value.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!declared.is_empty(), "the package declares {target:?}");
    declared
}

#[test]
fn test_init_with_the_default_package_authors_an_issue_scoped_registry_the_query_accepts() {
    let cli = JitCli::uninitialised().unwrap();
    let package = jit::test_utils::stage_repository_packages(cli.root(), DEFAULT_PACKAGE);
    let out = cli
        .command(["init", "--profile"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(out.status.success(), "profiled init failed: {out:?}");

    let policy = packaged_documentation_policy(cli.root());
    // Active configuration, not commented-out guidance: the application writes
    // the registry into the file, so the accessor reads it rather than answering
    // from an absent key.
    let registry = policy
        .issue_scoped_areas
        .clone()
        .expect("the applied policy should author its issue-scoped area registry");
    assert!(!registry.is_empty());

    // Every declared entry is a distinct area beneath the development root, and
    // the membership query accepts exactly what the package declared.
    let development_root = policy.development_root();
    for area in &registry {
        assert!(
            contains_path(&development_root, area) && *area != development_root,
            "{area} should be a distinct area under {development_root}"
        );
        assert!(
            policy.is_issue_scoped_area(area),
            "the membership query should accept the declared area {area}"
        );
    }

    // An area the registry does not declare is rejected, so a caller naming one
    // can be refused rather than silently handed a path.
    assert!(!policy.is_issue_scoped_area(&format!("{development_root}/undeclared-area")));
}

#[test]
fn test_init_with_the_default_package_plans_archival_under_a_configured_policy() {
    let cli = JitCli::uninitialised().unwrap();
    let package = jit::test_utils::stage_repository_packages(cli.root(), DEFAULT_PACKAGE);
    let out = cli
        .command(["init", "--profile"])
        .arg(&package)
        .output()
        .unwrap();
    assert!(out.status.success(), "profiled init failed: {out:?}");

    let policy = packaged_documentation_policy(cli.root());
    let area = policy
        .managed_paths()
        .into_iter()
        .next()
        .expect("the applied policy should classify at least one managed area");
    fs::create_dir_all(cli.root().join(&area)).unwrap();
    let document = format!("{area}/note.md");
    fs::write(cli.root().join(&document), "note").unwrap();

    let out = cli
        .command(["archive", "document", &document, "--json"])
        .output()
        .expect("failed to run jit archive document");
    assert!(out.status.success(), "archive preview failed: {out:?}");

    let plan: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(plan["policy_status"], "configured");
    assert!(
        !plan["blockers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|blocker| blocker["code"]
                .as_str()
                .is_some_and(|code| code.starts_with("policy-"))),
        "a configured policy should raise no policy blocker: {plan}"
    );
}

/// Init's ignore rules hide scratch content left by a mutation, including an
/// explicitly populated commit point.
#[test]
fn test_init_inside_git_leaves_no_untracked_scratch_content_after_a_mutation() {
    let cli = JitCli::uninitialised().unwrap();
    cli.init_git().unwrap();
    assert!(cli.command(["init"]).output().unwrap().status.success());

    let created = cli
        .command(["issue", "create", "A mutation"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "jit issue create failed: {created:?}"
    );

    let scratch = format!(".jit/{}/", jit::repository_state::RECORD_SCRATCH_DIR);
    let untracked_scratch = |label: &str| {
        let status = Command::new("git")
            .args(["status", "--porcelain", "-uall"])
            .current_dir(cli.root())
            .output()
            .unwrap();
        let rendered = String::from_utf8(status.stdout).unwrap();
        assert!(
            !rendered.contains(&scratch),
            "{label}: version-control status names scratch content under {scratch}: {rendered}"
        );
    };
    untracked_scratch("after a mutation");

    fs::create_dir_all(
        cli.root()
            .join(".jit")
            .join(jit::repository_state::RECORD_SCRATCH_DIR),
    )
    .unwrap();
    fs::write(
        cli.root()
            .join(".jit")
            .join(jit::repository_state::RECORD_SCRATCH_DIR)
            .join("redo.record"),
        b"a commit point",
    )
    .unwrap();
    untracked_scratch("with a commit point under the scratch directory");
}
