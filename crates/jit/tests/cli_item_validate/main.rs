//! `cli_item_validate` integration-test suite. Aggregates former per-file
//! integration targets as modules under one Cargo test target so Cargo links
//! and runs them as a single executable (jit:8d4f7084).

mod decision_kind_tests;
mod decision_risk_story_tests;
mod derived_state_repair_tests;
mod invariant_check_cli_tests;
mod invariant_registry_story_tests;
mod item_cli_tests;
mod markdown_kind_source_path_tests;
mod project_render_cli_tests;
mod registry_json_tests;
mod risk_kind_tests;
mod type_hierarchy_fix_tests;
mod validate_cli_rule_tests;
mod validate_document_tests;
mod validate_drift_builtin_tests;
mod validation_lease_tests;

fn create_issue(
    executor: &jit::commands::CommandExecutor<jit::storage::JsonFileStorage>,
    title: &str,
    body: &str,
) -> String {
    use jit::storage::IssueStore;

    let id = executor
        .create_issue(
            title.to_string(),
            body.to_string(),
            jit::domain::Priority::Normal,
            vec![],
            vec![],
            None,
            None,
            false,
        )
        .unwrap()
        .0;
    executor.storage().load_issue(&id).unwrap().short_id()
}

/// The package carrying the generic vocabulary these suites address: the type
/// hierarchy, the label namespaces, and the addressable item kinds an item is
/// resolved against.
pub(crate) const DEFAULT_PACKAGE: &str = "jit-default";

/// Initialize `dir` through the CLI with that package applied.
///
/// A repository declares its own vocabulary, so a suite that resolves
/// `@/<kind>/<id>` obtains the kinds by applying a package that declares them.
/// The package is assembled into the repository it is applied to, because an
/// application records its worktree-relative location.
pub(crate) fn initialize_with_default_vocabulary(dir: &std::path::Path) {
    let location = format!("packages/{DEFAULT_PACKAGE}");
    jit::test_utils::assemble_repository_package(DEFAULT_PACKAGE, &dir.join(&location))
        .expect("this repository's default package assembles");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_jit"))
        .args(["init", "--profile", DEFAULT_PACKAGE, "--from", &location])
        .current_dir(dir)
        .output()
        .expect("failed to run jit init");
    assert!(
        output.status.success(),
        "jit init --profile {DEFAULT_PACKAGE} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A fresh repository carrying the default package's vocabulary.
pub(crate) fn setup_repo_with_default_vocabulary() -> tempfile::TempDir {
    let temp = tempfile::TempDir::new().unwrap();
    initialize_with_default_vocabulary(temp.path());
    temp
}
