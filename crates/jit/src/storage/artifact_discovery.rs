//! Thin no-follow evidence acquisition for pure artifact discovery.

use crate::domain::artifact_classifier::ArtifactClassificationInventory;
use crate::domain::artifact_classifier::EmbeddedArtifactOwner;
use crate::domain::artifact_discovery::{
    discover_archive_artifacts as derive_archive_artifacts, expand_artifact_closure,
    ArtifactClosure, ArtifactClosureState, ArtifactDiscoveryError as DomainDiscoveryError,
    ArtifactEvidence, ArtifactEvidenceMap, ParsedArtifact,
};
use crate::domain::artifact_inventory::ExplicitRootInventory;
use crate::domain::artifact_plan::ArtifactVersion;
use crate::domain::Issue;
use crate::storage::artifact_planning::{
    read_working_tree_path_without_symlinks, WorkingTreeDiscoveryRead,
};
use crate::storage::{IssueStore, PathReadError};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Boundary failures while acquiring archive discovery evidence.
#[derive(Debug, Error)]
pub enum ArtifactDiscoveryError {
    #[error("failed to read artifact {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: PathReadError,
    },
    #[error(transparent)]
    Domain(#[from] DomainDiscoveryError),
}

/// Acquire one shared closure and derive selected artifacts plus embedded owners.
pub fn discover_archive_artifacts<S: IssueStore>(
    storage: &S,
    inventory: ExplicitRootInventory,
    issues: &[Issue],
) -> Result<(ArtifactClassificationInventory, Vec<EmbeddedArtifactOwner>), ArtifactDiscoveryError> {
    let selected = inventory
        .artifacts()
        .iter()
        .filter(|entry| entry.version() == &ArtifactVersion::WorkingTree)
        .map(|entry| entry.source().to_string());
    let owner_roots = issues
        .iter()
        .flat_map(|issue| issue.documents.iter())
        .filter(|document| document.commit.is_none())
        .map(|document| crate::domain::artifact_plan::normalize_artifact_path(&document.path));
    let roots = selected.chain(owner_roots).collect::<BTreeSet<_>>();
    let (evidence, parsed) = collect_closure_evidence(storage, roots)?;
    derive_archive_artifacts(inventory, issues, &evidence, &parsed).map_err(Into::into)
}

fn collect_closure_evidence<S: IssueStore>(
    storage: &S,
    roots: BTreeSet<String>,
) -> Result<(ArtifactEvidenceMap, BTreeMap<String, ParsedArtifact>), ArtifactDiscoveryError> {
    let mut evidence = ArtifactEvidenceMap::new();
    let mut state = ArtifactClosureState::new(roots);
    loop {
        let (paths, next) = match expand_artifact_closure(state, &evidence) {
            ArtifactClosure::Complete(parsed) => return Ok((evidence, parsed)),
            ArtifactClosure::Needs { paths, state } => (paths, state),
        };
        for path in paths {
            let fact = match read_working_tree_path_without_symlinks(storage, &path) {
                Ok(WorkingTreeDiscoveryRead::Bytes(bytes)) => ArtifactEvidence::File(bytes),
                Ok(WorkingTreeDiscoveryRead::Symlink) => ArtifactEvidence::Symlink,
                Ok(WorkingTreeDiscoveryRead::Unsupported) => ArtifactEvidence::Unsupported,
                Err(PathReadError::NotFound(_)) => ArtifactEvidence::Missing,
                Err(PathReadError::InvalidPath(_) | PathReadError::OutsideRepoRoot(_)) => {
                    ArtifactEvidence::InvalidPath
                }
                Err(source) => return Err(ArtifactDiscoveryError::Read { path, source }),
            };
            evidence.insert(path, fact);
        }
        state = next;
    }
}
