//! Shared test utilities
//!
//! Common helpers used across multiple test modules to reduce duplication.

#![cfg(any(test, feature = "test-support"))]

use crate::commands::CommandExecutor;
use crate::config::ItemKindConfig;
use crate::domain::{identity::IssueId, DocumentReference};
use crate::profile::{capture_package_tree, ProfilePackage, ProfilePackageModel};
use crate::repository_state::{Contribution, MapEntryTarget};
use crate::repository_state::{FileMode, RepositoryEntry, VirtualPath};
#[cfg(any(test, feature = "test-support"))]
use crate::repository_state::{RepositoryLayout, RepositoryRootEvidence};
use crate::storage::lock::LockGuard;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{
    discover_repository_layout, FileLocker, InMemoryStorage, IssueStore, JsonFileStorage,
};
use crate::test_taxonomy::{test_taxonomy, TestTaxonomy};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tempfile::TempDir;

/// Give a temporary data root a valid repository layout for loader tests.
#[cfg(any(test, feature = "test-support"))]
pub fn repository_layout_for_data_root(data_root: &Path) -> RepositoryLayout {
    let absolute_data_root = std::fs::canonicalize(data_root).unwrap_or_else(|_error| {
        let parent = data_root.parent().unwrap_or(data_root);
        let parent = std::fs::canonicalize(parent)
            .expect("temporary test worktree root has an absolute path");
        parent.join(
            data_root
                .file_name()
                .expect("temporary data root has a final component"),
        )
    });
    let worktree_root = absolute_data_root
        .parent()
        .expect("temporary data root has a parent");
    RepositoryLayout::new(
        RepositoryRootEvidence::new(worktree_root, "test-worktree", true),
        RepositoryRootEvidence::new(&absolute_data_root, "test-data", true),
    )
    .expect("temporary test roots form a valid repository layout")
}

/// A TOML string array of `values` rendered by `Display`, so a fixture lists
/// vocabulary values through their wire tokens.
pub fn toml_string_array<T: std::fmt::Display>(values: &[T]) -> String {
    let quoted = values
        .iter()
        .map(|value| format!("\"{value}\""))
        .collect::<Vec<_>>();
    format!("[{}]", quoted.join(", "))
}

/// A reference to `path` at HEAD with no label, type, format, or assets.
pub fn document_reference(path: impl Into<String>) -> DocumentReference {
    DocumentReference {
        path: path.into(),
        commit: None,
        label: None,
        doc_type: None,
        format: None,
        assets: Vec::new(),
    }
}

/// A reference to `path` pinned to `commit`.
pub fn document_reference_at(
    path: impl Into<String>,
    commit: impl Into<String>,
) -> DocumentReference {
    DocumentReference {
        commit: Some(commit.into()),
        ..document_reference(path)
    }
}

/// A reference to `path` at HEAD carrying the document type `doc_type`.
pub fn document_reference_typed(
    path: impl Into<String>,
    doc_type: impl Into<String>,
) -> DocumentReference {
    DocumentReference {
        doc_type: Some(doc_type.into()),
        ..document_reference(path)
    }
}

/// A deterministic canonical issue id; distinct numeric seeds yield distinct ids.
pub fn issue_id(seed: u128) -> IssueId {
    IssueId::from_uuid(uuid::Uuid::from_u128(seed.rotate_left(96)))
}

/// A deterministic canonical gate-run id; distinct numeric seeds yield distinct
/// ids.
pub fn gate_run_id(seed: u128) -> crate::domain::identity::GateRunId {
    crate::domain::identity::GateRunId::from_uuid(uuid::Uuid::from_u128(seed.rotate_left(96)))
}

/// A drafted issue carrying a fresh random id.
pub fn fixture_issue(title: String, description: String) -> crate::domain::Issue {
    crate::domain::Issue::draft(issue_id(uuid::Uuid::new_v4().as_u128()), title, description)
}

