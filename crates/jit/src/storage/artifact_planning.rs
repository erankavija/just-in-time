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
    validate_repo_relative_path(path)?;
    let repo_root = repository_root(storage)?;
    let mut candidate = repo_root.clone();
    for component in Path::new(path).components() {
        candidate.push(component.as_os_str());
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Ok(ArtifactLocation::Symlink);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ArtifactLocation::Missing);
            }
            Err(error) => return Err(error.into()),
        }
    }
    match storage.read_path_bytes(path, None) {
        Ok((bytes, _)) => Ok(ArtifactLocation::Regular(ContentIdentity::from_bytes(
            &bytes,
        ))),
        Err(PathReadError::NotFound(_)) => Ok(ArtifactLocation::Missing),
        Err(error) => Err(error.into()),
    }
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

fn path_has_symlink(repo_root: &Path, relative: &str) -> Result<bool> {
    let mut candidate = repo_root.to_path_buf();
    for component in Path::new(relative).components() {
        candidate.push(component.as_os_str());
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
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
