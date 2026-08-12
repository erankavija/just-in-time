//! Shared test utilities
//!
//! Common helpers used across multiple test modules to reduce duplication.

#![cfg(any(test, feature = "test-support"))]

use crate::commands::CommandExecutor;
use crate::commands::ProfileSelector;
use crate::profile::{capture_package_tree, CapturedPackageTree, ProfilePackage};
use crate::repository_state::{FileMode, VirtualPath};
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{discover_repository_layout, FileLocker, JsonFileStorage};
use crate::test_taxonomy::{test_taxonomy, TestTaxonomy};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime};
use tempfile::TempDir;

/// Serializes tests that temporarily change the process-wide working directory.
///
/// The mutex is recovered after poisoning because a panic while a guard is
/// alive runs its `Drop` implementation before the mutex guard is released,
/// so the directory has already been restored when another test acquires it.
static CURRENT_DIR_LOCK: Mutex<()> = Mutex::new(());

/// Serializes unit-test receipt-environment mutations with in-process fixture
/// consumers. Nextest setup and its test consumers are separate processes, so
/// this guard is deliberately absent outside `cfg(test)` builds.
#[cfg(test)]
static PROFILED_REPOSITORY_FIXTURE_ENV_LOCK: Mutex<()> = Mutex::new(());

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

