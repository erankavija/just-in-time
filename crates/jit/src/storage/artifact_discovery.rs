//! Storage-owned working-tree reads for recursive artifact discovery.
//!
//! The actual parsing and graph/path decisions live in the pure
//! [`crate::domain::artifact_discovery`] module. This adapter performs the
//! minimum I/O loop needed to feed that domain core and never reads pinned
//! historical entries.

use crate::domain::artifact_discovery::{
    parse_artifact, resolve_reference, DiscoveryGraph, ReferenceResolution,
};
use crate::domain::artifact_inventory::ExplicitRootInventory;
use crate::domain::artifact_plan::{
    ArtifactAction, ArtifactPlanEntry, ArtifactProvenance, ArtifactVersion, BlockerCode,
    PlanBlocker, PlanError, PlanTarget, PlanWarning, WarningCode,
};
use crate::storage::{IssueStore, PathReadError};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

/// Explicit roots expanded with their recursively discovered working-tree closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredArtifactInventory {
    target: PlanTarget,
    member_ids: Vec<String>,
    artifacts: Vec<ArtifactPlanEntry>,
    blockers: Vec<PlanBlocker>,
}

impl DiscoveredArtifactInventory {
    /// Normalized target inherited from explicit-root inventory.
    pub fn target(&self) -> &PlanTarget {
        &self.target
    }

    /// Resolved-hierarchy members inherited from explicit-root inventory.
    pub fn member_ids(&self) -> &[String] {
        &self.member_ids
    }

    /// Canonically ordered explicit and embedded artifact versions.
    pub fn artifacts(&self) -> &[ArtifactPlanEntry] {
        &self.artifacts
    }

    /// Target-level blockers, including failed pinned reads from inventory.
    pub fn blockers(&self) -> &[PlanBlocker] {
        &self.blockers
    }

    /// Consume the discovered inventory into archive-plan constructor fields.
    pub fn into_plan_parts(self) -> (PlanTarget, Vec<ArtifactPlanEntry>, Vec<PlanBlocker>) {
        (self.target, self.artifacts, self.blockers)
    }
}

/// Failures that cannot be represented as a deterministic plan diagnostic.
#[derive(Debug, Error)]
pub enum ArtifactDiscoveryError {
    /// A working-tree read failed for a reason other than absence or containment.
    #[error("failed to read artifact {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: PathReadError,
    },
    /// Discovery produced an invalid artifact-plan entry.
    #[error(transparent)]
    InvalidPlanEntry(#[from] PlanError),
    /// The pure graph and entry map diverged, indicating an implementation defect.
    #[error("artifact discovery graph has no entry for {0}")]
    MissingGraphEntry(String),
}