/// The event recording `body`, stamped with `id` and `timestamp` as the
/// mutation finalizer would stamp it.
pub fn stamped_event(
    body: crate::domain::EventBody,
    id: impl Into<String>,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> crate::domain::Event {
    crate::domain::Event::new(id.into(), timestamp, body)
}

/// A non-reconciling execution of `target` into `root` publishing each
/// `(source, destination)` pair with identity of the destination's own text.
pub fn archive_execution(
    target: crate::domain::artifact_plan::PlanTarget,
    root: &str,
    published: &[(&str, &str)],
) -> crate::domain::artifact_execution::ArchiveExecution {
    crate::domain::artifact_execution::ArchiveExecution {
        target,
        destination_root: root.to_string(),
        publications: published
            .iter()
            .map(
                |(source, destination)| crate::domain::artifact_execution::ArchivePublication {
                    source: Some((*source).to_string()),
                    destination: (*destination).to_string(),
                    content_identity: crate::domain::artifact_plan::ContentIdentity::from_bytes(
                        destination.as_bytes(),
                    ),
                    adopted: false,
                },
            )
            .collect(),
        reference_changes: Vec::new(),
        planned_deletions: Vec::new(),
        reconciling: false,
    }
}

#[cfg(test)]
mod fixture_issue_tests {
    use super::fixture_issue;

    proptest::proptest! {
        #[test]
        fn test_issue_id_is_canonical_deterministic_and_distinct(
            first in proptest::prelude::any::<u128>(),
            second in proptest::prelude::any::<u128>(),
        ) {
            use crate::domain::identity::IssueId;

            let id = super::issue_id(first);
            proptest::prop_assert!(IssueId::try_from(id.as_str()).is_ok());
            proptest::prop_assert_eq!(&id, &super::issue_id(first));
            proptest::prop_assert_eq!(id == super::issue_id(second), first == second);
        }
    }

    #[test]
    fn test_fixture_issue_carries_title_description_and_distinct_ids() {
        let first = fixture_issue("Title".into(), "Body".into());
        let second = fixture_issue("Title".into(), "Body".into());
        assert_eq!(
            (first.title.as_str(), first.description.as_str()),
            ("Title", "Body")
        );
        assert_ne!(first.id, second.id);
    }
}

/// Repository-relative directory holding the checked-in sources of the profile
/// packages this repository ships, one directory per package id.
pub const PROFILE_PACKAGE_SOURCES: &str = "profiles";

const KEYED_FIXTURE_SCHEMA: u32 = 1;
const KEYED_FIXTURE_DIRECTORY: &str = "jit-keyed-fixtures";
const KEYED_FIXTURE_TREE: &str = "tree";
const PROFILED_REPOSITORY_FIXTURE_SETUP: &str = "command-executor.initialize-fresh-repository.v1";
const KEYED_FIXTURE_STAGING: &str = ".keyed-fixture-staging-";
const KEYED_FIXTURE_RETIRED: &str = ".keyed-fixture-retired-";
/// Digest of the `jit` crate source this setup implementation was compiled from (`build.rs`).
pub const CRATE_SOURCE_SHA256: &str = env!("JIT_CRATE_SOURCE_SHA256");

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyedFixtureManifest {
    schema_version: u32,
    key: String,
    tree_sha256: String,
}

#[derive(Clone, Copy)]
struct ProfiledRepositoryFixtureSpec {
    id: &'static str,
    package_directory: &'static str,
}

const PROFILED_REPOSITORY_FIXTURES: [ProfiledRepositoryFixtureSpec; 2] = [
    ProfiledRepositoryFixtureSpec {
        id: "jit-default",
        package_directory: "packages",
    },
    ProfiledRepositoryFixtureSpec {
        id: "jit-dogfood",
        package_directory: PROFILE_PACKAGE_SOURCES,
    },
];

/// What one fixture profile's entry is built from, captured once per process.
struct ProfiledRepositoryFixtureInputs {
    key: String,
    packages: BTreeMap<String, ProfilePackage>,
}

static DIRECT_PROFILED_REPOSITORY_FIXTURES: Mutex<
    BTreeMap<String, Arc<ProfiledRepositoryFixtureInputs>>,
> = Mutex::new(BTreeMap::new());

/// A published fixture entry, kept from retirement while the lease lives.
#[derive(Debug)]
pub struct KeyedFixtureLease {
    entry: PathBuf,
    _family_lock: LockGuard,
}

impl KeyedFixtureLease {
    /// The immutable tree the entry's build wrote.
    pub fn tree(&self) -> PathBuf {
        self.entry.join(KEYED_FIXTURE_TREE)
    }
}

/// Leases `family`'s immutable tree for `key`, built by `build` on first use
/// under the Cargo target directory. A family keeps only its newest key.
///
/// The shared family lock protects readers; the exclusive lock protects
/// builders and retirement. A builder stages beside the keyed entry and
/// publishes it with one atomic no-replace rename. If a valid entry wins that
/// rename race, the builder uses it; corrupt or unrelated occupied entries
/// fail closed. A waiter waits for as long as the build runs: it fails with the
/// cause of a build that fails, and builds itself when the holder dies.
pub fn keyed_fixture(
    family: &str,
    key: &str,
    build: impl FnOnce(&Path) -> Result<()>,
) -> Result<KeyedFixtureLease> {
    let fixture_setup = std::env::current_exe().context("resolve fixture setup executable")?;
    let cache_root = keyed_fixture_cache_root(&fixture_setup)?;
    ensure_keyed_fixture(&cache_root, family, key, build)
}

/// The cache key over `fields`, each framed by its length.
pub fn keyed_fixture_key(fields: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    fields
        .iter()
        .for_each(|field| hash_field(&mut hasher, field));
    format!("{:x}", hasher.finalize())
}

/// A fresh repository carrying this checkout's profile package.
///
/// Its immutable baseline lives in the [`keyed_fixture`] cache, keyed on every
/// captured package path, byte, and mode, the `jit` crate source, and the setup
/// contract. Every caller receives an ordinary deep copy in a fresh temporary
/// directory, so process-level tests retain real filesystem and CLI boundaries
/// without sharing mutable repository state.
pub fn profiled_repository_fixture(id: &str, package_directory: &str) -> Result<TempDir> {
    let lease = cached_profiled_repository_fixture(id, package_directory)?;
    let clone = TempDir::new()?;
    copy_tree_exact(&lease.tree(), clone.path())?;
    Ok(clone)
}

/// A verified applied-profile repository imported into isolated memory state.
///
/// The returned temporary repository owns both the package locations the
/// profile was applied from and the file-backed baseline clone. Keep it
/// alive for as long as commands use the accompanying memory store.
pub fn profiled_in_memory_repository_fixture(
    id: &str,
) -> Result<(TempDir, crate::storage::InMemoryStorage)> {
    let spec = PROFILED_REPOSITORY_FIXTURES
        .iter()
        .find(|spec| spec.id == id)
        .with_context(|| format!("unsupported profiled-repository fixture '{id}'"))?;
    let repository = profiled_repository_fixture(id, spec.package_directory)?;
    let storage = crate::storage::InMemoryStorage::rooted_at(repository.path());
    storage.seed_repository_tree_fixture(repository.path())?;
    Ok((repository, storage))
}

fn cached_profiled_repository_fixture(
    id: &str,
    package_directory: &str,
) -> Result<KeyedFixtureLease> {
    let profile = fixture_profile(id, package_directory);
    let inputs = {
        let mut memo = DIRECT_PROFILED_REPOSITORY_FIXTURES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match memo.get(&profile) {
            Some(inputs) => Arc::clone(inputs),
            None => {
                let inputs = Arc::new(profiled_repository_fixture_inputs(id)?);
                memo.insert(profile.clone(), Arc::clone(&inputs));
                inputs
            }
        }
    };
    let package_root = VirtualPath::worktree(package_directory)?;
    keyed_fixture(&profile, &inputs.key, |repository| {
        initialize_profiled_repository_fixture(
            repository,
            id,
            package_root.relative().as_path(),
            &inputs.packages,
        )
    })
}

fn profiled_repository_fixture_inputs(id: &str) -> Result<ProfiledRepositoryFixtureInputs> {
    let packages = capture_profile_package_closure(id)?;
    let key = profiled_repository_fixture_key(
        &package_closure_sha256(packages.iter().flat_map(|(package_id, package)| {
            package.files().iter().map(move |(relative, bytes)| {
                (
                    package_id.as_str(),
                    relative.as_str(),
                    package.model().file_mode(relative),
                    bytes.as_slice(),
                )
            })
        })),
        CRATE_SOURCE_SHA256,
    );
    Ok(ProfiledRepositoryFixtureInputs { key, packages })
}

/// The cache directory name of one fixture profile: the package id and the
/// directory its packages are written to.
fn fixture_profile(id: &str, package_directory: &str) -> String {
    format!("{id}@{}", package_directory.replace(['/', '\\'], "+"))
}

fn initialize_profiled_repository_fixture(
    repository: &Path,
    id: &str,
    package_directory: &Path,
    packages: &BTreeMap<String, ProfilePackage>,
) -> Result<()> {
    let package_root = repository.join(package_directory);
    packages.iter().try_for_each(|(package_id, package)| {
        write_captured_package(package, &package_root.join(package_id))
    })?;
    let selected = package_root.join(id);
    if !selected.is_dir() {
        bail!("this repository authors no profile package '{id}'");
    }
    // A rebuildable cache needs no durability: initializing over memory spares the
    // build the fsyncs that wait on the host's whole writeback backlog.
    let storage = InMemoryStorage::rooted_at(repository);
    storage.seed_repository_tree_fixture(repository)?;
    let layout = storage.repository_layout();
    let executor = CommandExecutor::new(storage.clone()).with_layout(layout.clone());
    let selection = executor.profiles().select_profiles(
        &[PathBuf::from(&selected)],
        &crate::commands::ProfileVariableOptions::default(),
    )?;
    executor
        .profiles()
        .initialize_fresh_repository(repository, Some(&selection))?;
    write_repository_entries(&storage, &layout)
}

fn write_repository_entries(storage: &InMemoryStorage, layout: &RepositoryLayout) -> Result<()> {
    storage
        .repository_state()
        .entries
        .iter()
        .try_for_each(|(path, entry)| {
            let target = layout.resolve(path)?;
            match entry {
                RepositoryEntry::File { content, mode } => {
                    fs::create_dir_all(target.parent().context("entry has no parent")?)?;
                    fs::write(&target, content.bytes())?;
                    set_captured_mode(&target, *mode)
                }
                RepositoryEntry::Directory { .. } => Ok(fs::create_dir_all(&target)?),
                other => bail!("repository entry {path:?} is not writable: {other:?}"),
            }
        })
}

fn capture_profile_package_closure(id: &str) -> Result<BTreeMap<String, ProfilePackage>> {
    let checkout = repository_checkout();
    let layout = discover_repository_layout(&checkout, checkout.join(".jit"))?;
    let published = published_package_ids();
    let mut pending = vec![id.to_string()];
    let mut packages = BTreeMap::new();
    while let Some(package_id) = pending.pop() {
        if packages.contains_key(&package_id) {
            continue;
        }
        if !published.contains(&package_id) {
            bail!("profile package '{package_id}' in '{id}'s dependency closure is not published");
        }
        let source = VirtualPath::worktree(Path::new(PROFILE_PACKAGE_SOURCES).join(&package_id))?;
        let package = capture_package_tree(&source, &layout)?;
        pending.extend(
            package
                .model()
                .dependencies
                .iter()
                .map(|dependency| dependency.id.as_str().to_string()),
        );
        packages.insert(package_id, package);
    }
    Ok(packages)
}

/// Digest of a captured package closure, one `(package id, package-relative path, mode, bytes)`
/// per file.
fn package_closure_sha256<'a>(
    files: impl IntoIterator<Item = (&'a str, &'a str, FileMode, &'a [u8])>,
) -> String {
    let mut hasher = Sha256::new();
    files
        .into_iter()
        .for_each(|(package_id, relative, mode, bytes)| {
            hash_field(&mut hasher, package_id.as_bytes());
            hash_field(&mut hasher, relative.as_bytes());
            hash_field(
                &mut hasher,
                match mode {
                    FileMode::Regular => b"regular",
                    FileMode::Executable => b"executable",
                },
            );
            hash_field(&mut hasher, bytes);
        });
    format!("{:x}", hasher.finalize())
}

fn profiled_repository_fixture_key(
    package_closure_sha256: &str,
    crate_source_sha256: &str,
) -> String {
    keyed_fixture_key(&[
        PROFILED_REPOSITORY_FIXTURE_SETUP.as_bytes(),
        package_closure_sha256.as_bytes(),
        crate_source_sha256.as_bytes(),
    ])
}

/// Derive the cache context Cargo used to place a running artifact.
///
/// Cargo puts binaries directly under a profile directory and test executables
/// under that profile's `deps` directory. Stripping those two layout segments
/// produces either the target directory or a target-triple directory. This
/// uses the artifact Cargo actually ran, so it stays stable when a reused
/// executable embeds a different checkout in `CARGO_MANIFEST_DIR` or when a
/// relative `CARGO_TARGET_DIR` was resolved from a different process directory.
fn keyed_fixture_cache_root(runtime_artifact: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(runtime_artifact).with_context(|| {
        format!(
            "inspect keyed fixture runtime artifact {}",
            runtime_artifact.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "keyed fixture runtime artifact {} must be an ordinary file",
            runtime_artifact.display()
        );
    }
    let artifact = fs::canonicalize(runtime_artifact).with_context(|| {
        format!(
            "canonicalize keyed fixture runtime artifact {}",
            runtime_artifact.display()
        )
    })?;
    let artifact_directory = artifact.parent().with_context(|| {
        format!(
            "keyed fixture runtime artifact {} has no parent directory",
            artifact.display()
        )
    })?;
    let profile_directory = if artifact_directory.file_name() == Some("deps".as_ref()) {
        artifact_directory.parent().with_context(|| {
            format!(
                "keyed fixture deps directory {} has no profile parent",
                artifact_directory.display()
            )
        })?
    } else {
        artifact_directory
    };
    let target_context = profile_directory.parent().with_context(|| {
        format!(
            "keyed fixture family directory {} has no target parent",
            profile_directory.display()
        )
    })?;
    Ok(target_context.join(KEYED_FIXTURE_DIRECTORY))
}

/// The lock one family's consumers coordinate through.
fn keyed_fixture_lock(cache_root: &Path, family: &str) -> PathBuf {
    cache_root.join("locks").join(format!("{family}.lock"))
}

/// Leases the family's entry for `key`, building and publishing it under the
/// exclusive family lock when absent. A consumer fails only with its own build's
/// cause: a holder that failed or died leaves no entry, so the next consumer to
/// get the lock builds it. The kernel releases a dead holder's lock, and the
/// runner's per-test limit ends any wait, so lock waits are unbounded here.
fn ensure_keyed_fixture(
    cache_root: &Path,
    family: &str,
    key: &str,
    build: impl FnOnce(&Path) -> Result<()>,
) -> Result<KeyedFixtureLease> {
    let family_root = cache_root.join(family);
    let lock = keyed_fixture_lock(cache_root, family);
    fs::create_dir_all(&family_root)?;
    fs::create_dir_all(lock.parent().context("fixture lock has no parent")?)?;
    let entry = family_root.join(key);
    let locker = FileLocker::new(Duration::MAX);
    loop {
        let shared = locker.lock_shared(&lock)?;
        if is_published_fixture(&entry)? {
            validate_keyed_fixture(&entry, key)?;
            return Ok(KeyedFixtureLease {
                entry,
                _family_lock: shared,
            });
        }
        drop(shared);
        let Some(exclusive) = locker.try_lock_exclusive(&lock)? else {
            std::thread::sleep(Duration::from_millis(
                crate::runtime_defaults::LOCK_POLL_INTERVAL_MS,
            ));
            continue;
        };
        if is_published_fixture(&entry)? {
            continue;
        }
        publish_keyed_fixture(&family_root, &entry, key, build)?;
        return Ok(KeyedFixtureLease {
            entry,
            _family_lock: exclusive,
        });
    }
}

/// Builds in staging, publishes `entry` and retires the family's older entries.
fn publish_keyed_fixture(
    family_root: &Path,
    entry: &Path,
    key: &str,
    build: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    let staging = tempfile::Builder::new()
        .prefix(KEYED_FIXTURE_STAGING)
        .tempdir_in(family_root)?;
    let tree = staging.path().join(KEYED_FIXTURE_TREE);
    fs::create_dir(&tree)?;
    build(&tree)?;
    let manifest = KeyedFixtureManifest {
        schema_version: KEYED_FIXTURE_SCHEMA,
        key: key.to_string(),
        tree_sha256: tree_sha256(&tree)?,
    };
    fs::write(
        staging.path().join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let staging_path = staging.keep();
    if let Err(error) = crate::storage::publish_fixture_directory_noreplace(&staging_path, entry) {
        let _ = fs::remove_dir_all(&staging_path);
        if !is_published_fixture(entry)? {
            return Err(error)
                .with_context(|| format!("atomically publish keyed fixture {}", entry.display()));
        }
        validate_keyed_fixture(entry, key).map_err(|validation_error| {
            error.context(format!(
                "atomically publish keyed fixture {}; concurrent entry is invalid: {validation_error:#}",
                entry.display()
            ))
        })?;
    } else {
        validate_keyed_fixture(entry, key)?;
    }
    retire_older_fixture_entries(family_root, key)
}

fn is_published_fixture(entry: &Path) -> Result<bool> {
    match fs::symlink_metadata(entry) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("inspect keyed fixture entry {}", entry.display()))
        }
    }
}