const PROFILED_REPOSITORY_FIXTURE_SCHEMA: u32 = 1;
const PROFILED_REPOSITORY_FIXTURE_DIRECTORY: &str = "jit-profiled-repository-fixtures";
const PROFILED_REPOSITORY_FIXTURE_SETUP: &str = "command-executor.initialize-fresh-repository.v1";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfiledRepositoryFixtureManifest {
    schema_version: u32,
    key: String,
    tree_sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfiledRepositoryFixtureReceipt {
    schema_version: u32,
    run_id: String,
    entries: BTreeMap<String, PathBuf>,
}

struct VerifiedFixtureReceipt {
    path: PathBuf,
    sha256: String,
}

const NEXTEST_DEFAULT_REPOSITORY_RECEIPT: &str = "JIT_DEFAULT_REPOSITORY_FIXTURE_RECEIPT";
const NEXTEST_DOGFOOD_REPOSITORY_RECEIPT: &str = "JIT_DOGFOOD_REPOSITORY_FIXTURE_RECEIPT";

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

#[derive(Default)]
struct DirectProfiledRepositoryFixtures {
    entries: BTreeMap<String, PathBuf>,
    executable_digests: BTreeMap<OrdinaryFileIdentity, String>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct OrdinaryFileIdentity {
    canonical_path: PathBuf,
    length: u64,
    modified: SystemTime,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    change_seconds: i64,
    #[cfg(unix)]
    change_nanoseconds: i64,
}

struct ExecutableProvenance {
    canonical_path: PathBuf,
    sha256: String,
}

static DIRECT_PROFILED_REPOSITORY_FIXTURES: Mutex<Option<DirectProfiledRepositoryFixtures>> =
    Mutex::new(None);

/// A freshly initialized repository carrying this checkout's profile package.
///
/// The expensive initialization is published once as an immutable baseline in
/// the Cargo target directory. Its identity includes every captured package
/// file in the selected dependency closure (path, bytes, and mode), the selected
/// profile, a setup-contract token, the current test executable that contains
/// the setup implementation, and an optional scenario executable. Every caller
/// receives an ordinary deep copy in a fresh temporary directory, so process-
/// level tests retain real filesystem and CLI boundaries without sharing
/// mutable repository state.
///
/// Publication is serialized across nextest processes and is one atomic
/// directory rename. An occupied or corrupted entry is an error rather than a
/// reason to rebuild in place: source or executable changes select a new key,
/// while damage to an entry with the same key fails closed.
///
/// CLI integration tests pass `CARGO_BIN_EXE_jit` as `scenario_executable`;
/// in-process tests pass `None`.
pub fn profiled_repository_fixture(
    id: &str,
    package_directory: &str,
    scenario_executable: Option<&Path>,
) -> Result<TempDir> {
    #[cfg(test)]
    let nextest_entry = {
        let _receipt_environment_lock = PROFILED_REPOSITORY_FIXTURE_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        nextest_profiled_repository_fixture(id, package_directory)?
    };
    #[cfg(not(test))]
    let nextest_entry = nextest_profiled_repository_fixture(id, package_directory)?;
    if let Some(entry) = nextest_entry {
        let clone = TempDir::new()?;
        copy_tree_exact(&entry.join("repository"), clone.path())?;
        return Ok(clone);
    }

    let entry = direct_profiled_repository_fixture(id, package_directory, scenario_executable)?;
    let clone = TempDir::new()?;
    copy_tree_exact(&entry.join("repository"), clone.path())?;
    Ok(clone)
}

/// A verified applied-profile repository imported into isolated memory state.
///
/// The returned temporary repository owns both the package locations used by
/// applied-profile provenance and the file-backed baseline clone. Keep it
/// alive for as long as commands use the accompanying memory store.
pub fn profiled_in_memory_repository_fixture(
    id: &str,
) -> Result<(TempDir, crate::storage::InMemoryStorage)> {
    let spec = PROFILED_REPOSITORY_FIXTURES
        .iter()
        .find(|spec| spec.id == id)
        .with_context(|| format!("unsupported profiled-repository fixture '{id}'"))?;
    let repository = profiled_repository_fixture(id, spec.package_directory, None)?;
    let storage = crate::storage::InMemoryStorage::rooted_at(repository.path());
    storage.seed_repository_tree_fixture(repository.path())?;
    Ok((repository, storage))
}

fn direct_profiled_repository_fixture(
    id: &str,
    package_directory: &str,
    scenario_executable: Option<&Path>,
) -> Result<PathBuf> {
    let mut state = DIRECT_PROFILED_REPOSITORY_FIXTURES
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = state.get_or_insert_with(DirectProfiledRepositoryFixtures::default);
    let scenario_provenance = scenario_executable
        .map(|executable| executable_provenance(executable, &mut state.executable_digests))
        .transpose()?;
    let entry_key = direct_entry_key(id, package_directory, scenario_provenance.as_ref());
    if let Some(entry) = state.entries.get(&entry_key) {
        return Ok(entry.clone());
    }

    let packages = capture_profile_package_closure(id)?;
    let package_root = VirtualPath::worktree(package_directory)?;
    let fixture_setup = std::env::current_exe().context("resolve fixture setup executable")?;
    let key = profiled_repository_fixture_key(
        id,
        package_root.relative().as_path(),
        &fixture_setup,
        scenario_provenance.as_ref(),
        &packages,
    )?;
    let cache_root = profiled_repository_fixture_cache_root(&fixture_setup)?;
    let entry = ensure_profiled_repository_fixture(&cache_root, &key, |repository| {
        initialize_profiled_repository_fixture(
            repository,
            id,
            package_root.relative().as_path(),
            &packages,
        )
    })?;
    state.entries.insert(entry_key, entry.clone());
    Ok(entry)
}

fn direct_entry_key(
    id: &str,
    package_directory: &str,
    scenario_executable: Option<&ExecutableProvenance>,
) -> String {
    format!(
        "{}\0{}\0{}",
        receipt_entry_key(id, package_directory),
        scenario_executable
            .map(|provenance| provenance.canonical_path.to_string_lossy())
            .as_deref()
            .unwrap_or("<none>"),
        scenario_executable
            .map(|provenance| provenance.sha256.as_str())
            .unwrap_or("<none>")
    )
}

fn executable_provenance(
    executable: &Path,
    digests: &mut BTreeMap<OrdinaryFileIdentity, String>,
) -> Result<ExecutableProvenance> {
    let identity = ordinary_file_identity(executable)?;
    if let Some(sha256) = digests.get(&identity) {
        return Ok(ExecutableProvenance {
            canonical_path: identity.canonical_path,
            sha256: sha256.clone(),
        });
    }

    let mut hasher = Sha256::new();
    hash_file(&mut hasher, executable)
        .with_context(|| format!("hash scenario executable {}", executable.display()))?;
    let after_hash = ordinary_file_identity(executable)?;
    if identity != after_hash {
        bail!(
            "scenario executable {} changed while its provenance was captured",
            executable.display()
        );
    }
    let sha256 = format!("{:x}", hasher.finalize());
    digests.insert(identity.clone(), sha256.clone());
    Ok(ExecutableProvenance {
        canonical_path: identity.canonical_path,
        sha256,
    })
}

fn ordinary_file_identity(path: &Path) -> Result<OrdinaryFileIdentity> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspect scenario executable {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "scenario executable {} must be an ordinary file",
            path.display()
        );
    }
    Ok(OrdinaryFileIdentity {
        canonical_path: fs::canonicalize(path)
            .with_context(|| format!("canonicalize scenario executable {}", path.display()))?,
        length: metadata.len(),
        modified: metadata
            .modified()
            .with_context(|| format!("read modification time for {}", path.display()))?,
        #[cfg(unix)]
        device: metadata.dev(),
        #[cfg(unix)]
        inode: metadata.ino(),
        #[cfg(unix)]
        change_seconds: metadata.ctime(),
        #[cfg(unix)]
        change_nanoseconds: metadata.ctime_nsec(),
    })
}

