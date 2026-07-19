//! Pure closed repository-state capture and materialization vocabulary.
//!
//! This subsystem owns canonical paths, immutable images, bounded capture,
//! managed-document composition, exact deltas, and deterministic plan identity.
//! It performs no filesystem I/O and imports no validation, storage, command, or
//! profile modules.

mod image;
mod managed_document;
mod mutation;
mod path;

pub use image::{
    plan_hash, CaptureBudget, CaptureError, CaptureSpec, DeltaError, EntryIdentity,
    ExpectedPreimage, FileMode, LinkedWorktreeEvidence, LinkedWorktreeSourceClass,
    ListingFingerprint, MaterializationIntent, PinnedDocumentEvidence, PinnedSourceClass,
    PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositorySeed, RepositorySeedKind, SeedError, TargetClaim,
};
pub use managed_document::{
    compose_managed_documents, render_managed_document, ManagedDocumentClaim, ManagedDocumentError,
    RegionPlacement,
};
pub use mutation::{
    finalize, issue_draft, prefix_has_torn_tail, serialize_event, serialize_gate_run,
    serialize_issue, FixedMutationClock, IdAuthority, MutationClock, MutationContext,
    MutationError, MutationIntent, SystemMutationClock,
};
pub use path::{
    InjectivityProof, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryRootEvidence, RootRelativePath, VirtualPath,
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
    /// Exact bounded repository image from which this plan was derived.
    image: RepositoryImage,
    /// Exact normalized delta.
    delta: RepositoryDelta,
    /// Semantic hash covering the complete image and inputs.
    hash: String,
}

impl MaterializationPlan {
    /// The exact bounded repository image closed into this plan.
    pub fn image(&self) -> &RepositoryImage {
        &self.image
    }

    /// The exact normalized delta closed into this plan.
    pub fn delta(&self) -> &RepositoryDelta {
        &self.delta
    }

