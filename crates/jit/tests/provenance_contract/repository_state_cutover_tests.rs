//! Known current-tree regressions forbidden by the repository-state cutover.

use std::path::{Path, PathBuf};

fn production_source(path: &Path) -> String {
    let source = std::fs::read_to_string(path).unwrap();
    source
        .split_once("\n#[cfg(test)]\nmod ")
        .map_or(source.as_str(), |(production, _)| production)
        .to_string()
}

fn definition_owners(root: &Path, definition: &str) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut owners = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs")
                && production_source(&path).contains(definition)
            {
                owners.push(path.strip_prefix(root).unwrap().to_path_buf());
            }
        }
    }
    owners
}

#[test]
fn test_plan_named_cutover_consumers_do_not_call_known_legacy_publishers() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for consumer in ["init.rs", "profile.rs", "project.rs", "validate.rs"] {
        let source = production_source(&source_root.join("commands").join(consumer));
        for publisher in [
            "save_issue(",
            "restore_issue_verbatim(",
            "append_event(",
            "save_gate_run_result(",
            "save_gate_registry(",
            "save_gate_preset(",
            "write_repo_file(",
        ] {
            assert!(
                !source.contains(publisher),
                "known legacy publisher returned in commands/{consumer}: {publisher}"
            );
        }
    }
}

#[test]
fn test_cutover_has_no_known_predecessor_or_compatibility_module() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");
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
        assert!(!source_root.join(relative).exists(), "returned: {relative}");
    }

    let library = production_source(&source_root.join("lib.rs"));
    assert!(library.contains("pub mod repository_state;"));
    for alias in [
        "pub type Storage =",
        "pub type Gate =",
        "pub use crate::repository_state",
    ] {
        assert!(!library.contains(alias), "alias returned: {alias}");
    }

    let workspace = std::fs::read_to_string(crate_root.join("../../Cargo.toml")).unwrap();
    for package in ["crates/repository-state", "crates/declarations"] {
        assert!(
            !workspace.contains(package),
            "intermediate package: {package}"
        );
    }
    for module in ["repository_state", "declarations"] {
        assert!(!source_root.join(module).join("Cargo.toml").exists());
    }

    for (scope, wrapper) in [
        ("storage", "fn save_issue("),
        ("storage", "fn restore_issue_verbatim("),
        ("storage", "fn delete_issue("),
        ("storage", "fn append_event("),
        ("storage", "fn save_gate_run_result("),
        ("storage", "fn save_gate_registry("),
        ("storage", "fn save_gate_preset("),
        ("storage", "fn write_repo_file("),
        ("profile", "fn project_package("),
        ("profile", "fn capture_profile_snapshot("),
        ("profile", "fn plan_profile_application_against("),
    ] {
        assert!(
            definition_owners(&source_root.join(scope), wrapper).is_empty(),
            "plan-deleted wrapper returned in {scope}: {wrapper}"
        );
    }
}

#[test]
fn test_known_projection_renderer_definitions_keep_canonical_owners() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for (definition, file) in [
        ("fn render_projection_body(", "projection_render.rs"),
        ("fn render_invariants_markdown(", "projection.rs"),
        (
            "fn render_rules_and_gates_markdown(",
            "rules_gates_projection.rs",
        ),
    ] {
        assert_eq!(
            definition_owners(&source_root, definition),
            [PathBuf::from("repository_state").join(file)],
            "known renderer ownership changed: {definition}"
        );
    }
}