/// Prepare the verified immutable repository fixtures for one nextest run.
///
/// Nextest executes this once, before any matching test processes. The receipt
/// is bound to that run's own id and exported through `NEXTEST_ENV`; consumers
/// therefore skip repeated source and executable hashing but cannot accept a
/// receipt copied from another run. Direct libtest execution has no receipt and
/// takes the fully verified builder path above.
fn prepare_profiled_repository_fixture_receipt(
    id: &str,
    run_id: &str,
    setup_executable: &Path,
) -> Result<VerifiedFixtureReceipt> {
    let spec = PROFILED_REPOSITORY_FIXTURES
        .iter()
        .find(|spec| spec.id == id)
        .with_context(|| format!("unsupported profiled-repository fixture '{id}'"))?;
    let packages = capture_profile_package_closure(spec.id)?;
    let package_directory = VirtualPath::worktree(spec.package_directory)?;
    let key = profiled_repository_fixture_key(
        spec.id,
        package_directory.relative().as_path(),
        setup_executable,
        None,
        &packages,
    )?;
    let cache_root = profiled_repository_fixture_cache_root(setup_executable)?;
    let entry = ensure_profiled_repository_fixture(&cache_root, &key, |repository| {
        initialize_profiled_repository_fixture(
            repository,
            spec.id,
            package_directory.relative().as_path(),
            &packages,
        )
    })?;
    validate_profiled_repository_fixture(&entry, &key)?;
    let receipt = ProfiledRepositoryFixtureReceipt {
        schema_version: PROFILED_REPOSITORY_FIXTURE_SCHEMA,
        run_id: run_id.to_string(),
        entries: BTreeMap::from([(receipt_entry_key(spec.id, spec.package_directory), entry)]),
    };
    let receipt_path = cache_root.join(format!("run-{run_id}-{}.json", spec.id));
    let receipt_bytes = serde_json::to_vec(&receipt)?;
    let receipt_sha256 = format!("{:x}", Sha256::digest(&receipt_bytes));
    let receipt_stage = tempfile::NamedTempFile::new_in(&cache_root)?;
    fs::write(receipt_stage.path(), receipt_bytes)?;
    receipt_stage.as_file().sync_all()?;
    crate::storage::publish_fixture_file_noreplace(receipt_stage.path(), &receipt_path)?;
    Ok(VerifiedFixtureReceipt {
        path: receipt_path,
        sha256: receipt_sha256,
    })
}

/// Prepare one selected fixture receipt for nextest.
#[doc(hidden)]
pub fn prepare_nextest_profiled_repository_fixture(id: &str) -> Result<()> {
    let run_id = std::env::var("NEXTEST_RUN_ID").context("nextest setup has no run id")?;
    let nextest_env = PathBuf::from(
        std::env::var_os("NEXTEST_ENV").context("nextest setup has no environment output file")?,
    );
    let setup_executable = std::env::current_exe().context("resolve nextest setup executable")?;
    let receipt = prepare_profiled_repository_fixture_receipt(id, &run_id, &setup_executable)?;
    use std::io::Write;
    writeln!(
        fs::OpenOptions::new().append(true).open(nextest_env)?,
        "{}={}\n{}_SHA256={}",
        receipt_environment_name(id).context("fixture id has no receipt environment")?,
        receipt.path.display(),
        receipt_environment_name(id).context("fixture id has no receipt environment")?,
        receipt.sha256,
    )?;
    Ok(())
}

fn nextest_profiled_repository_fixture(
    id: &str,
    package_directory: &str,
) -> Result<Option<PathBuf>> {
    let runtime_artifact = std::env::current_exe()
        .context("resolve profiled-repository fixture consumer executable")?;
    nextest_profiled_repository_fixture_for_artifact(id, package_directory, &runtime_artifact)
}