/// Removes every family entry but `current`, including abandoned staging and
/// retirement directories. The caller holds the exclusive family lock, so no
/// reader is copying a retired entry; each is renamed out of its key before
/// removal, so an interrupted retirement never leaves a partial entry at a key.
fn retire_older_fixture_entries(family_root: &Path, current: &str) -> Result<()> {
    let retired = tempfile::Builder::new()
        .prefix(KEYED_FIXTURE_RETIRED)
        .tempdir_in(family_root)?;
    let retired_name = retired.path().file_name().map(OsStr::to_os_string);
    fs::read_dir(family_root)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .filter(|name| name != current && Some(name) != retired_name.as_ref())
        .try_for_each(|name| fs::rename(family_root.join(&name), retired.path().join(&name)))
        .with_context(|| format!("retire older keyed fixtures in {}", family_root.display()))?;
    retired
        .close()
        .with_context(|| format!("remove retired keyed fixtures in {}", family_root.display()))
}

fn validate_keyed_fixture(entry: &Path, expected_key: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(entry)?;
    if !metadata.file_type().is_dir() {
        bail!(
            "keyed fixture entry {} is occupied by a non-directory",
            entry.display()
        );
    }
    let manifest_path = entry.join("manifest.json");
    let manifest: KeyedFixtureManifest = serde_json::from_slice(
        &fs::read(&manifest_path)
            .with_context(|| format!("read keyed fixture manifest {}", manifest_path.display()))?,
    )
    .context("parse keyed fixture manifest")?;
    if manifest.schema_version != KEYED_FIXTURE_SCHEMA || manifest.key != expected_key {
        bail!(
            "keyed fixture manifest identity mismatch at {}",
            manifest_path.display()
        );
    }
    let actual = tree_sha256(&entry.join(KEYED_FIXTURE_TREE))?;
    if actual != manifest.tree_sha256 {
        bail!(
            "keyed fixture content mismatch at {}: expected {}, found {}",
            entry.display(),
            manifest.tree_sha256,
            actual
        );
    }
    Ok(())
}

fn write_captured_package(package: &ProfilePackage, destination: &Path) -> Result<()> {
    package.files().iter().try_for_each(|(relative, bytes)| {
        let path = destination.join(relative);
        fs::create_dir_all(path.parent().unwrap_or(destination))?;
        fs::write(&path, bytes)?;
        set_captured_mode(&path, package.model().file_mode(relative))
    })
}

fn tree_sha256(root: &Path) -> Result<String> {
    enum TreeEntry {
        Directory,
        File(FileMode, Vec<u8>),
    }

    fn visit(
        root: &Path,
        directory: &Path,
        entries: &mut BTreeMap<PathBuf, TreeEntry>,
    ) -> Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let relative = path.strip_prefix(root)?.to_path_buf();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                bail!("fixture tree contains symbolic link {}", path.display());
            }
            if metadata.is_dir() {
                entries.insert(relative, TreeEntry::Directory);
                visit(root, &path, entries)?;
            } else if metadata.is_file() {
                #[cfg(unix)]
                let mode = {
                    use std::os::unix::fs::PermissionsExt;
                    if metadata.permissions().mode() & 0o111 == 0 {
                        FileMode::Regular
                    } else {
                        FileMode::Executable
                    }
                };
                #[cfg(not(unix))]
                let mode = FileMode::Regular;
                entries.insert(relative, TreeEntry::File(mode, fs::read(&path)?));
            } else {
                bail!("fixture tree contains irregular entry {}", path.display());
            }
        }
        Ok(())
    }

    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() {
        bail!("fixture repository {} is not a directory", root.display());
    }
    let mut entries = BTreeMap::new();
    visit(root, root, &mut entries)?;
    let mut hasher = Sha256::new();
    for (relative, entry) in entries {
        hash_field(&mut hasher, relative.to_string_lossy().as_bytes());
        match entry {
            TreeEntry::Directory => hash_field(&mut hasher, b"directory"),
            TreeEntry::File(mode, bytes) => {
                hash_field(
                    &mut hasher,
                    match mode {
                        FileMode::Regular => b"regular",
                        FileMode::Executable => b"executable",
                    },
                );
                hash_field(&mut hasher, &bytes);
            }
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// Write an executable a test then runs, while other threads may fork.
///
/// A child forked while this process holds a file open for writing keeps a copy
/// of that descriptor until it execs, and exec of the file fails with `ETXTBSY`
/// meanwhile. So the file ends up as an inode only a child process ever wrote:
/// that child has exited, and no descriptor of this process ever named the inode.
#[cfg(unix)]
pub fn write_executable(path: &Path, contents: impl AsRef<[u8]>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
    let mut sealed = path.as_os_str().to_owned();
    sealed.push(".sealed");
    let copied = std::process::Command::new("cp")
        .arg("-p")
        .arg(path)
        .arg(&sealed)
        .status()?;
    anyhow::ensure!(copied.success(), "cp {} failed: {copied}", path.display());
    fs::rename(&sealed, path)?;
    Ok(())
}

/// Deep-copies the ordinary files and directories under `source` into the
/// existing `destination`, preserving modes; any other entry is refused.
pub fn copy_tree_exact(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "fixture tree contains symbolic link {}",
                source_path.display()
            );
        }
        if metadata.is_dir() {
            fs::create_dir(&destination_path)?;
            copy_tree_exact(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            fs::copy(&source_path, &destination_path)?;
            fs::set_permissions(&destination_path, metadata.permissions())?;
        } else {
            bail!(
                "fixture tree contains irregular entry {}",
                source_path.display()
            );
        }
    }
    Ok(())
}

/// Standard test repository setup with .jit and .git directories
///
/// Creates a temporary directory with initialized jit storage.
/// Returns both the TempDir (which cleans up on drop) and the storage instance.
pub fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> {
    let temp = TempDir::new()?;

    let jit_root = temp.path().join(".jit");
    let storage = JsonFileStorage::new(&jit_root);
    let layout = discover_repository_layout(temp.path(), &jit_root)?;
    CommandExecutor::new(storage.clone())
        .with_layout(layout)
        .profiles()
        .initialize_fresh_repository(temp.path(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, storage))
}

/// Seed the data root's engine-owned ignore file at `data_root` with the exact
/// bytes its producer composes.
///
/// A fixture that assembles a data root by hand rather than through
/// initialization is missing derived state every real repository carries, which
/// whole-repository validation reports as drift. Seeding it from the producer's
/// own rendering keeps the fixture free of a hand-maintained copy
/// (`@/inv/single-source-prose`).
pub fn seed_data_root_ignore(data_root: &Path) -> Result<()> {
    let bytes = crate::repository_state::render_data_root_ignore(b"")?;
    fs::write(data_root.join(".gitignore"), bytes)?;
    Ok(())
}

