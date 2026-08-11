//! The portable file one package travels in, and the read that verifies it.
//!
//! A package is a directory. Handing one to somebody means handing over a
//! directory, which says nothing about what was copied or whether it arrived
//! whole. This module gives a package one file that states which package it is
//! and what its content hashes to, and one read that holds an arrival against
//! that statement before anything is placed in a repository.
//!
//! # What the archive holds
//!
//! An uncompressed tar with a fixed layout: one metadata file at the archive
//! root naming the package and carrying its identity digest, and the package's
//! own tree beneath a single `package/` directory. Nothing else is admitted.
//! The archive is uncompressed by decision: an inflating layer's expanded size
//! is not knowable before inflation, so the bounds this module enforces during
//! extraction could not be enforced at all.
//!
//! # What the digest establishes
//!
//! Integrity, not authenticity. The digest travels inside the archive, so
//! whoever can rewrite the content can rewrite the digest with it. Recomputing
//! it from the extracted content detects truncation, corruption in transit, and
//! accidental modification. It says nothing about who produced the archive, and
//! it is not a signature: the channel an adopter obtained the archive over is
//! what carries that.
//!
//! # Reading an archive is reading hostile input
//!
//! Every byte of an arriving archive is untrusted, so the read refuses rather
//! than repairs. Entry names are validated as safe relative paths before any
//! content is admitted, only regular files and directories are admitted at all,
//! the package bounds are enforced against running counts as entries are read
//! rather than after they have landed, and a mode is compared against what the
//! manifest declares rather than adopted from the entry. The result is a value
//! in memory: this module opens nothing, writes nothing, and leaves the
//! decision of where a package goes to its caller.

use super::manifest::{ProfileId, ProfilePackageModel, MANIFEST_FILE_NAME};
use super::package::{
    ProfilePackage, ProfilePackageError, MAX_PROFILE_PACKAGE_BYTES, MAX_PROFILE_PACKAGE_FILES,
};
use super::package_capture::{compose_captured_tree, CapturedFile, CapturedPackageTree};
use crate::domain::repository_inputs::is_safe_relative_path;
use crate::repository_state::FileMode;
use crate::tar_format::reproducible_tar_header;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use tar::EntryType;

/// Archive-root file naming the package an archive carries.
const ARCHIVE_METADATA_ENTRY: &str = "jit-package-archive.toml";

/// Archive-root directory every package file sits beneath.
const PACKAGE_ENTRY_PREFIX: &str = "package";

/// The one archive wire this build reads and writes.
const ARCHIVE_VERSION: u32 = 1;

/// Mode every archived directory entry carries.
const DIRECTORY_MODE: u32 = 0o755;

/// Mode an archived file carries when its declaration is executable.
const EXECUTABLE_FILE_MODE: u32 = 0o755;

/// Mode every other archived file carries.
const REGULAR_FILE_MODE: u32 = 0o644;

/// Maximum bytes the archive metadata may occupy.
///
/// The metadata is four scalars, so this is generous by three orders of
/// magnitude; it exists so a hostile archive cannot make the read allocate for
/// a metadata entry that claims to be enormous.
const MAX_ARCHIVE_METADATA_BYTES: usize = 4 * 1024;

/// Maximum number of tar entries one package archive may hold.
///
/// A package archive holds the metadata, one entry per package file, and one
/// directory entry per distinct parent directory of those files, of which there
/// can be no more than one per file. The bound is therefore the package's own
/// file bound twice over plus the metadata and the `package/` root, and it
/// exists so an archive of many empty entries cannot exhaust inodes or run the
/// read forever.
pub const MAX_PROFILE_PACKAGE_ARCHIVE_ENTRIES: usize = 2 * MAX_PROFILE_PACKAGE_FILES + 2;

/// Maximum bytes one package archive may occupy.
///
/// The content bound is the package's own; the rest is tar framing. Each entry
/// costs a 512-byte header, up to 511 bytes of content padding, and, when its
/// path is too long for the header, a long-name entry of its own — 2 KiB per
/// entry covers all three, and the trailing end-of-archive blocks are a
/// rounding error beside it.
pub const MAX_PROFILE_PACKAGE_ARCHIVE_BYTES: usize =
    MAX_PROFILE_PACKAGE_BYTES + MAX_PROFILE_PACKAGE_ARCHIVE_ENTRIES * 2 * 1024;

/// The archive metadata wire, decoded as strictly as a package manifest is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ArchiveMetadataWire {
    archive: ArchiveMetadata,
}

/// What an archive states about the package it carries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ArchiveMetadata {
    /// Archive wire version.
    archive_version: u32,
    /// Stable identifier the packaged manifest declares.
    id: ProfileId,
    /// Semantic version the packaged manifest declares.
    version: String,
    /// Identity digest of the packaged content.
    package_hash: String,
}

