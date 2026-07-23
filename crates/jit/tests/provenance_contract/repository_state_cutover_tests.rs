//! Structural release guard for the direct repository-state cutover.

use std::path::{Path, PathBuf};

fn rust_sources(root: &Path) -> Vec<(PathBuf, String)> {
    fn visit(path: &Path, sources: &mut Vec<(PathBuf, String)>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, sources);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                sources.push((path.clone(), std::fs::read_to_string(path).unwrap()));
            }
        }
    }

    let mut sources = Vec::new();
    visit(root, &mut sources);
    sources
}

#[test]
fn test_repository_state_cutover_has_no_predecessor_or_compatibility_seam() {
    let crate_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_root = crate_root.join("src");

    for relative in [
        "profile/render.rs",
        "profile/snapshot.rs",
        "profile/drift.rs",
        "profile/preset.rs",
        "storage/config_store.rs",
    ] {
        assert!(
            !source_root.join(relative).exists(),
            "superseded source returned: {relative}"
        );
    }

    let sources = rust_sources(&source_root);
    let joined = sources
        .iter()
        .map(|(_, source)| source.as_str())
        .collect::<Vec<_>>()
        .join("\n");

    for predecessor in [
        "fn save_issue(",
        "fn restore_issue_verbatim(",
        "fn append_event(",
        "fn save_gate_run_result(",
        "fn save_gate_registry(",
        "fn save_gate_preset(",
        "fn write_repo_file(",
        "struct RuleMembershipSync",
        "struct PackageProjection",
        "struct RepositorySnapshot",
        "struct PresetInventory",
        "struct ProfileApplicationPlan",
        "fn project_package(",
        "fn capture_profile_snapshot(",
        "fn plan_profile_application_against(",
        "pub use crate::repository_state",
        "pub type Storage =",
    ] {
        assert!(
            !joined.contains(predecessor),
            "superseded publisher, inventory, or compatibility seam returned: {predecessor}"
        );
    }

    for (definition, owner) in [
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
    ] {
        let owners = sources
            .iter()
            .filter(|(_, source)| source.contains(definition))
            .map(|(path, _)| {
                path.strip_prefix(&source_root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect::<Vec<_>>();
        assert_eq!(owners, [owner], "renderer ownership drifted: {definition}");
    }
}
