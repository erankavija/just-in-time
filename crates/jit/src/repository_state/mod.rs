//! Pure closed repository-state capture and materialization vocabulary.
//!
//! This subsystem owns canonical paths, immutable images, bounded capture,
//! managed-document composition, exact deltas, and deterministic plan identity.
//! It performs no filesystem I/O and imports no validation, storage, command, or
//! profile modules.

mod archive;
mod default_rules;
mod export;
mod image;
mod index;
mod initialize;
mod managed_document;
mod materialize;
mod mutation;
mod overlay;
mod path;
mod profile_apply;
mod projection;
mod projection_render;
mod rule_serialize;
mod rules_document;
mod rules_gates_projection;

pub(crate) use archive::captured_archive_events;
pub use archive::finalize_archive_execution;
pub use default_rules::{
    default_rule_membership_diff, default_rule_membership_diff_from_identities, default_ruleset,
    hierarchy_config, reconcile_default_rules_with_config, type_hierarchy_known_schema,
    DefaultRuleMembershipDiff, TYPE_HIERARCHY_SCHEMA_FILE,
};
pub(crate) use export::{
    classify_repository_export, finalize_repository_export, ExternalExportPath,
    RepositoryExportDestination, RepositoryExportError, RepositoryExportIntent,
};
pub use image::{
    plan_hash, CaptureBudget, CaptureError, CaptureSpec, DeltaError, EntryIdentity,
    ExpectedPreimage, FileMode, LinkedWorktreeEvidence, LinkedWorktreeSourceClass,
    ListingFingerprint, MaterializationIntent, PinnedDocumentEvidence, PinnedSourceClass,
    PlanHashError, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositorySeed, RepositorySeedKind, SeedError, TargetClaim,
};
pub(crate) use index::{RepositoryIndex, RepositoryIndexError, SUPPORTED_INDEX_SCHEMA_VERSION};
pub use initialize::{
    finalize_initialization, finalize_profile_application, render_repo_config, GitattributesClaim,
    GitattributesStatus, InitializationError, InitializationScaffold, ProfileContribution,
    ProfileTargetContribution,
};
pub use managed_document::{
    compose_managed_documents, render_managed_document, ManagedDocumentClaim, ManagedDocumentError,
    RegionPlacement,
};
pub use materialize::{
    assemble_config, render_capture_closure, validate_capture_closure, ValidationCaptureClosure,
};
pub(crate) use mutation::captured_gate_run_result_paths;
pub use mutation::{
    finalize, finalize_audit_append, fresh_index_bytes, gate_run_result_relative_path, issue_draft,
    prefix_has_torn_tail, profile_applied_event, serialize_event, serialize_gate_run,
    serialize_issue, FixedMutationClock, IdAuthority, MutationClock, MutationContext,
    MutationError, MutationIntent, SystemMutationClock,
};

