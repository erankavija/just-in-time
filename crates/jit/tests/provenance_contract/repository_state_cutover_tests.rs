//! Current-tree structural guards for the direct repository-state cutover.

use std::path::{Path, PathBuf};

const TEST_MODULE_MARKER: &str = "\n#[cfg(test)]\nmod ";

fn production_source(path: &Path) -> String {
    let source = std::fs::read_to_string(path).unwrap();
    source
        .split_once(TEST_MODULE_MARKER)
        .map_or(source.as_str(), |(production, _)| production)
        .to_string()
}

fn rust_sources(root: &Path) -> Vec<(PathBuf, String)> {
    fn visit(path: &Path, sources: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push((path.clone(), production_source(&path)));
            }
        }
    }

    let mut sources = Vec::new();
    visit(root, &mut sources);
    sources.sort_by(|left, right| left.0.cmp(&right.0));
    sources
}

fn has_cargo_manifest(root: &Path) -> bool {
    std::fs::read_dir(root).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name().is_some_and(|name| name == "Cargo.toml")
            || path.is_dir() && has_cargo_manifest(&path)
    })
}

fn definition_owners(
    source_root: &Path,
    sources: &[(PathBuf, String)],
    definition: &str,
) -> Vec<String> {
    sources
        .iter()
        .filter(|(_, source)| source.contains(definition))
        .map(|(path, _)| {
            path.strip_prefix(source_root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect()
}

#[test]
fn test_cutover_consumers_publish_only_through_repository_state_sessions() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    // These are the four repository-materialization command consumers named by
    // the approved cutover plan. Test-only fixture writes are deliberately outside
    // the production slice inspected here.
    for relative in [
        "commands/init.rs",
        "commands/profile.rs",
        "commands/project.rs",
        "commands/validate.rs",
    ] {
        let source = production_source(&source_root.join(relative));
        for required in ["open_mutation_session", "session.apply("] {
            assert!(
                source.contains(required),
                "{relative} no longer publishes through the canonical session API: {required}"
            );
        }

        // D4/D15/D17 remove command-local filesystem/kernel publication and the
        // old per-record IssueStore publishers. External export commands are not
        // in this inventory because the plan explicitly keeps those outside the
        // repository-state publication boundary.
        for forbidden in [
            "std::fs::write(",
            "fs::write(",
            "std::fs::rename(",
            "fs::rename(",
            "File::create(",
            "OpenOptions::new(",
            "FileTransactionKernel",
            "atomic_write",
            ".save_issue(",
            ".restore_issue_verbatim(",
            ".append_event(",
            ".save_gate_run_result(",
            ".save_gate_registry(",
            ".save_gate_preset(",
            ".write_repo_file(",
        ] {
            assert!(
                !source.contains(forbidden),
                "command-local repository publisher returned in {relative}: {forbidden}"
            );
        }
    }
}

#[test]
fn test_cutover_has_only_final_modules_and_no_compatibility_seams() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");

    // Exact predecessor modules named by plan D2 and its deletion inventories.
    for relative in [
        "profile/render.rs",
        "profile/snapshot.rs",
        "profile/drift.rs",
        "profile/preset.rs",
        "profile/planner.rs",
        "storage/config_store.rs",
        "validation/rules.rs",
        "validation/defaults.rs",
        "validation/serialize.rs",
        "validation/project_render.rs",
        "validation/projection.rs",
    ] {
        assert!(
            !source_root.join(relative).exists(),
            "superseded intermediate module returned: {relative}"
        );
    }

    let library = production_source(&source_root.join("lib.rs"));
    for final_module in ["pub mod declarations;", "pub mod repository_state;"] {
        assert!(
            library.contains(final_module),
            "final crate-root owner is no longer direct: {final_module}"
        );
    }
    for compatibility in [
        "pub type Storage =",
        "pub type Gate =",
        "pub use crate::repository_state",
        "pub use crate::declarations",
    ] {
        assert!(
            !library.contains(compatibility),
            "crate-root compatibility seam returned: {compatibility}"
        );
    }
    for facade in ["profile/mod.rs", "storage/mod.rs", "validation/mod.rs"] {
        let source = production_source(&source_root.join(facade));
        for compatibility in [
            "pub use crate::repository_state",
            "pub use crate::declarations",
        ] {
            assert!(
                !source.contains(compatibility),
                "compatibility re-export returned in {facade}: {compatibility}"
            );
        }
    }

    // The cumulative implementation packages in D10/D16 are delivery ordering,
    // not separately adoptable architecture. The final engine is one crate-root
    // module with private implementation modules and no nested Cargo package.
    let repository_state_root = source_root.join("repository_state");
    assert!(!has_cargo_manifest(&repository_state_root));
    let repository_state_module = production_source(&repository_state_root.join("mod.rs"));
    assert!(
        !repository_state_module
            .lines()
            .any(|line| line.trim_start().starts_with("pub mod ")),
        "repository_state gained an independently exposed intermediate module"
    );

    // These plan-deleted APIs are checked as definitions in individual
    // production storage files, not as one joined-source substring inventory.
    // CommandExecutor::delete_issue remains the semantic command; only the old
    // storage publisher was deleted, so the owner boundary matters here.
    let storage_sources = rust_sources(&source_root.join("storage"));
    for removed_api in [
        "fn save_issue(",
        "fn restore_issue_verbatim(",
        "fn delete_issue(",
        "fn append_event(",
        "fn save_gate_run_result(",
        "fn save_gate_registry(",
        "fn save_gate_preset(",
        "fn write_repo_file(",
    ] {
        assert!(
            definition_owners(&source_root, &storage_sources, removed_api).is_empty(),
            "removed storage publisher wrapper returned: {removed_api}"
        );
    }

    let profile_sources = rust_sources(&source_root.join("profile"));
    for removed_api in [
        "fn project_package(",
        "fn capture_profile_snapshot(",
        "fn plan_profile_application_against(",
    ] {
        assert!(
            definition_owners(&source_root, &profile_sources, removed_api).is_empty(),
            "removed profile compatibility wrapper returned: {removed_api}"
        );
    }
}

#[test]
fn test_materialization_renderers_have_one_canonical_owner() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let sources = rust_sources(&source_root);
    for (definition, owner) in [
        (
            "fn render_managed_document(",
            "repository_state/managed_document.rs",
        ),
        (
            "fn render_projection_body(",
            "repository_state/projection_render.rs",
        ),
        (
            "fn render_invariants_markdown(",
            "repository_state/projection.rs",
        ),
        (
            "fn render_rules_and_gates_markdown(",
            "repository_state/rules_gates_projection.rs",
        ),
        (
            "fn serialize_ruleset(",
            "repository_state/rule_serialize.rs",
        ),
        ("fn render_repo_config(", "repository_state/initialize.rs"),
    ] {
        assert_eq!(
            definition_owners(&source_root, &sources, definition),
            [owner],
            "materialization renderer ownership drifted: {definition}"
        );
    }
}
