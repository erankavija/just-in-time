//! Pure closed repository-state capture and materialization vocabulary.
//!
//! This subsystem owns canonical paths, immutable images, bounded capture,
//! managed-document composition, exact deltas, and deterministic plan identity.
//! It performs no filesystem I/O and imports no validation, storage, command, or
//! profile modules.

mod image;
mod managed_document;
mod path;

pub use image::{
    plan_hash, CaptureBudget, CaptureError, CaptureSpec, DeltaError, EntryIdentity,
    ExpectedPreimage, FileMode, LinkedWorktreeEvidence, LinkedWorktreeSourceClass,
    ListingFingerprint, MaterializationIntent, PinnedDocumentEvidence, PinnedSourceClass,
    PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositorySeed, RepositorySeedKind, TargetClaim,
};
pub use managed_document::{
    compose_managed_documents, render_managed_document, ManagedDocumentClaim, ManagedDocumentError,
    RegionPlacement,
};
pub use path::{
    InjectivityProof, RepositoryLayout, RepositoryLayoutError, RepositoryRootEvidence,
    RootRelativePath, VirtualPath,
};

use crate::declarations::rules::RuleSet;
use crate::declarations::{ConfigurationDeclarations, GateRegistry};

/// Explicit declaration bundle consumed by the closed producer call graph.
pub struct RepositoryDeclarations<'a> {
    /// Parsed repository configuration.
    pub configuration: &'a ConfigurationDeclarations,
    /// Authored gate registry.
    pub gates: &'a GateRegistry,
    /// Authored rule registry.
    pub rules: &'a RuleSet,
}

/// Complete deterministic pure materialization result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationPlan {
    /// Exact normalized delta.
    pub delta: RepositoryDelta,
    /// Semantic hash covering the complete image and inputs.
    pub hash: String,
}

/// Invoke the constrained closed producer graph for one intent.
///
/// The foundation currently has no declaration-derived file producer until the
/// materializer package supplies those functions. The direct intent match is
/// deliberately closed: callers cannot register callbacks or choose individual
/// producer families. Adding a family requires extending this function.
pub fn derive_materializations(
    image: &RepositoryImage,
    _declarations: RepositoryDeclarations<'_>,
    seed: &RepositorySeed,
    intent: MaterializationIntent,
) -> Result<MaterializationPlan, RepositoryStateError> {
    let delta = match intent {
        MaterializationIntent::SemanticMutation => derive_semantic_mutation(image)?,
        MaterializationIntent::RenderConfiguredProjections => derive_project_render(image)?,
        MaterializationIntent::RepairDerivedState => derive_repair(image)?,
    };
    let hash = plan_hash(image, seed, &intent, &delta)?;
    Ok(MaterializationPlan { delta, hash })
}

/// Pure derivation failure.
#[derive(Debug, thiserror::Error)]
pub enum RepositoryStateError {
    /// Delta normalization rejected an alias or duplicate target.
    #[error(transparent)]
    Delta(#[from] DeltaError),
    /// Plan identity serialization failed.
    #[error(transparent)]
    PlanHash(#[from] PlanHashError),
}

fn derive_semantic_mutation(image: &RepositoryImage) -> Result<RepositoryDelta, DeltaError> {
    RepositoryDelta::new(image.layout(), Vec::new())
}

fn derive_project_render(image: &RepositoryImage) -> Result<RepositoryDelta, DeltaError> {
    RepositoryDelta::new(image.layout(), Vec::new())
}

fn derive_repair(image: &RepositoryImage) -> Result<RepositoryDelta, DeltaError> {
    RepositoryDelta::new(image.layout(), Vec::new())
}

/// Kind of mismatch between a captured image and expected materialization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MaterializationDriftKind {
    /// Expected target is absent.
    Missing,
    /// Present target bytes or mode differ.
    Stale,
    /// An explicitly owned deletion target remains present.
    Unexpected,
}

/// One deterministic derived-state mismatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaterializationDrift {
    /// Canonical target.
    pub path: VirtualPath,
    /// Mismatch classification.
    pub kind: MaterializationDriftKind,
}

/// Compare expected exact actions against the same closed image.
pub fn compare_materializations(
    image: &RepositoryImage,
    expected: &MaterializationPlan,
) -> Result<Vec<MaterializationDrift>, CaptureError> {
    expected
        .delta
        .actions()
        .iter()
        .filter_map(|action| {
            let (path, drift) = match action {
                RepositoryAction::CreateDirectory { path, .. } => {
                    let stale = !matches!(image.entry(path), Ok(RepositoryEntry::Directory { .. }));
                    (path, stale.then_some(MaterializationDriftKind::Missing))
                }
                RepositoryAction::WriteFile {
                    path, bytes, mode, ..
                } => {
                    let stale = !matches!(
                        image.entry(path),
                        Ok(RepositoryEntry::File { bytes: actual, mode: actual_mode, .. })
                            if actual == bytes && actual_mode == mode
                    );
                    (path, stale.then_some(MaterializationDriftKind::Stale))
                }
                RepositoryAction::SetMode { path, mode, .. } => {
                    let stale = !matches!(
                        image.entry(path),
                        Ok(RepositoryEntry::File { mode: actual, .. }) if actual == mode
                    );
                    (path, stale.then_some(MaterializationDriftKind::Stale))
                }
                RepositoryAction::DeleteFile { path, .. } => {
                    let unexpected = !matches!(image.entry(path), Ok(RepositoryEntry::Absent));
                    (
                        path,
                        unexpected.then_some(MaterializationDriftKind::Unexpected),
                    )
                }
            };
            drift.map(|kind| {
                Ok(MaterializationDrift {
                    path: path.clone(),
                    kind,
                })
            })
        })
        .collect()
}
