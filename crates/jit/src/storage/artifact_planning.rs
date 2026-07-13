//! Read-only filesystem facts for archive planning.

use crate::domain::artifact_classifier::{
    artifact_destination_root, artifact_mirror_destination, ArtifactClassificationFacts,
    ArtifactClassificationPolicy, ArtifactLocation, ArtifactLocationFacts,
    ContainerDestinationState,
};
use crate::domain::artifact_plan::{
    ArtifactPlanEntry, ArtifactVersion, ContentIdentity, PlanTarget,
};
use crate::storage::{validate_repo_relative_path, IssueStore, PathReadError};
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Inspect every source and possible mirror destination without following symlinks.
pub fn collect_artifact_classification_facts<S: IssueStore>(
    storage: &S,
    target: &PlanTarget,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    embedded_owners: Vec<crate::domain::artifact_classifier::EmbeddedArtifactOwner>,
) -> Result<ArtifactClassificationFacts> {
    let destination_root = artifact_destination_root(target, &policy.archive_root);
    let inspect_destinations = !policy.archive_root.is_empty();
    let locations = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .map(|artifact| {
            let source = inspect_location(storage, artifact.source())?;
            let destination = if inspect_destinations {
                inspect_location(
                    storage,
                    &artifact_mirror_destination(&destination_root, artifact.source()),
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
            inspect_container_destination(storage, &destination_root, id, artifacts)?
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
    } else {
        storage
            .read_path_bytes(path, None)
            .map(|(bytes, _)| WorkingTreeDiscoveryRead::Bytes(bytes))
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
