use super::manifest::{
    is_lowercase_kebab, ProfileManifest, MANIFEST_FILE_NAME, PROFILE_MANIFEST_VERSION,
};
use crate::repository_state::{Contribution, MapEntryTarget, ScalarTarget};
use cap_primitives::fs::FollowSymlinks;
use cap_std::ambient_authority;
use cap_std::fs::{Dir as CapDir, DirEntry as CapDirEntry, OpenOptions as CapOpenOptions};
use include_dir::Dir;
use semver::{Version, VersionReq};
use serde::Serialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

const TARGET_HASH_DOMAIN: &[u8] = b"jit-profile-target-v1\0";
const PACKAGE_HASH_DOMAIN: &[u8] = b"jit-profile-package-v1\0";

/// Maximum number of files, including the manifest, in one profile package.
pub const MAX_PROFILE_PACKAGE_FILES: usize = 512;

/// Maximum total bytes, including the manifest, in one profile package.
pub const MAX_PROFILE_PACKAGE_BYTES: usize = 4 * 1024 * 1024;

/// A lowercase hexadecimal SHA-256 digest.
pub type PackageHash = String;

/// Canonical package and per-repository-target hashes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfilePackageHashes {
    /// Hash of the canonical manifest plus every declared package source.
    pub package: PackageHash,
    /// Hash of ordered operations and source bytes grouped by repository target.
    pub targets: BTreeMap<String, PackageHash>,
}

/// Where a validated package's bytes came from.
///
/// Minted only by the two constructors of [`ProfilePackage`], each from the
/// route it took, so the source a package reports is the source its own bytes
/// were read through rather than a claim made about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfilePackageSource {
    /// Bytes compiled into the running binary.
    Embedded,
    /// Bytes read from this absolute, symlink-resolved directory.
    Directory(PathBuf),
}

/// A validated immutable package owning the bytes it validated.
///
/// A package comes either from a directory tree on disk
/// ([`from_directory`](Self::from_directory)) or from a directory embedded at
/// compile time ([`from_embedded_dir`](Self::from_embedded_dir)). Both routes
/// own their bytes and run one validation over one path-to-bytes map, so
/// packages built from identical content are indistinguishable in manifest,
/// package hash, and target digests. They differ in exactly one observable:
/// the [`source`](Self::source) each records for the bytes it admitted.
#[derive(Debug, Clone)]
pub struct ProfilePackage {
    manifest: ProfileManifest,
    files: BTreeMap<String, Vec<u8>>,
    hashes: ProfilePackageHashes,
    source: ProfilePackageSource,
}

impl ProfilePackage {
    /// Read and validate the package tree rooted at `directory`.
    ///
    /// The tree is untrusted external data, so the walk rejects an entry that is
    /// neither a regular file nor a subdirectory
    /// ([`IrregularEntry`](ProfilePackageError::IrregularEntry)) and an entry
    /// resolving outside `directory`
    /// ([`EscapingEntry`](ProfilePackageError::EscapingEntry)), each naming the
    /// entry, before the package validation every route shares sees the bytes.
    /// A directory that is absent or cannot be read is
    /// [`UnreadableDirectory`](ProfilePackageError::UnreadableDirectory), which
    /// no invalid package produces.
    ///
    /// The recorded [`source`](Self::source) is the resolved root the walk
    /// anchored its handle chain at, not the argument: `directory` may be
    /// relative and may traverse symbolic links, and the bytes admitted are the
    /// bytes below the root those resolve to.
    pub fn from_directory(directory: &Path) -> Result<Self, ProfilePackageError> {
        let (root, files) = read_package_directory(directory)?;
        Self::from_files(files, ProfilePackageSource::Directory(root))
    }

    /// Parse and validate a recursively embedded directory.
    pub fn from_embedded_dir(directory: &Dir<'_>) -> Result<Self, ProfilePackageError> {
        Self::from_files(embedded_files(directory), ProfilePackageSource::Embedded)
    }

    fn from_files(
        files: BTreeMap<String, Vec<u8>>,
        source: ProfilePackageSource,
    ) -> Result<Self, ProfilePackageError> {
        validate_package_bounds(&files)?;
        let manifest_bytes = files
            .get(MANIFEST_FILE_NAME)
            .ok_or(ProfilePackageError::MissingManifest)?;
        let manifest_text = std::str::from_utf8(manifest_bytes)
            .map_err(|source| ProfilePackageError::ManifestUtf8 { source })?;
        let manifest: ProfileManifest =
            toml::from_str(manifest_text).map_err(ProfilePackageError::ManifestToml)?;

        validate_manifest(&manifest, &files)?;
        let hashes = compute_hashes(&manifest, &files)?;

        Ok(Self {
            manifest,
            files,
            hashes,
            source,
        })
    }

    /// Parsed runtime manifest.
    pub fn manifest(&self) -> &ProfileManifest {
        &self.manifest
    }

    /// Where this package's validated bytes were read from.
    pub fn source(&self) -> &ProfilePackageSource {
        &self.source
    }

    /// Owned bytes for a declared package-relative source.
    pub fn source_bytes(&self, source: &str) -> Option<&[u8]> {
        self.files.get(source).map(Vec::as_slice)
    }

    /// Canonical package and target hashes.
    pub fn hashes(&self) -> &ProfilePackageHashes {
        &self.hashes
    }

    /// Package file count, including `manifest.toml`.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Total package byte size, including `manifest.toml`.
    pub fn byte_size(&self) -> usize {
        self.files.values().map(Vec::len).sum()
    }
}