/// Finalize one typed gate-registry edit, its audit event, and every coupled
/// declaration-derived materialization into one recoverable plan.
///
/// `declarations.gates` must be the proposed registry represented by the edit
/// intent. The base image remains the expected preimage for every action; an
/// overlay is used only while deriving projections so their contents reflect the
/// proposed registry before anything is published.
pub fn finalize_gate_registry_edit(
    layout: &RepositoryLayout,
    base: &RepositoryImage,
    context: &MutationContext,
    intents: &[MutationIntent],
    declarations: RepositoryDeclarations<'_>,
) -> anyhow::Result<MaterializationPlan> {
    let registry = intents
        .iter()
        .find_map(|intent| match intent {
            MutationIntent::EditGateRegistry { registry } => Some(&**registry),
            _ => None,
        })
        .ok_or_else(|| anyhow::anyhow!("gate registry finalization requires one typed edit"))?;
    if intents
        .iter()
        .filter(|intent| matches!(intent, MutationIntent::EditGateRegistry { .. }))
        .count()
        != 1
    {
        anyhow::bail!("gate registry finalization requires exactly one typed edit");
    }
    if declarations.gates != registry {
        anyhow::bail!("gate registry edit and projection declarations disagree");
    }

    let record_plan = finalize(layout, base, context, intents)?;
    let gate_path = VirtualPath::data("gates.toml")?;
    let gate_bytes = crate::declarations::serialize_gate_registry(registry)?;
    let overlaid = apply_overlay(base, std::iter::once((gate_path, Some(gate_bytes))))?;
    let mut actions = record_plan.delta().actions().to_vec();
    actions.extend(compose_complete(&overlaid, &declarations)?);
    let delta = RepositoryDelta::new(layout, actions)?;
    let seed = context.repository_seed(intents)?;
    Ok(MaterializationPlan::new(
        base,
        &seed,
        &MaterializationIntent::SemanticMutation,
        delta,
    )?)
}
pub use overlay::{apply_overlay, OverlayError};
pub use path::{
    InjectivityProof, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryRootEvidence, RootRelativePath, VirtualPath,
};
pub use profile_apply::{derive_profile_materializations, ProfileClaims};
pub use projection::{
    compose_projection, render_id_anchor_rows, render_invariants_markdown, require_target,
    splice_region, ProjectionError,
};
pub use projection_render::{render_projection_body, ProjectionInputs};
pub use rule_serialize::{
    render_rule_block, rules_file_header, serialize_ruleset, type_hierarchy_schema_content,
    SchemaFile, SerializedRuleSet,
};
pub use rules_document::{parse_rule_identities, rewrite_header, splice_default_membership};
pub use rules_gates_projection::render_rules_and_gates_markdown;

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
    declarations: RepositoryDeclarations<'_>,
    seed: &RepositorySeed,
    intent: MaterializationIntent,
) -> Result<MaterializationPlan, RepositoryStateError> {
    let delta = match &intent {
        MaterializationIntent::SemanticMutation => derive_semantic_mutation(image, &declarations)?,
        MaterializationIntent::RenderConfiguredProjections { selected } => {
            derive_project_render(image, &declarations, selected.as_ref())?
        }
        MaterializationIntent::RepairDerivedState => derive_repair(image, &declarations)?,
        MaterializationIntent::InitializeRepository | MaterializationIntent::ApplyProfile => {
            // Init and profile application do not derive their bytes from
            // declarations already present in the image (init CREATES those
            // declarations); they are finalized by the dedicated
            // `finalize_initialization`/`finalize_profile_application` entries.
            return Err(RepositoryStateError::Producer(
                "initialize/apply-profile intents are finalized by their dedicated entries, \
                 not the declaration-derived producer graph"
                    .to_string(),
            ));
        }
        MaterializationIntent::RepositoryExport => {
            return Err(RepositoryStateError::Producer(
                "repository exports are finalized by finalize_repository_export".to_string(),
            ));
        }
    };
    MaterializationPlan::new(image, seed, &intent, delta).map_err(Into::into)
}

