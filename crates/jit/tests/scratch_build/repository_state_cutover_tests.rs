//! Known current-tree regressions forbidden by the repository-state cutover.
//!
//! Publisher access itself is compiler-enforced rather than asserted here: the
//! in-repository writers (`storage::atomic_write`) and the transaction kernel
//! (`storage::file_transaction`) are `pub(in crate::storage)`, so `crate::commands`
//! cannot name them and a regression fails the build. This file covers the
//! properties visibility cannot express — deleted modules and wrappers,
//! canonical renderer ownership, and the modules that must reach the filesystem
//! only through a named discipline or not at all — because `std::fs` is public
//! to every crate and no `pub(in ...)` restriction reaches it. Each such test
//! carries a guard asserting the mechanism it is about is still present, so an
//! absent call shape can only mean the discipline held rather than that the
//! code it was about moved away.

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

fn contains_cargo_manifest(root: &Path) -> bool {
    std::fs::read_dir(root).unwrap().any(|entry| {
        let path = entry.unwrap().path();
        path.file_name().is_some_and(|name| name == "Cargo.toml")
            || path.is_dir() && contains_cargo_manifest(&path)
    })
}

/// Raw `std::fs` publication is the one residual channel into a repository root
/// that visibility cannot close: `std::fs` is public to every crate, so no
/// `pub(in ...)` restriction reaches it. This is therefore a deliberate,
/// recorded source-text exception to the compiler-enforced mechanism, and it is
/// kept as narrow as the exception requires — the four cutover command modules,
/// the four call shapes that publish. A crate-wide `clippy.toml`
/// `disallowed_methods` entry is the wrong instrument: `commands/snapshot.rs`
/// legitimately stages exports with raw `std::fs` into temp directories outside
/// the repository roots, which such a rule would forbid.
#[test]
fn test_cutover_command_modules_publish_no_raw_filesystem_writes() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for consumer in ["init.rs", "profile.rs", "project.rs", "validate.rs"] {
        let source = production_source(&source_root.join("commands").join(consumer));
        for publisher in [
            "fs::write(",
            "fs::rename(",
            "File::create(",
            "OpenOptions::new(",
        ] {
            assert!(
                !source.contains(publisher),
                "raw filesystem publication returned in commands/{consumer}: {publisher}"
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
    for facade in ["profile/mod.rs", "storage/mod.rs", "validation/mod.rs"] {
        let source = production_source(&source_root.join(facade));
        for seam in [
            "pub use crate::repository_state",
            "pub use crate::declarations",
            "pub type Storage =",
            "pub type Gate =",
        ] {
            assert!(!source.contains(seam), "facade seam in {facade}: {seam}");
        }
    }

    let workspace = std::fs::read_to_string(crate_root.join("../../Cargo.toml")).unwrap();
    for package in ["crates/repository-state", "crates/declarations"] {
        assert!(
            !workspace.contains(package),
            "intermediate package: {package}"
        );
    }
    for module in ["repository_state", "declarations"] {
        assert!(!contains_cargo_manifest(&source_root.join(module)));
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
        ("fn render_managed_document(", "managed_document.rs"),
        ("fn render_projection_body(", "projection_render.rs"),
        ("fn render_invariants_markdown(", "projection.rs"),
        (
            "fn render_rules_and_gates_markdown(",
            "rules_gates_projection.rs",
        ),
        ("fn serialize_ruleset(", "rule_serialize.rs"),
        ("fn render_repo_config(", "initialize.rs"),
    ] {
        assert_eq!(
            definition_owners(&source_root, definition),
            [PathBuf::from("repository_state").join(file)],
            "known renderer ownership changed: {definition}"
        );
    }
}

/// A package read must resolve each source once, at the open, and take
/// everything it decides from that handle.
///
/// Pathname-addressed reads are the one shape visibility cannot forbid:
/// `std::fs` is public to every crate. A capture that stats a declared target
/// and then reads it by name answers two questions about two objects whenever
/// something replaces the name in between — the read follows a link the check
/// never saw, and the mode judged is not the mode of the bytes admitted. The
/// property is structural rather than behavioural because reproducing that
/// interval in a test means racing it, and a test that must win a race to fail
/// is a test that reports nothing on the runs it loses.
#[test]
fn test_package_capture_addresses_no_declared_source_by_pathname() {
    let capture = production_source(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/profile/package_capture.rs"),
    );
    for reader in [
        "fs::read(",
        "fs::read_to_string(",
        "fs::metadata(",
        "fs::symlink_metadata(",
        "fs::canonicalize(",
        "File::open(",
    ] {
        assert!(
            !capture.contains(reader),
            "package capture addressed a source by pathname: {reader}"
        );
    }
    assert!(
        capture.contains("open_path_nofollow("),
        "package capture no longer reads through the shared no-follow open, so \
         the absence of pathname readers above establishes nothing"
    );
}

/// One no-follow opener serves every route into a package.
///
/// Walking a package directory and capturing a tree from declared repository
/// files both need the same open — no link followed into the name, no
/// resolution out of the anchoring directory, non-blocking so an entry of the
/// wrong kind cannot hold the reader open forever. A second copy is where one
/// of those subtleties goes missing (`@/inv/convention-convergence`), so the
/// options are built in exactly one place.
#[test]
fn test_profile_package_reads_build_no_follow_options_in_one_place() {
    let profile = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/profile");
    let owners = definition_owners(&profile, "_cap_fs_ext_follow");
    assert_eq!(
        owners,
        vec![PathBuf::from("nofollow.rs")],
        "each entry names a profile module building its own no-follow open"
    );
}

/// Reading a package archive touches nothing outside the caller's memory.
///
/// This is what makes "a refused or interrupted add leaves no partially
/// extracted package" true by construction rather than by cleanup: extraction
/// produces a value, and the only thing that reaches the filesystem is the
/// recoverable transaction the command layer publishes the finished tree
/// through. A module that opened a staging directory would have to unwind it on
/// every refusal path, and the one path that forgot would be the one that
/// leaked. It is also what makes the read local: no network client, no
/// subprocess, and no environment lookup can be reached from a module that
/// names none of them.
///
/// The property is structural because the alternative is to interrupt a real
/// extraction at each of its refusal points and inspect the filesystem after
/// each one, which tests the points someone remembered to enumerate rather than
/// the absence of the capability.
#[test]
fn test_package_archive_read_reaches_no_filesystem_network_or_process() {
    let archive = production_source(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/profile/package_archive.rs"),
    );
    for capability in [
        "fs::",
        "File::",
        "std::net",
        "ureq",
        "reqwest",
        "Command::",
        "env::",
        "tempfile",
    ] {
        assert!(
            !archive.contains(capability),
            "the package archive module reached for {capability}"
        );
    }
    for entry_point in [
        "pub fn pack_package_archive(",
        "pub fn read_package_archive(",
    ] {
        assert!(
            archive.contains(entry_point),
            "{entry_point} no longer lives here, so the absence of the capabilities above \
             establishes nothing"
        );
    }
}

/// A package tree reaches the worktree through one publication.
///
/// Capturing a tree from the repository files a manifest declares and adding
/// one from a portable archive differ in where the bytes came from, not in how
/// they land: both derive one plan against one captured image and publish it
/// through the shared recoverable transaction. A second publication route is
/// where the no-replace guarantee, the whole-tree delta, or the retry on a
/// concurrent change goes missing for one of them
/// (`@/inv/convention-convergence`).
#[test]
fn test_package_tree_publication_has_one_owner_in_the_command_layer() {
    let profile =
        production_source(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src/commands/profile.rs"));
    assert_eq!(
        profile.matches("finalize_package_tree_capture(").count(),
        1,
        "a package tree is finalized in more than one place in the profile commands"
    );
    assert_eq!(
        profile.matches("PackageTreeCapture::new(").count(),
        1,
        "a package tree publication is declared in more than one place in the profile commands"
    );
    for caller in [
        "TreePlacement::Republish",
        "TreePlacement::RequireAbsent",
        "fn publish_package_tree(",
    ] {
        assert!(
            profile.contains(caller),
            "{caller} is absent, so the counts above are not about the two publications"
        );
    }
}