/// Why an archive could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum PackageArchiveError {
    /// An entry is neither a regular file nor a directory.
    #[error("profile package archive entry '{path}' is neither a regular file nor a directory")]
    UnsupportedEntry {
        /// Entry name as the archive spells it.
        path: String,
    },
    /// An entry names an absolute path, a traversal, or another unsafe shape.
    #[error("profile package archive entry '{path}' is not a safe relative path")]
    UnsafeEntryPath {
        /// Entry name as the archive spells it.
        path: String,
    },
    /// An entry sits outside the archive layout.
    #[error("profile package archive entry '{path}' is not part of the archive layout")]
    UnexpectedEntry {
        /// Entry name as the archive spells it.
        path: String,
    },
    /// One name is carried by two entries.
    #[error("profile package archive carries '{path}' more than once")]
    DuplicateEntry {
        /// Repeated entry name.
        path: String,
    },
    /// An entry carries a mode its declaration does not.
    #[error(
        "profile package archive entry '{path}' carries mode {actual:o}, but the packaged \
         manifest declares mode {expected:o}"
    )]
    UnexpectedEntryMode {
        /// Entry name as the archive spells it.
        path: String,
        /// Mode the entry carries.
        actual: u32,
        /// Mode the manifest declares for it.
        expected: u32,
    },
    /// A required archive entry is absent.
    #[error("profile package archive is missing '{0}'")]
    MissingEntry(&'static str),
    /// The metadata is not UTF-8.
    #[error("profile package archive metadata is not UTF-8: {0}")]
    MetadataUtf8(#[source] std::str::Utf8Error),
    /// The metadata is not the recognized wire.
    #[error("invalid profile package archive metadata: {0}")]
    MetadataToml(#[source] toml::de::Error),
    /// The metadata could not be written.
    #[error("failed to serialize profile package archive metadata: {0}")]
    MetadataSerialization(#[source] toml::ser::Error),
    /// The archive wire has no reader in this build.
    #[error("unsupported profile package archive version {actual}; expected {expected}")]
    ArchiveVersion {
        /// Version the archive declares.
        actual: u32,
        /// Version this build reads.
        expected: u32,
    },
    /// The metadata states something the packaged manifest contradicts.
    #[error(
        "profile package archive states {field} '{declared}', but the packaged manifest \
         declares '{packaged}'"
    )]
    MetadataDisagreement {
        /// Metadata field that disagrees.
        field: &'static str,
        /// Value the metadata states.
        declared: String,
        /// Value the packaged manifest declares.
        packaged: String,
    },
    /// The extracted content does not hash to the digest the archive carries.
    #[error(
        "profile package archive content hashes to '{computed}', but the archive carries \
         '{declared}'"
    )]
    IdentityMismatch {
        /// Digest the archive carries.
        declared: String,
        /// Digest recomputed from the extracted content.
        computed: String,
    },
    /// The archive is larger than a package archive may be.
    #[error("profile package archive is {byte_size} bytes; the maximum is {max_bytes} bytes")]
    ArchiveBytes {
        /// Observed size.
        byte_size: usize,
        /// Maximum supported size.
        max_bytes: usize,
    },
    /// The archive metadata is larger than four scalars can be.
    #[error(
        "profile package archive metadata is {byte_size} bytes; the maximum is {max_bytes} bytes"
    )]
    MetadataBytes {
        /// Observed size.
        byte_size: usize,
        /// Maximum supported size.
        max_bytes: usize,
    },
    /// The archive holds more entries than a package archive may.
    #[error("profile package archive holds more than {max_entries} entries")]
    ArchiveEntries {
        /// Maximum supported entry count.
        max_entries: usize,
    },
    /// The archived content is not a valid package.
    #[error("the archived content is not a valid profile package: {0}")]
    InvalidPackage(#[source] ProfilePackageError),
    /// The archive could not be walked.
    #[error("cannot read profile package archive: {0}")]
    Io(#[from] std::io::Error),
}

/// Build the portable archive carrying `package`.
///
/// The bytes are a function of the package alone: entries are written in
/// canonical package order through [`reproducible_tar_header`], so packing the
/// same package twice produces the same file, and packing it on another machine
/// produces that same file again.
///
/// Each file is written with the mode its manifest declaration implies rather
/// than with any mode the package directory happened to carry, so the manifest
/// stays the authority on which assets are executable.
///
/// # Errors
///
/// [`PackageArchiveError::MetadataSerialization`] when the metadata cannot be
/// encoded, and [`PackageArchiveError::Io`] when the in-memory tar writer
/// fails.
pub fn pack_package_archive(package: &ProfilePackage) -> Result<Vec<u8>, PackageArchiveError> {
    let model = package.model();
    let metadata = toml::to_string(&ArchiveMetadataWire {
        archive: ArchiveMetadata {
            archive_version: ARCHIVE_VERSION,
            id: model.id.clone(),
            version: model.version.clone(),
            package_hash: package.hashes().package.clone(),
        },
    })
    .map_err(PackageArchiveError::MetadataSerialization)?;

    let executable = declared_executable_sources(model);
    let mut builder = tar::Builder::new(Vec::new());
    append_entry(
        &mut builder,
        EntryType::Regular,
        REGULAR_FILE_MODE,
        ARCHIVE_METADATA_ENTRY,
        metadata.as_bytes(),
    )?;
    for directory in archived_directories(package.files().keys().map(String::as_str)) {
        append_entry(
            &mut builder,
            EntryType::Directory,
            DIRECTORY_MODE,
            &directory,
            &[],
        )?;
    }
    for (source, bytes) in package.files() {
        append_entry(
            &mut builder,
            EntryType::Regular,
            declared_file_mode(&executable, source),
            &format!("{PACKAGE_ENTRY_PREFIX}/{source}"),
            bytes,
        )?;
    }
    builder.finish()?;
    Ok(builder.into_inner()?)
}

/// Read one archive into the package tree it carries, refusing anything the
/// archive says about itself that its own content does not bear out.
///
/// The read is total before it returns anything: every entry is admitted or
/// refused, the content is validated as a package, its identity is recomputed
/// from the extracted bytes, and the metadata's identity, version, and digest
/// are held against that. A returned tree is therefore a package this
/// repository can publish, whose identity is the one the archive claimed.
///
/// # Errors
///
/// Every [`PackageArchiveError`]: an entry of an inadmissible kind, an unsafe
/// or unexpected entry name, a repeated entry, a mode the packaged manifest
/// does not declare, absent metadata, metadata that is not the recognized wire
/// or states a version this build does not read, content over the package
/// bounds, content that is not a valid package, and a digest the extracted
/// content does not reproduce.
pub fn read_package_archive(bytes: &[u8]) -> Result<CapturedPackageTree, PackageArchiveError> {
    let extracted = extract_archive_entries(bytes)?;
    let metadata = decode_archive_metadata(&extracted.metadata)?;
    if metadata.archive_version != ARCHIVE_VERSION {
        return Err(PackageArchiveError::ArchiveVersion {
            actual: metadata.archive_version,
            expected: ARCHIVE_VERSION,
        });
    }

    // The manifest is read before the tree is composed, for the same reason a
    // capture reads it first: it is what says which sources exist and which of
    // them are executable, and neither answer may come from the archive.
    let manifest =
        extracted
            .files
            .get(MANIFEST_FILE_NAME)
            .ok_or(PackageArchiveError::InvalidPackage(
                ProfilePackageError::MissingManifest,
            ))?;
    let model = ProfilePackage::parse_manifest(&manifest.bytes)
        .map_err(PackageArchiveError::InvalidPackage)?;
    let executable = declared_executable_sources(&model);
    for (source, file) in &extracted.files {
        let expected = declared_file_mode(&executable, source);
        if file.mode != expected {
            return Err(PackageArchiveError::UnexpectedEntryMode {
                path: format!("{PACKAGE_ENTRY_PREFIX}/{source}"),
                actual: file.mode,
                expected,
            });
        }
    }

    // What the entries carried, which is what lets the shared package
    // validation refuse an executable no declaration accounts for. The modes
    // published below are the manifest's declarations, never these.
    let observed_executable = extracted
        .files
        .iter()
        .filter(|(_, file)| file.mode & 0o111 != 0)
        .map(|(source, _)| source.clone())
        .collect::<BTreeSet<_>>();
    let files = extracted
        .files
        .into_iter()
        .map(|(source, file)| {
            let mode = if executable.contains(source.as_str()) {
                FileMode::Executable
            } else {
                FileMode::Regular
            };
            (
                source,
                CapturedFile {
                    bytes: file.bytes,
                    mode,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    let tree = compose_captured_tree(files, observed_executable)
        .map_err(PackageArchiveError::InvalidPackage)?;
    require_metadata_agreement(&metadata, &tree)?;
    Ok(tree)
}

/// The sources a manifest declares executable.
///
/// The one derivation of what an archived file's mode is, read by the write
/// side to set it and by the read side to refuse a mode that disagrees, so the
/// two sides cannot drift apart.
fn declared_executable_sources(model: &ProfilePackageModel) -> BTreeSet<&str> {
    model
        .assets
        .iter()
        .filter(|asset| asset.executable)
        .map(|asset| asset.source.as_str())
        .collect()
}

/// The mode one archived package file carries.
fn declared_file_mode(executable: &BTreeSet<&str>, source: &str) -> u32 {
    if executable.contains(source) {
        EXECUTABLE_FILE_MODE
    } else {
        REGULAR_FILE_MODE
    }
}

/// Every directory entry a package tree needs, parents before their children.
///
/// Sorting by depth over an already name-ordered set is what puts a parent
/// before the child it holds, which is what a tar reader extracting the archive
/// with ordinary tools needs.
fn archived_directories<'a>(sources: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut directories = sources
        .flat_map(|source| {
            let segments = source.split('/').collect::<Vec<_>>();
            (0..segments.len().saturating_sub(1))
                .map(|depth| format!("{PACKAGE_ENTRY_PREFIX}/{}", segments[..=depth].join("/")))
                .collect::<Vec<_>>()
        })
        .chain(std::iter::once(PACKAGE_ENTRY_PREFIX.to_string()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    directories.sort_by_key(|directory| directory.matches('/').count());
    directories
}

/// Append one entry under the reproducible header discipline.
fn append_entry(
    builder: &mut tar::Builder<Vec<u8>>,
    entry_type: EntryType,
    mode: u32,
    path: &str,
    bytes: &[u8],
) -> Result<(), PackageArchiveError> {
    let mut header = reproducible_tar_header(entry_type, mode, bytes.len() as u64);
    builder.append_data(&mut header, path, bytes)?;
    Ok(())
}

/// One archived package file as the archive carried it.
struct ArchivedFile {
    bytes: Vec<u8>,
    mode: u32,
}

/// Everything one archive walk admitted.
struct ExtractedArchive {
    metadata: Vec<u8>,
    files: BTreeMap<String, ArchivedFile>,
}

/// What one admitted entry name addresses.
enum ArchiveEntryName<'a> {
    /// The archive metadata at the archive root.
    Metadata,
    /// The `package/` root itself.
    PackageRoot,
    /// A package-relative path beneath `package/`.
    PackageRelative(&'a str),
}

/// Classify one validated entry name against the archive layout.
fn classify_entry_name(name: &str) -> Option<ArchiveEntryName<'_>> {
    if name == ARCHIVE_METADATA_ENTRY {
        Some(ArchiveEntryName::Metadata)
    } else if name == PACKAGE_ENTRY_PREFIX {
        Some(ArchiveEntryName::PackageRoot)
    } else {
        name.strip_prefix(PACKAGE_ENTRY_PREFIX)
            .and_then(|rest| rest.strip_prefix('/'))
            .map(ArchiveEntryName::PackageRelative)
    }
}

/// Walk one archive, admitting what the layout allows and refusing the rest.
///
/// The bounds are applied to running counts as the walk proceeds — before an
/// entry's declared size is trusted, and again against the bytes actually read
/// — so an archive over budget is refused while it is being read rather than
/// after its bytes have been held.
fn extract_archive_entries(bytes: &[u8]) -> Result<ExtractedArchive, PackageArchiveError> {
    if bytes.len() > MAX_PROFILE_PACKAGE_ARCHIVE_BYTES {
        return Err(PackageArchiveError::ArchiveBytes {
            byte_size: bytes.len(),
            max_bytes: MAX_PROFILE_PACKAGE_ARCHIVE_BYTES,
        });
    }
    let mut archive = tar::Archive::new(bytes);
    let mut metadata: Option<Vec<u8>> = None;
    let mut files: BTreeMap<String, ArchivedFile> = BTreeMap::new();
    let mut byte_size = 0usize;
    let mut entries = 0usize;

    for entry in archive.entries()? {
        let mut entry = entry?;
        entries = entries.saturating_add(1);
        if entries > MAX_PROFILE_PACKAGE_ARCHIVE_ENTRIES {
            return Err(PackageArchiveError::ArchiveEntries {
                max_entries: MAX_PROFILE_PACKAGE_ARCHIVE_ENTRIES,
            });
        }
        let kind = entry.header().entry_type();
        let name = entry_name(&entry)?;
        let mode = entry.header().mode()? & 0o7777;

        match (kind, classify_entry_name(&name)) {
            (
                EntryType::Directory,
                Some(ArchiveEntryName::PackageRoot | ArchiveEntryName::PackageRelative(_)),
            ) => {
                if mode != DIRECTORY_MODE {
                    return Err(PackageArchiveError::UnexpectedEntryMode {
                        path: name,
                        actual: mode,
                        expected: DIRECTORY_MODE,
                    });
                }
            }
            (EntryType::Regular, Some(ArchiveEntryName::Metadata)) => {
                if metadata.is_some() {
                    return Err(PackageArchiveError::DuplicateEntry { path: name });
                }
                let mut content = Vec::new();
                entry
                    .by_ref()
                    .take(MAX_ARCHIVE_METADATA_BYTES as u64 + 1)
                    .read_to_end(&mut content)?;
                if content.len() > MAX_ARCHIVE_METADATA_BYTES {
                    return Err(PackageArchiveError::MetadataBytes {
                        byte_size: content.len(),
                        max_bytes: MAX_ARCHIVE_METADATA_BYTES,
                    });
                }
                metadata = Some(content);
            }
            (EntryType::Regular, Some(ArchiveEntryName::PackageRelative(source))) => {
                let source = source.to_string();
                if files.contains_key(&source) {
                    return Err(PackageArchiveError::DuplicateEntry { path: name });
                }
                let admitted = files.len().saturating_add(1);
                let declared = usize::try_from(entry.size()).unwrap_or(usize::MAX);
                require_package_bounds(admitted, byte_size.saturating_add(declared))?;
                let headroom = MAX_PROFILE_PACKAGE_BYTES.saturating_sub(byte_size);
                let mut content = Vec::new();
                entry
                    .by_ref()
                    .take(headroom as u64 + 1)
                    .read_to_end(&mut content)?;
                require_package_bounds(admitted, byte_size.saturating_add(content.len()))?;
                byte_size = byte_size.saturating_add(content.len());
                files.insert(
                    source,
                    ArchivedFile {
                        bytes: content,
                        mode,
                    },
                );
            }
            // An admissible kind at a name the layout does not place it at:
            // content outside `package/`, or the metadata name carried by a
            // directory.
            (EntryType::Regular | EntryType::Directory, _) => {
                return Err(PackageArchiveError::UnexpectedEntry { path: name })
            }
            _ => return Err(PackageArchiveError::UnsupportedEntry { path: name }),
        }
    }

    Ok(ExtractedArchive {
        metadata: metadata.ok_or(PackageArchiveError::MissingEntry(ARCHIVE_METADATA_ENTRY))?,
        files,
    })
}

/// The name one entry carries, refused unless it is a safe relative path.
///
/// The name is taken as the archive stored it rather than as a platform would
/// resolve it, and the shared repository-input rule decides it: an absolute
/// path, a `..` component, a drive or UNC prefix, a backslash, a control
/// character, or an empty segment is refused here, before the entry's kind is
/// even consulted.
fn entry_name<R: Read>(entry: &tar::Entry<'_, R>) -> Result<String, PackageArchiveError> {
    let raw = entry.path_bytes();
    let unsafe_path = || PackageArchiveError::UnsafeEntryPath {
        path: String::from_utf8_lossy(&raw).into_owned(),
    };
    let name = std::str::from_utf8(&raw).map_err(|_| unsafe_path())?;
    // A tar directory entry conventionally carries a trailing separator, which
    // the shared path rule refuses as an empty final segment.
    let name = name.strip_suffix('/').unwrap_or(name);
    if !is_safe_relative_path(name) {
        return Err(unsafe_path());
    }
    Ok(name.to_string())
}

/// Hold a partially extracted package against the bounds the package model
/// imposes, so the walk stops at the bound rather than after it.
fn require_package_bounds(file_count: usize, byte_size: usize) -> Result<(), PackageArchiveError> {
    if file_count > MAX_PROFILE_PACKAGE_FILES || byte_size > MAX_PROFILE_PACKAGE_BYTES {
        return Err(PackageArchiveError::InvalidPackage(
            ProfilePackageError::PackageBounds {
                file_count,
                byte_size,
                max_files: MAX_PROFILE_PACKAGE_FILES,
                max_bytes: MAX_PROFILE_PACKAGE_BYTES,
            },
        ));
    }
    Ok(())
}

/// Decode the archive metadata through the same strict discipline a manifest is
/// decoded with: unknown fields are refused rather than ignored.
fn decode_archive_metadata(bytes: &[u8]) -> Result<ArchiveMetadata, PackageArchiveError> {
    let text = std::str::from_utf8(bytes).map_err(PackageArchiveError::MetadataUtf8)?;
    toml::from_str::<ArchiveMetadataWire>(text)
        .map(|wire| wire.archive)
        .map_err(PackageArchiveError::MetadataToml)
}

/// Hold what the archive states about its package against what the package
/// itself declares and hashes to.
fn require_metadata_agreement(
    metadata: &ArchiveMetadata,
    tree: &CapturedPackageTree,
) -> Result<(), PackageArchiveError> {
    let disagreement = |field, declared: String, packaged: String| {
        (declared != packaged).then_some(PackageArchiveError::MetadataDisagreement {
            field,
            declared,
            packaged,
        })
    };
    let mismatch = disagreement("id", metadata.id.to_string(), tree.model().id.to_string())
        .or_else(|| {
            disagreement(
                "version",
                metadata.version.clone(),
                tree.model().version.clone(),
            )
        });
    if let Some(error) = mismatch {
        return Err(error);
    }
    if metadata.package_hash != tree.hashes().package {
        return Err(PackageArchiveError::IdentityMismatch {
            declared: metadata.package_hash.clone(),
            computed: tree.hashes().package.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One archive entry as a test states it, so a test can hand over exactly
    /// the archive an adversary would.
    #[derive(Debug, Clone)]
    struct ArchiveEntrySpec {
        kind: EntryType,
        mode: u32,
        name: String,
        bytes: Vec<u8>,
    }

    /// The checked-in fixture package: a nested source, a declared-executable
    /// asset, and a managed region, so one package exercises files, directories,
    /// and both modes.
    fn fixture_package() -> ProfilePackage {
        ProfilePackage::from_directory(&crate::test_utils::profile_package_fixture(
            "synthetic-valid",
        ))
        .expect("the synthetic fixture is a valid package")
    }

    /// Read one archive back into the entries it holds.
    fn entry_specs(archive: &[u8]) -> Vec<ArchiveEntrySpec> {
        tar::Archive::new(archive)
            .entries()
            .expect("an archive lists its entries")
            .map(|entry| {
                let mut entry = entry.expect("each entry reads");
                let kind = entry.header().entry_type();
                let mode = entry.header().mode().expect("each entry carries a mode");
                let name = String::from_utf8(entry.path_bytes().into_owned())
                    .expect("the archive names entries in UTF-8");
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut bytes).expect("entry content reads");
                ArchiveEntrySpec {
                    kind,
                    mode,
                    name,
                    bytes,
                }
            })
            .collect()
    }

    /// Build one archive from stated entries under the same header discipline
    /// the production writer uses, so a rebuilt archive differs from a packed
    /// one only where the test says it does.
    ///
    /// The name is written into the header field rather than through the tar
    /// writer's path API, which refuses the very names this suite has to be
    /// able to hand over: an absolute path and a traversal are exactly what an
    /// adversary writes, and a defence never handed a hostile name is a defence
    /// never tested. Every name here fits the header field, so no long-name
    /// extension is involved.
    fn archive_of(specs: &[ArchiveEntrySpec]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for spec in specs {
            let mut header = reproducible_tar_header(spec.kind, spec.mode, spec.bytes.len() as u64);
            let name = spec.name.as_bytes();
            let field = &mut header
                .as_gnu_mut()
                .expect("the reproducible header is a GNU header")
                .name;
            assert!(
                name.len() < field.len(),
                "a stated entry name fits the header field"
            );
            field[..name.len()].copy_from_slice(name);
            header.set_cksum();
            builder
                .append(&header, spec.bytes.as_slice())
                .expect("a stated entry appends");
        }
        builder.finish().expect("the stated archive finishes");
        builder.into_inner().expect("the stated archive closes")
    }

    /// The refusal one perturbation of the fixture archive produces.
    ///
    /// The same rebuild without the perturbation is read first and must be
    /// accepted, so a refusal observed here was caused by what the test changed
    /// rather than by the rebuild itself.
    fn refusal_after(perturb: impl FnOnce(&mut Vec<ArchiveEntrySpec>)) -> PackageArchiveError {
        let packed = pack_package_archive(&fixture_package()).expect("the fixture package packs");
        let mut specs = entry_specs(&packed);
        read_package_archive(&archive_of(&specs))
            .expect("the unperturbed rebuild is a valid archive");
        perturb(&mut specs);
        read_package_archive(&archive_of(&specs)).expect_err("the perturbed archive is refused")
    }

    /// One entry of the packed fixture, by the name the archive gives it.
    fn spec_named<'a>(specs: &'a mut [ArchiveEntrySpec], name: &str) -> &'a mut ArchiveEntrySpec {
        specs
            .iter_mut()
            .find(|spec| spec.name == name)
            .unwrap_or_else(|| panic!("the packed fixture carries '{name}'"))
    }

    /// An archive is a function of the package it carries, not of the directory
    /// it was read from or the run that wrote it.
    #[test]
    fn test_pack_package_archive_writes_identical_bytes_for_two_copies_of_one_package() {
        let temp = tempfile::tempdir().expect("temporary directory");
        let fixture = crate::test_utils::profile_package_fixture("synthetic-valid");
        let first = crate::test_utils::copy_package_tree(&fixture, &temp.path().join("first"));
        let second = crate::test_utils::copy_package_tree(&fixture, &temp.path().join("second"));

        let first = pack_package_archive(
            &ProfilePackage::from_directory(&first).expect("the first copy is a package"),
        )
        .expect("the first copy packs");
        let second = pack_package_archive(
            &ProfilePackage::from_directory(&second).expect("the second copy is a package"),
        )
        .expect("the second copy packs");

        assert!(!first.is_empty(), "an archive of a package has content");
        assert_eq!(
            first, second,
            "two copies of one package packed to different bytes"
        );
    }

    /// What the archive states about its package is what a read recomputes from
    /// the content, down to the bytes and the mode of every file.
    #[test]
    fn test_read_package_archive_recovers_the_packed_package_content_and_identity() {
        let package = fixture_package();
        let packed = pack_package_archive(&package).expect("the fixture package packs");

        let tree = read_package_archive(&packed).expect("a packed archive reads back");

        assert_eq!(tree.model().id, package.model().id);
        assert_eq!(tree.model().version, package.model().version);
        assert_eq!(
            tree.hashes().package,
            package.hashes().package,
            "the recomputed identity differs from the packed package's own"
        );
        assert_eq!(
            tree.files()
                .iter()
                .map(|(source, file)| (source.clone(), file.bytes.clone()))
                .collect::<BTreeMap<_, _>>(),
            package.files().clone(),
            "the archive did not carry the package's own bytes"
        );
        // The fixture declares one executable asset and ships its bytes without
        // the permission, so the published mode can only have come from the
        // manifest.
        assert_eq!(
            tree.files()
                .iter()
                .filter(|(_, file)| file.mode == FileMode::Executable)
                .map(|(source, _)| source.as_str())
                .collect::<Vec<_>>(),
            package
                .model()
                .assets
                .iter()
                .filter(|asset| asset.executable)
                .map(|asset| asset.source.as_str())
                .collect::<Vec<_>>()
        );
    }

    /// Content that does not reproduce the digest the archive carries is
    /// refused, which is what detects an archive damaged in transit.
    #[test]
    fn test_read_package_archive_refuses_content_that_does_not_reproduce_the_carried_digest() {
        let refusal = refusal_after(|specs| {
            spec_named(specs, "package/assets/workflow.txt")
                .bytes
                .extend_from_slice(b"corruption");
        });

        assert!(
            matches!(refusal, PackageArchiveError::IdentityMismatch { .. }),
            "altered content was not held against the carried digest: {refusal}"
        );
    }

    /// An entry naming a path outside the package directory is refused before
    /// any of it is admitted, whichever escape it attempts.
    #[test]
    fn test_read_package_archive_refuses_an_entry_naming_a_path_outside_the_package_directory() {
        for name in [
            "/etc/passwd",
            "package/../../escape",
            "../escape",
            "package/./here",
            "..",
        ] {
            let refusal = refusal_after(|specs| {
                specs.push(ArchiveEntrySpec {
                    kind: EntryType::Regular,
                    mode: REGULAR_FILE_MODE,
                    name: name.to_string(),
                    bytes: b"payload".to_vec(),
                });
            });
            assert!(
                matches!(refusal, PackageArchiveError::UnsafeEntryPath { .. }),
                "'{name}' was not refused as an unsafe entry path: {refusal}"
            );
        }
    }

    /// A safe relative name the archive layout does not place anything at is
    /// refused rather than extracted somewhere plausible.
    #[test]
    fn test_read_package_archive_refuses_an_entry_the_archive_layout_does_not_place() {
        for name in ["elsewhere/file", "packages/file", "loose-file"] {
            let refusal = refusal_after(|specs| {
                specs.push(ArchiveEntrySpec {
                    kind: EntryType::Regular,
                    mode: REGULAR_FILE_MODE,
                    name: name.to_string(),
                    bytes: b"payload".to_vec(),
                });
            });
            assert!(
                matches!(refusal, PackageArchiveError::UnexpectedEntry { .. }),
                "'{name}' was not refused as an entry outside the archive layout: {refusal}"
            );
        }
    }

    /// Only regular files and directories are admitted; every other tar entry
    /// kind is refused outright rather than interpreted.
    #[test]
    fn test_read_package_archive_refuses_an_entry_that_is_not_a_regular_file_or_directory() {
        for kind in [
            EntryType::Symlink,
            EntryType::Link,
            EntryType::Char,
            EntryType::Block,
            EntryType::Fifo,
        ] {
            let refusal = refusal_after(|specs| {
                specs.push(ArchiveEntrySpec {
                    kind,
                    mode: REGULAR_FILE_MODE,
                    name: "package/assets/link".to_string(),
                    bytes: Vec::new(),
                });
            });
            assert!(
                matches!(refusal, PackageArchiveError::UnsupportedEntry { .. }),
                "a {kind:?} entry was not refused outright: {refusal}"
            );
        }
    }

    /// A mode is compared against what the manifest declares rather than
    /// adopted, in both directions: an entry claiming a permission no
    /// declaration carries, and one dropping the permission a declaration does.
    #[test]
    fn test_read_package_archive_refuses_a_mode_the_packaged_manifest_does_not_declare() {
        for (name, mode) in [
            ("package/assets/workflow.txt", EXECUTABLE_FILE_MODE),
            ("package/nested/scripts/check.sh", REGULAR_FILE_MODE),
            ("package/manifest.toml", 0o777),
            ("package/nested", 0o700),
        ] {
            let refusal = refusal_after(|specs| spec_named(specs, name).mode = mode);
            assert!(
                matches!(refusal, PackageArchiveError::UnexpectedEntryMode { .. }),
                "'{name}' at mode {mode:o} was not refused: {refusal}"
            );
        }
    }

    /// What the archive says about which package it carries must be what the
    /// packaged manifest declares.
    #[test]
    fn test_read_package_archive_refuses_metadata_that_names_another_package() {
        for (field, replacement) in [
            ("id = \"synthetic-workflow\"", "id = \"other-workflow\""),
            ("version = \"1.2.3\"", "version = \"9.9.9\""),
        ] {
            let refusal = refusal_after(|specs| {
                let metadata = spec_named(specs, ARCHIVE_METADATA_ENTRY);
                metadata.bytes = String::from_utf8(metadata.bytes.clone())
                    .expect("metadata is UTF-8")
                    .replace(field, replacement)
                    .into_bytes();
            });
            assert!(
                matches!(refusal, PackageArchiveError::MetadataDisagreement { .. }),
                "metadata replacing '{field}' was not refused: {refusal}"
            );
        }
    }

    /// An archive of a wire this build does not read is refused rather than
    /// read as the wire it happens to resemble.
    #[test]
    fn test_read_package_archive_refuses_an_archive_wire_this_build_does_not_read() {
        let refusal = refusal_after(|specs| {
            let metadata = spec_named(specs, ARCHIVE_METADATA_ENTRY);
            metadata.bytes = String::from_utf8(metadata.bytes.clone())
                .expect("metadata is UTF-8")
                .replace("archive-version = 1", "archive-version = 99")
                .into_bytes();
        });

        assert!(
            matches!(refusal, PackageArchiveError::ArchiveVersion { .. }),
            "an unreadable archive wire was not refused: {refusal}"
        );
    }

    /// An archive that states nothing about the package it carries is refused;
    /// there is no route that reads content without a statement to hold it
    /// against.
    #[test]
    fn test_read_package_archive_refuses_an_archive_carrying_no_metadata() {
        let refusal =
            refusal_after(|specs| specs.retain(|spec| spec.name != ARCHIVE_METADATA_ENTRY));

        assert!(
            matches!(refusal, PackageArchiveError::MissingEntry(_)),
            "an archive with no metadata was not refused: {refusal}"
        );
    }

    /// One name carried twice is refused rather than resolved to whichever
    /// entry the walk happened to see last.
    #[test]
    fn test_read_package_archive_refuses_one_name_carried_by_two_entries() {
        let refusal = refusal_after(|specs| {
            let repeated = spec_named(specs, "package/assets/workflow.txt").clone();
            specs.push(repeated);
        });

        assert!(
            matches!(refusal, PackageArchiveError::DuplicateEntry { .. }),
            "a repeated entry was not refused: {refusal}"
        );
    }

    /// An entry that would be refused for a different reason, appended after
    /// whatever a bounds test puts over budget.
    ///
    /// Which refusal comes back is what says *when* the bound was applied. A
    /// bound applied to running counts stops at the oversized content and
    /// reports the budget; a bound applied to the whole extraction afterwards
    /// would walk on and report this entry instead — which is the difference
    /// between refusing a size bomb and holding one first.
    fn poison_entry() -> ArchiveEntrySpec {
        ArchiveEntrySpec {
            kind: EntryType::Symlink,
            mode: REGULAR_FILE_MODE,
            name: "package/assets/poison".to_string(),
            bytes: Vec::new(),
        }
    }

    /// The package's own file bound stops the walk where it is exceeded.
    #[test]
    fn test_read_package_archive_refuses_more_files_than_a_package_may_hold() {
        let refusal = refusal_after(|specs| {
            specs.extend(
                (0..=MAX_PROFILE_PACKAGE_FILES).map(|index| ArchiveEntrySpec {
                    kind: EntryType::Regular,
                    mode: REGULAR_FILE_MODE,
                    name: format!("package/assets/filler-{index}"),
                    bytes: Vec::new(),
                }),
            );
            specs.push(poison_entry());
        });

        assert!(
            matches!(
                refusal,
                PackageArchiveError::InvalidPackage(ProfilePackageError::PackageBounds { .. })
            ),
            "the file bound did not stop the walk where it was exceeded: {refusal}"
        );
    }

    /// The package's own byte bound stops the walk before the bytes past it are
    /// held, so an oversized archive is not a memory-exhaustion primitive.
    #[test]
    fn test_read_package_archive_refuses_more_bytes_than_a_package_may_hold() {
        let refusal = refusal_after(|specs| {
            specs.push(ArchiveEntrySpec {
                kind: EntryType::Regular,
                mode: REGULAR_FILE_MODE,
                name: "package/assets/filler".to_string(),
                bytes: vec![0u8; MAX_PROFILE_PACKAGE_BYTES + 1],
            });
            specs.push(poison_entry());
        });

        assert!(
            matches!(
                refusal,
                PackageArchiveError::InvalidPackage(ProfilePackageError::PackageBounds { .. })
            ),
            "the byte bound did not stop the walk where it was exceeded: {refusal}"
        );
    }

    /// A file larger than a package archive may be is refused before it is
    /// parsed at all, so an enormous file handed over as an archive is not a
    /// memory-exhaustion primitive either.
    #[test]
    fn test_read_package_archive_refuses_a_file_larger_than_a_package_archive_may_be() {
        let oversized = vec![0u8; MAX_PROFILE_PACKAGE_ARCHIVE_BYTES + 1];

        let refusal = read_package_archive(&oversized).expect_err("an oversized file is refused");

        assert!(
            matches!(refusal, PackageArchiveError::ArchiveBytes { .. }),
            "an oversized file was not refused by size: {refusal}"
        );
    }

    /// The metadata is four scalars; an entry claiming to be far more than that
    /// is refused rather than held.
    #[test]
    fn test_read_package_archive_refuses_metadata_larger_than_four_scalars_can_be() {
        let refusal = refusal_after(|specs| {
            spec_named(specs, ARCHIVE_METADATA_ENTRY).bytes =
                vec![b' '; MAX_ARCHIVE_METADATA_BYTES + 1];
        });

        assert!(
            matches!(refusal, PackageArchiveError::MetadataBytes { .. }),
            "oversized metadata was not refused by size: {refusal}"
        );
    }

    /// Metadata that is not the recognized wire is refused rather than read
    /// past to whatever fields it happens to carry.
    #[test]
    fn test_read_package_archive_refuses_metadata_that_is_not_the_recognized_wire() {
        for (scenario, metadata) in [
            ("not TOML at all", b"{ not toml }".to_vec()),
            (
                "a field the wire does not declare",
                b"[archive]\narchive-version = 1\nid = \"synthetic-workflow\"\n\
                  version = \"1.2.3\"\npackage-hash = \"00\"\nextra = true\n"
                    .to_vec(),
            ),
            ("bytes that are not UTF-8", vec![0xff, 0xfe, 0xfd]),
        ] {
            let refusal =
                refusal_after(|specs| spec_named(specs, ARCHIVE_METADATA_ENTRY).bytes = metadata);
            assert!(
                matches!(
                    refusal,
                    PackageArchiveError::MetadataToml(_) | PackageArchiveError::MetadataUtf8(_)
                ),
                "metadata carrying {scenario} was not refused: {refusal}"
            );
        }
    }

    /// Content the packaged manifest does not declare is refused by the same
    /// package validation every other route runs, so an archive cannot smuggle
    /// a file into a package that does not declare it.
    #[test]
    fn test_read_package_archive_refuses_content_the_packaged_manifest_does_not_declare() {
        let refusal = refusal_after(|specs| {
            specs.push(ArchiveEntrySpec {
                kind: EntryType::Regular,
                mode: REGULAR_FILE_MODE,
                name: "package/assets/undeclared.txt".to_string(),
                bytes: b"undeclared".to_vec(),
            });
        });

        assert!(
            matches!(
                refusal,
                PackageArchiveError::InvalidPackage(ProfilePackageError::ExtraContent(_))
            ),
            "undeclared archived content was not refused: {refusal}"
        );
    }
}