/// Read and validation failures for immutable profile packages.
#[derive(Debug, thiserror::Error)]
pub enum ProfilePackageError {
    /// The package directory is absent or cannot be read.
    #[error("cannot read profile package directory '{path}': {source}")]
    UnreadableDirectory {
        /// Path whose read failed.
        path: String,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// A package directory entry is neither a regular file nor a subdirectory.
    #[error("profile package entry '{path}' is not a regular file")]
    IrregularEntry {
        /// Package-relative entry path.
        path: String,
    },
    /// A package directory entry resolves outside the package root.
    #[error("profile package entry '{path}' escapes the package root")]
    EscapingEntry {
        /// Package-relative entry path.
        path: String,
    },
    /// The root manifest is absent.
    #[error("profile package is missing root manifest.toml")]
    MissingManifest,
    /// The manifest is not UTF-8 TOML.
    #[error("profile manifest is not UTF-8: {source}")]
    ManifestUtf8 {
        /// UTF-8 parser error.
        source: std::str::Utf8Error,
    },
    /// TOML does not match the runtime manifest wire type.
    #[error("invalid profile manifest: {0}")]
    ManifestToml(#[source] toml::de::Error),
    /// Unsupported wire version.
    #[error("unsupported profile manifest version {actual}; expected {expected}")]
    ManifestVersion {
        /// Parsed version.
        actual: u32,
        /// Supported version.
        expected: u32,
    },
    /// A package declares itself as a dependency.
    #[error("profile '{package}' cannot depend on itself ('{dependency}')")]
    SelfDependency {
        /// Package that owns the manifest.
        package: String,
        /// Offending dependency entry.
        dependency: String,
    },
    /// Invalid semantic package version.
    #[error("invalid profile version '{value}': {source}")]
    InvalidVersion {
        /// Authored value.
        value: String,
        /// Semver parser error.
        source: semver::Error,
    },
    /// Invalid compatible JIT version requirement.
    #[error("invalid compatible JIT requirement '{value}': {source}")]
    InvalidCompatibility {
        /// Authored value.
        value: String,
        /// Semver parser error.
        source: semver::Error,
    },
    /// Unsafe absolute, traversal, platform-prefix, or empty path.
    #[error("unsafe {field} path '{path}'")]
    UnsafePath {
        /// Manifest field.
        field: &'static str,
        /// Rejected value.
        path: String,
    },
    /// A package source is declared more than once.
    #[error("package source '{0}' is declared more than once")]
    DuplicateSource(String),
    /// Two file/region declarations target the same repository path.
    #[error("repository content target '{0}' is declared more than once")]
    DuplicateContentTarget(String),
    /// A declaration references absent package bytes.
    #[error("declared package source '{0}' is missing")]
    MissingContent(String),
    /// Package bytes have no declaration.
    #[error("package source '{0}' is not declared by the manifest")]
    ExtraContent(String),
    /// Invalid semantic contribution.
    #[error("invalid contribution at index {index}: {message}")]
    InvalidContribution {
        /// Manifest order.
        index: usize,
        /// Specific contract failure.
        message: String,
    },
    /// Duplicate semantic contribution identity.
    #[error("duplicate contribution identity '{0}'")]
    DuplicateContribution(String),
    /// Invalid region marker identity.
    #[error("invalid region id '{0}'; expected lowercase-kebab")]
    InvalidRegionId(String),
    /// Canonical serialization unexpectedly failed.
    #[error("failed to serialize canonical profile data: {0}")]
    CanonicalSerialization(#[source] serde_json::Error),
    /// File count or byte size exceeds the package budget.
    #[error(
        "profile package exceeds bounds: {file_count} files/{byte_size} bytes; \
         maximum is {max_files} files/{max_bytes} bytes"
    )]
    PackageBounds {
        /// Actual file count.
        file_count: usize,
        /// Actual total bytes.
        byte_size: usize,
        /// Maximum supported file count.
        max_files: usize,
        /// Maximum supported total bytes.
        max_bytes: usize,
    },
}

fn embedded_files(directory: &Dir<'_>) -> BTreeMap<String, Vec<u8>> {
    fn visit(directory: &Dir<'_>, files: &mut BTreeMap<String, Vec<u8>>) {
        directory.files().for_each(|file| {
            files.insert(
                normalize_package_path(file.path()),
                file.contents().to_vec(),
            );
        });
        directory.dirs().for_each(|child| visit(child, files));
    }

    let mut files = BTreeMap::new();
    visit(directory, &mut files);
    files
}

/// Walk the package tree rooted at `root` into its resolved root and a
/// package-relative byte map.
///
/// The returned root is the resolved path the handle chain is anchored at, so
/// it names the directory the returned bytes actually came from.
///
/// Containment holds by construction rather than by inspection. Every entry is
/// reached by a no-follow `openat` against the directory handle that listed it
/// — the same capability-handle discipline the storage layer's tree walks use
/// (`@/invariant/convention-convergence`) — so the bytes admitted to a package
/// are the bytes of the entry the walk classified. There is no second lookup of
/// the entry's name after its kind is decided, and therefore no interval in
/// which replacing that name with a symbolic link could redirect the read: the
/// open of a symbolic link fails outright, and the handle chain is anchored at
/// `root`, so no ancestor can be redirected either.
///
/// The pending list is explicit rather than recursive, and the budget bounds
/// the read twice — once from the opened file's own size, and again by reading
/// at most one byte past what remains, so a file that grows after its size is
/// taken cannot be read past the budget either. [`validate_package_bounds`]
/// stays the authority over what was actually read.
fn read_package_directory(
    root: &Path,
) -> Result<(PathBuf, BTreeMap<String, Vec<u8>>), ProfilePackageError> {
    let root = fs::canonicalize(root).map_err(|source| unreadable(root, source))?;
    let handle = CapDir::open_ambient_dir(&root, ambient_authority())
        .map_err(|source| unreadable(&root, source))?;
    let mut pending = vec![(handle, String::new())];
    let mut files = BTreeMap::new();
    let mut byte_size = 0usize;
    while let Some((directory, prefix)) = pending.pop() {
        let listing = directory
            .entries()
            .map_err(|source| unreadable(&root.join(&prefix), source))?;
        for entry in listing {
            let entry = entry.map_err(|source| unreadable(&root.join(&prefix), source))?;
            let relative = package_child_path(&prefix, &entry.file_name());
            let kind = entry
                .file_type()
                .map_err(|source| unreadable(&root.join(&relative), source))?;
            if kind.is_dir() {
                pending.push((open_child_directory(&entry, &root, &relative)?, relative));
            } else if kind.is_file() {
                let bytes = read_regular_entry(&entry, &root, &relative, files.len(), byte_size)?;
                byte_size = byte_size.saturating_add(bytes.len());
                files.insert(relative, bytes);
            } else {
                return Err(rejected_entry(&root, relative));
            }
        }
    }
    Ok((root, files))
}

/// Open one listed entry through a no-follow `openat` on the handle that listed
/// it, so the object opened is the object listed or nothing at all.
///
/// The open is non-blocking because opening is where an entry of the wrong kind
/// gets to make a decision on the reader's behalf: a pipe or a device opened for
/// reading waits for a peer, and a reader waiting forever rejects nothing. With
/// `O_NONBLOCK` the open returns at once and the caller's handle-metadata check
/// refuses the entry by name. The flag has no effect on the regular files a
/// package is made of, whose reads are unaffected by it.
fn open_entry_nofollow(entry: &CapDirEntry, maybe_dir: bool) -> std::io::Result<cap_std::fs::File> {
    let mut options = CapOpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    options._cap_fs_ext_maybe_dir(maybe_dir);
    options._cap_fs_ext_nonblock(true);
    entry.open_with(&options)
}

/// Descend into a listed subdirectory, or report why it is not one.
fn open_child_directory(
    entry: &CapDirEntry,
    root: &Path,
    relative: &str,
) -> Result<CapDir, ProfilePackageError> {
    let opened = open_entry_nofollow(entry, true)
        .map_err(|source| entry_open_failure(root, relative, source))?;
    let is_dir = opened
        .metadata()
        .map_err(|source| unreadable(&root.join(relative), source))?
        .is_dir();
    if is_dir {
        Ok(CapDir::from_std_file(opened.into_std()))
    } else {
        Err(rejected_entry(root, relative.to_string()))
    }
}

/// Read one listed regular file, bounded by what the budget still admits.
///
/// The kind and the size both come from the opened handle rather than from the
/// name, so they describe the object whose bytes this returns.
fn read_regular_entry(
    entry: &CapDirEntry,
    root: &Path,
    relative: &str,
    file_count: usize,
    byte_size: usize,
) -> Result<Vec<u8>, ProfilePackageError> {
    let opened = open_entry_nofollow(entry, false)
        .map_err(|source| entry_open_failure(root, relative, source))?;
    let metadata = opened
        .metadata()
        .map_err(|source| unreadable(&root.join(relative), source))?;
    if !metadata.is_file() {
        return Err(rejected_entry(root, relative.to_string()));
    }
    let admitted = file_count.saturating_add(1);
    let declared = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
    if let Some(error) = package_bounds_failure(admitted, byte_size.saturating_add(declared)) {
        return Err(error);
    }
    let headroom = MAX_PROFILE_PACKAGE_BYTES.saturating_sub(byte_size);
    let mut bytes = Vec::new();
    opened
        .take(headroom as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| unreadable(&root.join(relative), source))?;
    match package_bounds_failure(admitted, byte_size.saturating_add(bytes.len())) {
        Some(error) => Err(error),
        None => Ok(bytes),
    }
}

/// Why an entry that is not a usable regular file or directory is refused.
///
/// Reporting only. Containment is established by the no-follow open, so a stale
/// answer here cannot admit anything; it only decides which name the refusal
/// carries.
fn rejected_entry(root: &Path, relative: String) -> ProfilePackageError {
    if escapes_root(root, &root.join(&relative)) {
        ProfilePackageError::EscapingEntry { path: relative }
    } else {
        ProfilePackageError::IrregularEntry { path: relative }
    }
}

/// A failed open: a refusal when the name cannot be package content, I/O
/// otherwise.
///
/// Two errors say the listed name is not a regular file, whatever the listing
/// said. Refusing to follow a symbolic link reports `ELOOP` under POSIX and
/// `EMLINK` on the BSDs. Reading a socket, or a device with nothing behind it,
/// reports `ENXIO` — the kinds that cannot be opened for reading at all, as
/// distinct from the pipes and devices that open non-blockingly and are refused
/// by their handle's metadata.
fn entry_open_failure(root: &Path, relative: &str, source: std::io::Error) -> ProfilePackageError {
    let unopenable_kind = matches!(
        source.raw_os_error(),
        Some(code)
            if code == nix::libc::ELOOP || code == nix::libc::EMLINK || code == nix::libc::ENXIO
    );
    if unopenable_kind {
        rejected_entry(root, relative.to_string())
    } else {
        unreadable(&root.join(relative), source)
    }
}

/// Whether `path` resolves outside `root`.
///
/// An entry that cannot be resolved at all is not an escape: it is left to the
/// kind check, which names what it is.
fn escapes_root(root: &Path, path: &Path) -> bool {
    fs::canonicalize(path).is_ok_and(|resolved| !resolved.starts_with(root))
}

fn unreadable(path: &Path, source: std::io::Error) -> ProfilePackageError {
    ProfilePackageError::UnreadableDirectory {
        path: path.display().to_string(),
        source,
    }
}

/// The package-relative path of `name` inside the directory at `prefix`.
fn package_child_path(prefix: &str, name: &std::ffi::OsStr) -> String {
    let name = name.to_string_lossy();
    if prefix.is_empty() {
        name.into_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

fn normalize_package_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => Some(value.to_string_lossy()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn validate_package_bounds(files: &BTreeMap<String, Vec<u8>>) -> Result<(), ProfilePackageError> {
    let byte_size = files
        .values()
        .fold(0usize, |total, bytes| total.saturating_add(bytes.len()));
    package_bounds_failure(files.len(), byte_size).map_or(Ok(()), Err)
}

/// The budget failure a package of this size carries, or `None` within budget.
fn package_bounds_failure(file_count: usize, byte_size: usize) -> Option<ProfilePackageError> {
    (file_count > MAX_PROFILE_PACKAGE_FILES || byte_size > MAX_PROFILE_PACKAGE_BYTES).then_some(
        ProfilePackageError::PackageBounds {
            file_count,
            byte_size,
            max_files: MAX_PROFILE_PACKAGE_FILES,
            max_bytes: MAX_PROFILE_PACKAGE_BYTES,
        },
    )
}

fn validate_manifest(
    manifest: &ProfileManifest,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<(), ProfilePackageError> {
    if manifest.profile.manifest_version != PROFILE_MANIFEST_VERSION {
        return Err(ProfilePackageError::ManifestVersion {
            actual: manifest.profile.manifest_version,
            expected: PROFILE_MANIFEST_VERSION,
        });
    }
    if let Some(dependency) = manifest
        .dependencies
        .iter()
        .find(|dependency| *dependency == &manifest.profile.id)
    {
        return Err(ProfilePackageError::SelfDependency {
            package: manifest.profile.id.to_string(),
            dependency: dependency.to_string(),
        });
    }
    Version::parse(&manifest.profile.version).map_err(|source| {
        ProfilePackageError::InvalidVersion {
            value: manifest.profile.version.clone(),
            source,
        }
    })?;
    VersionReq::parse(&manifest.profile.jit).map_err(|source| {
        ProfilePackageError::InvalidCompatibility {
            value: manifest.profile.jit.clone(),
            source,
        }
    })?;

    let mut contribution_ids = BTreeSet::new();
    for (index, contribution) in manifest.contributions.iter().enumerate() {
        validate_contribution(index, contribution)?;
        let identity = contribution_identity(contribution);
        if !contribution_ids.insert(identity.clone()) {
            return Err(ProfilePackageError::DuplicateContribution(identity));
        }
    }

    let mut declared_sources = BTreeSet::new();
    let mut content_targets = BTreeSet::new();
    for asset in &manifest.assets {
        validate_relative_path("asset source", &asset.source)?;
        validate_relative_path("asset target", &asset.target)?;
        insert_unique_source(&mut declared_sources, &asset.source)?;
        if !content_targets.insert(asset.target.clone()) {
            return Err(ProfilePackageError::DuplicateContentTarget(
                asset.target.clone(),
            ));
        }
    }
    for region in &manifest.regions {
        validate_relative_path("region source", &region.source)?;
        validate_relative_path("region target", &region.target)?;
        if !is_lowercase_kebab(&region.region_id) {
            return Err(ProfilePackageError::InvalidRegionId(
                region.region_id.clone(),
            ));
        }
        insert_unique_source(&mut declared_sources, &region.source)?;
        if !content_targets.insert(region.target.clone()) {
            return Err(ProfilePackageError::DuplicateContentTarget(
                region.target.clone(),
            ));
        }
    }

    for source in &declared_sources {
        if !files.contains_key(source) {
            return Err(ProfilePackageError::MissingContent(source.clone()));
        }
    }
    for path in files
        .keys()
        .filter(|path| path.as_str() != MANIFEST_FILE_NAME)
    {
        if !declared_sources.contains(path) {
            return Err(ProfilePackageError::ExtraContent(path.clone()));
        }
    }
    Ok(())
}

fn insert_unique_source(
    sources: &mut BTreeSet<String>,
    source: &str,
) -> Result<(), ProfilePackageError> {
    if source == MANIFEST_FILE_NAME {
        return Err(ProfilePackageError::DuplicateSource(source.to_string()));
    }
    if sources.insert(source.to_string()) {
        Ok(())
    } else {
        Err(ProfilePackageError::DuplicateSource(source.to_string()))
    }
}

fn validate_contribution(
    index: usize,
    contribution: &Contribution,
) -> Result<(), ProfilePackageError> {
    let invalid = |message: String| ProfilePackageError::InvalidContribution { index, message };
    match contribution {
        Contribution::Scalar { target, value } => {
            if value.trim().is_empty() {
                return Err(invalid("scalar value must not be empty".to_string()));
            }
            if *target == ScalarTarget::ValidationStrictness {
                value
                    .parse::<crate::validation::Strictness>()
                    .map_err(|error| {
                        invalid(format!("validation strictness is invalid: {error}"))
                    })?;
            }
            Ok(())
        }
        Contribution::MapEntry {
            target,
            identity,
            value,
        } => {
            if identity.trim().is_empty() {
                return Err(invalid("identity must not be empty".to_string()));
            }
            match target {
                MapEntryTarget::TypeHierarchyTypes
                    if value.as_u64().is_none_or(|level| level == 0) =>
                {
                    Err(invalid(
                        "type-hierarchy-types value must be a positive integer".to_string(),
                    ))
                }
                MapEntryTarget::LabelAssociations if !value.is_string() => Err(invalid(
                    "label-associations value must be a string".to_string(),
                )),
                MapEntryTarget::Namespaces | MapEntryTarget::ItemKinds if !value.is_object() => {
                    Err(invalid(format!(
                        "{target:?} value must be a complete table"
                    )))
                }
                _ => Ok(()),
            }
        }
        Contribution::SetString { value, .. } if value.trim().is_empty() => {
            Err(invalid("set string must not be empty".to_string()))
        }
        Contribution::SetString { .. } => Ok(()),
        Contribution::KeyedArray {
            target,
            identity,
            value,
        } => {
            let table = value
                .as_object()
                .ok_or_else(|| invalid("keyed-array value must be a complete table".to_string()))?;
            let field = target.identity_field();
            match table.get(field).and_then(Value::as_str) {
                Some(actual) if actual == identity && !identity.is_empty() => Ok(()),
                Some(actual) => Err(invalid(format!(
                    "{target:?} identity '{identity}' does not match value.{field} '{actual}'"
                ))),
                None => Err(invalid(format!(
                    "{target:?} value must contain string field '{field}'"
                ))),
            }
        }
        Contribution::Projection { value, .. } => {
            validate_relative_path("projection target", &value.target)
                .map_err(|error| invalid(error.to_string()))
        }
    }
}

fn validate_relative_path(field: &'static str, path: &str) -> Result<(), ProfilePackageError> {
    let parsed = Path::new(path);
    let has_windows_prefix = path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic);
    let has_noncanonical_segment = path
        .split('/')
        .any(|segment| segment.is_empty() || matches!(segment, "." | ".."));
    let safe = !path.is_empty()
        && !path.contains('\\')
        && !path.contains(':')
        && !path.chars().any(char::is_control)
        && !has_windows_prefix
        && !has_noncanonical_segment
        && !parsed.is_absolute()
        && parsed.components().all(|component| match component {
            Component::Normal(value) => value != "." && value != "..",
            _ => false,
        });
    if safe {
        Ok(())
    } else {
        Err(ProfilePackageError::UnsafePath {
            field,
            path: path.to_string(),
        })
    }
}

fn contribution_identity(contribution: &Contribution) -> String {
    let semantic = match contribution {
        Contribution::Scalar { target, .. } => format!("scalar:{target:?}"),
        Contribution::MapEntry {
            target, identity, ..
        } => format!("map-entry:{target:?}:{identity}"),
        Contribution::SetString { target, value } => {
            format!("set-string:{target:?}:{value}")
        }
        Contribution::KeyedArray {
            target, identity, ..
        } => format!("keyed-array:{target:?}:{identity}"),
        Contribution::Projection { name, .. } => {
            format!("projection:{name}")
        }
    };
    format!("{}:{semantic}", contribution.registry_path())
}

fn compute_hashes(
    manifest: &ProfileManifest,
    files: &BTreeMap<String, Vec<u8>>,
) -> Result<ProfilePackageHashes, ProfilePackageError> {
    let mut target_frames: BTreeMap<String, Vec<Vec<u8>>> = BTreeMap::new();

    for contribution in &manifest.contributions {
        target_frames
            .entry(contribution.registry_path().to_string())
            .or_default()
            .push(canonical_bytes(contribution)?);
    }
    for asset in &manifest.assets {
        let mut frame = canonical_bytes(asset)?;
        append_frame(
            &mut frame,
            files
                .get(&asset.source)
                .ok_or_else(|| ProfilePackageError::MissingContent(asset.source.clone()))?,
        );
        target_frames
            .entry(asset.target.clone())
            .or_default()
            .push(frame);
    }
    for region in &manifest.regions {
        let mut frame = canonical_bytes(region)?;
        append_frame(
            &mut frame,
            files
                .get(&region.source)
                .ok_or_else(|| ProfilePackageError::MissingContent(region.source.clone()))?,
        );
        target_frames
            .entry(region.target.clone())
            .or_default()
            .push(frame);
    }

    let targets = target_frames
        .into_iter()
        .map(|(target, frames)| {
            let mut hasher = Sha256::new();
            hasher.update(TARGET_HASH_DOMAIN);
            hash_frame(&mut hasher, target.as_bytes());
            frames
                .iter()
                .for_each(|frame| hash_frame(&mut hasher, frame));
            (target, format!("{:x}", hasher.finalize()))
        })
        .collect();

    let mut package = Sha256::new();
    package.update(PACKAGE_HASH_DOMAIN);
    hash_frame(&mut package, &canonical_bytes(manifest)?);
    for (path, contents) in files
        .iter()
        .filter(|(path, _)| path.as_str() != MANIFEST_FILE_NAME)
    {
        hash_frame(&mut package, path.as_bytes());
        hash_frame(&mut package, contents);
    }

    Ok(ProfilePackageHashes {
        package: format!("{:x}", package.finalize()),
        targets,
    })
}

fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ProfilePackageError> {
    let canonical = canonical_json(value)?;
    serde_json::to_vec(&canonical).map_err(ProfilePackageError::CanonicalSerialization)
}

fn canonical_json<T: Serialize>(value: &T) -> Result<Value, ProfilePackageError> {
    serde_json::to_value(value)
        .map(canonicalize_value)
        .map_err(ProfilePackageError::CanonicalSerialization)
}

fn canonicalize_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonicalize_value).collect()),
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, canonicalize_value(value)))
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .collect::<Map<_, _>>(),
        ),
        scalar => scalar,
    }
}