    /// The semantic identity computed from the captured image and complete plan inputs.
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Close a delta into a plan whose identity is computed from all plan inputs.
    pub(crate) fn new(
        image: &RepositoryImage,
        seed: &RepositorySeed,
        intent: &MaterializationIntent,
        delta: RepositoryDelta,
    ) -> Result<Self, PlanHashError> {
        let hash = plan_hash(image, seed, intent, &delta)?;
        Ok(Self {
            image: image.clone(),
            delta,
            hash,
        })
    }
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
    MaterializationPlan::new(image, seed, &intent, delta).map_err(Into::into)
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
        .delta()
        .actions()
        .iter()
        .try_fold(Vec::new(), |mut drifts, action| {
            let path = match action {
                RepositoryAction::CreateDirectory { path, .. }
                | RepositoryAction::WriteFile { path, .. }
                | RepositoryAction::SetMode { path, .. }
                | RepositoryAction::DeleteFile { path, .. } => path,
            };
            let captured = image.entry(path)?;
            let drift = match (action, captured) {
                (RepositoryAction::CreateDirectory { .. }, RepositoryEntry::Directory { .. }) => {
                    None
                }
                (RepositoryAction::CreateDirectory { .. }, _) => {
                    Some(MaterializationDriftKind::Missing)
                }
                (
                    RepositoryAction::WriteFile { bytes, mode, .. },
                    RepositoryEntry::File {
                        bytes: actual,
                        mode: actual_mode,
                        ..
                    },
                ) if actual == bytes && actual_mode == mode => None,
                (RepositoryAction::WriteFile { .. }, _) => Some(MaterializationDriftKind::Stale),
                (
                    RepositoryAction::SetMode { mode, .. },
                    RepositoryEntry::File {
                        mode: actual_mode, ..
                    },
                ) if actual_mode == mode => None,
                (RepositoryAction::SetMode { .. }, _) => Some(MaterializationDriftKind::Stale),
                (RepositoryAction::DeleteFile { .. }, RepositoryEntry::Absent) => None,
                (RepositoryAction::DeleteFile { .. }, _) => {
                    Some(MaterializationDriftKind::Unexpected)
                }
            };
            if let Some(kind) = drift {
                drifts.push(MaterializationDrift {
                    path: path.clone(),
                    kind,
                });
            }
            Ok(drifts)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn budget() -> CaptureBudget {
        CaptureBudget {
            max_paths: 8,
            max_listings: 0,
            max_bytes: 128,
            max_depth: 4,
        }
    }

    fn plan(image: &RepositoryImage, actions: Vec<RepositoryAction>) -> MaterializationPlan {
        let seed = RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "repository-state-test".into(),
            },
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        MaterializationPlan::new(
            image,
            &seed,
            &MaterializationIntent::SemanticMutation,
            RepositoryDelta::new(image.layout(), actions).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn test_compare_materializations_propagates_undiscovered_for_every_action() {
        let layout = layout();
        let image = RepositoryImage::close(
            layout.clone(),
            CaptureSpec::phase_one(Vec::<VirtualPath>::new(), budget()).unwrap(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let target = VirtualPath::data("unrequested.json").unwrap();
        let occupant = EntryIdentity::for_bytes("occupant", b"captured").unwrap();
        let actions = [
            RepositoryAction::CreateDirectory {
                path: target.clone(),
                owner: "directory".into(),
                expected: ExpectedPreimage::Absent,
            },
            RepositoryAction::WriteFile {
                path: target.clone(),
                owner: "writer".into(),
                expected: ExpectedPreimage::Absent,
                bytes: b"expected".to_vec(),
                mode: FileMode::Regular,
            },
            RepositoryAction::SetMode {
                path: target.clone(),
                owner: "mode".into(),
                expected: ExpectedPreimage::File {
                    identity: occupant.clone(),
                    mode: FileMode::Regular,
                },
                mode: FileMode::Executable,
            },
            RepositoryAction::DeleteFile {
                path: target.clone(),
                owner: "delete".into(),
                expected: ExpectedPreimage::File {
                    identity: occupant,
                    mode: FileMode::Regular,
                },
            },
        ];

        for action in actions {
            assert_eq!(
                compare_materializations(&image, &plan(&image, vec![action])),
                Err(CaptureError::UndiscoveredRepositoryPath(target.clone()))
            );
        }
    }

    #[test]
    fn test_compare_materializations_classifies_captured_mismatches() {
        let layout = layout();
        let create = VirtualPath::data("create").unwrap();
        let delete = VirtualPath::data("delete").unwrap();
        let set_mode = VirtualPath::data("set-mode").unwrap();
        let write = VirtualPath::data("write").unwrap();
        let delete_bytes = b"delete me".to_vec();
        let mode_bytes = b"mode".to_vec();
        let write_bytes = b"old".to_vec();
        let delete_identity = EntryIdentity::for_bytes("delete", &delete_bytes).unwrap();
        let mode_identity = EntryIdentity::for_bytes("mode", &mode_bytes).unwrap();
        let image = RepositoryImage::close(
            layout.clone(),
            CaptureSpec::phase_one(
                [
                    create.clone(),
                    delete.clone(),
                    set_mode.clone(),
                    write.clone(),
                ],
                budget(),
            )
            .unwrap(),
            BTreeMap::from([
                (create.clone(), RepositoryEntry::Absent),
                (
                    delete.clone(),
                    RepositoryEntry::File {
                        identity: delete_identity.clone(),
                        bytes: delete_bytes,
                        mode: FileMode::Regular,
                    },
                ),
                (
                    set_mode.clone(),
                    RepositoryEntry::File {
                        identity: mode_identity.clone(),
                        bytes: mode_bytes,
                        mode: FileMode::Regular,
                    },
                ),
                (
                    write.clone(),
                    RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes("write", &write_bytes).unwrap(),
                        bytes: write_bytes,
                        mode: FileMode::Regular,
                    },
                ),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        let expected = plan(
            &image,
            vec![
                RepositoryAction::CreateDirectory {
                    path: create.clone(),
                    owner: "directory".into(),
                    expected: ExpectedPreimage::Absent,
                },
                RepositoryAction::DeleteFile {
                    path: delete.clone(),
                    owner: "delete".into(),
                    expected: ExpectedPreimage::File {
                        identity: delete_identity,
                        mode: FileMode::Regular,
                    },
                },
                RepositoryAction::SetMode {
                    path: set_mode.clone(),
                    owner: "mode".into(),
                    expected: ExpectedPreimage::File {
                        identity: mode_identity,
                        mode: FileMode::Regular,
                    },
                    mode: FileMode::Executable,
                },
                RepositoryAction::WriteFile {
                    path: write.clone(),
                    owner: "writer".into(),
                    expected: ExpectedPreimage::Absent,
                    bytes: b"new".to_vec(),
                    mode: FileMode::Regular,
                },
            ],
        );

        assert_eq!(
            compare_materializations(&image, &expected).unwrap(),
            vec![
                MaterializationDrift {
                    path: create,
                    kind: MaterializationDriftKind::Missing,
                },
                MaterializationDrift {
                    path: delete,
                    kind: MaterializationDriftKind::Unexpected,
                },
                MaterializationDrift {
                    path: set_mode,
                    kind: MaterializationDriftKind::Stale,
                },
                MaterializationDrift {
                    path: write,
                    kind: MaterializationDriftKind::Stale,
                },
            ]
        );
    }
}