/// Append well-formed events for unrelated issues until the log exceeds
/// `bytes`, so a capture over it is decided by nothing but its own tail.
pub fn grow_event_log_beyond(storage: &JsonFileStorage, bytes: u64) -> Result<()> {
    use std::io::Write;

    let path = storage.root().join("events.jsonl");
    let mut log = std::io::BufWriter::new(
        fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?,
    );
    let mut written = fs::metadata(&path)?.len();
    // One serialized event is the template; each line differs from it by the
    // event id and the issue id, so the log grows without per-line serialization.
    let template_issue = crate::domain::Issue::draft(
        issue_id(0),
        "Accumulated history".to_string(),
        String::new(),
    );
    let template_issue_id = template_issue.id.to_string();
    let template_event_id = "01J00000000000000000000000".to_string();
    let template = serde_json::to_string(&stamped_event(
        crate::domain::EventBody::draft_issue_created(&template_issue),
        template_event_id.clone(),
        chrono::Utc::now(),
    ))?;
    let mut ordinal = 0_u64;
    while written <= bytes {
        let line = template
            .replace(&template_issue_id, issue_id(u128::from(ordinal)).as_ref())
            .replace(
                &template_event_id,
                &format!("01J0000000000000000000{ordinal:04}"),
            );
        writeln!(log, "{line}")?;
        written += line.len() as u64 + 1;
        ordinal += 1;
    }
    log.flush()?;
    Ok(())
}

/// An executor over the file-backed repository rooted at `worktree`, whose data
/// root is `worktree/.jit`, carrying the layout discovered for that pair.
///
/// Opens what exists; it neither creates nor initialises the repository.
pub fn file_executor(worktree: &Path) -> Result<CommandExecutor<JsonFileStorage>> {
    let jit_root = worktree.join(".jit");
    let layout = discover_repository_layout(worktree, &jit_root)?;
    Ok(CommandExecutor::new(JsonFileStorage::new(&jit_root)).with_layout(layout))
}

/// Build an initialised file-backed repository whose `.jit/config.toml` is
/// `config`, written before initialization so the initializer's
/// existing-configuration path derives the repository's coupled rules and
/// schemas from the caller's declarations.
///
/// Returns the temporary directory (removed on drop) and the executor over it.
pub fn setup_test_repo_with_config(
    config: &str,
) -> Result<(TempDir, CommandExecutor<JsonFileStorage>)> {
    let temp = TempDir::new()?;
    fs::create_dir_all(temp.path().join(".jit"))?;
    fs::write(temp.path().join(".jit/config.toml"), config)?;

    let executor = file_executor(temp.path())?;
    executor
        .profiles()
        .initialize_fresh_repository(temp.path(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, executor))
}

/// Configuration fragment projecting the gate registry to `docs/gates.md`.
pub const GATE_PROJECTION_CONFIG: &str = "\n[item_kinds.gate]\nsection = \"success_criteria\"\n\
id-pattern = \"[a-z][a-z0-9-]*\"\nmarkers = []\nlink-namespaces = [\"enforces\"]\n\
scope = \"project\"\nsource = { toml = \".jit/gates.toml\", table = \"gates\", \
id-field = \"key\", text-field = \"description\" }\nsource-of-truth = \"registry-first\"\n\n\
[projection.gates]\nkind = \"gate\"\nmode = \"separate-file\"\n\
target = \"docs/gates.md\"\nstyle = \"id-anchor\"\n";

/// Package-manifest fragment declaring a gate, which leaves a rendered
/// [`GATE_PROJECTION_CONFIG`] projection stale.
pub const NEW_GATE_CONTRIBUTION: &str = "\n[[contribution]]\nkind = \"keyed-array\"\n\
target = \"gates\"\nidentity = \"new-gate\"\nvalue = { version = 1, key = \"new-gate\", \
title = \"New\", description = \"New gate\", stage = \"postcheck\", mode = \"manual\" }\n";

/// A temporary directory holding exactly `files`, each a worktree-relative path
/// (parents created) with its contents.
///
/// Fails before writing when a path is absolute or has a `..`, root or prefix
/// component, since it would leave the directory.
pub fn declaration_dir(files: &[(&str, &str)]) -> Result<TempDir> {
    if let Some((escaping, _)) = files.iter().find(|(relative, _)| {
        !Path::new(relative)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    }) {
        anyhow::bail!("declaration path '{escaping}' is not confined to its directory");
    }
    let dir = TempDir::new()?;
    files.iter().try_for_each(|(relative, contents)| {
        let path = dir.path().join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, contents).map_err(anyhow::Error::from)
    })?;
    Ok(dir)
}

/// Build a repository from the fixture's explicitly declared custom taxonomy.
///
/// The returned taxonomy is the same value used to author the configuration,
/// allowing callers to build labels and assertions without repeating vocabulary
/// literals.
pub fn setup_test_repo_with_taxonomy() -> Result<(TempDir, JsonFileStorage, TestTaxonomy)> {
    let taxonomy = test_taxonomy();
    let (temp, executor) = setup_test_repo_with_config(&taxonomy.config_fragment())?;
    Ok((temp, executor.storage().clone(), taxonomy))
}

#[cfg(test)]
mod file_setup_tests {
    use super::*;

    const CALLER_CONFIG: &str = "[worktree]\nenforce_leases = \"off\"\n";

    #[test]
    fn test_setup_test_repo_with_config_keeps_the_callers_configuration() {
        let (temp, _executor) = setup_test_repo_with_config(CALLER_CONFIG).unwrap();

        let config: toml::Value =
            toml::from_str(&fs::read_to_string(temp.path().join(".jit/config.toml")).unwrap())
                .unwrap();
        assert_eq!(config["worktree"]["enforce_leases"].as_str(), Some("off"));
    }

    #[test]
    fn test_setup_test_repo_with_config_yields_an_initialised_repository() {
        let (_temp, executor) = setup_test_repo_with_config(CALLER_CONFIG).unwrap();

        executor
            .validation()
            .validate_repository_report()
            .unwrap()
            .structural_failure
            .map_or(Ok(()), Err)
            .unwrap();
    }

    #[test]
    fn test_file_executor_layout_is_the_discovered_layout_of_the_repository() {
        let (temp, _storage) = setup_test_repo().unwrap();

        let executor = file_executor(temp.path()).unwrap();

        let discovered = discover_repository_layout(temp.path(), temp.path().join(".jit")).unwrap();
        assert_eq!(executor.repository_layout().unwrap(), &discovered);
        assert_eq!(executor.storage().root(), temp.path().join(".jit"));
    }

