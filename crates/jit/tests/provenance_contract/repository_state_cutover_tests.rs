//! Current-tree guards for removed repository-state predecessors.

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
fn test_repository_state_cutover_has_no_known_predecessor_files_or_storage_alias() {
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

    let library = std::fs::read_to_string(source_root.join("lib.rs")).unwrap();
    assert!(
        !library.contains("pub type Storage ="),
        "superseded Storage compatibility alias returned"
    );
}

#[test]
fn test_projection_renderers_have_one_canonical_owner() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let sources = rust_sources(&source_root);
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