fn nextest_profiled_repository_fixture_for_artifact(
    id: &str,
    package_directory: &str,
    runtime_artifact: &Path,
) -> Result<Option<PathBuf>> {
    let Some(receipt_environment) = receipt_environment_name(id) else {
        return Ok(None);
    };
    let Some(receipt_path) = std::env::var_os(receipt_environment) else {
        return Ok(None);
    };
    let run_id =
        std::env::var("NEXTEST_RUN_ID").context("fixture receipt has no nextest run id")?;
    let receipt_path = PathBuf::from(receipt_path);
    let metadata = fs::symlink_metadata(&receipt_path)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("profiled-repository fixture receipt is not an ordinary file");
    }
    let receipt_bytes = fs::read(&receipt_path)?;
    let expected_sha256 = std::env::var(format!("{receipt_environment}_SHA256"))
        .context("fixture receipt has no content identity")?;
    let actual_sha256 = format!("{:x}", Sha256::digest(&receipt_bytes));
    if actual_sha256 != expected_sha256 {
        bail!("profiled-repository fixture receipt content mismatch");
    }
    let receipt: ProfiledRepositoryFixtureReceipt = serde_json::from_slice(&receipt_bytes)?;
    if receipt.schema_version != PROFILED_REPOSITORY_FIXTURE_SCHEMA || receipt.run_id != run_id {
        bail!("profiled-repository fixture receipt does not belong to this nextest run");
    }
    let entry = receipt
        .entries
        .get(&receipt_entry_key(id, package_directory))
        .with_context(|| format!("nextest fixture receipt has no {id} at {package_directory}"))?;
    let cache_root = profiled_repository_fixture_cache_root(runtime_artifact)?;
    if entry.parent() != Some(cache_root.as_path()) {
        bail!("profiled-repository fixture receipt names an entry outside its cache");
    }
    let entry_metadata = fs::symlink_metadata(entry)?;
    if !entry_metadata.file_type().is_dir() || entry_metadata.file_type().is_symlink() {
        bail!("profiled-repository fixture receipt names a non-directory entry");
    }
    let manifest_path = entry.join("manifest.json");
    let manifest_metadata = fs::symlink_metadata(&manifest_path)?;
    if !manifest_metadata.file_type().is_file() || manifest_metadata.file_type().is_symlink() {
        bail!("profiled-repository fixture manifest is not an ordinary file");
    }
    let manifest: ProfiledRepositoryFixtureManifest =
        serde_json::from_slice(&fs::read(manifest_path)?)?;
    if manifest.schema_version != PROFILED_REPOSITORY_FIXTURE_SCHEMA
        || entry.file_name().and_then(|name| name.to_str()) != Some(manifest.key.as_str())
        || !entry.join("repository").is_dir()
    {
        bail!("profiled-repository fixture receipt names an invalid cache entry");
    }
    Ok(Some(entry.clone()))
}

fn receipt_environment_name(id: &str) -> Option<&'static str> {
    match id {
        "jit-default" => Some(NEXTEST_DEFAULT_REPOSITORY_RECEIPT),
        "jit-dogfood" => Some(NEXTEST_DOGFOOD_REPOSITORY_RECEIPT),
        _ => None,
    }
}

fn receipt_entry_key(id: &str, package_directory: &str) -> String {
    format!("{id}\0{package_directory}")
}

fn initialize_profiled_repository_fixture(
    repository: &Path,
    id: &str,
    package_directory: &Path,
    packages: &BTreeMap<String, CapturedPackageTree>,
) -> Result<()> {
    let package_root = repository.join(package_directory);
    packages.iter().try_for_each(|(package_id, package)| {
        write_captured_package(package, &package_root.join(package_id))
    })?;
    let selected = package_root.join(id);
    if !selected.is_dir() {
        bail!("this repository authors no profile package '{id}'");
    }
    let jit_root = repository.join(".jit");
    let storage = JsonFileStorage::new(&jit_root);
    let layout = discover_repository_layout(repository, &jit_root)?;
    CommandExecutor::new(storage)
        .with_layout(layout)
        .initialize_fresh_repository(repository, Some(&[ProfileSelector::path(&selected)]))?;
    Ok(())
}

