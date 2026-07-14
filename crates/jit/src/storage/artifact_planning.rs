//! Read-only filesystem facts for archive planning.

use crate::domain::artifact_classifier::{
    artifact_mirror_destination, ArtifactClassificationFacts, ArtifactClassificationPolicy,
    ArtifactLocation, ArtifactLocationFacts, ContainerDestinationState,
};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArtifactPlanEntry, ArtifactVersion, ContentIdentity, PlanTarget,
};
use crate::storage::{validate_repo_relative_path, IssueStore, PathReadError};
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Storage-resolved destination for a container archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContainerDestination {
    /// The frozen marker-backed, legacy, or newly preferred destination root.
    pub destination_root: String,
    /// Every marker-backed root when duplicate ownership makes execution unsafe.
    pub conflicting_roots: Vec<String>,
}

/// Resolve a preferred container destination against durable marker ownership.
///
/// Only immediate, non-symlink children of the archive root are inspected.
/// A unique matching marker freezes that directory. If no marker matches, an
/// existing legacy short-id path is adopted before the preferred slugged path.
///
/// # Errors
///
/// Returns an error for invalid repository-relative paths or when filesystem
/// metadata, marker contents, or archive-directory entries cannot be read.
pub fn resolve_container_destination<S: IssueStore>(
    storage: &S,
    preferred_root: &str,
    legacy_root: &str,
    container_id: &str,
) -> Result<ResolvedContainerDestination> {
    validate_repo_relative_path(preferred_root)?;
    validate_repo_relative_path(legacy_root)?;
    let repo_root = repository_root(storage)?;
    let archive_root = Path::new(legacy_root)
        .parent()
        .ok_or_else(|| anyhow!("container archive destination has no archive root"))?;
    let archive_root_text = archive_root.to_string_lossy().replace('\\', "/");
    validate_repo_relative_path(&archive_root_text)?;

    let archive_directory = repo_root.join(archive_root);
    let can_scan = !path_has_symlink(&repo_root, &archive_root_text)?
        && fs::symlink_metadata(&archive_directory)
            .map(|metadata| metadata.is_dir())
            .or_else(|error| {
                (error.kind() == std::io::ErrorKind::NotFound)
                    .then_some(false)
                    .ok_or(error)
            })?;
    let mut matching_roots = if can_scan {
        fs::read_dir(&archive_directory)?
            .map(|entry| -> Result<Option<String>> {
                let entry = entry?;
                let metadata = fs::symlink_metadata(entry.path())?;
                if !metadata.is_dir() || metadata.file_type().is_symlink() {
                    return Ok(None);
                }
                let marker = entry.path().join(".jit-container");
                let marker_metadata = match fs::symlink_metadata(&marker) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        return Ok(None);
                    }
                    Err(error) => return Err(error.into()),
                };
                if !marker_metadata.is_file() || marker_metadata.file_type().is_symlink() {
                    return Ok(None);
                }
                let owner = fs::read(marker)?;
                let owner = trim_ascii_whitespace(&owner);
                if owner != container_id.as_bytes() {
                    return Ok(None);
                }
                let file_name = entry.file_name();
                let file_name = file_name
                    .to_str()
                    .ok_or_else(|| anyhow!("archive directory name is not valid UTF-8"))?;
                Ok(Some(normalize_artifact_path(&format!(
                    "{archive_root_text}/{file_name}"
                ))))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    matching_roots.sort();

    if let Some(destination_root) = matching_roots.first().cloned() {
        let conflicting_roots = if matching_roots.len() > 1 {
            matching_roots
        } else {
            Vec::new()
        };
        return Ok(ResolvedContainerDestination {
            destination_root,
            conflicting_roots,
        });
    }

    let legacy_exists = match fs::symlink_metadata(repo_root.join(legacy_root)) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    Ok(ResolvedContainerDestination {
        destination_root: if legacy_exists {
            legacy_root.to_string()
        } else {
            preferred_root.to_string()
        },
        conflicting_roots: Vec::new(),
    })
}

fn trim_ascii_whitespace(mut bytes: &[u8]) -> &[u8] {
    while bytes.first().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[1..];
    }
    while bytes.last().is_some_and(u8::is_ascii_whitespace) {
        bytes = &bytes[..bytes.len() - 1];
    }
    bytes
}

