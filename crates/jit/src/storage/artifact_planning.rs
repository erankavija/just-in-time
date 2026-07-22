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
use crate::storage::{validate_repo_relative_path, IssueStore, PathReadError};
use anyhow::{anyhow, Result};
use std::fs;
use std::path::{Path, PathBuf};

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
    let repo_root = repository_root(storage).map_err(PathReadError::Other)?;
    if path_has_symlink(&repo_root, path)? {
        return Ok(ArtifactEvidence::Symlink);
    }
    let absolute = repo_root.join(path);
    let metadata = match fs::symlink_metadata(&absolute) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ArtifactEvidence::Missing)
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.is_file() {
        return storage
            .read_path_bytes(path, None)
            .map(|(bytes, _)| ArtifactEvidence::File(bytes))
            .or_else(|error| match error {
                PathReadError::NotFound(_) => Ok(ArtifactEvidence::Missing),
                error => Err(error),
            });
    }
    if !metadata.is_dir() {
        return Ok(ArtifactEvidence::Unsupported);
    }
    let entries = match listing_scope {
        ArtifactListingScope::MetadataOnly => Vec::new(),
        ArtifactListingScope::ImmediateChildren => list_immediate_children(&absolute, &repo_root)?,
        ArtifactListingScope::RecursiveFiles => list_recursive_files(&absolute, &repo_root)?,
    };
    Ok(ArtifactEvidence::Directory {
        scope: listing_scope,
        entries,
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

fn list_immediate_children(
    directory: &Path,
    repo_root: &Path,
) -> Result<Vec<String>, PathReadError> {
    let mut entries = fs::read_dir(directory)?
        .map(|entry| {
            entry
                .map_err(PathReadError::from)
                .and_then(|entry| relative_path(entry.path(), repo_root))
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

fn list_recursive_files(directory: &Path, repo_root: &Path) -> Result<Vec<String>, PathReadError> {
    let mut pending = vec![directory.to_path_buf()];
    let mut files = Vec::new();
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(current)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                pending.push(entry.path());
            } else {
                files.push(relative_path(entry.path(), repo_root)?);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn relative_path(path: PathBuf, repo_root: &Path) -> Result<String, PathReadError> {
    path.strip_prefix(repo_root)
        .map_err(anyhow::Error::from)?
        .to_str()
        .ok_or_else(|| anyhow!("artifact path is not valid UTF-8"))
        .map(|path| path.replace('\\', "/"))
        .map_err(PathReadError::Other)
}

fn repository_root<S: IssueStore>(storage: &S) -> Result<PathBuf> {
    storage
        .root()
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("invalid storage path: {}", storage.root().display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::storage::JsonFileStorage;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    #[test]
    fn test_inspect_artifact_evidence_distinguishes_complete_listing_scopes_without_following_symlinks(
    ) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();
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