fn capture_profile_package_closure(id: &str) -> Result<BTreeMap<String, CapturedPackageTree>> {
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

fn profiled_repository_fixture_key(
    id: &str,
    package_directory: &Path,
    setup_executable: &Path,
    scenario_executable: Option<&ExecutableProvenance>,
    packages: &BTreeMap<String, CapturedPackageTree>,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hash_field(
        &mut hasher,
        &PROFILED_REPOSITORY_FIXTURE_SCHEMA.to_le_bytes(),
    );
    hash_field(&mut hasher, PROFILED_REPOSITORY_FIXTURE_SETUP.as_bytes());
    hash_field(&mut hasher, std::env::consts::OS.as_bytes());
    hash_field(&mut hasher, id.as_bytes());
    hash_field(&mut hasher, package_directory.to_string_lossy().as_bytes());
    hash_file(&mut hasher, setup_executable).with_context(|| {
        format!(
            "hash profiled-repository fixture setup executable {}",
            setup_executable.display()
        )
    })?;
    if let Some(scenario_executable) = scenario_executable {
        hash_field(
            &mut hasher,
            scenario_executable
                .canonical_path
                .to_string_lossy()
                .as_bytes(),
        );
        hash_field(&mut hasher, scenario_executable.sha256.as_bytes());
    }
    for (package_id, package) in packages {
        hash_field(&mut hasher, package_id.as_bytes());
        for (relative, file) in package.files() {
            hash_field(&mut hasher, relative.as_bytes());
            hash_field(
                &mut hasher,
                match file.mode {
                    FileMode::Regular => b"regular",
                    FileMode::Executable => b"executable",
                },
            );
            hash_field(&mut hasher, &file.bytes);
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Derive the cache context Cargo used to place a running artifact.
///
/// Cargo puts binaries directly under a profile directory and test executables
/// under that profile's `deps` directory. Stripping those two layout segments
/// produces either the target directory or a target-triple directory. This
/// uses the artifact Cargo actually ran, so it stays stable when a reused
/// executable embeds a different checkout in `CARGO_MANIFEST_DIR` or when a
/// relative `CARGO_TARGET_DIR` was resolved from a different process directory.
fn profiled_repository_fixture_cache_root(runtime_artifact: &Path) -> Result<PathBuf> {
    let metadata = fs::symlink_metadata(runtime_artifact).with_context(|| {
        format!(
            "inspect profiled-repository fixture runtime artifact {}",
            runtime_artifact.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        bail!(
            "profiled-repository fixture runtime artifact {} must be an ordinary file",
            runtime_artifact.display()
        );
    }
    let artifact = fs::canonicalize(runtime_artifact).with_context(|| {
        format!(
            "canonicalize profiled-repository fixture runtime artifact {}",
            runtime_artifact.display()
        )
    })?;
    let artifact_directory = artifact.parent().with_context(|| {
        format!(
            "profiled-repository fixture runtime artifact {} has no parent directory",
            artifact.display()
        )
    })?;
    let profile_directory = if artifact_directory.file_name() == Some("deps".as_ref()) {
        artifact_directory.parent().with_context(|| {
            format!(
                "profiled-repository fixture deps directory {} has no profile parent",
                artifact_directory.display()
            )
        })?
    } else {
        artifact_directory
    };
    let target_context = profile_directory.parent().with_context(|| {
        format!(
            "profiled-repository fixture profile directory {} has no target parent",
            profile_directory.display()
        )
    })?;
    Ok(target_context.join(PROFILED_REPOSITORY_FIXTURE_DIRECTORY))
}

fn ensure_profiled_repository_fixture(
    cache_root: &Path,
    key: &str,
    build: impl FnOnce(&Path) -> Result<()>,
) -> Result<PathBuf> {
    fs::create_dir_all(cache_root)?;
    let lock_root = cache_root.join("locks");
    fs::create_dir_all(&lock_root)?;
    let _lock = FileLocker::new(Duration::from_secs(120))
        .lock_exclusive(&lock_root.join(format!("{key}.lock")))?;
    let entry = cache_root.join(key);
    match fs::symlink_metadata(&entry) {
        Ok(_) => {
            validate_profiled_repository_fixture(&entry, key)?;
            return Ok(entry);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "inspect profiled-repository fixture entry {}",
                    entry.display()
                )
            });
        }
    }

    let staging = tempfile::Builder::new()
        .prefix(".profiled-repository-staging-")
        .tempdir_in(cache_root)?;
    let repository = staging.path().join("repository");
    fs::create_dir(&repository)?;
    build(&repository)?;
    let tree_sha256 = tree_sha256(&repository)?;
    let manifest = ProfiledRepositoryFixtureManifest {
        schema_version: PROFILED_REPOSITORY_FIXTURE_SCHEMA,
        key: key.to_string(),
        tree_sha256,
    };
    fs::write(
        staging.path().join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;

    let staging_path = staging.keep();
    if let Err(error) = crate::storage::publish_fixture_directory_noreplace(&staging_path, &entry) {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(error).with_context(|| {
            format!(
                "atomically publish profiled-repository fixture {}",
                entry.display()
            )
        });
    }
    validate_profiled_repository_fixture(&entry, key)?;
    Ok(entry)
}

fn validate_profiled_repository_fixture(entry: &Path, expected_key: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(entry)?;
    if !metadata.file_type().is_dir() {
        bail!(
            "profiled-repository fixture entry {} is occupied by a non-directory",
            entry.display()
        );
    }
    let manifest_path = entry.join("manifest.json");
    let manifest: ProfiledRepositoryFixtureManifest =
        serde_json::from_slice(&fs::read(&manifest_path).with_context(|| {
            format!(
                "read profiled-repository fixture manifest {}",
                manifest_path.display()
            )
        })?)
        .context("parse profiled-repository fixture manifest")?;
    if manifest.schema_version != PROFILED_REPOSITORY_FIXTURE_SCHEMA || manifest.key != expected_key
    {
        bail!(
            "profiled-repository fixture manifest identity mismatch at {}",
            manifest_path.display()
        );
    }
    let actual = tree_sha256(&entry.join("repository"))?;
    if actual != manifest.tree_sha256 {
        bail!(
            "profiled-repository fixture content mismatch at {}: expected {}, found {}",
            entry.display(),
            manifest.tree_sha256,
            actual
        );
    }
    Ok(())
}

fn write_captured_package(package: &CapturedPackageTree, destination: &Path) -> Result<()> {
    package.files().iter().try_for_each(|(relative, file)| {
        let path = destination.join(relative);
        fs::create_dir_all(path.parent().unwrap_or(destination))?;
        fs::write(&path, &file.bytes)?;
        set_captured_mode(&path, file.mode)
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

fn hash_file(hasher: &mut Sha256, path: &Path) -> Result<()> {
    let mut file = fs::File::open(path)?;
    hash_field(hasher, &file.metadata()?.len().to_le_bytes());
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        hasher.update(&buffer[..read]);
    }
}

fn copy_tree_exact(source: &Path, destination: &Path) -> Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.file_type().is_symlink() {
            bail!(
                "profiled-repository fixture contains symbolic link {}",
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
                "profiled-repository fixture contains irregular entry {}",
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
    for (relative, file) in captured.files() {
        let path = destination.join(relative);
        fs::create_dir_all(path.parent().unwrap_or(destination))?;
        fs::write(&path, &file.bytes)?;
        set_captured_mode(&path, file.mode)?;
    }
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
        "[profile]\nmanifest-version = 1\nid = \"bounded-package\"\nversion = \"1.0.0\"\n\
         jit = \">=0.2.0, <2.0.0\"\n"
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
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};

    struct EnvironmentRestore {
        values: Vec<(String, Option<std::ffi::OsString>)>,
    }

    impl EnvironmentRestore {
        fn set(values: impl IntoIterator<Item = (String, std::ffi::OsString)>) -> Self {
            let values = values
                .into_iter()
                .map(|(name, value)| {
                    let previous = std::env::var_os(&name);
                    std::env::set_var(&name, value);
                    (name, previous)
                })
                .collect();
            Self { values }
        }

        fn remove(names: impl IntoIterator<Item = String>) -> Self {
            let values = names
                .into_iter()
                .map(|name| {
                    let previous = std::env::var_os(&name);
                    std::env::remove_var(&name);
                    (name, previous)
                })
                .collect();
            Self { values }
        }
    }

    impl Drop for EnvironmentRestore {
        fn drop(&mut self) {
            for (name, value) in self.values.drain(..) {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }

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
    /// Observed the way `storage::json`'s repository-root reads observe theirs:
    /// the process working directory is moved to an unrelated place for the
    /// call, so a relative path anywhere in the resolution would find nothing,
    /// and restored before anything is asserted.
    #[test]
    fn test_capture_repository_package_resolves_the_checkout_independently_of_the_working_directory(
    ) {
        let checkout = repository_checkout();
        assert!(
            checkout.is_absolute(),
            "a relative checkout would be resolved against the working directory: {}",
            checkout.display()
        );

        let elsewhere = TempDir::new().unwrap();
        let workspace = TempDir::new().unwrap();
        let id = package_carrying_drawn_content();
        let destination = workspace.path().join("package");
        let captured = {
            let _cwd = CurrentDirGuard::new(elsewhere.path()).unwrap();
            capture_repository_package(&id, &destination)
        };

        let package = captured.expect("the package captures from an unrelated working directory");
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

        assert_eq!(second.hashes(), first.hashes());
        let reread = ProfilePackage::from_directory(&destination)
            .expect("the republished destination holds a package");
        assert_eq!(reread.hashes(), second.hashes());
        assert_eq!(reread.file_count(), second.file_count());
    }

    #[test]
    fn test_profiled_repository_fixture_publication_is_concurrent_and_corruption_fails_closed() {
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
                    ensure_profiled_repository_fixture(&cache_root, "same-key", |repository| {
                        builds.fetch_add(1, Ordering::SeqCst);
                        fs::write(repository.join("state"), b"coherent")?;
                        Ok(())
                    })
                    .unwrap()
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
            fs::read(entries[0].join("repository/state")).unwrap(),
            b"coherent"
        );
        let repeated = ensure_profiled_repository_fixture(&cache_root, "same-key", |_| {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap();
        assert_eq!(repeated, entries[0]);
        assert_eq!(
            builds.load(Ordering::SeqCst),
            1,
            "an equivalent later consumer must reuse the same publication"
        );

        fs::write(entries[0].join("repository/state"), b"corrupt").unwrap();
        let error = ensure_profiled_repository_fixture(&cache_root, "same-key", |_| {
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
            fs::read(entries[0].join("repository/state")).unwrap(),
            b"corrupt",
            "a corrupt immutable entry must not be silently replaced"
        );
    }

    #[test]
    fn test_profiled_repository_fixture_refuses_an_occupied_publication_path() {
        let cache = TempDir::new().unwrap();
        fs::write(cache.path().join("occupied"), b"not a fixture").unwrap();
        let built = AtomicUsize::new(0);

        let error = ensure_profiled_repository_fixture(cache.path(), "occupied", |_| {
            built.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
        .unwrap_err();

        assert!(
            error.to_string().contains("occupied by a non-directory"),
            "unexpected occupied-path refusal: {error:#}"
        );
        assert_eq!(built.load(Ordering::SeqCst), 0);
        assert_eq!(
            fs::read(cache.path().join("occupied")).unwrap(),
            b"not a fixture"
        );
    }

    #[test]
    fn test_profiled_repository_fixture_no_replace_preserves_raced_occupied_directory() {
        let cache = TempDir::new().unwrap();
        let occupied = cache.path().join("raced");

        let error = ensure_profiled_repository_fixture(cache.path(), "raced", |repository| {
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
            fs::read_dir(cache.path())
                .unwrap()
                .filter_map(Result::ok)
                .all(|entry| !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".profiled-repository-staging-")),
            "a refused publication must clean its private staging directory"
        );
    }

    #[test]
    fn test_profiled_repository_fixture_clones_are_isolated_and_profiled() {
        let first =
            profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES, None).unwrap();
        let second =
            profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES, None).unwrap();
        let profile_record = Path::new(".jit/profiles/jit-dogfood.json");

        assert!(first.path().join(profile_record).is_file());
        assert_eq!(
            fs::read(first.path().join(profile_record)).unwrap(),
            fs::read(second.path().join(profile_record)).unwrap()
        );
        fs::write(first.path().join("AGENTS.md"), b"mutated clone").unwrap();
        assert_ne!(
            fs::read(first.path().join("AGENTS.md")).unwrap(),
            fs::read(second.path().join("AGENTS.md")).unwrap(),
            "fixture clones must not share mutable files"
        );

        let third =
            profiled_repository_fixture("jit-dogfood", PROFILE_PACKAGE_SOURCES, None).unwrap();
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
        const PROFILE_RECORD: &str = ".jit/profiles/jit-dogfood.json";
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
            first.repository_file_mode_fixture(PROFILE_RECORD).unwrap(),
            FileMode::Regular
        );
        assert_eq!(
            first.read_repo_file(PROFILE_RECORD).unwrap(),
            Some(fs::read_to_string(first_source.path().join(PROFILE_RECORD)).unwrap()),
            "the memory adapter must retain the baseline's exact applied-profile record"
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
    fn test_profiled_repository_fixture_receipt_rejects_wrong_run_and_content_identity() {
        let _lock = PROFILED_REPOSITORY_FIXTURE_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let cache = TempDir::new().unwrap();
        let setup_artifact = cache.path().join("debug/jit");
        fs::create_dir_all(setup_artifact.parent().unwrap()).unwrap();
        fs::write(&setup_artifact, b"fixture setup artifact").unwrap();
        let id = "jit-default";
        let expected_run = "expected-run";
        let environment = receipt_environment_name(id).unwrap();
        let receipt =
            prepare_profiled_repository_fixture_receipt(id, expected_run, &setup_artifact).unwrap();
        let _environment = EnvironmentRestore::set([
            ("NEXTEST_RUN_ID".to_string(), "wrong-run".into()),
            (
                environment.to_string(),
                receipt.path.clone().into_os_string(),
            ),
            (
                format!("{environment}_SHA256"),
                receipt.sha256.clone().into(),
            ),
        ]);

        let wrong_run = nextest_profiled_repository_fixture(id, "packages").unwrap_err();
        assert!(wrong_run.to_string().contains("does not belong"));
        std::env::set_var("NEXTEST_RUN_ID", expected_run);
        std::env::set_var(format!("{environment}_SHA256"), "0".repeat(64));
        let wrong_digest = nextest_profiled_repository_fixture(id, "packages").unwrap_err();
        assert!(wrong_digest.to_string().contains("content mismatch"));
    }

    #[test]
    fn test_profiled_repository_fixture_cache_root_follows_runtime_artifact_layouts() {
        let _lock = PROFILED_REPOSITORY_FIXTURE_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let workspace = TempDir::new().unwrap();
        let default_target = workspace.path().join("default-target");
        let absolute_custom_target = TempDir::new().unwrap();
        let relative_custom_target = workspace.path().join("relative-custom-target");
        let _misleading_environment = EnvironmentRestore::set([(
            "CARGO_TARGET_DIR".to_string(),
            "relative-to-a-different-process-directory".into(),
        )]);
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
                profiled_repository_fixture_cache_root(&artifact).unwrap(),
                expected_target_context.join(PROFILED_REPOSITORY_FIXTURE_DIRECTORY),
                "{} must select the cache context Cargo placed it under",
                artifact.display()
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_profiled_repository_fixture_cache_root_refuses_non_ordinary_runtime_artifact() {
        use std::os::unix::fs::symlink;

        let workspace = TempDir::new().unwrap();
        let target = workspace.path().join("debug/jit");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        symlink(std::env::current_exe().unwrap(), &target).unwrap();

        let error = profiled_repository_fixture_cache_root(&target).unwrap_err();
        assert!(error.to_string().contains("must be an ordinary file"));
    }

    #[test]
    fn test_profiled_repository_fixture_receipt_binds_setup_and_consumer_to_shared_runtime_target()
    {
        let _lock = PROFILED_REPOSITORY_FIXTURE_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let runtime_target = TempDir::new().unwrap();
        let setup_artifact = runtime_target.path().join("debug/jit");
        let consumer_artifact = runtime_target.path().join("debug/deps/fixture-consumer");
        fs::create_dir_all(setup_artifact.parent().unwrap()).unwrap();
        fs::create_dir_all(consumer_artifact.parent().unwrap()).unwrap();
        let source = runtime_target.path().join("provenance.rs");
        fs::write(
            &source,
            "fn main() { print!(\"{}\", env!(\"CARGO_MANIFEST_DIR\")); }",
        )
        .unwrap();
        let setup_checkout = runtime_target.path().join("isolated-setup-checkout");
        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let setup_compile = std::process::Command::new(&rustc)
            .args(["--edition=2021", source.to_str().unwrap(), "-o"])
            .arg(&setup_artifact)
            .env("CARGO_MANIFEST_DIR", &setup_checkout)
            .output()
            .unwrap();
        assert!(
            setup_compile.status.success(),
            "compile the isolated setup artifact: {}",
            String::from_utf8_lossy(&setup_compile.stderr)
        );
        let consumer_compile = std::process::Command::new(&rustc)
            .args(["--edition=2021", source.to_str().unwrap(), "-o"])
            .arg(&consumer_artifact)
            .env("CARGO_MANIFEST_DIR", repository_checkout())
            .output()
            .unwrap();
        assert!(
            consumer_compile.status.success(),
            "compile the consumer artifact: {}",
            String::from_utf8_lossy(&consumer_compile.stderr)
        );

        let setup_provenance = std::process::Command::new(&setup_artifact)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(setup_provenance.stdout).unwrap(),
            setup_checkout.to_string_lossy(),
            "the setup artifact must embed the isolated checkout it was compiled from"
        );
        let consumer_provenance = std::process::Command::new(&consumer_artifact)
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8(consumer_provenance.stdout).unwrap(),
            repository_checkout().to_string_lossy(),
            "the consumer artifact must embed this test executable's checkout"
        );
        assert_ne!(
            setup_checkout,
            repository_checkout(),
            "setup and consumer artifacts must have differing compile-time checkout provenance"
        );
        let _target_directory = EnvironmentRestore::remove(["CARGO_TARGET_DIR".to_string()]);
        let id = "jit-default";
        let run_id = "shared-runtime-target";
        let environment = receipt_environment_name(id).unwrap();
        let receipt =
            prepare_profiled_repository_fixture_receipt(id, run_id, &setup_artifact).unwrap();

        let expected_cache_root = runtime_target
            .path()
            .join(PROFILED_REPOSITORY_FIXTURE_DIRECTORY);
        assert_eq!(
            receipt.path.parent(),
            Some(expected_cache_root.as_path()),
            "the setup receipt must be published beside its runtime artifact, not its source checkout"
        );

        let _receipt_environment = EnvironmentRestore::set([
            ("NEXTEST_RUN_ID".to_string(), run_id.into()),
            (
                environment.to_string(),
                receipt.path.clone().into_os_string(),
            ),
            (
                format!("{environment}_SHA256"),
                receipt.sha256.clone().into(),
            ),
        ]);
        let entry =
            nextest_profiled_repository_fixture_for_artifact(id, "packages", &consumer_artifact)
                .unwrap();

        assert!(
            entry.is_some_and(|entry| entry.parent() == Some(expected_cache_root.as_path())),
            "a same-run consumer in the shared runtime target must accept the setup receipt"
        );
    }

    #[test]
    fn test_direct_profiled_repository_fixture_scenario_identity_does_not_alias() {
        let workspace = TempDir::new().unwrap();
        let first = workspace.path().join("first");
        let second = workspace.path().join("second");
        fs::write(&first, b"same scenario bytes").unwrap();
        fs::write(&second, b"same scenario bytes").unwrap();
        let mut digests = BTreeMap::new();
        let first_provenance = executable_provenance(&first, &mut digests).unwrap();
        let second_provenance = executable_provenance(&second, &mut digests).unwrap();

        assert_ne!(
            direct_entry_key("jit-default", "packages", Some(&first_provenance)),
            direct_entry_key("jit-default", "packages", Some(&second_provenance)),
            "distinct scenario executables must not alias the process-local memo"
        );

        let replacement = workspace.path().join("replacement");
        fs::write(&replacement, b"different scenario bytes").unwrap();
        fs::rename(&replacement, &first).unwrap();
        let replacement_provenance = executable_provenance(&first, &mut digests).unwrap();

        assert_ne!(
            direct_entry_key("jit-default", "packages", Some(&first_provenance)),
            direct_entry_key("jit-default", "packages", Some(&replacement_provenance)),
            "replacing an executable at the same path must select a new process-local memo entry"
        );
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