/// Inspect every source and possible mirror destination without following symlinks.
pub fn collect_artifact_classification_facts<S: IssueStore>(
    storage: &S,
    target: &PlanTarget,
    destination_root: &str,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    embedded_owners: Vec<crate::domain::artifact_classifier::EmbeddedArtifactOwner>,
) -> Result<ArtifactClassificationFacts> {
    let inspect_destinations = !policy.archive_root.is_empty();
    let locations = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .map(|artifact| {
            let source = inspect_location(storage, artifact.source())?;
            let destination = if inspect_destinations {
                inspect_location(
                    storage,
                    &artifact_mirror_destination(destination_root, artifact.source()),
                )?
            } else {
                ArtifactLocation::Missing
            };
            Ok((
                artifact.source().to_string(),
                ArtifactLocationFacts {
                    source,
                    destination,
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;

    let container_destination = match target {
        PlanTarget::Container { id } if inspect_destinations => {
            inspect_container_destination(storage, destination_root, id, artifacts)?
        }
        _ => ContainerDestinationState::Absent,
    };

    Ok(ArtifactClassificationFacts {
        locations,
        embedded_owners,
        container_destination,
    })
}

fn inspect_location<S: IssueStore>(storage: &S, path: &str) -> Result<ArtifactLocation> {
    if working_tree_path_has_symlink(storage, path)? {
        return Ok(ArtifactLocation::Symlink);
    }
    if working_tree_path_is_unsupported(storage, path)? {
        return Ok(ArtifactLocation::Unsupported);
    }
    match storage.read_path_bytes(path, None) {
        Ok((bytes, _)) => Ok(ArtifactLocation::Regular(ContentIdentity::from_bytes(
            &bytes,
        ))),
        Err(PathReadError::NotFound(_)) => Ok(ArtifactLocation::Missing),
        Err(error) => Err(error.into()),
    }
}

/// Outcome of a working-tree discovery read that refuses symbolic-link paths.
pub(crate) enum WorkingTreeDiscoveryRead {
    /// Bytes read from a path whose existing components were all non-symlinks.
    Bytes(Vec<u8>),
    /// The leaf or an intermediate component is a symbolic link.
    Symlink,
    /// An existing filesystem object is neither a regular file nor a symlink.
    Unsupported,
}

/// Read one repository-relative working-tree path only when no component is a
/// symbolic link.
///
/// This is deliberately narrower than [`IssueStore::read_path_bytes`]: archive
/// discovery must preserve symlink artifacts for classification without ever
/// parsing their referents, while snapshot and other callers retain the
/// existing general read contract.
pub(crate) fn read_working_tree_path_without_symlinks<S: IssueStore>(
    storage: &S,
    path: &str,
) -> Result<WorkingTreeDiscoveryRead, PathReadError> {
    if working_tree_path_has_symlink(storage, path)? {
        Ok(WorkingTreeDiscoveryRead::Symlink)
    } else if working_tree_path_is_unsupported(storage, path)? {
        Ok(WorkingTreeDiscoveryRead::Unsupported)
    } else {
        storage
            .read_path_bytes(path, None)
            .map(|(bytes, _)| WorkingTreeDiscoveryRead::Bytes(bytes))
    }
}

/// Report whether the leaf is an existing non-regular, non-symlink object.
///
/// Missing paths remain ordinary storage reads so discovery can preserve its
/// existing missing-root and missing-edge diagnostics. Metadata failures other
/// than absence stay fatal instead of being flattened into plan diagnostics.
fn working_tree_path_is_unsupported<S: IssueStore>(
    storage: &S,
    path: &str,
) -> Result<bool, PathReadError> {
    validate_repo_relative_path(path)?;
    let repo_root = repository_root(storage).map_err(PathReadError::Other)?;
    match fs::symlink_metadata(repo_root.join(path)) {
        Ok(metadata) => Ok(!metadata.is_file() && !metadata.file_type().is_symlink()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(PathReadError::from(error)),
    }
}

/// Report whether any existing component of a repository-relative working-tree
/// path is a symbolic link.
///
/// Recursive discovery uses this probe before reading parseable artifacts, so
/// neither a symlink root nor a symlink reached through an embedded edge can
/// contribute referent-derived edges to an archive plan. Classification uses
/// the same boundary to record the artifact itself as `symlink-artifact`.
pub(crate) fn working_tree_path_has_symlink<S: IssueStore>(
    storage: &S,
    path: &str,
) -> Result<bool, PathReadError> {
    validate_repo_relative_path(path)?;
    let repo_root = repository_root(storage).map_err(PathReadError::Other)?;
    path_has_symlink(&repo_root, path).map_err(PathReadError::from)
}

fn inspect_container_destination<S: IssueStore>(
    storage: &S,
    destination_root: &str,
    container_id: &str,
    artifacts: &[ArtifactPlanEntry],
) -> Result<ContainerDestinationState> {
    let repo_root = repository_root(storage)?;
    let destination = repo_root.join(destination_root);
    if path_has_symlink(&repo_root, destination_root)? {
        return Ok(ContainerDestinationState::Symlink);
    }
    let metadata = match fs::symlink_metadata(&destination) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ContainerDestinationState::Absent);
        }
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_dir() {
        return Ok(ContainerDestinationState::MarkerlessWithUnaccountedEntries);
    }

    let marker = destination.join(".jit-container");
    match fs::symlink_metadata(&marker) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Ok(ContainerDestinationState::Symlink);
        }
        Ok(metadata) if metadata.is_file() => {
            let owner = fs::read_to_string(marker)?.trim().to_string();
            return Ok(if owner == container_id {
                ContainerDestinationState::OwnedByTarget
            } else {
                ContainerDestinationState::OwnedByOther(owner)
            });
        }
        Ok(_) => return Ok(ContainerDestinationState::MarkerlessWithUnaccountedEntries),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let accounted = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .map(|artifact| artifact_mirror_destination(destination_root, artifact.source()))
        .collect::<BTreeSet<_>>();
    let entries = collect_files(&destination, &repo_root)?;
    Ok(if entries.iter().all(|entry| accounted.contains(entry)) {
        ContainerDestinationState::MarkerlessAccounted
    } else {
        ContainerDestinationState::MarkerlessWithUnaccountedEntries
    })
}

fn path_has_symlink(repo_root: &Path, relative: &str) -> std::io::Result<bool> {
    let mut candidate = repo_root.to_path_buf();
    for component in Path::new(relative).components() {
        candidate.push(component.as_os_str());
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

fn collect_files(directory: &Path, repo_root: &Path) -> Result<Vec<String>> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = Vec::new();
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.file_type().is_symlink() {
                files.push(
                    entry
                        .path()
                        .strip_prefix(repo_root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            } else if metadata.is_dir() {
                pending.push(entry.path());
            } else {
                files.push(
                    entry
                        .path()
                        .strip_prefix(repo_root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    files.sort();
    Ok(files)
}

fn repository_root<S: IssueStore>(storage: &S) -> Result<PathBuf> {
    storage
        .root()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("invalid storage path: {}", storage.root().display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::JsonFileStorage;
    use tempfile::TempDir;

    const CONTAINER: &str = "abcdef12-3456-7890-abcd-ef1234567890";

    fn storage() -> (TempDir, JsonFileStorage) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        (repo, storage)
    }

    #[test]
    fn test_resolve_container_destination_reuses_marker_after_preferred_slug_changes() {
        let (repo, storage) = storage();
        let original = repo.path().join("archive/abcdef12-original-slug");
        fs::create_dir_all(&original).unwrap();
        fs::write(original.join(".jit-container"), format!("{CONTAINER}\n")).unwrap();

        let resolved = resolve_container_destination(
            &storage,
            "archive/abcdef12-renamed-slug",
            "archive/abcdef12",
            CONTAINER,
        )
        .unwrap();

        assert_eq!(resolved.destination_root, "archive/abcdef12-original-slug");
        assert!(resolved.conflicting_roots.is_empty());
    }

    #[test]
    fn test_resolve_container_destination_adopts_existing_legacy_short_id_path() {
        let (repo, storage) = storage();
        fs::create_dir_all(repo.path().join("archive/abcdef12")).unwrap();

        let resolved = resolve_container_destination(
            &storage,
            "archive/abcdef12-readable",
            "archive/abcdef12",
            CONTAINER,
        )
        .unwrap();

        assert_eq!(resolved.destination_root, "archive/abcdef12");
        assert!(resolved.conflicting_roots.is_empty());
    }

    #[test]
    fn test_resolve_container_destination_reports_every_duplicate_marker_root() {
        let (repo, storage) = storage();
        for slug in ["first", "second"] {
            let root = repo.path().join(format!("archive/abcdef12-{slug}"));
            fs::create_dir_all(&root).unwrap();
            fs::write(root.join(".jit-container"), format!("{CONTAINER}\n")).unwrap();
        }

        let resolved = resolve_container_destination(
            &storage,
            "archive/abcdef12-new",
            "archive/abcdef12",
            CONTAINER,
        )
        .unwrap();

        assert_eq!(resolved.destination_root, "archive/abcdef12-first");
        assert_eq!(
            resolved.conflicting_roots,
            [
                "archive/abcdef12-first".to_string(),
                "archive/abcdef12-second".to_string(),
            ]
        );
    }
}