    #[test]
    fn test_declaration_dir_holds_exactly_the_supplied_files() {
        let dir = declaration_dir(&[("rules.toml", "a"), ("nested/gates.toml", "b")]).unwrap();

        let mut found = Vec::new();
        let mut pending = vec![dir.path().to_path_buf()];
        while let Some(current) = pending.pop() {
            for entry in fs::read_dir(&current).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    pending.push(path);
                } else {
                    let relative = path.strip_prefix(dir.path()).unwrap().to_path_buf();
                    found.push((relative, fs::read_to_string(&path).unwrap()));
                }
            }
        }
        found.sort();
        assert_eq!(
            found,
            vec![
                (PathBuf::from("nested/gates.toml"), "b".to_string()),
                (PathBuf::from("rules.toml"), "a".to_string()),
            ]
        );
    }

    #[test]
    fn test_declaration_dir_rejects_paths_leaving_the_directory_before_writing() {
        let outside = TempDir::new().unwrap();
        let absolute = outside.path().join("escaped.toml");
        let absolute = absolute.to_str().unwrap();
        let traversing = Path::new("..")
            .join(outside.path().file_name().unwrap())
            .join("escaped-by-traversal.toml");
        let traversing = traversing.to_str().unwrap();

        [absolute, traversing].iter().for_each(|escaping| {
            let error = declaration_dir(&[("inside.toml", "a"), (escaping, "b")]).unwrap_err();

            assert!(error.to_string().contains(escaping), "{error}");
        });

        assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    }

    #[test]
    fn test_declaration_dir_without_files_is_empty() {
        let dir = declaration_dir(&[]).unwrap();

        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
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

/// This repository's profile package `id`, captured from its checkout and
/// written at `destination`.
///
/// One route serves every test that needs this repository's own package. The
/// capture itself is the product's, [`capture_package_tree`]: the manifest and
/// the sources the package authors itself come from the checked-in package
/// directory named after `id` under [`PROFILE_PACKAGE_SOURCES`], and every live
/// asset's bytes come from the repository file its declaration targets, so a
/// caller reads the checkout's current repository files.
///
/// What this fixture supplies around it is the write: the product publishes a
/// captured tree through a repository's own mutation session, and a test that
/// wants this checkout's package in a temporary directory has no session over
/// this checkout to publish through — nor may it open one, since that would
/// make a test a writer of the developer's repository.
///
/// `destination` belongs to the caller. That is what lets a test which applies
/// the package name a directory inside the repository it applies it to: an
/// application records the package's worktree-relative location and refuses a
/// package read from outside the worktree. A caller that only reads
/// declarations names a temporary directory it owns.
///
/// Each call captures afresh and replaces `destination` whole, so calling twice
/// at one destination answers with the checkout's state at each call. Nothing
/// is cached between calls: a cached tree would answer from the state at the
/// first call, and this repository's managed-region and executable-mode
/// contracts assert about the checkout at the moment they read it, so a stale
/// answer would report an agreement that no longer holds.
///
/// # Errors
///
/// Every failure [`capture_package_tree`] reports — a declared source absent
/// from either side, a source that is a symbolic link, is not an ordinary file
/// or resolves outside the worktree, a manifest that does not parse, and
/// content that does not validate as a package — plus the filesystem failures
/// of writing the tree and reading it back.
pub fn capture_repository_package(id: &str, destination: &Path) -> Result<ProfilePackage> {
    let checkout = repository_checkout();
    let layout = discover_repository_layout(&checkout, checkout.join(".jit"))?;
    let source = VirtualPath::worktree(Path::new(PROFILE_PACKAGE_SOURCES).join(id))?;
    let captured = capture_package_tree(&source, &layout)?;

    if fs::symlink_metadata(destination).is_ok() {
        fs::remove_dir_all(destination)?;
    }
    write_captured_package(&captured, destination)?;
    Ok(ProfilePackage::from_directory(destination)?)
}

/// Publish one captured file's declared mode, on the platforms that carry one.
#[cfg(unix)]
fn set_captured_mode(path: &Path, mode: FileMode) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let bits = match mode {
        FileMode::Executable => 0o755,
        FileMode::Regular => 0o644,
    };
    fs::set_permissions(path, fs::Permissions::from_mode(bits))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_captured_mode(_path: &Path, _mode: FileMode) -> Result<()> {
    Ok(())
}
const SHIPPED_PACKAGE_ID: &str = "jit-default";

/// The item-kind declarations `manifest` contributes, keyed by kind name.
///
/// Read from the package model's `item-kinds` map-entry contributions, so a
/// kind, alias, mode or id pattern exists here only where the manifest says so.
///
/// Panics when a contribution's value is not an item-kind declaration.
pub fn item_kinds_contributed_by(
    manifest: &ProfilePackageModel,
) -> BTreeMap<String, ItemKindConfig> {
    manifest
        .contributions
        .iter()
        .filter_map(|contribution| match contribution {
            Contribution::MapEntry {
                target: MapEntryTarget::ItemKinds,
                identity,
                value,
            } => Some((identity, value)),
            _ => None,
        })
        .map(|(identity, value)| {
            let kind = serde_json::from_value(value.clone())
                .unwrap_or_else(|error| panic!("item kind '{identity}' does not parse: {error}"));
            (identity.clone(), kind)
        })
        .collect()
}

/// The shipped item kinds in typed form, read from the default package's
/// manifest in this checkout.
///
/// Panics when the manifest cannot be read or does not parse as a package.
pub fn shipped_item_kinds() -> BTreeMap<String, ItemKindConfig> {
    let path = repository_checkout()
        .join(PROFILE_PACKAGE_SOURCES)
        .join(SHIPPED_PACKAGE_ID)
        .join("manifest.toml");
    let bytes = fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let manifest = ProfilePackage::parse_manifest(&bytes)
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    item_kinds_contributed_by(&manifest)
}

/// The shipped declaration of kind `name`.
///
/// Panics when the default package ships no such kind.
pub fn shipped_item_kind(name: &str) -> ItemKindConfig {
    shipped_item_kinds()
        .remove(name)
        .unwrap_or_else(|| panic!("the default package ships no '{name}' kind"))
}

/// The shipped item kinds as the `[item_kinds.<kind>]` tables of a repository
/// `config.toml`, serialized from [`shipped_item_kinds`].
pub fn shipped_item_kinds_toml() -> String {
    item_kinds_toml(shipped_item_kinds())
}

/// The `[item_kinds.<kind>]` tables of the named shipped kinds only.
///
/// Panics when the default package ships no kind of one of the names.
pub fn shipped_item_kind_tables(names: &[&str]) -> String {
    item_kinds_toml(
        names
            .iter()
            .map(|name| (name.to_string(), shipped_item_kind(name)))
            .collect(),
    )
}

fn item_kinds_toml(item_kinds: BTreeMap<String, ItemKindConfig>) -> String {
    #[derive(Serialize)]
    struct ConfigKinds {
        item_kinds: BTreeMap<String, ItemKindConfig>,
    }
    toml::to_string(&ConfigKinds { item_kinds }).expect("a declared item kind serializes to TOML")
}

#[cfg(test)]
mod shipped_item_kinds_tests {
    use super::*;

    fn default_manifest_bytes() -> Vec<u8> {
        let path = repository_checkout()
            .join(PROFILE_PACKAGE_SOURCES)
            .join(SHIPPED_PACKAGE_ID)
            .join("manifest.toml");
        fs::read(path).unwrap()
    }

    #[test]
    fn test_item_kinds_contributed_by_manifest_edit_changes_derivation() {
        let original = String::from_utf8(default_manifest_bytes()).unwrap();
        let before = item_kinds_contributed_by(
            &ProfilePackage::parse_manifest(original.as_bytes()).unwrap(),
        );
        let (kind, declaration) = before.iter().next().expect("the package ships kinds");
        let pattern = declaration.id_pattern.clone().unwrap();
        let edited = original.replacen(
            &format!("id-pattern = \"{pattern}\""),
            "id-pattern = \"EDITED-[0-9]+\"",
            1,
        );
        assert_ne!(edited, original, "the edit must land in the manifest");

        let after =
            item_kinds_contributed_by(&ProfilePackage::parse_manifest(edited.as_bytes()).unwrap());

        assert_ne!(before, after);
        assert_eq!(after[kind].id_pattern.as_deref(), Some("EDITED-[0-9]+"));
    }

    #[test]
    fn test_shipped_item_kinds_toml_round_trips_to_typed_form() {
        let config: crate::config::JitConfig = toml::from_str(&shipped_item_kinds_toml()).unwrap();

        let declared: BTreeMap<_, _> = config.item_kinds.unwrap().into_iter().collect();
        assert!(!declared.is_empty());
        assert_eq!(declared, shipped_item_kinds());
    }
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
/// does not capture, which is a defect in the checkout rather than a condition
/// a test distinguishes.
pub fn stage_repository_packages(worktree: &Path, id: &str) -> PathBuf {
    let staged = worktree.join(PROFILE_PACKAGE_SOURCES);
    published_package_ids().into_iter().for_each(|package| {
        capture_repository_package(&package, &staged.join(&package)).unwrap_or_else(|error| {
            panic!("this repository's {package} package captures: {error}")
        });
    });
    let location = staged.join(id);
    assert!(
        location.is_dir(),
        "this repository authors no profile package '{id}'"
    );
    location
}

/// This repository's profile package `id` captured into a temporary
/// destination, answered with the directory that owns it.
///
/// The fixture over [`capture_repository_package`] for a caller that reads a
/// package's declarations and has no repository to publish it into. The
/// returned directory holds the published tree, so the package's recorded
/// source stays readable for as long as the caller keeps it; a caller that
/// applies the package names its own destination and calls the entry point
/// directly.
///
/// Panics when the package does not capture, which is a defect in the checkout
/// rather than a condition a test distinguishes.
pub fn temporary_repository_package(id: &str) -> (TempDir, ProfilePackage) {
    let workspace = TempDir::new().expect("create a package destination");
    let package = capture_repository_package(id, &workspace.path().join(id))
        .unwrap_or_else(|error| panic!("this repository's {id} package captures: {error}"));
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

/// Apply what `selectors` name, with no variable inputs, the way `jit profile
/// apply` does.
pub fn apply_selected_packages<S: IssueStore + crate::storage::RepositoryStateStore>(
    executor: &CommandExecutor<S>,
    selectors: &[PathBuf],
) -> Result<crate::profile::ProfileComposedApplyResult> {
    executor
        .profiles()
        .select_profiles(
            selectors,
            &crate::commands::ProfileVariableOptions::default(),
        )
        .and_then(|selection| executor.profiles().apply_profiles(&selection))
}

/// Move `id` to `state` the way `jit issue update --state` does.
pub fn transition_issue<S: IssueStore + crate::storage::RepositoryStateStore>(
    executor: &CommandExecutor<S>,
    id: &IssueId,
    state: crate::domain::State,
) -> Result<Vec<String>> {
    executor.issues().update_issue(
        id,
        None,
        None,
        None,
        Some(state),
        Vec::new(),
        Vec::new(),
        None,
        None,
        false,
    )
}

/// The selector naming the directory `package` was read from.
fn selector_of(package: &crate::profile::ProfilePackage) -> PathBuf {
    package
        .directory()
        .expect("the selector names a package read from a directory")
        .to_path_buf()
}

/// Apply `package` and the packages it depends on.
pub fn apply_package<S: IssueStore + crate::storage::RepositoryStateStore>(
    executor: &CommandExecutor<S>,
    package: &crate::profile::ProfilePackage,
) -> Result<crate::profile::ProfileComposedApplyResult> {
    apply_selected_packages(executor, &[selector_of(package)])
}

/// The packages selecting `package` reaches, dependency first.
pub fn selected_closure<S: IssueStore + crate::storage::RepositoryStateStore>(
    executor: &CommandExecutor<S>,
    package: &crate::profile::ProfilePackage,
) -> Result<Vec<crate::profile::ProfilePackage>> {
    executor
        .profiles()
        .select_profiles(
            &[selector_of(package)],
            &crate::commands::ProfileVariableOptions::default(),
        )
        .map(|selection| {
            selection
                .members()
                .iter()
                .map(|member| member.package().clone())
                .collect()
        })
}

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
        .map(|dependency| format!("\n[[dependency]]\nid = \"{dependency}\"\nversion = \"*\"\n"))
        .collect::<String>();
    let rewritten = source.assets.iter().fold(
        authored.replace(
            &format!("id = \"{}\"", source.id),
            &format!("id = \"{id}\""),
        ) + &declared,
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

/// Directory levels each asset of [`write_package_tree_at_model_limits`] sits
/// below, sharing none of them with another asset.
///
/// Every level multiplies the distinct directories a publication of the package
/// has to enumerate, and the package model bounds none of them, so this is the
/// dimension along which a route carrying an assumed path shape breaks. Nine
/// path components is the shape reported against `jit profile add`.
pub const UNSHARED_ASSET_DIRECTORY_LEVELS: usize = 8;

/// Write a package tree at the limits the package model permits, and return
/// `root`.
///
/// The shape a route carrying a whole package has to survive, authored once
/// because more than one such route must be held to it
/// (`@/inv/shared-test-contracts`). Three limits at once:
///
/// - the file count is exactly [`MAX_PROFILE_PACKAGE_FILES`], the manifest
///   included, so nothing that scales per file has headroom left;
/// - the total size sits just under [`MAX_PROFILE_PACKAGE_BYTES`], filled by
///   spreading the budget the manifest does not occupy across the assets;
/// - every declared source is long enough that a tar writer cannot fit its name
///   in a header field, so each file costs a long-name entry as well.
///
/// Each asset also sits at the end of a chain of [`UNSHARED_ASSET_DIRECTORY_LEVELS`]
/// directories no other asset shares, so the package's distinct directory count
/// is a multiple of its file count rather than bounded by it. That is the
/// property the package model leaves unconstrained — it bounds file count and
/// total size, and says nothing about path depth — so a route whose cost scales
/// with directories rather than with files fails here and nowhere else.
///
/// [`MAX_PROFILE_PACKAGE_FILES`]: crate::profile::MAX_PROFILE_PACKAGE_FILES
/// [`MAX_PROFILE_PACKAGE_BYTES`]: crate::profile::MAX_PROFILE_PACKAGE_BYTES
pub fn write_package_tree_at_model_limits(root: &Path) -> PathBuf {
    let assets = crate::profile::MAX_PROFILE_PACKAGE_FILES - 1;
    // Long enough that the composed source path exceeds the 100 bytes a tar
    // header holds, which is what forces the long-name entry.
    let padding = "x".repeat(70);
    let source = |index: usize| {
        (0..UNSHARED_ASSET_DIRECTORY_LEVELS)
            .map(|level| format!("level-{level}-{index:03}/"))
            .chain(std::iter::once(format!("source-{padding}-{index:03}.txt")))
            .collect::<String>()
    };
    let manifest = std::iter::once(
        "[profile]\nmanifest-version = 2\nid = \"bounded-package\"\nversion = \"1.0.0\"\n\
         compatible-jit = \">=0.2.0, <2.0.0\"\n"
            .to_string(),
    )
    .chain((0..assets).map(|index| {
        format!(
            "\n[[asset]]\nsource = \"{}\"\ntarget = \"out/target-{padding}-{index:03}.txt\"\n",
            source(index)
        )
    }))
    .collect::<String>();

    let filler = crate::profile::MAX_PROFILE_PACKAGE_BYTES
        .saturating_sub(manifest.len())
        .saturating_div(assets);
    fs::create_dir_all(root).expect("create the bounded package root");
    fs::write(root.join("manifest.toml"), &manifest).expect("write the bounded package manifest");
    for index in 0..assets {
        let path = root.join(source(index));
        fs::create_dir_all(path.parent().expect("an asset has a parent"))
            .expect("create the bounded package asset directory");
        fs::write(&path, vec![b'.'; filler]).expect("write a bounded package asset");
    }
    root.to_path_buf()
}

/// Copy an on-disk profile package tree to `root`, creating it and every
/// declared parent, and return `root`.
///
/// A test that applies a package must read it from inside the repository it is
/// applied to, because a package is addressed by its worktree-relative
/// location.
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

/// Create test WorktreePaths from a TempDir, as a primary checkout.
pub fn create_test_paths(temp: &TempDir) -> WorktreePaths {
    primary_checkout_paths(temp.path())
}

/// The detected paths of a primary checkout at `root`.
pub fn primary_checkout_paths(root: impl AsRef<Path>) -> WorktreePaths {
    linked_checkout_paths(&root, &root)
}

/// The detected paths of the checkout at `checkout`, whose primary is at `primary`.
pub fn linked_checkout_paths(
    primary: impl AsRef<Path>,
    checkout: impl AsRef<Path>,
) -> WorktreePaths {
    let (primary, checkout) = (primary.as_ref(), checkout.as_ref());
    WorktreePaths {
        git_repository: true,
        common_dir: primary.join(".git"),
        worktree_root: checkout.to_path_buf(),
        local_jit: checkout.join(".jit"),
        shared_jit: primary.join(".git/jit"),
        relocated_worktree_checkout: None,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn test_write_executable_runs_while_other_threads_fork() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let forking = AtomicBool::new(true);
        let dir = TempDir::new().unwrap();
        std::thread::scope(|scope| {
            (0..8).for_each(|_| {
                scope.spawn(|| {
                    while forking.load(Ordering::Relaxed) {
                        std::process::Command::new("true").status().unwrap();
                    }
                });
            });
            let outcomes = (0..200)
                .map(|index| {
                    let script = dir.path().join(format!("script-{index}"));
                    write_executable(&script, "#!/bin/sh\nexit 0\n").unwrap();
                    std::process::Command::new(&script).status()
                })
                .collect::<Vec<_>>();
            forking.store(false, Ordering::Relaxed);
            assert!(
                outcomes.iter().all(|outcome| outcome.is_ok()),
                "a script failed to start: {outcomes:?}"
            );
        });
    }

    use super::*;
    use crate::repository_state::{BOOTSTRAP_LOCK_FILE, RECORD_SCRATCH_DIR};
    use crate::storage::IssueStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    /// The shipped package that carries more than a manifest, which is the one
    /// whose capture actually draws content out of the checkout.
    ///
    /// Derived rather than named: a package this repository stops shipping, or
    /// one that stops declaring sources, moves the answer instead of leaving a
    /// literal here that no longer points at a package with content.
    fn package_carrying_drawn_content() -> String {
        let workspace = TempDir::new().expect("a package destination");
        published_package_ids()
            .into_iter()
            .find(|id| {
                capture_repository_package(id, &workspace.path().join(id))
                    .unwrap_or_else(|error| panic!("{id} does not capture: {error}"))
                    .file_count()
                    > 1
            })
            .expect("a shipped package carries more than a manifest")
    }

    #[test]
    fn test_create_test_paths_uses_owned_temporary_directory_as_worktree_root() {
        let temp = TempDir::new().unwrap();

        assert_eq!(create_test_paths(&temp).worktree_root, temp.path());
    }

    /// Every package this repository ships captures under the id its source
    /// directory is named after, which is what makes an id a sufficient way to
    /// name one.
    #[test]
    fn test_capture_repository_package_answers_the_shipped_package_its_id_names() {
        let shipped = published_package_ids();
        assert!(
            shipped.len() > 1,
            "this repository ships one package source directory, so a rule over \
             them says nothing about naming: {shipped:?}"
        );

        let workspace = TempDir::new().unwrap();
        let misnamed: Vec<(String, String)> = shipped
            .iter()
            .map(|id| {
                let package = capture_repository_package(id, &workspace.path().join(id))
                    .unwrap_or_else(|error| panic!("{id} does not capture: {error}"));
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
    /// Tests run in the crate directory, below the checkout root, so a
    /// resolution relative to the working directory would find no package
    /// sources there.
    #[test]
    fn test_capture_repository_package_resolves_the_checkout_independently_of_the_working_directory(
    ) {
        let checkout = repository_checkout();
        assert!(
            checkout.is_absolute(),
            "a relative checkout would be resolved against the working directory: {}",
            checkout.display()
        );

        let workspace = TempDir::new().unwrap();
        let id = package_carrying_drawn_content();
        let destination = workspace.path().join("package");

        let package = capture_repository_package(&id, &destination)
            .expect("the package captures from the crate directory");
        assert_eq!(package.model().id.as_str(), id);
        assert!(
            package.file_count() > 1,
            "the captured package carries only a manifest, so a capture that \
             read nothing from the checkout would pass this"
        );
    }

    /// A second call at one destination is answered rather than refused, and
    /// what it leaves there is one whole tree.
    ///
    /// A fixture that merged into its previous destination would leave a tree
    /// the manifest does not describe, which is what reading the destination
    /// back as a package catches.
    #[test]
    fn test_capture_repository_package_republishes_over_its_own_previous_destination() {
        let workspace = TempDir::new().unwrap();
        let destination = workspace.path().join("package");
        let id = package_carrying_drawn_content();

        let first = capture_repository_package(&id, &destination).unwrap();
        let second = capture_repository_package(&id, &destination).unwrap();

        assert_eq!(second.package_hash(), first.package_hash());
        let reread = ProfilePackage::from_directory(&destination)
            .expect("the republished destination holds a package");
        assert_eq!(reread.package_hash(), second.package_hash());
        assert_eq!(reread.file_count(), second.file_count());
    }

    #[test]
    fn test_profiled_repository_fixture_memory_build_writes_the_tree_a_file_build_writes() {
        PROFILED_REPOSITORY_FIXTURES.iter().for_each(|spec| {
            let packages = capture_profile_package_closure(spec.id).unwrap();
            let package_directory = Path::new(spec.package_directory);
            let root = TempDir::new().unwrap();
            let built = root.path().join("built/repository");
            let reference = root.path().join("reference/repository");
            fs::create_dir_all(&built).unwrap();
            fs::create_dir_all(&reference).unwrap();
            initialize_profiled_repository_fixture(&built, spec.id, package_directory, &packages)
                .unwrap();

            packages.iter().for_each(|(package_id, package)| {
                write_captured_package(package, &reference.join(package_directory).join(package_id))
                    .unwrap()
            });
            let jit_root = reference.join(".jit");
            let executor = CommandExecutor::new(JsonFileStorage::new(&jit_root))
                .with_layout(discover_repository_layout(&reference, &jit_root).unwrap());
            let selection = executor
                .profiles()
                .select_profiles(
                    &[reference.join(package_directory).join(spec.id)],
                    &crate::commands::ProfileVariableOptions::default(),
                )
                .unwrap();
            executor
                .profiles()
                .initialize_fresh_repository(&reference, Some(&selection))
                .unwrap();
            // Machine-local state the file backend's next session creates on demand.
            fs::remove_file(reference.join(BOOTSTRAP_LOCK_FILE)).unwrap();
            let scratch = jit_root.join(RECORD_SCRATCH_DIR);
            if scratch.exists() {
                fs::remove_dir(scratch).unwrap();
            }

            // Each initialization mints its own event ids and timestamps.
            let events = |repository: &Path| -> Vec<serde_json::Value> {
                fs::read_to_string(repository.join(".jit/events.jsonl"))
                    .unwrap()
                    .lines()
                    .map(|line| {
                        let mut event: serde_json::Value = serde_json::from_str(line).unwrap();
                        let fields = event.as_object_mut().unwrap();
                        fields.remove("id");
                        fields.remove("timestamp");
                        event
                    })
                    .collect()
            };
            assert_eq!(events(&built), events(&reference), "{}", spec.id);
            fs::copy(
                reference.join(".jit/events.jsonl"),
                built.join(".jit/events.jsonl"),
            )
            .unwrap();
            assert_eq!(
                tree_sha256(&built).unwrap(),
                tree_sha256(&reference).unwrap(),
                "{}",
                spec.id
            );
        });
    }

    #[test]
    fn test_keyed_fixture_publication_is_concurrent_and_corruption_fails_closed() {
        let cache = TempDir::new().unwrap();
        let cache_root = cache.path().to_path_buf();
        let builds = Arc::new(AtomicUsize::new(0));
        let start = Arc::new(Barrier::new(2));
        let workers = (0..2)
            .map(|_| {
                let cache_root = cache_root.clone();
                let builds = Arc::clone(&builds);
                let start = Arc::clone(&start);
                std::thread::spawn(move || {
                    start.wait();
                    ensure_keyed_fixture(&cache_root, "family", "same-key", |repository| {
                        builds.fetch_add(1, Ordering::SeqCst);
                        fs::write(repository.join("state"), b"coherent")?;
                        Ok(())
                    })
                    .unwrap()
                    .entry
                })
            })
            .collect::<Vec<_>>();
        let entries = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(entries[0], entries[1]);
        assert_eq!(
            fs::read(entries[0].join("tree/state")).unwrap(),
            b"coherent"
        );
        let repeated = ensure_keyed_fixture(&cache_root, "family", "same-key", |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(repeated.entry, entries[0]);
        drop(repeated);
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "an equivalent later consumer must reuse the same publication"
        );

        fs::write(entries[0].join("tree/state"), b"corrupt").unwrap();
        let error = ensure_keyed_fixture(&cache_root, "family", "same-key", |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();
        assert!(
            error.to_string().contains("content mismatch"),
            "unexpected corruption refusal: {error:#}"
        );
        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert_eq!(
            fs::read(entries[0].join("tree/state")).unwrap(),
            b"corrupt",
            "a corrupt immutable entry must not be silently replaced"
        );
    }

    #[test]
    fn test_keyed_fixture_cold_concurrent_consumers_build_once_within_budget() {
        const CONSUMERS: usize = 6;
        let cache = TempDir::new().unwrap();
        let builds = AtomicUsize::new(0);
        let start = Barrier::new(CONSUMERS);
        let began = std::time::Instant::now();
        let entries = std::thread::scope(|scope| {
            (0..CONSUMERS)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        ensure_keyed_fixture(cache.path(), "family", "cold-key", |repository| {
                            builds.fetch_add(1, Ordering::SeqCst);
                            std::thread::sleep(Duration::from_millis(200));
                            fs::write(repository.join("state"), b"built")?;
                            Ok(())
                        })
                        .unwrap()
                        .entry
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|consumer| consumer.join().unwrap())
                .collect::<Vec<_>>()
        });

        assert_eq!(builds.load(Ordering::SeqCst), 1);
        assert!(entries.windows(2).all(|pair| pair[0] == pair[1]));
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn test_keyed_fixture_failing_builds_each_report_their_own_cause() {
        const CONSUMERS: usize = 6;
        let cache = TempDir::new().unwrap();
        let start = Barrier::new(CONSUMERS);
        let builds = AtomicUsize::new(0);

        std::thread::scope(|scope| {
            (0..CONSUMERS)
                .map(|consumer| {
                    let (cache, start, builds) = (&cache, &start, &builds);
                    scope.spawn(move || {
                        start.wait();
                        let error = ensure_keyed_fixture(cache.path(), "family", "key", |_| {
                            builds.fetch_add(1, Ordering::SeqCst);
                            bail!("consumer {consumer}'s cause")
                        })
                        .unwrap_err();
                        (consumer, format!("{error:#}"))
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .for_each(|(consumer, error)| {
                    assert!(
                        error.contains(&format!("consumer {consumer}'s cause")),
                        "{error}"
                    );
                });
        });

        assert_eq!(builds.load(Ordering::SeqCst), CONSUMERS);
    }

    #[test]
    fn test_keyed_fixture_waiter_builds_after_its_holder_fails() {
        let cache = TempDir::new().unwrap();
        let held = Barrier::new(2);

        let lease = std::thread::scope(|scope| {
            let holder = scope.spawn(|| {
                ensure_keyed_fixture(cache.path(), "family", "key", |_| {
                    held.wait();
                    std::thread::sleep(Duration::from_millis(200));
                    bail!("the holder's cause")
                })
            });
            held.wait();
            let waiter = scope.spawn(|| {
                ensure_keyed_fixture(cache.path(), "family", "key", |repository| {
                    fs::write(repository.join("state"), b"built")?;
                    Ok(())
                })
            });
            assert!(holder.join().unwrap().is_err());
            waiter.join().unwrap().unwrap()
        });

        assert_eq!(fs::read(lease.entry.join("tree/state")).unwrap(), b"built");
    }

    #[test]
    fn test_keyed_fixture_waiter_builds_when_the_holder_dies_without_an_outcome() {
        let cache = TempDir::new().unwrap();
        let lock = keyed_fixture_lock(cache.path(), "family");
        fs::create_dir_all(lock.parent().unwrap()).unwrap();
        let holder = FileLocker::new(Duration::from_secs(1))
            .lock_exclusive(&lock)
            .unwrap();
        let built = AtomicUsize::new(0);

        let lease = std::thread::scope(|scope| {
            let waiter = scope.spawn(|| {
                ensure_keyed_fixture(cache.path(), "family", "key", |repo| {
                    built.fetch_add(1, Ordering::SeqCst);
                    fs::write(repo.join("state"), b"built")?;
                    Ok(())
                })
            });
            std::thread::sleep(Duration::from_millis(200));
            assert!(!waiter.is_finished(), "the holder still owns the build");
            drop(holder);
            waiter.join().unwrap().unwrap()
        });

        assert_eq!(built.load(Ordering::SeqCst), 1);
        assert!(lease.entry.join("tree/state").is_file());
    }

    #[test]
    fn test_keyed_fixture_refuses_an_occupied_publication_path() {
        let cache = TempDir::new().unwrap();
        let occupied = cache.path().join("family/occupied");
        fs::create_dir_all(occupied.parent().unwrap()).unwrap();
        fs::write(&occupied, b"not a fixture").unwrap();
        let built = AtomicUsize::new(0);

        let error = ensure_keyed_fixture(cache.path(), "family", "occupied", |_| {
            built.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();

        assert!(
            error.to_string().contains("occupied by a non-directory"),
            "unexpected occupied-path refusal: {error:#}"
        );
        assert_eq!(built.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(&occupied).unwrap(), b"not a fixture");
    }

    #[test]
    fn test_keyed_fixture_no_replace_preserves_raced_occupied_directory() {
        let cache = TempDir::new().unwrap();
        let family_root = cache.path().join("family");
        let occupied = family_root.join("raced");

        let error = ensure_keyed_fixture(cache.path(), "family", "raced", |repository| {
            fs::write(repository.join("state"), b"candidate")?;
            fs::create_dir(&occupied)?;
            fs::write(occupied.join("bystander"), b"wins")?;
            Ok(())
        })
        .unwrap_err();

        assert!(
            format!("{error:#}").contains("atomically publish"),
            "unexpected raced-occupant refusal: {error:#}"
        );
        assert_eq!(fs::read(occupied.join("bystander")).unwrap(), b"wins");
        assert!(
            fs::read_dir(&family_root)
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(KEYED_FIXTURE_STAGING)),
            "a refused publication must clean its private staging directory"
        );
    }

    #[test]
    fn test_keyed_fixture_uses_a_valid_concurrent_publication() {
        let cache = TempDir::new().unwrap();
        let entry = cache.path().join("family/raced-key");
        let winner = ensure_keyed_fixture(cache.path(), "family", "raced-key", |repository| {
            fs::write(repository.join("state"), b"candidate")?;
            let winner_repository = entry.join(KEYED_FIXTURE_TREE);
            fs::create_dir_all(&winner_repository)?;
            fs::write(winner_repository.join("state"), b"winner")?;
            let manifest = KeyedFixtureManifest {
                schema_version: KEYED_FIXTURE_SCHEMA,
                key: "raced-key".to_string(),
                tree_sha256: tree_sha256(&winner_repository)?,
            };
            fs::write(
                entry.join("manifest.json"),
                serde_json::to_vec_pretty(&manifest)?,
            )?;
            Ok(())
        })
        .unwrap();

        assert_eq!(winner.entry, entry);
        assert_eq!(
            fs::read(winner.entry.join("tree/state")).unwrap(),
            b"winner"
        );
        assert!(
            fs::read_dir(cache.path().join("family"))
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(KEYED_FIXTURE_STAGING)),
            "a losing builder must clean up its private staging directory"
        );
    }

    #[test]
    fn test_profiled_repository_fixture_clones_are_isolated_and_profiled() {
        let first = profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES).unwrap();
        let second = profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES).unwrap();
        let profile_asset = Path::new(".agents/skills/jit-manage/SKILL.md");

        assert!(first.path().join(profile_asset).is_file());
        assert_eq!(
            fs::read(first.path().join(profile_asset)).unwrap(),
            fs::read(second.path().join(profile_asset)).unwrap()
        );
        fs::write(first.path().join("AGENTS.md"), b"mutated clone").unwrap();
        assert_ne!(
            fs::read(first.path().join("AGENTS.md")).unwrap(),
            fs::read(second.path().join("AGENTS.md")).unwrap(),
            "fixture clones must not share mutable files"
        );

        let third = profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES).unwrap();
        assert_eq!(
            fs::read(second.path().join("AGENTS.md")).unwrap(),
            fs::read(third.path().join("AGENTS.md")).unwrap(),
            "mutating a clone must not alter the immutable baseline"
        );
    }

    #[test]
    fn test_profiled_in_memory_repository_fixture_preserves_modes_and_clone_isolation() {
        use crate::storage::IssueStore;

        const EXECUTABLE: &str =
            ".agents/skills/jit-execution-lead/scripts/check-leak-into-main.sh";
        const REGULAR: &str = ".agents/skills/jit-manage/SKILL.md";
        let (first_source, first) = profiled_in_memory_repository_fixture("jit-dogfood").unwrap();
        let (second_source, second) = profiled_in_memory_repository_fixture("jit-dogfood").unwrap();

        assert_eq!(
            first.read_repo_file(EXECUTABLE).unwrap(),
            Some(fs::read_to_string(first_source.path().join(EXECUTABLE)).unwrap())
        );
        assert_eq!(
            first.repository_file_mode_fixture(EXECUTABLE).unwrap(),
            FileMode::Executable,
            "the memory adapter must retain executable profile assets"
        );
        assert_eq!(
            second.repository_file_mode_fixture(EXECUTABLE).unwrap(),
            FileMode::Executable
        );
        assert_eq!(
            first.repository_file_mode_fixture(REGULAR).unwrap(),
            FileMode::Regular
        );
        assert_eq!(
            first.read_repo_file(REGULAR).unwrap(),
            Some(fs::read_to_string(first_source.path().join(REGULAR)).unwrap()),
            "the memory adapter must retain the baseline's exact regular profile asset"
        );

        first.add_worktree_file("AGENTS.md", "mutated memory clone");
        assert_ne!(
            first.read_repo_file("AGENTS.md").unwrap(),
            second.read_repo_file("AGENTS.md").unwrap(),
            "memory fixture clones must not share mutable aggregate state"
        );
        assert_eq!(
            second.read_repo_file("AGENTS.md").unwrap(),
            Some(fs::read_to_string(second_source.path().join("AGENTS.md")).unwrap())
        );
    }

    #[test]
    fn test_keyed_fixture_cache_root_follows_runtime_artifact_layouts() {
        let workspace = TempDir::new().unwrap();
        let default_target = workspace.path().join("default-target");
        let absolute_custom_target = TempDir::new().unwrap();
        let relative_custom_target = workspace.path().join("relative-custom-target");
        let cases = [
            (default_target.join("debug/jit"), default_target.clone()),
            (
                absolute_custom_target.path().join("debug/deps/consumer"),
                absolute_custom_target.path().to_path_buf(),
            ),
            (
                relative_custom_target.join("debug/jit"),
                relative_custom_target.clone(),
            ),
            (
                relative_custom_target.join("x86_64-unknown-linux-gnu/debug/deps/consumer"),
                relative_custom_target.join("x86_64-unknown-linux-gnu"),
            ),
        ];

        for (artifact, expected_target_context) in cases {
            fs::create_dir_all(artifact.parent().unwrap()).unwrap();
            fs::write(&artifact, b"fixture runtime artifact").unwrap();
            assert_eq!(
                keyed_fixture_cache_root(&artifact).unwrap(),
                expected_target_context.join(KEYED_FIXTURE_DIRECTORY),
                "{} must select the cache context Cargo placed it under",
                artifact.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_keyed_fixture_cache_root_refuses_non_ordinary_runtime_artifact() {
        use std::os::unix::fs::symlink;

        let workspace = TempDir::new().unwrap();
        let target = workspace.path().join("debug/jit");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        symlink(std::env::current_exe().unwrap(), &target).unwrap();

        let error = keyed_fixture_cache_root(&target).unwrap_err();
        assert!(error.to_string().contains("must be an ordinary file"));
    }

    #[test]
    fn test_profiled_repository_fixture_key_changes_with_a_package_byte() {
        let packages = capture_profile_package_closure("jit-dogfood").unwrap();
        let mut files = packages
            .iter()
            .flat_map(|(package_id, package)| {
                package.files().iter().map(move |(relative, bytes)| {
                    (
                        package_id.clone(),
                        relative.clone(),
                        package.model().file_mode(relative),
                        bytes.clone(),
                    )
                })
            })
            .collect::<Vec<_>>();
        let key = |files: &[(String, String, FileMode, Vec<u8>)]| {
            profiled_repository_fixture_key(
                &package_closure_sha256(files.iter().map(|(package_id, relative, mode, bytes)| {
                    (
                        package_id.as_str(),
                        relative.as_str(),
                        *mode,
                        bytes.as_slice(),
                    )
                })),
                CRATE_SOURCE_SHA256,
            )
        };
        let captured = key(&files);
        assert_eq!(
            profiled_repository_fixture_inputs("jit-dogfood")
                .unwrap()
                .key,
            captured,
            "the fixture must key on this closure and the compiled crate source"
        );

        files[0].3[0] ^= 1;
        assert_ne!(key(&files), captured);
    }

    #[test]
    fn test_profiled_repository_fixture_key_changes_with_a_crate_source_byte() {
        use crate::crate_source_digest::{crate_source_sha256, CRATE_SOURCE_ROOTS};

        let crate_root = TempDir::new().unwrap();
        fs::create_dir(crate_root.path().join("src")).unwrap();
        [
            ("Cargo.toml", "[package]\n"),
            ("build.rs", "fn main() {}\n"),
            ("src/lib.rs", "pub fn setup() {}\n"),
        ]
        .iter()
        .for_each(|(path, contents)| fs::write(crate_root.path().join(path), contents).unwrap());
        let key = || {
            profiled_repository_fixture_key(
                "package-closure",
                &crate_source_sha256(crate_root.path(), &CRATE_SOURCE_ROOTS).unwrap(),
            )
        };
        let original = key();
        assert_eq!(key(), original);

        fs::write(
            crate_root.path().join("src/lib.rs"),
            "pub fn setup() {}\n\n",
        )
        .unwrap();
        assert_ne!(key(), original);
    }

    #[test]
    fn test_keyed_fixture_publication_retires_the_profiles_older_entries() {
        let cache = TempDir::new().unwrap();
        let publish = |profile: &str, key: &str| {
            ensure_keyed_fixture(cache.path(), profile, key, |repository| {
                fs::write(repository.join("state"), key)?;
                Ok(())
            })
            .unwrap()
            .entry
        };
        publish("family", "first");
        let neighbour = publish("neighbour", "first");
        let abandoned = cache
            .path()
            .join("family")
            .join(format!("{KEYED_FIXTURE_STAGING}abandoned"));
        fs::create_dir(&abandoned).unwrap();

        let current = publish("family", "second");

        let remaining = fs::read_dir(cache.path().join("family"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(remaining, [current]);
        validate_keyed_fixture(&neighbour, "first").unwrap();
    }

    #[test]
    fn test_keyed_fixture_retirement_waits_for_a_concurrent_reader() {
        let cache = TempDir::new().unwrap();
        let write_key = |key: &'static str| {
            move |repository: &Path| -> Result<()> {
                fs::write(repository.join("state"), key)?;
                Ok(())
            }
        };
        let reader =
            ensure_keyed_fixture(cache.path(), "family", "current", write_key("current")).unwrap();

        let current = reader.entry.clone();
        let next = std::thread::scope(|scope| {
            let next = scope
                .spawn(|| ensure_keyed_fixture(cache.path(), "family", "next", write_key("next")));
            std::thread::sleep(Duration::from_millis(200));
            assert!(!next.is_finished(), "retirement must wait for the reader");
            let clone = TempDir::new().unwrap();
            copy_tree_exact(&reader.entry.join(KEYED_FIXTURE_TREE), clone.path()).unwrap();
            assert_eq!(fs::read(clone.path().join("state")).unwrap(), b"current");
            drop(reader);
            next.join().unwrap().unwrap()
        });
        assert_eq!(fs::read(next.entry.join("tree/state")).unwrap(), b"next");
        assert!(!current.exists(), "the released entry must be retired");
    }

    #[test]
    fn test_setup_test_repo_with_taxonomy_declares_and_exposes_exact_vocabulary() {
        let (_temp, storage, taxonomy) = setup_test_repo_with_taxonomy().unwrap();
        let config = crate::config::JitConfig::load(&storage.repository_layout().unwrap()).unwrap();
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