/// Recursively discover supported local dependencies of working-tree roots.
///
/// Pinned entries are copied directly to the result and never queued for a
/// read, so a readable pin imposes no working-tree constraint. Failed pinned
/// reads already recorded by explicit inventory remain `pinned-read-failed`.
pub fn discover_artifact_dependencies<S: IssueStore>(
    storage: &S,
    inventory: ExplicitRootInventory,
) -> Result<DiscoveredArtifactInventory, ArtifactDiscoveryError> {
    let (target, member_ids, roots, blockers) = inventory.into_discovery_parts();
    let mut historical = roots
        .iter()
        .filter(|entry| entry.version().is_pinned())
        .cloned()
        .collect::<Vec<_>>();
    let mut working = roots
        .into_iter()
        .filter(|entry| !entry.version().is_pinned())
        .map(|entry| (entry.source().to_string(), entry))
        .collect::<BTreeMap<_, _>>();
    let explicit_paths = working.keys().cloned().collect::<BTreeSet<_>>();
    let mut graph = DiscoveryGraph::new(explicit_paths.iter().cloned());
    let mut referencing_paths = BTreeMap::<String, BTreeSet<String>>::new();
    let mut missing_paths = BTreeSet::<String>::new();

    while let Some(path) = graph.next_path() {
        let bytes = match storage.read_path_bytes(&path, None) {
            Ok((bytes, _)) => bytes,
            Err(PathReadError::NotFound(_)) => {
                let entry = working
                    .get_mut(&path)
                    .ok_or_else(|| ArtifactDiscoveryError::MissingGraphEntry(path.clone()))?;
                if explicit_paths.contains(&path) {
                    append_blocker(
                        entry,
                        PlanBlocker::new(BlockerCode::MissingSource, Some(&path)),
                    )?;
                } else {
                    missing_paths.insert(path.clone());
                    let parents = referencing_paths.get(&path).cloned().unwrap_or_default();
                    for parent in parents {
                        let parent_entry = working.get_mut(&parent).ok_or_else(|| {
                            ArtifactDiscoveryError::MissingGraphEntry(parent.clone())
                        })?;
                        append_warning(
                            parent_entry,
                            PlanWarning::new(WarningCode::MissingEdgeTarget, Some(&path)),
                        )?;
                    }
                }
                continue;
            }
            Err(PathReadError::InvalidPath(_) | PathReadError::OutsideRepoRoot(_)) => {
                let entry = working
                    .get_mut(&path)
                    .ok_or_else(|| ArtifactDiscoveryError::MissingGraphEntry(path.clone()))?;
                append_blocker(
                    entry,
                    PlanBlocker::new(BlockerCode::RepositoryEscape, Some(&path)),
                )?;
                continue;
            }
            Err(source) => {
                return Err(ArtifactDiscoveryError::Read { path, source });
            }
        };

        let parsed = parse_artifact(&path, &bytes);
        let mut edges = Vec::new();
        let mut entry_blockers = Vec::new();
        let mut entry_warnings = Vec::new();
        if parsed.dynamic_loading_suspected() {
            entry_warnings.push(PlanWarning::new(
                WarningCode::DynamicLoadingSuspected,
                Some(&path),
            ));
        }

        for reference in parsed.references() {
            match resolve_reference(&path, reference) {
                ReferenceResolution::Ignored => {}
                ReferenceResolution::External(edge) => {
                    edges.push(edge);
                    entry_warnings.push(PlanWarning::new(WarningCode::ExternalEdge, Some(&path)));
                }
                ReferenceResolution::RepositoryEscape { edge, blocker } => {
                    edges.push(edge);
                    entry_blockers.push(blocker);
                }
                ReferenceResolution::Local { edge, target } => {
                    edges.push(edge);
                    referencing_paths
                        .entry(target.clone())
                        .or_default()
                        .insert(path.clone());
                    if missing_paths.contains(&target) {
                        entry_warnings.push(PlanWarning::new(
                            WarningCode::MissingEdgeTarget,
                            Some(&target),
                        ));
                    }
                    if !working.contains_key(&target) {
                        working.insert(
                            target.clone(),
                            ArtifactPlanEntry::new(
                                &target,
                                ArtifactVersion::WorkingTree,
                                ArtifactAction::Retain,
                            )
                            .with_provenance(vec![ArtifactProvenance::Embedded]),
                        );
                    }
                    graph.enqueue(target);
                }
            }
        }

        let entry = working
            .get_mut(&path)
            .ok_or_else(|| ArtifactDiscoveryError::MissingGraphEntry(path.clone()))?;
        let mut updated = entry
            .clone()
            .with_edges(merge(entry.edges(), edges))
            .with_blockers(merge(entry.blockers(), entry_blockers))
            .with_warnings(merge(entry.warnings(), entry_warnings));
        if let Some(format) = parsed.format() {
            updated = updated.with_format(format);
        }
        updated.normalize()?;
        *entry = updated;
    }

    historical.extend(working.into_values());
    historical.sort_by_key(ArtifactPlanEntry::identity);
    Ok(DiscoveredArtifactInventory {
        target,
        member_ids,
        artifacts: historical,
        blockers,
    })
}

fn append_blocker(entry: &mut ArtifactPlanEntry, blocker: PlanBlocker) -> Result<(), PlanError> {
    let mut updated = entry
        .clone()
        .with_blockers(merge(entry.blockers(), [blocker]));
    updated.normalize()?;
    *entry = updated;
    Ok(())
}

fn append_warning(entry: &mut ArtifactPlanEntry, warning: PlanWarning) -> Result<(), PlanError> {
    let mut updated = entry
        .clone()
        .with_warnings(merge(entry.warnings(), [warning]));
    updated.normalize()?;
    *entry = updated;
    Ok(())
}

fn merge<T: Clone>(existing: &[T], additional: impl IntoIterator<Item = T>) -> Vec<T> {
    existing.iter().cloned().chain(additional).collect()
}
