//! No-follow filesystem evidence acquisition for pure archive planning.

use crate::domain::artifact_classifier::{
    artifact_mirror_destination, classification_facts_from_evidence,
    resolve_container_destination as derive_container_destination, ArtifactClassificationFacts,
    ArtifactClassificationPolicy, EmbeddedArtifactOwner, ResolvedContainerDestination,
};
use crate::domain::artifact_discovery::{
    ArtifactEvidence, ArtifactEvidenceMap, ArtifactListingScope,
};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArtifactPlanEntry, ArtifactVersion, PlanTarget,
};
use crate::repository_state::RootRelativePath;
use crate::storage::file_transaction::open_regular_file_nofollow;
use crate::storage::repository_state_store::{open_absolute_dir_nofollow, open_child_dir_nofollow};
use crate::storage::{validate_repo_relative_path, IssueStore, PathReadError};
use anyhow::{anyhow, Result};
use cap_std::fs::Dir;
use std::io::{ErrorKind, Read};
use std::path::Path;

/// Acquire the exact marker, child, and legacy evidence needed to resolve a destination.
pub fn resolve_container_destination<S: IssueStore>(
    storage: &S,
    preferred_root: &str,
    legacy_root: &str,
    container_id: &str,
) -> Result<ResolvedContainerDestination> {
    validate_repo_relative_path(preferred_root)?;
    validate_repo_relative_path(legacy_root)?;
    let archive_root = Path::new(legacy_root)
        .parent()
        .ok_or_else(|| anyhow!("container archive destination has no archive root"))?;
    let archive_root = normalize_artifact_path(&archive_root.to_string_lossy());
    validate_repo_relative_path(&archive_root)?;

    let mut evidence = ArtifactEvidenceMap::new();
    let archive = inspect_artifact_evidence(
        storage,
        &archive_root,
        ArtifactListingScope::ImmediateChildren,
    )?;
    if let ArtifactEvidence::Directory { entries, .. } = &archive {
        for child in entries {
            let child_evidence =
                inspect_artifact_evidence(storage, child, ArtifactListingScope::MetadataOnly)?;
            if matches!(child_evidence, ArtifactEvidence::Directory { .. }) {
                let marker = format!("{child}/.jit-container");
                evidence.insert(
                    marker.clone(),
                    inspect_artifact_evidence(
                        storage,
                        &marker,
                        ArtifactListingScope::MetadataOnly,
                    )?,
                );
            }
            evidence.insert(child.clone(), child_evidence);
        }
    }
    evidence.insert(archive_root, archive);
    if !evidence.contains_key(legacy_root) {
        evidence.insert(
            legacy_root.to_string(),
            inspect_artifact_evidence(storage, legacy_root, ArtifactListingScope::MetadataOnly)?,
        );
    }
    Ok(derive_container_destination(
        preferred_root,
        legacy_root,
        container_id,
        &evidence,
    )?)
}

/// Acquire every source, mirror, marker, and destination listing used by classification.
pub fn collect_artifact_classification_facts<S: IssueStore>(
    storage: &S,
    target: &PlanTarget,
    destination_root: &str,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    embedded_owners: Vec<EmbeddedArtifactOwner>,
) -> Result<ArtifactClassificationFacts> {
    let inspect_destinations = !policy.archive_root.is_empty();
    let mut evidence = ArtifactEvidenceMap::new();
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
    {
        let source = artifact.source();
        evidence.insert(
            source.to_string(),
            inspect_artifact_evidence(storage, source, ArtifactListingScope::MetadataOnly)?,
        );
        if inspect_destinations {
            let destination = artifact_mirror_destination(destination_root, source);
            evidence.insert(
                destination.clone(),
                inspect_artifact_evidence(
                    storage,
                    &destination,
                    ArtifactListingScope::MetadataOnly,
                )?,
            );
        }
    }
    if matches!(target, PlanTarget::Container { .. }) && inspect_destinations {
        let destination = inspect_artifact_evidence(
            storage,
            destination_root,
            ArtifactListingScope::RecursiveFiles,
        )?;
        if matches!(destination, ArtifactEvidence::Directory { .. }) {
            let marker = format!("{destination_root}/.jit-container");
            evidence.insert(
                marker.clone(),
                inspect_artifact_evidence(storage, &marker, ArtifactListingScope::MetadataOnly)?,
            );
        }
        evidence.insert(destination_root.to_string(), destination);
    }
    classification_facts_from_evidence(
        target,
        destination_root,
        artifacts,
        policy,
        embedded_owners,
        &evidence,
    )
}