fn append_frame(destination: &mut Vec<u8>, frame: &[u8]) {
    destination.extend_from_slice(&(frame.len() as u64).to_be_bytes());
    destination.extend_from_slice(frame);
}

fn hash_frame(hasher: &mut Sha256, frame: &[u8]) {
    hasher.update((frame.len() as u64).to_be_bytes());
    hasher.update(frame);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::{profile_manifest_schema, ProfileId};
    use crate::repository_state::{
        Contribution, KeyedArrayTarget, MapEntryTarget, ScalarTarget, SetStringTarget,
    };
    use include_dir::{include_dir, Dir};
    use std::path::PathBuf;
    use tempfile::TempDir;

    static VALID_PACKAGE: Dir<'_> =
        include_dir!("$CARGO_MANIFEST_DIR/tests/fixtures/profile-packages/synthetic-valid");

    fn package() -> ProfilePackage {
        ProfilePackage::from_embedded_dir(&VALID_PACKAGE).expect("valid synthetic package")
    }

    /// Run the shared package validation over a synthetic path-to-bytes map.
    ///
    /// The map is authored here rather than read from anywhere, so it enters
    /// through the compiled-in source; every caller asserts about validation
    /// outcomes, which the two routes share.
    fn validated_package(
        files: BTreeMap<String, Vec<u8>>,
    ) -> Result<ProfilePackage, ProfilePackageError> {
        ProfilePackage::from_files(files, ProfilePackageSource::Embedded)
    }

    /// The checked-in tree the compile-time fixture embeds.
    fn fixture_tree() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/profile-packages/synthetic-valid")
    }

    /// A writable copy of the fixture tree, rooted inside `temp`.
    ///
    /// Written from the compile-time fixture rather than copied from the
    /// checkout, so both construction routes read one authored source
    /// (`@/invariant/shared-test-contracts`).
    fn writable_package_tree(temp: &TempDir) -> PathBuf {
        writable_package_tree_at(&temp.path().join("package"))
    }

    /// A writable copy of the fixture tree at the given root, created.
    fn writable_package_tree_at(root: &Path) -> PathBuf {
        crate::test_utils::write_package_tree(&VALID_PACKAGE, root)
    }

    /// Replace the package manifest under `root` with `manifest_text()` mutated.
    fn rewrite_manifest(root: &Path, old: &str, new: &str) {
        std::fs::write(
            root.join(MANIFEST_FILE_NAME),
            manifest_text().replace(old, new),
        )
        .expect("rewrite package manifest");
    }

    fn manifest_text() -> String {
        VALID_PACKAGE
            .get_file(MANIFEST_FILE_NAME)
            .expect("fixture manifest")
            .contents_utf8()
            .expect("UTF-8 manifest")
            .to_string()
    }

    fn parse_modified(old: &str, new: &str) -> Result<ProfileManifest, toml::de::Error> {
        toml::from_str(&manifest_text().replace(old, new))
    }

    fn schema_has_property(value: &Value, property: &str) -> bool {
        match value {
            Value::Array(values) => values
                .iter()
                .any(|value| schema_has_property(value, property)),
            Value::Object(values) => {
                values
                    .get("properties")
                    .and_then(Value::as_object)
                    .is_some_and(|properties| properties.contains_key(property))
                    || values
                        .values()
                        .any(|value| schema_has_property(value, property))
            }
            _ => false,
        }
    }

    #[test]
    fn test_embedded_package_recurses_and_preserves_manifest_order() {
        let package = package();
        assert_eq!(package.file_count(), 4);
        assert!(package.file_count() <= MAX_PROFILE_PACKAGE_FILES);
        assert!(package.byte_size() <= MAX_PROFILE_PACKAGE_BYTES);
        assert_eq!(package.manifest().profile.id.as_str(), "synthetic-workflow");
        assert_eq!(package.manifest().contributions.len(), 10);
        assert!(matches!(
            package.manifest().contributions.first(),
            Some(Contribution::MapEntry {
                target: MapEntryTarget::TypeHierarchyTypes,
                identity,
                ..
            }) if identity == "initiative"
        ));
        assert!(matches!(
            package.manifest().contributions.get(6),
            Some(Contribution::KeyedArray {
                target: KeyedArrayTarget::Gates,
                identity,
                ..
            }) if identity == "synthetic-review"
        ));
        assert!(matches!(
            package.manifest().contributions.last(),
            Some(Contribution::Projection { name, .. }) if name == "rules-and-gates"
        ));
        assert_eq!(
            package.source_bytes("nested/scripts/check.sh"),
            Some(b"#!/bin/sh\nexit 0\n".as_slice())
        );
        assert!(package.manifest().assets[1].executable);
        assert!(package.manifest().dependencies.is_empty());
    }

    #[test]
    fn test_manifest_schema_is_generated_from_runtime_wire_type() {
        let package = package();
        let schema = serde_json::to_value(profile_manifest_schema()).unwrap();
        let instance = serde_json::to_value(package.manifest()).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&instance)
            .expect("runtime manifest must satisfy generated schema");

        let schema_text = schema.to_string();
        assert!(schema_text.contains("manifest-version"));
        assert!(schema_text.contains("projection"));
        assert!(!schema_has_property(&schema, "hook"));
        assert!(schema_has_property(&schema, "dependencies"));
        assert!(!schema_has_property(&schema, "variables"));
    }

    #[test]
    fn test_manifest_dependency_is_reported_in_runtime_inspection_and_schema() {
        let manifest =
            manifest_text().replace("[profile]", "dependencies = [\"jit-default\"]\n\n[profile]");
        let mut files = package().files.clone();
        files.insert(MANIFEST_FILE_NAME.to_string(), manifest.into_bytes());

        let package = validated_package(files).unwrap();
        assert_eq!(
            package.manifest().dependencies,
            vec![ProfileId::try_from("jit-default").unwrap()]
        );

        let schema = serde_json::to_value(profile_manifest_schema()).unwrap();
        let instance = serde_json::to_value(package.manifest()).unwrap();
        jsonschema::validator_for(&schema)
            .unwrap()
            .validate(&instance)
            .expect("manifest with dependency must satisfy generated schema");
        assert_eq!(instance["dependencies"][0], "jit-default");
    }

    #[test]
    fn test_manifest_inspection_carries_scalar_and_set_configuration_contributions() {
        let manifest: ProfileManifest = toml::from_str(
            r#"
[profile]
manifest-version = 1
id = "configuration"
version = "1.0.0"
jit = ">=1.0.0"

[[contribution]]
kind = "scalar"
target = "documentation-development-root"
value = "workspace"

[[contribution]]
kind = "scalar"
target = "validation-default-type"
value = "work-item"

[[contribution]]
kind = "set-string"
target = "documentation-managed-paths"
value = "workspace/active"
"#,
        )
        .expect("configuration contribution vocabulary parses");

        assert!(matches!(
            manifest.contributions.first(),
            Some(Contribution::Scalar {
                target: ScalarTarget::DocumentationDevelopmentRoot,
                value
            }) if value == "workspace"
        ));
        assert!(matches!(
            manifest.contributions.get(1),
            Some(Contribution::Scalar {
                target: ScalarTarget::ValidationDefaultType,
                value
            }) if value == "work-item"
        ));
        assert!(matches!(
            manifest.contributions.get(2),
            Some(Contribution::SetString {
                target: SetStringTarget::DocumentationManagedPaths,
                value
            }) if value == "workspace/active"
        ));

        let inspection = serde_json::to_value(&manifest).unwrap();
        assert_eq!(
            inspection["contribution"][0]["target"],
            "documentation-development-root"
        );
        assert_eq!(
            inspection["contribution"][2]["target"],
            "documentation-managed-paths"
        );
    }

    #[test]
    fn test_hashes_are_stable_grouped_by_target_and_cover_source_drift() {
        let first = package();
        let second = package();
        assert_eq!(first.hashes(), second.hashes());
        assert_eq!(
            first.hashes().package,
            "40a33c9798b0523436977f492bef09a4deddc9da7293fede33623c101b34a417"
        );
        assert_eq!(first.hashes().targets.len(), 7);
        assert_eq!(
            first.hashes().targets[".jit/config.toml"],
            "0b22a166dff17fbf500cd9682ba0348b3219aac7582736f9df8371f592de4694"
        );
        assert!(first.hashes().targets.contains_key(".jit/gates.toml"));
        assert!(first.hashes().targets.contains_key("AGENTS.md"));
        assert!(first.hashes().targets.contains_key("bin/check.sh"));

        let mut changed = first.files.clone();
        changed.insert(
            "nested/scripts/check.sh".to_string(),
            b"#!/bin/sh\nexit 1\n".to_vec(),
        );
        let changed = validated_package(changed).unwrap();
        assert_ne!(first.hashes().package, changed.hashes().package);
        assert_ne!(
            first.hashes().targets["bin/check.sh"],
            changed.hashes().targets["bin/check.sh"]
        );
        assert_eq!(
            first.hashes().targets["docs/workflow.txt"],
            changed.hashes().targets["docs/workflow.txt"]
        );

        let reordered_manifest = manifest_text().replacen(
            "[[contribution]]\nkind = \"map-entry\"\ntarget = \"type-hierarchy-types\"\nidentity = \"initiative\"\nvalue = 1\n\n",
            "",
            1,
        ) + "\n[[contribution]]\nkind = \"map-entry\"\ntarget = \"type-hierarchy-types\"\nidentity = \"initiative\"\nvalue = 1\n";
        let mut reordered = first.files.clone();
        reordered.insert(
            MANIFEST_FILE_NAME.to_string(),
            reordered_manifest.into_bytes(),
        );
        let reordered = validated_package(reordered).unwrap();
        assert_ne!(
            first.hashes().targets[".jit/config.toml"],
            reordered.hashes().targets[".jit/config.toml"]
        );
    }

    #[test]
    fn test_manifest_rejects_hooks_unknown_kinds_and_incomplete_projections() {
        let hook = parse_modified(
            "jit = \">=0.2.0, <2.0.0\"",
            "jit = \">=0.2.0, <2.0.0\"\nhook = \"install.sh\"",
        )
        .unwrap_err()
        .to_string();
        assert!(hook.contains("unknown field `hook`"), "{hook}");

        let kind = parse_modified("kind = \"map-entry\"", "kind = \"arbitrary-hook\"")
            .unwrap_err()
            .to_string();
        assert!(kind.contains("unknown variant `arbitrary-hook`"), "{kind}");

        let incomplete = parse_modified(
            "value = { kind = \"invariant\", mode = \"region\", target = \"AGENTS.md\", style = \"id-anchor\" }",
            "value = { kind = \"invariant\", mode = \"region\", target = \"AGENTS.md\" }",
        )
        .unwrap_err()
        .to_string();
        assert!(incomplete.contains("missing field `style`"), "{incomplete}");
    }

    #[test]
    fn test_manifest_rejects_malformed_dependency_id_during_parse() {
        let error = parse_modified("[profile]", "dependencies = [\"Jit Default\"]\n\n[profile]")
            .unwrap_err()
            .to_string();
        assert!(error.contains("Jit Default"), "{error}");
        assert!(error.contains("lowercase-kebab"), "{error}");
    }

    #[test]
    fn test_validation_rejects_self_referential_dependency_by_id() {
        let manifest = manifest_text().replace(
            "[profile]",
            "dependencies = [\"synthetic-workflow\"]\n\n[profile]",
        );
        let mut files = package().files.clone();
        files.insert(MANIFEST_FILE_NAME.to_string(), manifest.into_bytes());

        let error = validated_package(files).unwrap_err().to_string();
        assert!(error.contains("synthetic-workflow"), "{error}");
        assert!(error.contains("cannot depend on itself"), "{error}");
    }

    #[test]
    fn test_validation_rejects_unsafe_paths_and_invalid_semantics() {
        let mut files = package().files.clone();
        let unsafe_manifest = manifest_text().replace(
            "target = \"docs/workflow.txt\"",
            "target = \"../outside.txt\"",
        );
        files.insert(MANIFEST_FILE_NAME.to_string(), unsafe_manifest.into_bytes());
        assert!(matches!(
            validated_package(files),
            Err(ProfilePackageError::UnsafePath { .. })
        ));

        let windows_absolute = manifest_text().replace(
            "target = \"docs/workflow.txt\"",
            "target = \"C:/outside.txt\"",
        );
        let mut files = package().files.clone();
        files.insert(
            MANIFEST_FILE_NAME.to_string(),
            windows_absolute.into_bytes(),
        );
        assert!(matches!(
            validated_package(files),
            Err(ProfilePackageError::UnsafePath { .. })
        ));

        let bad_map = parse_modified("value = 1", "value = \"one\"").unwrap();
        let files = package().files.clone();
        assert!(matches!(
            validate_manifest(&bad_map, &files),
            Err(ProfilePackageError::InvalidContribution { index: 0, .. })
        ));

        let bad_key = parse_modified(
            "value = { key = \"synthetic-review\", title = \"Synthetic review\" }",
            "value = { key = \"other-review\", title = \"Synthetic review\" }",
        )
        .unwrap();
        assert!(matches!(
            validate_manifest(&bad_key, &files),
            Err(ProfilePackageError::InvalidContribution { index: 6, .. })
        ));
    }

    #[test]
    fn test_validation_rejects_invalid_identity_version_and_compatibility() {
        let package = package();
        let cases = [
            (
                "id = \"synthetic-workflow\"",
                "id = \"Synthetic Workflow\"",
                "invalid profile id",
            ),
            (
                "version = \"1.2.3\"",
                "version = \"01.2.3\"",
                "invalid profile version",
            ),
            (
                "jit = \">=0.2.0, <2.0.0\"",
                "jit = \"not a requirement\"",
                "invalid compatible JIT requirement",
            ),
            (
                "manifest-version = 1",
                "manifest-version = 2",
                "unsupported profile manifest version",
            ),
        ];

        for (old, new, expected) in cases {
            let manifest = manifest_text().replace(old, new);
            let mut files = package.files.clone();
            files.insert(MANIFEST_FILE_NAME.to_string(), manifest.into_bytes());
            let error = validated_package(files).unwrap_err().to_string();
            assert!(error.contains(expected), "{error}");
        }
    }

    #[test]
    fn test_validation_rejects_missing_extra_and_duplicate_content() {
        let package = package();

        let mut missing = package.files.clone();
        missing.remove("assets/workflow.txt");
        assert!(matches!(
            validated_package(missing),
            Err(ProfilePackageError::MissingContent(path)) if path == "assets/workflow.txt"
        ));

        let mut extra = package.files.clone();
        extra.insert("assets/undeclared.txt".to_string(), b"extra".to_vec());
        assert!(matches!(
            validated_package(extra),
            Err(ProfilePackageError::ExtraContent(path)) if path == "assets/undeclared.txt"
        ));

        let duplicate_manifest = manifest_text().replace(
            "source = \"nested/scripts/check.sh\"",
            "source = \"assets/workflow.txt\"",
        );
        let mut duplicate = package.files.clone();
        duplicate.insert(
            MANIFEST_FILE_NAME.to_string(),
            duplicate_manifest.into_bytes(),
        );
        assert!(matches!(
            validated_package(duplicate),
            Err(ProfilePackageError::DuplicateSource(path)) if path == "assets/workflow.txt"
        ));

        let reserved_manifest = manifest_text().replace(
            "source = \"nested/scripts/check.sh\"",
            "source = \"manifest.toml\"",
        );
        let mut reserved = package.files.clone();
        reserved.insert(
            MANIFEST_FILE_NAME.to_string(),
            reserved_manifest.into_bytes(),
        );
        assert!(matches!(
            validated_package(reserved),
            Err(ProfilePackageError::DuplicateSource(path)) if path == "manifest.toml"
        ));

        let duplicate_contribution = manifest_text().replace(
            "[[asset]]\nsource = \"assets/workflow.txt\"",
            "[[contribution]]\nkind = \"projection\"\nname = \"invariants\"\nvalue = { kind = \"invariant\", mode = \"separate-file\", target = \"OTHER.md\", style = \"full\" }\n\n[[asset]]\nsource = \"assets/workflow.txt\"",
        );
        let mut duplicate = package.files.clone();
        duplicate.insert(
            MANIFEST_FILE_NAME.to_string(),
            duplicate_contribution.into_bytes(),
        );
        assert!(matches!(
            validated_package(duplicate),
            Err(ProfilePackageError::DuplicateContribution(identity))
                if identity.contains("invariants")
        ));
    }

    #[test]
    fn test_validation_rejects_package_count_and_size_over_budget() {
        let package = package();

        let mut too_many = package.files.clone();
        for index in 0..MAX_PROFILE_PACKAGE_FILES {
            too_many.insert(format!("undeclared/{index}.txt"), b"x".to_vec());
        }
        assert!(matches!(
            validated_package(too_many),
            Err(ProfilePackageError::PackageBounds {
                file_count,
                ..
            }) if file_count > MAX_PROFILE_PACKAGE_FILES
        ));

        let mut too_large = package.files.clone();
        too_large.insert(
            "undeclared/oversized.bin".to_string(),
            vec![b'x'; MAX_PROFILE_PACKAGE_BYTES],
        );
        assert!(matches!(
            validated_package(too_large),
            Err(ProfilePackageError::PackageBounds {
                byte_size,
                ..
            }) if byte_size > MAX_PROFILE_PACKAGE_BYTES
        ));
    }

    #[test]
    fn test_embedding_dependency_contract_has_no_optional_features_and_production_tree() {
        let manifest = include_str!("../../Cargo.toml");
        assert!(
            manifest.contains("include_dir = { version = \"0.7.4\", default-features = false }"),
            "embedding dependency must remain pinned without optional features"
        );
        let production_tree =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../profiles/jit-dogfood");
        assert!(production_tree.join(MANIFEST_FILE_NAME).is_file());
        assert!(!manifest_text().contains("jit-dogfood"));
    }

    #[test]
    fn test_from_directory_reads_the_same_package_as_the_compile_time_route() {
        let embedded = package();
        let read = ProfilePackage::from_directory(&fixture_tree()).expect("valid package tree");

        assert_eq!(read.manifest(), embedded.manifest());
        assert_eq!(read.hashes().package, embedded.hashes().package);
        assert_eq!(read.hashes().targets, embedded.hashes().targets);
        assert_eq!(read.file_count(), embedded.file_count());
        assert_eq!(read.byte_size(), embedded.byte_size());
        assert!(read
            .manifest()
            .assets
            .iter()
            .all(|asset| read.source_bytes(&asset.source) == embedded.source_bytes(&asset.source)));

        // The package outlives the tree it was read from, which is what owning
        // the validated bytes buys: this temporary directory is deleted before
        // anything below reads the package back.
        let owned = {
            let temp = TempDir::new().unwrap();
            let root = writable_package_tree(&temp);
            ProfilePackage::from_directory(&root).expect("valid package tree")
        };
        assert_eq!(owned.hashes(), embedded.hashes());
        assert!(
            owned
                .manifest()
                .assets
                .iter()
                .all(|asset| owned.source_bytes(&asset.source)
                    == embedded.source_bytes(&asset.source))
        );
    }

    #[test]
    fn test_from_directory_records_the_resolved_root_its_bytes_were_read_from() {
        // Two copies of one authored tree differ only in where they sit, so the
        // recorded source follows the directory each package was read through
        // rather than anything about the bytes, which are identical.
        let temp = TempDir::new().unwrap();
        let first = writable_package_tree(&temp);
        let second = writable_package_tree_at(&temp.path().join("elsewhere/package"));

        let first_package = ProfilePackage::from_directory(&first).expect("valid package tree");
        let second_package = ProfilePackage::from_directory(&second).expect("valid package tree");

        assert_eq!(first_package.hashes(), second_package.hashes());
        assert_eq!(
            first_package.source(),
            &ProfilePackageSource::Directory(fs::canonicalize(&first).unwrap())
        );
        assert_eq!(
            second_package.source(),
            &ProfilePackageSource::Directory(fs::canonicalize(&second).unwrap())
        );
        assert_ne!(first_package.source(), second_package.source());
    }

    #[test]
    fn test_from_directory_records_the_root_a_relative_argument_resolves_to() {
        // The recorded source has to name the directory the walk anchored at,
        // so a package read through a relative or link-traversing argument
        // still reports where its bytes came from.
        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        let resolved = fs::canonicalize(&root).unwrap();
        let indirect = root.join("..").join(root.file_name().unwrap());

        assert_eq!(
            ProfilePackage::from_directory(&indirect)
                .expect("valid package tree")
                .source(),
            &ProfilePackageSource::Directory(resolved)
        );
    }

    #[test]
    fn test_from_embedded_dir_records_a_compiled_in_source() {
        assert_eq!(package().source(), &ProfilePackageSource::Embedded);
    }

    #[test]
    fn test_from_directory_applies_the_package_count_and_size_bounds() {
        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::fs::create_dir_all(root.join("undeclared")).unwrap();
        (0..MAX_PROFILE_PACKAGE_FILES).for_each(|index| {
            std::fs::write(root.join(format!("undeclared/{index}.txt")), b"x").unwrap();
        });
        assert!(matches!(
            ProfilePackage::from_directory(&root),
            Err(ProfilePackageError::PackageBounds { file_count, max_files, .. })
                if file_count > max_files
        ));

        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::fs::write(
            root.join("oversized.bin"),
            vec![b'x'; MAX_PROFILE_PACKAGE_BYTES],
        )
        .unwrap();
        assert!(matches!(
            ProfilePackage::from_directory(&root),
            Err(ProfilePackageError::PackageBounds { byte_size, max_bytes, .. })
                if byte_size > max_bytes
        ));
    }

    #[test]
    fn test_from_directory_applies_the_path_shape_and_declaration_defences() {
        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        rewrite_manifest(
            &root,
            "target = \"docs/workflow.txt\"",
            "target = \"../outside.txt\"",
        );
        assert!(matches!(
            ProfilePackage::from_directory(&root),
            Err(ProfilePackageError::UnsafePath { path, .. }) if path == "../outside.txt"
        ));

        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::fs::remove_file(root.join("assets/workflow.txt")).unwrap();
        assert!(matches!(
            ProfilePackage::from_directory(&root),
            Err(ProfilePackageError::MissingContent(path)) if path == "assets/workflow.txt"
        ));

        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::fs::write(root.join("assets/undeclared.txt"), b"extra").unwrap();
        assert!(matches!(
            ProfilePackage::from_directory(&root),
            Err(ProfilePackageError::ExtraContent(path)) if path == "assets/undeclared.txt"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_from_directory_rejects_an_entry_that_is_not_a_regular_file() {
        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        nix::unistd::mkfifo(&root.join("assets/pipe"), nix::sys::stat::Mode::S_IRWXU).unwrap();
        let error = ProfilePackage::from_directory(&root).unwrap_err();
        assert!(
            matches!(&error, ProfilePackageError::IrregularEntry { path } if path == "assets/pipe"),
            "{error}"
        );
        assert!(error.to_string().contains("assets/pipe"), "{error}");

        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::os::unix::fs::symlink("workflow.txt", root.join("assets/link.txt")).unwrap();
        let error = ProfilePackage::from_directory(&root).unwrap_err();
        assert!(
            matches!(&error, ProfilePackageError::IrregularEntry { path } if path == "assets/link.txt"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_from_directory_rejects_an_entry_escaping_the_package_root() {
        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::fs::write(temp.path().join("outside.txt"), b"outside").unwrap();
        std::os::unix::fs::symlink("../../outside.txt", root.join("assets/escape.txt")).unwrap();
        let error = ProfilePackage::from_directory(&root).unwrap_err();
        assert!(
            matches!(&error, ProfilePackageError::EscapingEntry { path } if path == "assets/escape.txt"),
            "{error}"
        );
        assert!(error.to_string().contains("assets/escape.txt"), "{error}");

        let temp = TempDir::new().unwrap();
        let root = writable_package_tree(&temp);
        std::os::unix::fs::symlink(temp.path(), root.join("escape")).unwrap();
        assert!(matches!(
            ProfilePackage::from_directory(&root),
            Err(ProfilePackageError::EscapingEntry { path }) if path == "escape"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_read_regular_entry_refuses_a_name_relinked_outward_after_it_was_listed() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(writable_package_tree(&temp)).unwrap();
        std::fs::write(temp.path().join("outside.txt"), b"outside bytes").unwrap();

        let assets = CapDir::open_ambient_dir(root.join("assets"), ambient_authority()).unwrap();
        let entry = assets
            .entries()
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name() == "workflow.txt")
            .expect("the declared asset is listed");
        assert!(
            entry.file_type().unwrap().is_file(),
            "the walk classifies this entry as a regular file"
        );

        // The state a concurrent rename produces between the classification
        // above and the read below: same name, now a link out of the package.
        std::fs::remove_file(root.join("assets/workflow.txt")).unwrap();
        std::os::unix::fs::symlink(
            temp.path().join("outside.txt"),
            root.join("assets/workflow.txt"),
        )
        .unwrap();

        let error = read_regular_entry(&entry, &root, "assets/workflow.txt", 0, 0)
            .expect_err("bytes from outside the package root must not be admitted");
        assert!(
            matches!(&error, ProfilePackageError::EscapingEntry { path } if path == "assets/workflow.txt"),
            "{error}"
        );
        assert!(
            ProfilePackage::from_directory(&root).is_err(),
            "a package whose entry resolves outside the root cannot be read at all"
        );
    }

    /// Run one package read off-thread and report what it produced, or `None`
    /// when it had not finished within `budget`.
    ///
    /// A read that waits for something is a defect these tests have to be able
    /// to observe: on its own thread it becomes an assertable `None` instead of
    /// a suite that never ends.
    #[cfg(unix)]
    fn read_within(
        budget: std::time::Duration,
        read: impl FnOnce() -> Result<Vec<u8>, ProfilePackageError> + Send + 'static,
    ) -> Option<Result<Vec<u8>, String>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || sender.send(read().map_err(|error| error.to_string())));
        receiver.recv_timeout(budget).ok()
    }

    #[cfg(unix)]
    #[test]
    fn test_read_regular_entry_refuses_a_name_replaced_by_a_pipe_after_it_was_listed() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(writable_package_tree(&temp)).unwrap();

        let assets = CapDir::open_ambient_dir(root.join("assets"), ambient_authority()).unwrap();
        let entry = assets
            .entries()
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name() == "workflow.txt")
            .expect("the declared asset is listed");
        assert!(
            entry.file_type().unwrap().is_file(),
            "the walk classifies this entry as a regular file"
        );

        // The state a concurrent rename produces between the classification
        // above and the read below: same name, now a pipe with no writer. An
        // open that waits for a peer would never reject anything.
        std::fs::remove_file(root.join("assets/workflow.txt")).unwrap();
        nix::unistd::mkfifo(
            &root.join("assets/workflow.txt"),
            nix::sys::stat::Mode::S_IRWXU,
        )
        .unwrap();

        let read_root = root.clone();
        let outcome = read_within(std::time::Duration::from_secs(10), move || {
            read_regular_entry(&entry, &read_root, "assets/workflow.txt", 0, 0)
        })
        .expect("reading an entry replaced by a pipe must return rather than wait for a writer");
        let error = outcome.expect_err("a pipe is not package content");
        assert!(
            error.contains("assets/workflow.txt") && error.contains("not a regular file"),
            "{error}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_read_regular_entry_returns_a_multi_megabyte_regular_file_whole() {
        // The non-blocking open must be inert for the regular files a package
        // is made of, at a size that takes more than one read to drain.
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(writable_package_tree(&temp)).unwrap();
        let content = (0..3 * 1024 * 1024u32)
            .map(|index| index as u8)
            .collect::<Vec<_>>();
        std::fs::write(root.join("assets/large.bin"), &content).unwrap();

        let assets = CapDir::open_ambient_dir(root.join("assets"), ambient_authority()).unwrap();
        let entry = assets
            .entries()
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name() == "large.bin")
            .expect("the written file is listed");

        let bytes = read_regular_entry(&entry, &root, "assets/large.bin", 0, 0).unwrap();
        assert_eq!(bytes, content);
        assert_eq!(bytes, std::fs::read(root.join("assets/large.bin")).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn test_read_regular_entry_refuses_a_name_replaced_by_a_socket_after_it_was_listed() {
        let temp = TempDir::new().unwrap();
        let root = std::fs::canonicalize(writable_package_tree(&temp)).unwrap();

        let assets = CapDir::open_ambient_dir(root.join("assets"), ambient_authority()).unwrap();
        let entry = assets
            .entries()
            .unwrap()
            .map(Result::unwrap)
            .find(|entry| entry.file_name() == "workflow.txt")
            .expect("the declared asset is listed");

        std::fs::remove_file(root.join("assets/workflow.txt")).unwrap();
        let _listener =
            std::os::unix::net::UnixListener::bind(root.join("assets/workflow.txt")).unwrap();

        let read_root = root.clone();
        let outcome = read_within(std::time::Duration::from_secs(10), move || {
            read_regular_entry(&entry, &read_root, "assets/workflow.txt", 0, 0)
        })
        .expect("reading an entry replaced by a socket must return");
        let error = outcome.expect_err("a socket is not package content");
        assert!(
            error.contains("assets/workflow.txt") && error.contains("not a regular file"),
            "{error}"
        );
    }

    #[test]
    fn test_from_directory_reports_an_absent_or_unreadable_directory() {
        let temp = TempDir::new().unwrap();
        let absent = temp.path().join("no-such-package");
        let error = ProfilePackage::from_directory(&absent).unwrap_err();
        assert!(
            matches!(&error, ProfilePackageError::UnreadableDirectory { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound),
            "{error}"
        );
        assert!(error.to_string().contains("no-such-package"), "{error}");

        let file = temp.path().join("package-file");
        std::fs::write(&file, b"not a directory").unwrap();
        let error = ProfilePackage::from_directory(&file).unwrap_err();
        assert!(
            matches!(&error, ProfilePackageError::UnreadableDirectory { .. }),
            "{error}"
        );
    }
}
