//! Shared test utilities
//!
//! Common helpers used across multiple test modules to reduce duplication.

#![cfg(any(test, feature = "test-support"))]

use crate::commands::CommandExecutor;
use crate::hierarchy_templates::HierarchyTemplate;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{discover_repository_layout, JsonFileStorage};
use crate::test_taxonomy::{test_taxonomy, TestTaxonomy};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

/// Standard test repository setup with .jit and .git directories
///
/// Creates a temporary directory with initialized jit storage.
/// Returns both the TempDir (which cleans up on drop) and the storage instance.
///
/// # Example
/// ```no_run
/// use jit::test_utils::setup_test_repo;
///
/// let (temp, storage) = setup_test_repo().unwrap();
/// // Use temp.path() and storage for tests
/// ```
pub fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> {
    let temp = TempDir::new()?;

    let jit_root = temp.path().join(".jit");
    let storage = JsonFileStorage::new(&jit_root);
    let layout = discover_repository_layout(temp.path(), &jit_root)?;
    CommandExecutor::new(storage.clone())
        .with_layout(layout)
        .initialize_fresh_repository(temp.path(), &HierarchyTemplate::default(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, storage))
}

/// Build a repository from the fixture's explicitly declared custom taxonomy.
///
/// The configuration is written before initialization, so the initializer's
/// existing-configuration path consumes these declarations and derives the
/// repository's coupled rules and schemas from them. The returned taxonomy is
/// the same value used to author that configuration, allowing callers to build
/// labels and assertions without repeating vocabulary literals.
pub fn setup_test_repo_with_taxonomy() -> Result<(TempDir, JsonFileStorage, TestTaxonomy)> {
    let taxonomy = test_taxonomy();
    let temp = TempDir::new()?;
    let jit_root = temp.path().join(".jit");
    fs::create_dir_all(&jit_root)?;
    fs::write(jit_root.join("config.toml"), taxonomy.config_fragment())?;

    let storage = JsonFileStorage::new(&jit_root);
    let layout = discover_repository_layout(temp.path(), &jit_root)?;
    CommandExecutor::new(storage.clone())
        .with_layout(layout)
        .initialize_fresh_repository(temp.path(), &taxonomy.hierarchy_template(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, storage, taxonomy))
}

/// Write a compile-time-embedded profile package tree to `root`, creating it
/// and every declared parent, and return `root`.
///
/// One writer serves every test that needs a profile package on disk, so the
/// directory route reads back exactly the authored fixture the compile-time
/// route embeds instead of a per-module copy of it.
pub fn write_package_tree(package: &include_dir::Dir<'_>, root: &Path) -> PathBuf {
    fn write(directory: &include_dir::Dir<'_>, root: &Path) {
        directory.files().for_each(|file| {
            let path = root.join(file.path());
            fs::create_dir_all(path.parent().expect("package file has a parent"))
                .expect("create package parent directory");
            fs::write(path, file.contents()).expect("write package file");
        });
        directory.dirs().for_each(|child| write(child, root));
    }

    fs::create_dir_all(root).expect("create package root");
    write(package, root);
    root.to_path_buf()
}

/// Copy an on-disk profile package tree to `root`, creating it and every
/// declared parent, and return `root`.
///
/// A test that applies a package must read it from inside the repository it is
/// applied to, because profile application records the package's worktree-
/// relative location.
pub fn copy_package_tree(source: &Path, root: &Path) -> PathBuf {
    fn copy(source: &Path, root: &Path) {
        fs::read_dir(source)
            .expect("read package source directory")
            .for_each(|entry| {
                let entry = entry.expect("read package source entry");
                let source_path = entry.path();
                let destination = root.join(entry.file_name());
                if source_path.is_dir() {
                    fs::create_dir_all(&destination).expect("create package directory");
                    copy(&source_path, &destination);
                } else {
                    fs::create_dir_all(destination.parent().expect("package file has a parent"))
                        .expect("create package parent directory");
                    fs::copy(source_path, destination).expect("copy package file");
                }
            });
    }

    fs::create_dir_all(root).expect("create package root");
    copy(source, root);
    root.to_path_buf()
}

/// Create test WorktreePaths from a TempDir
///
/// Generates a WorktreePaths structure suitable for testing,
/// with standard paths relative to the temp directory.
///
/// # Example
/// ```no_run
/// use jit::test_utils::{setup_test_repo, create_test_paths};
///
/// let (temp, _storage) = setup_test_repo().unwrap();
/// let paths = create_test_paths(&temp);
/// assert_eq!(paths.worktree_root, temp.path());
/// ```
pub fn create_test_paths(temp: &TempDir) -> WorktreePaths {
    WorktreePaths {
        common_dir: temp.path().join(".git"),
        worktree_root: temp.path().to_path_buf(),
        local_jit: temp.path().join(".jit"),
        shared_jit: temp.path().join(".git/jit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::IssueStore;

    #[test]
    fn test_setup_test_repo_with_taxonomy_declares_and_exposes_exact_vocabulary() {
        let (_temp, storage, taxonomy) = setup_test_repo_with_taxonomy().unwrap();
        let config = crate::config::JitConfig::load(storage.root()).unwrap();
        let hierarchy = config
            .type_hierarchy
            .expect("taxonomy fixture must declare a type hierarchy");
        let namespaces = config
            .namespaces
            .expect("taxonomy fixture must declare namespaces");

        assert_eq!(hierarchy.types, taxonomy.hierarchy);
        assert_eq!(
            hierarchy.strategic_types,
            Some(taxonomy.strategic_types.clone())
        );
        assert_eq!(
            hierarchy.label_associations,
            Some(taxonomy.label_associations.clone())
        );
        assert_eq!(
            config
                .validation
                .as_ref()
                .and_then(|validation| validation.default_type.as_deref()),
            Some(taxonomy.default_type.as_str())
        );
        assert_eq!(namespaces.len(), taxonomy.namespaces.len());
        assert!(taxonomy.namespaces.iter().all(|(name, expected)| {
            namespaces.get(name).is_some_and(|actual| {
                actual.description == expected.description && actual.unique == expected.unique
            })
        }));
    }
}
