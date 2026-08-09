//! Shared test utilities
//!
//! Common helpers used across multiple test modules to reduce duplication.

#![cfg(any(test, feature = "test-support"))]

use crate::commands::CommandExecutor;
use crate::profile::package_assembly::{assemble_package_tree, PackageAssemblyError};
use crate::profile::ProfilePackage;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{discover_repository_layout, JsonFileStorage};
use crate::test_taxonomy::{test_taxonomy, TestTaxonomy};
use anyhow::Result;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use tempfile::TempDir;

/// Serializes tests that temporarily change the process-wide working directory.
///
/// The mutex is recovered after poisoning because a panic while a guard is
/// alive runs its `Drop` implementation before the mutex guard is released,
/// so the directory has already been restored when another test acquires it.
static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Change the process working directory for a scope and restore it on drop.
///
/// This guard is intended for tests only. The process-wide lock prevents two
/// tests from observing or changing the working directory at the same time,
/// and restoration also occurs while unwinding after a panic.
#[must_use = "dropping the guard restores the process working directory"]
pub struct CurrentDirGuard {
    original: PathBuf,
    _lock: MutexGuard<'static, ()>,
}

impl CurrentDirGuard {
    /// Change the process working directory, retaining the directory to which
    /// it must be restored when this guard is dropped.
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let lock = CURRENT_DIR_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let original = std::env::current_dir()?;
        std::env::set_current_dir(path)?;

        Ok(Self {
            original,
            _lock: lock,
        })
    }
}

impl Drop for CurrentDirGuard {
    fn drop(&mut self) {
        let _ = std::env::set_current_dir(&self.original);
    }
}

/// Repository-relative directory holding the checked-in sources of the profile
/// packages this repository ships, one directory per package id.
pub const PROFILE_PACKAGE_SOURCES: &str = "profiles";

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
        .initialize_fresh_repository(temp.path(), None)?;
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
        .initialize_fresh_repository(temp.path(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, storage, taxonomy))
}

/// The checkout these sources were compiled from.
///
/// Resolved from the crate's own manifest directory, an absolute path fixed
/// when the crate was compiled, so the answer names one location whatever
/// directory a test process runs in.
fn repository_checkout() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("the workspace root is two levels above the jit crate manifest")
        .to_path_buf()
}

/// This repository's profile package `id`, assembled from its checkout and
/// published at `destination`.
///
/// One route serves every test that needs this repository's own package. The
/// manifest and the sources the package authors itself come from the checked-in
/// package directory named after `id` under [`PROFILE_PACKAGE_SOURCES`]; every
/// live asset's bytes come from the repository file its declaration targets. A
/// caller therefore reads the checkout's current repository files.
///
/// `destination` belongs to the caller. That is what lets a test which applies
/// the package name a directory inside the repository it applies it to: an
/// application records the package's worktree-relative location and refuses a
/// package read from outside the worktree. A caller that only reads
/// declarations names a temporary directory it owns.
///
/// Each call assembles afresh and takes `destination` over whole, so calling
/// twice at one destination answers with the checkout's state at each call.
/// Nothing is cached between calls: a cached tree would answer from the state
/// at the first call, and this repository's managed-region and executable-mode
/// contracts assert about the checkout at the moment they read it, so a stale
/// answer would report an agreement that no longer holds.
///
/// # Errors
///
/// Every failure [`assemble_package_tree`] reports: a declared source absent
/// from either side, a checked-in manifest that does not parse or declares
/// something invalid, a staged tree that does not validate as a package, and a
/// destination occupied at the moment of publication.
pub fn assemble_repository_package(
    id: &str,
    destination: &Path,
) -> Result<ProfilePackage, PackageAssemblyError> {
    let checkout = repository_checkout();
    assemble_package_tree(
        &checkout.join(PROFILE_PACKAGE_SOURCES).join(id),
        &checkout,
        destination,
    )
}