/// Finalize an authored `config.toml` edit plus the complete coupled producer set
/// as one plan — the single-declaration config-mutation session entry.
///
/// The command supplies the edited, preservation-safe `config.toml` bytes; this
/// parses them through the pure configuration parser first, so a malformed edit is
/// a typed planning error and never a published file. It then overlays the edited
/// bytes onto the captured `base` and derives the complete owned-materialization
/// set — default rules and their schemas, plus every configured projection — from
/// that edited configuration, exactly as [`derive_semantic_mutation`] does. The
/// authored `config.toml` write carries the captured base preimage, and every
/// derived action its own base preimage, so `session.apply` revalidates against
/// exactly what was captured; the whole set publishes under the existing
/// [`MaterializationIntent::SemanticMutation`].
///
/// This is a constrained, single-declaration entry invoking the closed complete
/// producer set — `config.toml` only, never a generic authored-bytes seam. Other
/// authored-declaration edits adopt this same pattern per caller.
pub fn finalize_config_edit(
    base: &RepositoryImage,
    edited_config_bytes: &[u8],
    declarations: RepositoryDeclarations<'_>,
    seed: &RepositorySeed,
) -> Result<MaterializationPlan, RepositoryStateError> {
    // The edit must parse before any planning: a malformed config is a typed
    // planning error, not a published file.
    crate::declarations::parse_configuration(edited_config_bytes).map_err(|error| {
        RepositoryStateError::Producer(format!("invalid config.toml edit: {error:#}"))
    })?;

    let config_path = VirtualPath::data("config.toml")?;
    let overlay = std::iter::once((config_path.clone(), Some(edited_config_bytes.to_vec())))
        .collect::<std::collections::BTreeMap<_, _>>();
    let overlaid = apply_overlay(base, overlay).map_err(|error| {
        RepositoryStateError::Producer(format!("config overlay failed: {error}"))
    })?;

    // The authored config write carries the base preimage; the complete producer
    // set derives the coupled schemas/rule-membership/projections from the edited
    // configuration over the overlaid image (their targets keep their base
    // preimages, since only `config.toml` is overlaid).
    let mut actions = vec![RepositoryAction::WriteFile {
        path: config_path.clone(),
        owner: "authored-config".to_string(),
        expected: ExpectedPreimage::of(
            base.entry(&config_path)
                .map_err(|error| RepositoryStateError::producer(error.into()))?,
        ),
        bytes: edited_config_bytes.to_vec(),
        mode: FileMode::Regular,
    }];
    actions.extend(compose_complete(&overlaid, &declarations)?);
    let delta = RepositoryDelta::new(base.layout(), actions)?;
    MaterializationPlan::new(base, seed, &MaterializationIntent::SemanticMutation, delta)
        .map_err(Into::into)
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
    /// Managed-document composition rejected an ambiguous or malformed claim.
    #[error(transparent)]
    ManagedDocument(#[from] ManagedDocumentError),
    /// Layout classification rejected a producer path.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    /// A producer read an uncaptured path or malformed captured bytes.
    #[error("materialization producer failed: {0}")]
    Producer(String),
    /// Ownership of a materialization boundary cannot be proven, so repair is
    /// refused before publication rather than risk rewriting or deleting authored
    /// content (ownership matrix: "never rewrites an authored boundary it cannot
    /// prove"). Carries a human description of the ambiguous boundary.
    #[error("ambiguous materialization ownership, not repairable: {0}")]
    AmbiguousOwnership(String),
}

impl RepositoryStateError {
    /// Wrap an opaque producer failure (an `anyhow` error from a relocated
    /// projection/serialization producer) into the typed derivation error.
    fn producer(error: anyhow::Error) -> Self {
        Self::Producer(format!("{error:#}"))
    }

    /// Wrap a projection-composition producer failure, preserving a typed
    /// [`ManagedDocumentError`] so a managed-region fault (an absent required
    /// region, a competing claim) keeps its identity for exit-code mapping instead
    /// of collapsing into an opaque string.
    fn projection_producer(error: anyhow::Error) -> Self {
        match error.downcast::<ManagedDocumentError>() {
            Ok(managed) => Self::ManagedDocument(managed),
            Err(error) => Self::producer(error),
        }
    }
}

/// The complete owned-materialization producer set: default rules and their
/// schemas, plus every configured projection, each derived from declared authority
/// (`@/inv/single-source-prose`). A caller cannot invoke a subset — the intent
/// selects the whole set. Shared projection targets compose through the one
/// managed-document primitive, never last-writer-wins; `rules.toml` splices only
/// the generated default-family spans, preserving authored content unconditionally.
fn compose_complete(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<Vec<RepositoryAction>, RepositoryStateError> {
    let config = materialize::assemble_config(image).map_err(RepositoryStateError::producer)?;
    let mut actions = materialize::compose_default_ruleset(image, &config)?;
    actions.extend(
        // A semantic mutation is complete over EVERY declared projection.
        materialize::compose_configured_projections(image, &config, declarations, None)
            .map_err(RepositoryStateError::producer)?,
    );
    Ok(actions)
}

/// Semantic-mutation intent: always invokes the complete producer set.
fn derive_semantic_mutation(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<RepositoryDelta, RepositoryStateError> {
    Ok(RepositoryDelta::new(
        image.layout(),
        compose_complete(image, declarations)?,
    )?)
}

/// Render-only intent: a constrained-complete operation over the configured
/// projections in scope. `selected` scopes WHICH declared projections participate
/// (declaration scope), never which producer families run: every in-scope
/// projection is composed completely from its own declared sources, and an
/// out-of-scope projection contributes no action so its target is left untouched.
fn derive_project_render(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
    selected: Option<&std::collections::BTreeSet<String>>,
) -> Result<RepositoryDelta, RepositoryStateError> {
    let config = materialize::assemble_config(image).map_err(RepositoryStateError::producer)?;
    let actions =
        materialize::compose_configured_projections(image, &config, declarations, selected)
            .map_err(RepositoryStateError::projection_producer)?;
    Ok(RepositoryDelta::new(image.layout(), actions)?)
}

/// Repair intent: the same complete expected state, whose per-target composition is
/// itself ownership-safe — `rules.toml` splices only generated default spans,
/// region projections splice only their managed region, and full-file projections
/// replace a target the declaration proves. Ambiguous ownership fails before
/// publication rather than rewriting an authored boundary.
fn derive_repair(
    image: &RepositoryImage,
    declarations: &RepositoryDeclarations<'_>,
) -> Result<RepositoryDelta, RepositoryStateError> {
    Ok(RepositoryDelta::new(
        image.layout(),
        compose_complete(image, declarations)?,
    )?)
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