/// Inspect one repository-relative path without following any symlink component.
pub(crate) fn inspect_artifact_evidence<S: IssueStore>(
    storage: &S,
    path: &str,
    listing_scope: ArtifactListingScope,
) -> Result<ArtifactEvidence, PathReadError> {
    if matches!(
        validate_repo_relative_path(path),
        Err(PathReadError::InvalidPath(_) | PathReadError::OutsideRepoRoot(_))
    ) {
        return Ok(ArtifactEvidence::InvalidPath);
    }
    validate_repo_relative_path(path)?;
    let layout = storage.repository_layout().map_err(PathReadError::Other)?;
    let root = open_absolute_dir_nofollow(layout.worktree_root()).map_err(other)?;
    inspect_artifact_from_root(&root, path, listing_scope)
}

fn inspect_artifact_from_root(
    root: &Dir,
    path: &str,
    listing_scope: ArtifactListingScope,
) -> Result<ArtifactEvidence, PathReadError> {
    let relative = RootRelativePath::parse(path)
        .map_err(|error| PathReadError::InvalidPath(error.to_string()))?;
    let components = relative
        .as_path()
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) = components
        .split_last()
        .ok_or_else(|| PathReadError::InvalidPath(path.to_string()))?;
    let mut parent = root.try_clone().map_err(PathReadError::from)?;
    for component in parents {
        let metadata = match parent.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(ArtifactEvidence::Missing)
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            return Ok(ArtifactEvidence::Symlink);
        }
        parent = open_child_dir_nofollow(&parent, component).map_err(other)?;
    }
    let metadata = match parent.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ArtifactEvidence::Missing),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() {
        return Ok(ArtifactEvidence::Symlink);
    }
    if metadata.is_file() {
        let mut file =
            open_regular_file_nofollow(&parent, &leaf.to_string_lossy()).map_err(other)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(PathReadError::from)?;
        return Ok(ArtifactEvidence::File(bytes));
    }
    if !metadata.is_dir() {
        return Ok(ArtifactEvidence::Unsupported);
    }
    let directory = open_child_dir_nofollow(&parent, leaf).map_err(other)?;
    let entries = match listing_scope {
        ArtifactListingScope::MetadataOnly => Vec::new(),
        ArtifactListingScope::ImmediateChildren => list_immediate_children(&directory, path)?,
        ArtifactListingScope::RecursiveFiles => list_recursive_files(&directory, path)?,
    };
    Ok(ArtifactEvidence::Directory {
        scope: listing_scope,
        entries,
    })
}

fn list_immediate_children(directory: &Dir, relative: &str) -> Result<Vec<String>, PathReadError> {
    let mut entries = directory
        .entries()?
        .map(|entry| {
            entry
                .map_err(PathReadError::from)
                .and_then(|entry| child_path(relative, entry.file_name()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

fn list_recursive_files(directory: &Dir, relative: &str) -> Result<Vec<String>, PathReadError> {
    let mut pending = vec![(directory.try_clone()?, relative.to_string())];
    let mut files = Vec::new();
    while let Some((current, prefix)) = pending.pop() {
        for entry in current.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let path = child_path(&prefix, name.clone())?;
            let metadata = current.symlink_metadata(&name)?;
            if metadata.is_dir() && !metadata.is_symlink() {
                let child = open_child_dir_nofollow(&current, &name).map_err(other)?;
                pending.push((child, path));
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn child_path(relative: &str, name: std::ffi::OsString) -> Result<String, PathReadError> {
    name.to_str()
        .ok_or_else(|| anyhow!("artifact path is not valid UTF-8"))
        .map(|name| format!("{relative}/{name}"))
        .map_err(PathReadError::Other)
}

fn other(error: impl Into<anyhow::Error>) -> PathReadError {
    PathReadError::Other(error.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::storage::JsonFileStorage;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn test_inspect_artifact_evidence_distinguishes_complete_listing_scopes_without_following_symlinks(
    ) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        storage.configure_repository_layout(&layout);
        fs::create_dir_all(repo.path().join("archive/sub")).unwrap();
        fs::write(repo.path().join("archive/sub/nested.md"), "nested").unwrap();
        fs::create_dir(repo.path().join("outside")).unwrap();
        fs::write(repo.path().join("outside/hidden.md"), "hidden").unwrap();
        symlink(
            repo.path().join("outside"),
            repo.path().join("archive/link"),
        )
        .unwrap();

        let immediate =
            inspect_artifact_evidence(&storage, "archive", ArtifactListingScope::ImmediateChildren)
                .unwrap();
        assert_eq!(
            immediate,
            ArtifactEvidence::Directory {
                scope: ArtifactListingScope::ImmediateChildren,
                entries: vec!["archive/link".into(), "archive/sub".into()],
            }
        );
        assert_eq!(
            inspect_artifact_evidence(
                &storage,
                "archive",
                ArtifactListingScope::ImmediateChildren,
            )
            .unwrap(),
            immediate
        );
        assert_eq!(
            inspect_artifact_evidence(&storage, "archive", ArtifactListingScope::RecursiveFiles,)
                .unwrap(),
            ArtifactEvidence::Directory {
                scope: ArtifactListingScope::RecursiveFiles,
                entries: vec!["archive/link".into(), "archive/sub/nested.md".into()],
            }
        );
    }
}