/// The manifest id of every profile package this repository publishes, sorted.
///
/// Read from the checkout's [`PROFILE_PACKAGE_SOURCES`] listing, which is where
/// this repository publishes its packages: a package added there joins every
/// caller's set without an edit, and a caller that named its packages would go
/// on ignoring it. One directory is one package id.
///
/// Panics when the listing cannot be read, and when it comes back empty: a walk
/// over nothing reports nothing, so either would let a caller pass by vacuity
/// rather than by the property holding.
pub fn published_package_ids() -> Vec<String> {
    let sources = repository_checkout().join(PROFILE_PACKAGE_SOURCES);
    let mut ids: Vec<String> = fs::read_dir(&sources)
        .unwrap_or_else(|error| {
            panic!(
                "failed to list the checkout's package sources at {}: {error}",
                sources.display()
            )
        })
        .map(|entry| {
            entry
                .expect("read a package source entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(
        !ids.is_empty(),
        "{} publishes no profile package",
        sources.display()
    );
    ids.sort();
    ids
}

/// Assemble every profile package this repository authors into
/// `worktree`/[`PROFILE_PACKAGE_SOURCES`], and answer with the location of `id`.
///
/// A package is applied from inside the worktree it is applied to, and a
/// package's declared dependency is looked for in a directory named after it
/// beside the package declaring it. Staging the whole authored set therefore
/// makes any one of them applicable, whatever it depends on, without a caller
/// restating the dependency graph (`@/invariant/shared-test-contracts`).
///
/// Panics when the checkout's package sources cannot be listed or a package
/// does not assemble, which is a defect in the checkout rather than a condition
/// a test distinguishes.
pub fn stage_repository_packages(worktree: &Path, id: &str) -> PathBuf {
    let staged = worktree.join(PROFILE_PACKAGE_SOURCES);
    published_package_ids().into_iter().for_each(|package| {
        assemble_repository_package(&package, &staged.join(&package)).unwrap_or_else(|error| {
            panic!("this repository's {package} package assembles: {error}")
        });
    });
    let location = staged.join(id);
    assert!(
        location.is_dir(),
        "this repository authors no profile package '{id}'"
    );
    location
}

/// This repository's profile package `id` assembled into a temporary
/// destination, answered with the directory that owns it.
///
/// The fixture over [`assemble_repository_package`] for a caller that reads a
/// package's declarations and has no repository to publish it into. The
/// returned directory holds the published tree, so the package's recorded
/// source stays readable for as long as the caller keeps it; a caller that
/// applies the package names its own destination and calls the entry point
/// directly.
///
/// Panics when the package does not assemble, which is a defect in the checkout
/// rather than a condition a test distinguishes.
pub fn temporary_repository_package(id: &str) -> (TempDir, ProfilePackage) {
    let workspace = TempDir::new().expect("create a package destination");
    let package = assemble_repository_package(id, &workspace.path().join(id))
        .unwrap_or_else(|error| panic!("this repository's {id} package assembles: {error}"));
    (workspace, package)
}

/// The checked-in source tree of the profile-package fixture named `name`.
///
/// Fixture packages are ordinary directories under the crate's own
/// `tests/fixtures/profile-packages/`, resolved from the crate root so the
/// answer is independent of the process working directory. One resolver serves
/// every test that stages a fixture package, so no caller spells the fixture
/// prefix for itself (`@/invariant/shared-test-contracts`).
pub fn profile_package_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/profile-packages")
        .join(name)
}

/// Copy the profile-package fixture tree at `source` to `root` under a manifest
/// rewritten to declare `id`, a dependency on each of `dependencies`, and asset
/// targets named for `id`, and read the package back from there.
///
/// No package this repository ships declares a dependency, so every test of
/// composition authors one. One rewrite serves them all
/// (`@/invariant/shared-test-contracts`), and renaming the asset targets after
/// the id is what keeps two authored packages from publishing the same file.
pub fn write_package_declaring(
    source: &Path,
    root: &Path,
    id: &str,
    dependencies: &[&str],
) -> crate::profile::ProfilePackage {
    let tree = copy_package_tree(source, root);
    let manifest_path = tree.join(crate::profile::MANIFEST_FILE_NAME);
    let authored = fs::read_to_string(&manifest_path).expect("read the package manifest");
    let source = crate::profile::ProfilePackage::parse_manifest(authored.as_bytes())
        .expect("the source package manifest parses");
    let declared = dependencies
        .iter()
        .map(|dependency| format!("\"{dependency}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let rewritten = source.assets.iter().fold(
        authored
            .replace(
                &format!("id = \"{}\"", source.id),
                &format!("id = \"{id}\""),
            )
            .replace(
                "[profile]",
                &format!("dependencies = [{declared}]\n\n[profile]"),
            ),
        |manifest, asset| {
            let renamed = Path::new(&asset.target)
                .parent()
                .map(|parent| parent.join(id))
                .unwrap_or_else(|| PathBuf::from(id))
                .with_extension(
                    Path::new(&asset.target)
                        .extension()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .as_ref(),
                );
            manifest.replace(
                &format!("target = \"{}\"", asset.target),
                &format!("target = \"{}\"", renamed.display()),
            )
        },
    );
    fs::write(&manifest_path, rewritten).expect("write the rewritten package manifest");
    crate::profile::ProfilePackage::from_directory(&tree).expect("a valid package tree")
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
    use crate::profile::package_assembly::PACKAGE_SOURCE_PATH;
    use crate::storage::IssueStore;

    /// The directory names under [`PROFILE_PACKAGE_SOURCES`], which is the set
    /// of package sources this repository ships.
    fn shipped_package_directories() -> Vec<String> {
        fs::read_dir(repository_checkout().join(PROFILE_PACKAGE_SOURCES))
            .expect("the checkout carries the profile package sources")
            .map(|entry| {
                entry
                    .expect("read a package source entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect()
    }

    /// The id of the package whose sources [`PACKAGE_SOURCE_PATH`] names, taken
    /// from that declaration so this module states no second location for it.
    fn assembled_package_id() -> &'static str {
        Path::new(PACKAGE_SOURCE_PATH)
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .expect("the package source path names a directory")
    }

    #[test]
    fn test_current_dir_guard_restores_after_panic() {
        let elsewhere = TempDir::new().unwrap();
        let mut original = None;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _cwd = CurrentDirGuard::new(elsewhere.path()).unwrap();
            original = Some(_cwd.original.clone());
            assert_eq!(std::env::current_dir().unwrap(), elsewhere.path());
            panic!("the guard must restore the directory during unwinding");
        }));

        assert!(result.is_err());
        let _verification_lock = CURRENT_DIR_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(std::env::current_dir().unwrap(), original.unwrap());
    }

    /// Every package this repository ships assembles under the id its source
    /// directory is named after, which is what makes an id a sufficient way to
    /// name one.
    #[test]
    fn test_assemble_repository_package_answers_the_shipped_package_its_id_names() {
        // The assembly module's own package-source path sits under the
        // directory this module resolves an id against, so the two agree about
        // where package sources live rather than each stating it.
        assert_eq!(
            Path::new(PACKAGE_SOURCE_PATH).parent(),
            Some(Path::new(PROFILE_PACKAGE_SOURCES)),
            "the assembled package's sources sit outside the shipped package directory"
        );

        let shipped = shipped_package_directories();
        assert!(
            shipped.len() > 1,
            "this repository ships one package source directory, so a rule over \
             them says nothing about naming: {shipped:?}"
        );

        let workspace = TempDir::new().unwrap();
        let misnamed: Vec<(String, String)> = shipped
            .iter()
            .map(|id| {
                let package = assemble_repository_package(id, &workspace.path().join(id))
                    .unwrap_or_else(|error| panic!("{id} does not assemble: {error}"));
                (id.clone(), package.model().id.to_string())
            })
            .filter(|(directory, declared)| directory != declared)
            .collect();
        assert_eq!(
            misnamed,
            Vec::<(String, String)>::new(),
            "each entry pairs a package source directory with the id its \
             manifest declares, which the directory is expected to be named after"
        );
    }

    /// The checkout is resolved from the crate's own manifest directory, so the
    /// entry point answers the same from any working directory.
    ///
    /// Observed the way `storage::json`'s repository-root reads observe theirs:
    /// the process working directory is moved to an unrelated place for the
    /// call, so a relative path anywhere in the resolution would find nothing,
    /// and restored before anything is asserted.
    #[test]
    fn test_assemble_repository_package_resolves_the_checkout_independently_of_the_working_directory(
    ) {
        let checkout = repository_checkout();
        assert!(
            checkout.is_absolute(),
            "a relative checkout would be resolved against the working directory: {}",
            checkout.display()
        );

        let elsewhere = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let destination = workspace.path().join("package");
        let assembled = {
            let _cwd = CurrentDirGuard::new(elsewhere.path()).unwrap();
            assemble_repository_package(assembled_package_id(), &destination)
        };

        let package = assembled.expect("the package assembles from an unrelated working directory");
        assert_eq!(package.model().id.as_str(), assembled_package_id());
        assert!(
            package.file_count() > 1,
            "the assembled package carries more than a manifest"
        );
    }

    /// A second call at one destination is answered rather than refused, and
    /// what it leaves there is one whole tree.
    ///
    /// The publication underneath is atomic no-replace, so a run that merely
    /// renamed onto an occupied destination would fail here; a run that merged
    /// into it would leave a tree the manifest does not describe.
    #[test]
    fn test_assemble_repository_package_republishes_over_its_own_previous_destination() {
        let workspace = TempDir::new().unwrap();
        let destination = workspace.path().join("package");
        let id = assembled_package_id();

        let first = assemble_repository_package(id, &destination).unwrap();
        let second = assemble_repository_package(id, &destination).unwrap();

        assert_eq!(second.hashes(), first.hashes());
        let reread = ProfilePackage::from_directory(&destination)
            .expect("the republished destination holds a package");
        assert_eq!(reread.hashes(), second.hashes());
        assert_eq!(reread.file_count(), second.file_count());
    }

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
