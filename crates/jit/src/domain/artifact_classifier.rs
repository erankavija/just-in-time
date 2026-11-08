//! Pure edge-aware classification for dependency-aware artifact archival.
//!
//! Callers inspect the repository and supply immutable location and ownership
//! facts. This module applies policy, per-reference selection, and the
//! destination/source constraint calculus without performing I/O.

use crate::config::DocumentationConfig;
use crate::domain::artifact_discovery::{
    ArtifactEvidence, ArtifactEvidenceMap, ArtifactListingScope,
};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArtifactAction, ArtifactEdge, ArtifactOwner, ArtifactPlan,
    ArtifactPlanEntry, ArtifactProvenance, ArtifactVersion, BlockerCode, ContentIdentity, EdgeKind,
    EdgeResolutionMode, EvidenceCode, PendingDeletion, PlanBlocker, PlanError, PlanTarget,
    PolicyStatus, ReferenceChange, WarningCode,
};
use crate::domain::identity::IssueId;
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::Issue;
use crate::domain::State;
use crate::labels::type_value_of;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Explicit documentation policy used by the classifier.
///
/// The development root is the outer boundary: a source it does not contain is
/// retained where it is, whichever areas claim it, so a linked source file,
/// script, or repository-root document keeps its single copy instead of
/// defeating the whole plan (`@/issue/8e071e18/decision/D-14`). The archive
/// root is read first, because a source already beneath it is a published
/// mirror rather than a working-tree artifact.
///
/// Inside the development root the configured areas classify one
/// repository-relative source: a permanent root retains its source with no
/// mirror, and a managed root may relocate it. A source there that matches no
/// configured area raises [`BlockerCode::UnmanagedSelectedRoot`], keeping a
/// mistyped area entry diagnosable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClassificationPolicy {
    /// Whether every mutation-authorizing policy field was explicitly set.
    pub status: PolicyStatus,
    /// Repository-relative root holding the development areas under archival.
    pub development_root: String,
    /// Repository-relative roots eligible for archival.
    pub managed_paths: Vec<String>,
    /// Repository-relative roots whose sources must remain available.
    pub permanent_paths: Vec<String>,
    /// Exact repository-relative areas that organize one owner directory per issue.
    pub issue_scoped_areas: Vec<String>,
    /// Repository-relative destination root.
    pub archive_root: String,
}

impl ArtifactClassificationPolicy {
    /// Build policy from authored configuration while retaining its explicitness status.
    pub fn from_documentation(documentation: Option<&DocumentationConfig>) -> Self {
        Self {
            status: PolicyStatus::from_documentation(documentation),
            development_root: documentation
                .map(DocumentationConfig::development_root)
                .unwrap_or_default(),
            managed_paths: documentation
                .map(DocumentationConfig::managed_paths)
                .unwrap_or_default(),
            permanent_paths: documentation
                .map(DocumentationConfig::permanent_paths)
                .unwrap_or_default(),
            issue_scoped_areas: documentation
                .map(DocumentationConfig::issue_scoped_areas)
                .unwrap_or_default(),
            archive_root: documentation
                .map(DocumentationConfig::archive_root)
                .unwrap_or_default(),
        }
        .normalized()
    }

    /// Construct an explicit configured policy, primarily for embedded callers and tests.
    pub fn configured(
        development_root: impl Into<String>,
        managed_paths: Vec<String>,
        permanent_paths: Vec<String>,
        archive_root: impl Into<String>,
    ) -> Self {
        Self {
            status: PolicyStatus::Configured,
            development_root: development_root.into(),
            managed_paths,
            permanent_paths,
            issue_scoped_areas: Vec::new(),
            archive_root: archive_root.into(),
        }
        .normalized()
    }

    /// Declare the exact areas that organize one owner directory per issue.
    pub fn with_issue_scoped_areas(mut self, areas: Vec<String>) -> Self {
        self.issue_scoped_areas = normalized_paths(areas);
        self
    }

    /// Whether the configured archive root already contains this source.
    fn is_archived(&self, path: &str) -> bool {
        contains_path(&self.archive_root, path)
    }

    /// Whether a configured managed root makes this source eligible to relocate.
    fn is_managed(&self, path: &str) -> bool {
        self.managed_paths
            .iter()
            .any(|root| contains_path(root, path))
    }

    /// Whether a configured permanent root claims this source.
    fn is_permanent(&self, path: &str) -> bool {
        self.permanent_paths
            .iter()
            .any(|root| contains_path(root, path))
    }

    /// Whether this source is left exactly where it is.
    ///
    /// A source outside the development root — a source file, a script, an
    /// agent asset, a repository-root document — is not the repository's
    /// development record, and a permanent-path source is a living document.
    /// An archive neither relocates nor duplicates either: the plan schedules
    /// no destination and admits no descendant of it, so a linked hub no longer
    /// draws the set it cites into the plan and no frozen second copy exists
    /// (`@/issue/8e071e18/decision/D-14`, `@/inv/single-source-prose`).
    ///
    /// The boundary admits no area carve-out: a managed root the development
    /// root does not contain is retained just the same. The archive root is the
    /// one path this leaves alone, because a source already beneath it is a
    /// published mirror rather than a working-tree artifact and carries its
    /// own already-archived treatment.
    pub fn retains_source_in_place(&self, path: &str) -> bool {
        !self.is_archived(path)
            && (self.is_outside_development_root(path) || self.is_permanent(path))
    }

    /// Whether the configured development root fails to contain this source.
    ///
    /// A policy with no authored development root leaves every source inside
    /// it, so an unconfigured or repository-wide root keeps reporting a
    /// selected root that matches no configured area as
    /// [`BlockerCode::UnmanagedSelectedRoot`] instead of silently retaining it.
    fn is_outside_development_root(&self, path: &str) -> bool {
        !self.development_root.is_empty() && !contains_path(&self.development_root, path)
    }

    fn normalized(mut self) -> Self {
        self.development_root = normalize_artifact_path(&self.development_root);
        self.managed_paths = normalized_paths(self.managed_paths);
        self.permanent_paths = normalized_paths(self.permanent_paths);
        self.issue_scoped_areas = normalized_paths(self.issue_scoped_areas);
        self.archive_root = normalize_artifact_path(&self.archive_root);
        self
    }
}

/// Filesystem state captured by the storage boundary for one path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactLocation {
    /// No filesystem object exists at the path.
    Missing,
    /// A regular file exists with this byte identity.
    Regular(ContentIdentity),
    /// The path or any traversed component is a symbolic link.
    Symlink,
    /// An existing filesystem object is neither a regular file nor a symlink.
    Unsupported,
}

/// Source and computed-mirror facts for one working-tree artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactLocationFacts {
    pub source: ArtifactLocation,
    pub destination: ArtifactLocation,
}

/// One supported embedded ownership relation found in the repository-wide closure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddedArtifactOwner {
    /// Artifact reached from the issue-linked document's supported closure.
    pub artifact: String,
    /// Direct issue-linked working-tree document from which the closure starts.
    pub root: String,
    /// Full durable owner issue id.
    pub issue: IssueId,
    /// Current lifecycle state of the owner.
    pub state: State,
    /// Pre-archive origin, so an `Archived` owner classifies by its effective
    /// terminal state (`jit:45a140ae`).
    pub archived_from: Option<State>,
    /// Whether the owner belongs to the selected resolved subtree.
    pub inside_subtree: bool,
}

impl EmbeddedArtifactOwner {
    /// Whether this owner is terminal for archival classification, accounting for
    /// terminality-preserving `Archived`
    /// ([`crate::domain::is_effectively_terminal`]).
    pub fn is_effectively_terminal(&self) -> bool {
        crate::domain::is_effectively_terminal(self.state, self.archived_from)
    }
}

/// Whether a container's short-id destination directory admits the archive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerDestinationState {
    /// The directory is absent, marked for this container, or holds only entries this plan accounts for.
    Available,
    /// Another container owns it, or it holds entries this plan does not account for.
    Conflict,
    /// The directory path or a traversed component is a symbolic link.
    Symlink,
}

/// All I/O-derived facts consumed by pure classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClassificationFacts {
    /// Facts keyed by normalized repository-relative source path.
    pub locations: BTreeMap<String, ArtifactLocationFacts>,
    /// Embedded owners from every issue-linked working-tree document, not just the target.
    pub embedded_owners: Vec<EmbeddedArtifactOwner>,
    /// Marker/occupancy state for a container destination directory.
    pub container_destination: ContainerDestinationState,
}

/// Marker-backed destination chosen from a closed evidence snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContainerDestination {
    /// The frozen marker-backed or preferred destination root.
    pub destination_root: String,
    /// Every marker-backed root when duplicate ownership makes execution unsafe.
    pub conflicting_roots: Vec<String>,
}

/// Stable failures while resolving a container destination from closed evidence.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum ArtifactEvidenceError {
    /// A path implied by complete evidence was not itself captured.
    #[error("archive evidence is incomplete for {path}")]
    IncompleteEvidence { path: String },
    /// The archive root was captured with a listing that cannot prove its children.
    #[error("archive evidence for {path} is not an immediate-child listing")]
    WrongListingScope { path: String },
    /// The container marker holds bytes that are not a UTF-8 container id.
    #[error("container marker is not valid UTF-8: {path}")]
    InvalidMarker { path: String },
}

/// File name of the marker that binds an archive destination to its container.
const CONTAINER_MARKER_FILE: &str = ".jit-container";

/// Path of the container marker under `root`.
pub fn container_marker_path(root: &str) -> String {
    format!("{root}/{CONTAINER_MARKER_FILE}")
}

/// Resolve marker ownership and preferred placement from closed evidence.
pub fn resolve_container_destination(
    archive_root: &str,
    preferred_root: &str,
    container_id: &IssueId,
    evidence: &ArtifactEvidenceMap,
) -> Result<ResolvedContainerDestination, ArtifactEvidenceError> {
    let archive_root = normalize_artifact_path(archive_root);
    let archive_evidence = required_resolution_evidence(evidence, &archive_root)?;
    let children = match archive_evidence {
        ArtifactEvidence::Directory { scope, entries } => {
            if *scope != ArtifactListingScope::ImmediateChildren {
                return Err(ArtifactEvidenceError::WrongListingScope { path: archive_root });
            }
            entries.as_slice()
        }
        ArtifactEvidence::Missing
        | ArtifactEvidence::Symlink
        | ArtifactEvidence::Unsupported
        | ArtifactEvidence::File(_)
        | ArtifactEvidence::InvalidPath => &[],
    };
    if !matches!(archive_evidence, ArtifactEvidence::Directory { .. }) {
        return Ok(ResolvedContainerDestination {
            destination_root: preferred_root.to_string(),
            conflicting_roots: Vec::new(),
        });
    }
    let mut matching_roots = Vec::new();
    for child in children {
        if matches!(
            required_resolution_evidence(evidence, child)?,
            ArtifactEvidence::Directory { .. }
        ) {
            let marker = container_marker_path(child);
            if matches!(
                required_resolution_evidence(evidence, &marker)?,
                ArtifactEvidence::File(owner) if owner.trim_ascii() == container_id.as_str().as_bytes()
            ) {
                matching_roots.push(child.clone());
            }
        }
    }
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

    Ok(ResolvedContainerDestination {
        destination_root: preferred_root.to_string(),
        conflicting_roots: Vec::new(),
    })
}

/// The value captured for `path`, or `IncompleteEvidence` naming it.
fn required_resolution_evidence<'a, V>(
    evidence: &'a BTreeMap<String, V>,
    path: &str,
) -> Result<&'a V, ArtifactEvidenceError> {
    evidence
        .get(path)
        .ok_or_else(|| ArtifactEvidenceError::IncompleteEvidence {
            path: path.to_string(),
        })
}

/// Derive source, mirror, and container occupancy facts from closed evidence.
pub fn classification_facts_from_evidence(
    target: &PlanTarget,
    destination_root: &str,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    embedded_owners: Vec<EmbeddedArtifactOwner>,
    evidence: &ArtifactEvidenceMap,
) -> Result<ArtifactClassificationFacts, ArtifactEvidenceError> {
    let inspect_destinations = !policy.archive_root.is_empty();
    let layout = ArchiveLayout::new(target, destination_root, policy, artifacts);
    let locations = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .map(|artifact| {
            let source = artifact.source();
            let destination = layout.destination(source);
            Ok((
                source.to_string(),
                ArtifactLocationFacts {
                    source: location_from_evidence(source, evidence)?,
                    destination: if inspect_destinations {
                        destination
                            .as_deref()
                            .map(|destination| location_from_evidence(destination, evidence))
                            .transpose()?
                            .unwrap_or(ArtifactLocation::Missing)
                    } else {
                        ArtifactLocation::Missing
                    },
                },
            ))
        })
        .collect::<Result<BTreeMap<_, _>, ArtifactEvidenceError>>()?;
    let container_destination = match target {
        PlanTarget::Container { id } if inspect_destinations => {
            container_destination_from_evidence(destination_root, policy, id, artifacts, evidence)?
        }
        _ => ContainerDestinationState::Available,
    };
    Ok(ArtifactClassificationFacts {
        locations,
        embedded_owners,
        container_destination,
    })
}

fn location_from_evidence(
    path: &str,
    evidence: &ArtifactEvidenceMap,
) -> Result<ArtifactLocation, ArtifactEvidenceError> {
    Ok(match required_resolution_evidence(evidence, path)? {
        ArtifactEvidence::File(bytes) => {
            ArtifactLocation::Regular(ContentIdentity::from_bytes(bytes))
        }
        ArtifactEvidence::Missing => ArtifactLocation::Missing,
        ArtifactEvidence::Symlink => ArtifactLocation::Symlink,
        ArtifactEvidence::Unsupported
        | ArtifactEvidence::Directory { .. }
        | ArtifactEvidence::InvalidPath => ArtifactLocation::Unsupported,
    })
}

fn container_destination_from_evidence(
    destination_root: &str,
    policy: &ArtifactClassificationPolicy,
    container_id: &IssueId,
    artifacts: &[ArtifactPlanEntry],
    evidence: &ArtifactEvidenceMap,
) -> Result<ContainerDestinationState, ArtifactEvidenceError> {
    let target = PlanTarget::Container {
        id: container_id.clone(),
    };
    let entries = match required_resolution_evidence(evidence, destination_root)? {
        ArtifactEvidence::Missing => return Ok(ContainerDestinationState::Available),
        ArtifactEvidence::Symlink => return Ok(ContainerDestinationState::Symlink),
        ArtifactEvidence::File(_)
        | ArtifactEvidence::Unsupported
        | ArtifactEvidence::InvalidPath => return Ok(ContainerDestinationState::Conflict),
        ArtifactEvidence::Directory { scope, entries } => {
            if *scope != ArtifactListingScope::RecursiveFiles {
                return Err(ArtifactEvidenceError::WrongListingScope {
                    path: destination_root.to_string(),
                });
            }
            entries
        }
    };
    let marker_path = container_marker_path(destination_root);
    match required_resolution_evidence(evidence, &marker_path)? {
        ArtifactEvidence::Symlink => return Ok(ContainerDestinationState::Symlink),
        ArtifactEvidence::File(owner) => {
            let owner = std::str::from_utf8(owner.trim_ascii()).map_err(|_| {
                ArtifactEvidenceError::InvalidMarker {
                    path: marker_path.clone(),
                }
            })?;
            return Ok(if owner == container_id.as_str() {
                ContainerDestinationState::Available
            } else {
                ContainerDestinationState::Conflict
            });
        }
        ArtifactEvidence::Unsupported
        | ArtifactEvidence::Directory { .. }
        | ArtifactEvidence::InvalidPath => return Ok(ContainerDestinationState::Conflict),
        ArtifactEvidence::Missing => {}
    }
    let layout = ArchiveLayout::new(&target, destination_root, policy, artifacts);
    let accounted = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .filter_map(|artifact| layout.destination(artifact.source()))
        .collect::<BTreeSet<_>>();
    Ok(if entries.iter().all(|entry| accounted.contains(entry)) {
        ContainerDestinationState::Available
    } else {
        ContainerDestinationState::Conflict
    })
}

/// Inventory data accepted from recursive discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClassificationInventory {
    pub target: PlanTarget,
    pub artifacts: Vec<ArtifactPlanEntry>,
    pub blockers: Vec<PlanBlocker>,
}

impl ArtifactClassificationInventory {
    /// Construct input from the parts returned by discovered inventory.
    pub fn new(
        target: PlanTarget,
        artifacts: Vec<ArtifactPlanEntry>,
        blockers: Vec<PlanBlocker>,
    ) -> Self {
        Self {
            target,
            artifacts,
            blockers,
        }
    }

    /// Canonically ordered discovered artifact entries.
    pub fn artifacts(&self) -> &[ArtifactPlanEntry] {
        &self.artifacts
    }
}

/// Apply repository ownership, reference selection, and the edge-aware action calculus.
///
/// `destination_root` is the root the storage boundary resolved for the target.
pub fn classify_artifacts(
    inventory: ArtifactClassificationInventory,
    destination_root: &str,
    policy: ArtifactClassificationPolicy,
    facts: ArtifactClassificationFacts,
) -> Result<ArtifactPlan, PlanError> {
    let mut plan_blockers = inventory.blockers;
    append_container_destination_blocker(
        &inventory.target,
        &facts.container_destination,
        destination_root,
        &mut plan_blockers,
    );

    let embedded_owners = embedded_owners_by_path(&facts.embedded_owners);
    let inventory_paths = inventory
        .artifacts
        .iter()
        .map(|entry| entry.source().to_string())
        .collect::<BTreeSet<_>>();
    let document_all_terminal = document_owners_all_terminal(
        &inventory.target,
        &inventory.artifacts,
        &facts.embedded_owners,
        &inventory_paths,
    );
    if matches!(inventory.target, PlanTarget::Document { .. }) && !document_all_terminal {
        plan_blockers.push(PlanBlocker::new(
            BlockerCode::DocumentNonTerminalOwner,
            target_document_path(&inventory.target),
        ));
    }

    let paths = inventory
        .artifacts
        .iter()
        .filter(|entry| entry.version() == &ArtifactVersion::WorkingTree)
        .map(|entry| entry.source().to_string())
        .collect::<BTreeSet<_>>();
    let unpreservable_parents = unpreservable_parents(&inventory.artifacts, &inventory_paths);
    let mut needs_destination = selected_destination_roots(
        &inventory.target,
        &inventory.artifacts,
        &policy,
        document_all_terminal,
        &mut plan_blockers,
    );
    propagate_relative_destinations(
        &inventory.artifacts,
        &policy,
        &facts.locations,
        &paths,
        &mut needs_destination,
    );
    let layout = ArchiveLayout::new(
        &inventory.target,
        destination_root,
        &policy,
        &inventory.artifacts,
    );
    destination_source_conflicts(&layout, &needs_destination)
        .into_values()
        .filter(|sources| sources.len() > 1)
        .flatten()
        .for_each(|source| {
            plan_blockers.push(PlanBlocker::new(
                BlockerCode::DestinationConflict,
                Some(source),
            ));
        });
    let mut source_constraints =
        root_relative_targets(&inventory.artifacts, &facts.locations, &paths);
    // A parent that stays at its source, copied or retained, keeps citing its
    // relative targets there, so they stay too. Each pass can only add
    // constraints, so the fixpoint is reached within one pass per artifact.
    let artifacts = loop {
        let classified = inventory
            .artifacts
            .iter()
            .cloned()
            .map(|entry| {
                classify_entry(
                    entry,
                    &inventory.target,
                    &policy,
                    &facts.locations,
                    &embedded_owners,
                    document_all_terminal,
                    &needs_destination,
                    &source_constraints,
                    &unpreservable_parents,
                    &layout,
                )
            })
            .collect::<Result<Vec<_>, PlanError>>()?;
        let kept = classified
            .iter()
            .filter(|entry| entry.action() != ArtifactAction::Move)
            .flat_map(|entry| entry.edges())
            .filter(|edge| {
                edge.kind == EdgeKind::Supported
                    && edge.resolution_mode == EdgeResolutionMode::Relative
            })
            .filter_map(|edge| present_target(edge, &facts.locations, &paths))
            .filter(|target| !source_constraints.contains(*target))
            .cloned()
            .collect::<BTreeSet<_>>();
        if kept.is_empty() {
            break classified;
        }
        source_constraints.extend(kept);
    };
    // Execution refuses a layout this check rejects, so preview reports it as
    // a blocker rather than as an eligible plan.
    if let Err(violation) = validate_proposed_artifact_layout(&artifacts) {
        plan_blockers.push(PlanBlocker::new(
            BlockerCode::UnpreservableLayout,
            Some(match violation {
                ProposedLayoutViolation::EdgeUnavailable { parent, .. } => parent,
                ProposedLayoutViolation::TargetAbsent { target } => target,
            }),
        ));
    }

    ArtifactPlan::new(
        inventory.target,
        destination_root,
        policy.status,
        artifacts,
        plan_blockers,
        Vec::new(),
    )
}

/// Group every publishing source by its canonical destination.
///
/// The ordered maps make any collision report deterministic. Classification
/// emits one blocker naming each conflicting source, while ordinary occupied
/// destination checks name the destination itself.
fn destination_source_conflicts(
    layout: &ArchiveLayout<'_>,
    sources: &BTreeSet<String>,
) -> BTreeMap<String, Vec<String>> {
    sources.iter().fold(BTreeMap::new(), |mut grouped, source| {
        if let Some(destination) = layout.destination(source) {
            grouped
                .entry(destination)
                .or_insert_with(Vec::new)
                .push(source.clone());
        }
        grouped
    })
}

/// A proposed layout that leaves a supported archive edge unresolvable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProposedLayoutViolation {
    /// A supported archive edge target is absent from the proposed plan.
    #[error("supported archive edge target is absent from plan: {target}")]
    TargetAbsent { target: String },
    /// A supported archive edge does not resolve after the proposed move.
    #[error(
        "supported edge {reference} from {parent} resolves to {resolved} in the proposed layout, not an available target location ({available})",
        available = .available.join(", ")
    )]
    EdgeUnavailable {
        reference: String,
        parent: String,
        resolved: String,
        available: Vec<String>,
    },
}

/// Verify that every supported edge still resolves to an available path after execution.
///
/// An edge into an artifact the plan retains in place imposes no constraint.
/// Such a target is a fixed point the archive may neither relocate nor
/// reproduce (`@/issue/8e071e18/decision/D-14`), so demanding that it stay
/// reachable from every proposed parent location would forbid relocating
/// anything that cites it, and the plan already records the retention as
/// [`EvidenceCode::OutsideDevelopmentRoot`] or [`EvidenceCode::PermanentPath`]. A relative citation of such a
/// target no longer resolves from the archived copy, which is the cost the
/// decision accepts against duplicating the repository into every archive.
pub fn validate_proposed_layout(plan: &ArtifactPlan) -> Result<(), ProposedLayoutViolation> {
    validate_proposed_artifact_layout(plan.artifacts())
}

/// [`validate_proposed_layout`] over a plan's classified artifacts.
fn validate_proposed_artifact_layout(
    artifacts: &[ArtifactPlanEntry],
) -> Result<(), ProposedLayoutViolation> {
    let by_source = artifacts
        .iter()
        .map(|artifact| (artifact.source(), artifact))
        .collect::<BTreeMap<_, _>>();
    for parent in artifacts {
        for edge in parent
            .edges()
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Supported)
        {
            let Some(target_source) = edge.target.as_deref() else {
                continue;
            };
            if parent.plan_warnings().iter().any(|warning| {
                warning.code == WarningCode::MissingEdgeTarget
                    && warning.path.as_deref() == Some(target_source)
            }) {
                continue;
            }
            let target = by_source.get(target_source).ok_or_else(|| {
                ProposedLayoutViolation::TargetAbsent {
                    target: target_source.to_string(),
                }
            })?;
            if target.evidence().iter().any(|code| {
                matches!(
                    code,
                    EvidenceCode::OutsideDevelopmentRoot | EvidenceCode::PermanentPath
                )
            }) {
                continue;
            }
            let available = proposed_available_paths(target);
            // Resolve on the reference's path component only: discovery strips
            // the query string and fragment before computing a target (see
            // `resolve_reference` in `artifact_discovery.rs`), but `available`
            // never carries one either, so matching the raw reference here
            // would reject every anchor- or query-bearing edge.
            for parent_path in proposed_available_paths(parent) {
                let resolved = match edge.resolution_mode {
                    EdgeResolutionMode::Relative => {
                        resolve_relative_reference(&parent_path, &edge.reference)
                    }
                    EdgeResolutionMode::RootRelative => normalize_artifact_path(
                        edge.reference
                            .split(['?', '#'])
                            .next()
                            .unwrap_or_default()
                            .trim_start_matches('/'),
                    ),
                    EdgeResolutionMode::External => continue,
                };
                if !available.contains(&resolved) {
                    return Err(ProposedLayoutViolation::EdgeUnavailable {
                        reference: edge.reference.clone(),
                        parent: parent.source().to_string(),
                        resolved,
                        available: available.iter().cloned().collect(),
                    });
                }
            }
        }
    }
    Ok(())
}

fn proposed_available_paths(artifact: &ArtifactPlanEntry) -> BTreeSet<String> {
    match artifact.action() {
        ArtifactAction::Move => artifact
            .destination()
            .map(str::to_string)
            .into_iter()
            .collect(),
        ArtifactAction::Copy => [Some(artifact.source()), artifact.destination()]
            .into_iter()
            .flatten()
            .map(str::to_string)
            .collect(),
        ArtifactAction::Retain | ArtifactAction::Block => {
            BTreeSet::from([artifact.source().to_string()])
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn classify_entry(
    entry: ArtifactPlanEntry,
    target: &PlanTarget,
    policy: &ArtifactClassificationPolicy,
    locations: &BTreeMap<String, ArtifactLocationFacts>,
    embedded_owners: &BTreeMap<String, Vec<&EmbeddedArtifactOwner>>,
    document_all_terminal: bool,
    needs_destination: &BTreeSet<String>,
    source_constraints: &BTreeSet<String>,
    unpreservable_parents: &BTreeSet<String>,
    layout: &ArchiveLayout<'_>,
) -> Result<ArtifactPlanEntry, PlanError> {
    if entry.version().is_pinned() {
        let evidence = merge(entry.evidence(), [EvidenceCode::PinnedHistorical]);
        return Ok(entry
            .with_action(ArtifactAction::Retain)
            .with_evidence(evidence));
    }

    let source = entry.source().to_string();
    let location = required_resolution_evidence(locations, &source)?.clone();
    let archived_source = policy.is_archived(&source);
    let permanent = policy.is_permanent(&source);
    let outside_development_root = !archived_source && policy.is_outside_development_root(&source);
    let managed = policy.is_managed(&source);
    let explicit = entry.provenance().contains(&ArtifactProvenance::Explicit);
    let embedded = entry.provenance().contains(&ArtifactProvenance::Embedded);

    let owners = select_direct_owners(
        entry.owners(),
        target,
        document_all_terminal,
        archived_source,
    );
    let repository_embedded = embedded_owners.get(&source).cloned().unwrap_or_default();
    let direct_outside_owner = matches!(target, PlanTarget::Container { .. })
        && owners.iter().any(|owner| !owner.inside_subtree);
    let outside_owner = direct_outside_owner
        || repository_embedded
            .iter()
            .any(|owner| embedded_owner_is_outside(owner, target));
    let active_owner = owners.iter().any(|owner| !owner.is_effectively_terminal())
        || repository_embedded
            .iter()
            .any(|owner| !owner.is_effectively_terminal());
    let unselected_unpinned = owners
        .iter()
        .any(|owner| !owner.pinned && !owner.selected_for_relink);
    let unmanaged_embedded =
        embedded && !managed && !permanent && !archived_source && !outside_development_root;

    let mut evidence = entry.evidence().to_vec();
    evidence.extend(
        [
            (permanent, EvidenceCode::PermanentPath),
            (
                outside_development_root,
                EvidenceCode::OutsideDevelopmentRoot,
            ),
            (outside_owner, EvidenceCode::OutsideOwner),
            (active_owner, EvidenceCode::ActiveOwner),
            (unmanaged_embedded, EvidenceCode::UnmanagedPath),
            (archived_source, EvidenceCode::ArchivedSource),
        ]
        .into_iter()
        .filter_map(|(present, evidence)| present.then_some(evidence)),
    );

    // A source retained in place never earns a destination, so the pair below
    // always selects `Retain` for it.
    let wants_destination = needs_destination.contains(&source);
    let needs_source = archived_source
        || outside_owner
        || active_owner
        || unselected_unpinned
        || unmanaged_embedded
        || source_constraints.contains(&source)
        || !wants_destination;
    let mut action = match (wants_destination, needs_source) {
        (true, true) => ArtifactAction::Copy,
        (true, false) => ArtifactAction::Move,
        (false, _) => ArtifactAction::Retain,
    };

    let mirror = layout
        .destination(&source)
        .unwrap_or_else(|| source.clone());
    let mut blockers = entry
        .blockers()
        .iter()
        .filter(|blocker| {
            blocker.code != BlockerCode::MissingSource
                || !matches!(location.destination, ArtifactLocation::Regular(_))
        })
        .cloned()
        .collect::<Vec<_>>();
    if unpreservable_parents.contains(&source) {
        blockers.push(PlanBlocker::new(
            BlockerCode::UnpreservableLayout,
            Some(&source),
        ));
    }
    let source_missing = matches!(location.source, ArtifactLocation::Missing);
    let embedded_missing = embedded && source_missing;
    if explicit
        && source_missing
        && !matches!(location.destination, ArtifactLocation::Regular(_))
        && !blockers
            .iter()
            .any(|blocker| blocker.code == BlockerCode::MissingSource)
    {
        blockers.push(PlanBlocker::new(BlockerCode::MissingSource, Some(&source)));
    }
    if matches!(location.source, ArtifactLocation::Symlink)
        || matches!(location.destination, ArtifactLocation::Symlink)
    {
        blockers.push(PlanBlocker::new(
            BlockerCode::SymlinkArtifact,
            Some(&source),
        ));
    }
    if matches!(location.source, ArtifactLocation::Unsupported) {
        blockers.push(PlanBlocker::new(
            BlockerCode::UnsupportedArtifactType,
            Some(&source),
        ));
    }
    if matches!(location.destination, ArtifactLocation::Unsupported) {
        blockers.push(PlanBlocker::new(
            BlockerCode::DestinationConflict,
            Some(&mirror),
        ));
    }

    let already_archived = if archived_source && explicit {
        true
    } else if wants_destination {
        match (&location.source, &location.destination) {
            (ArtifactLocation::Regular(source), ArtifactLocation::Regular(destination))
                if source == destination =>
            {
                true
            }
            (ArtifactLocation::Missing, ArtifactLocation::Regular(_)) => true,
            (_, ArtifactLocation::Regular(_)) => {
                blockers.push(PlanBlocker::new(
                    BlockerCode::DestinationConflict,
                    Some(&mirror),
                ));
                false
            }
            _ => false,
        }
    } else {
        false
    };

    if embedded_missing && !already_archived {
        action = ArtifactAction::Retain;
    }
    if !blockers.is_empty() {
        action = ArtifactAction::Block;
    }

    // Only `Move` and `Copy` write the destination a relink would name.
    // `Retain` leaves the source as the only file that exists
    // (`@/issue/8e071e18/decision/D-14`), and `Block` writes nothing either,
    // so neither may carry a relink regardless of what the ownership facts
    // alone selected above.
    let relinks_to_destination = matches!(action, ArtifactAction::Move | ArtifactAction::Copy);
    let owners = owners
        .into_iter()
        .map(|mut owner| {
            owner.selected_for_relink = owner.selected_for_relink && relinks_to_destination;
            owner
        })
        .collect::<Vec<_>>();

    let reference_changes = if archived_source && explicit {
        Vec::new()
    } else {
        owners
            .iter()
            .filter(|owner| owner.selected_for_relink)
            .map(|owner| ReferenceChange {
                issue: owner.issue.clone(),
                document_index: owner.document_index,
                from_path: source.clone(),
                to_path: mirror.clone(),
            })
            .collect()
    };
    let deletion_identity = match (&action, &location.source) {
        (ArtifactAction::Move, ArtifactLocation::Regular(identity)) => Some(identity.clone()),
        _ => None,
    };
    let pending_deletions = deletion_identity
        .clone()
        .map(|content_identity| PendingDeletion {
            source: source.clone(),
            content_identity,
        })
        .into_iter()
        .collect::<Vec<_>>();
    let publication_identity = match (
        &action,
        already_archived,
        &location.source,
        &location.destination,
    ) {
        (
            ArtifactAction::Move | ArtifactAction::Copy,
            true,
            _,
            ArtifactLocation::Regular(identity),
        ) => Some(identity.clone()),
        (
            ArtifactAction::Move | ArtifactAction::Copy,
            false,
            ArtifactLocation::Regular(identity),
            _,
        ) => Some(identity.clone()),
        _ => deletion_identity,
    };

    let mut classified = entry
        .with_action(action)
        .with_owners(owners)
        .with_reference_changes(reference_changes)
        .with_pending_deletions(pending_deletions)
        .with_evidence(evidence)
        .with_blockers(blockers)
        .with_already_archived(already_archived);
    if matches!(action, ArtifactAction::Move | ArtifactAction::Copy) {
        classified = classified.with_destination(mirror);
    }
    if let Some(identity) = publication_identity {
        classified = classified.with_content_identity(identity);
    }
    Ok(classified)
}

fn selected_destination_roots(
    target: &PlanTarget,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    document_all_terminal: bool,
    blockers: &mut Vec<PlanBlocker>,
) -> BTreeSet<String> {
    artifacts
        .iter()
        .filter(|entry| {
            entry.version() == &ArtifactVersion::WorkingTree
                && entry.provenance().contains(&ArtifactProvenance::Explicit)
        })
        .filter_map(|entry| {
            let source = entry.source();
            // A root retained in place earns no destination, and its absence
            // from the managed areas is the policy's intent rather than a
            // mistyped area entry.
            if policy.retains_source_in_place(source) {
                return None;
            }
            let archived = policy.is_archived(source);
            let managed = policy.is_managed(source);
            let selected = match target {
                // An ownerless root is a document its subtree owns by location.
                PlanTarget::Container { .. } => {
                    entry.owners().is_empty()
                        || entry.owners().iter().any(|owner| {
                            owner.inside_subtree && owner.is_effectively_terminal() && !owner.pinned
                        })
                }
                PlanTarget::Document { .. } => document_all_terminal,
            };
            if selected && !archived && !managed {
                blockers.push(PlanBlocker::new(
                    BlockerCode::UnmanagedSelectedRoot,
                    Some(source),
                ));
                return None;
            }
            (selected && !archived).then(|| source.to_string())
        })
        .collect()
}

/// Extend `needs_destination` to every relatively linked target of an artifact
/// that already has one, so a relative reference still resolves after the
/// mirror reproduces the parent's directory depth.
///
/// A target the policy retains in place is excluded: it
/// keeps its single copy wherever it is, so no relocation drags it into the
/// archive (`@/issue/8e071e18/decision/D-14`).
fn propagate_relative_destinations(
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    locations: &BTreeMap<String, ArtifactLocationFacts>,
    paths: &BTreeSet<String>,
    needs_destination: &mut BTreeSet<String>,
) {
    let edges = artifact_edges(artifacts);
    loop {
        let additional = edges
            .iter()
            .filter(|(parent, edge)| {
                needs_destination.contains(*parent)
                    && edge.resolution_mode == EdgeResolutionMode::Relative
            })
            .filter_map(|(_, edge)| {
                present_target(edge, locations, paths)
                    .or_else(|| archived_target(edge, locations, paths))
            })
            .filter(|target| !policy.retains_source_in_place(target))
            .filter(|target| !needs_destination.contains(*target))
            .cloned()
            .collect::<BTreeSet<_>>();
        if additional.is_empty() {
            break;
        }
        needs_destination.extend(additional);
    }
}

/// Targets of root-relative edges, which name the same source path from every
/// parent location and so must stay there.
fn root_relative_targets(
    artifacts: &[ArtifactPlanEntry],
    locations: &BTreeMap<String, ArtifactLocationFacts>,
    paths: &BTreeSet<String>,
) -> BTreeSet<String> {
    artifact_edges(artifacts)
        .into_iter()
        .filter(|(_, edge)| edge.resolution_mode == EdgeResolutionMode::RootRelative)
        .filter_map(|(_, edge)| present_target(edge, locations, paths).cloned())
        .collect()
}

fn artifact_edges(artifacts: &[ArtifactPlanEntry]) -> Vec<(&str, &ArtifactEdge)> {
    artifacts
        .iter()
        .flat_map(|entry| {
            entry
                .edges()
                .iter()
                .filter(|edge| edge.kind == EdgeKind::Supported)
                .map(move |edge| (entry.source(), edge))
        })
        .collect()
}

fn present_target<'a>(
    edge: &'a ArtifactEdge,
    locations: &BTreeMap<String, ArtifactLocationFacts>,
    paths: &'a BTreeSet<String>,
) -> Option<&'a String> {
    let target = edge
        .target
        .as_ref()
        .filter(|target| paths.contains(*target))?;
    let missing = locations
        .get(target)
        .is_some_and(|fact| matches!(fact.source, ArtifactLocation::Missing));
    (!missing).then_some(target)
}

/// An inventoried edge target an earlier run already archived: gone from its
/// source and present at its destination, where the edge must keep resolving.
fn archived_target<'a>(
    edge: &'a ArtifactEdge,
    locations: &BTreeMap<String, ArtifactLocationFacts>,
    paths: &'a BTreeSet<String>,
) -> Option<&'a String> {
    edge.target
        .as_ref()
        .filter(|target| paths.contains(*target))
        .filter(|target| {
            locations.get(*target).is_some_and(|fact| {
                matches!(fact.source, ArtifactLocation::Missing)
                    && matches!(fact.destination, ArtifactLocation::Regular(_))
            })
        })
}

fn select_direct_owners(
    owners: &[ArtifactOwner],
    target: &PlanTarget,
    document_all_terminal: bool,
    archived_source: bool,
) -> Vec<ArtifactOwner> {
    owners
        .iter()
        .cloned()
        .map(|mut owner| {
            owner.selected_for_relink = !owner.pinned
                && !archived_source
                && match target {
                    PlanTarget::Container { .. } => {
                        owner.inside_subtree && owner.is_effectively_terminal()
                    }
                    PlanTarget::Document { .. } => document_all_terminal,
                };
            owner
        })
        .collect()
}

fn document_owners_all_terminal(
    target: &PlanTarget,
    artifacts: &[ArtifactPlanEntry],
    embedded: &[EmbeddedArtifactOwner],
    inventory_paths: &BTreeSet<String>,
) -> bool {
    if !matches!(target, PlanTarget::Document { .. }) {
        return true;
    }
    artifacts
        .iter()
        .flat_map(ArtifactPlanEntry::owners)
        .all(|owner| owner.is_effectively_terminal())
        && embedded.iter().all(|owner| {
            !inventory_paths.contains(&normalize_artifact_path(&owner.artifact))
                || owner.is_effectively_terminal()
        })
}

fn unpreservable_parents(
    artifacts: &[ArtifactPlanEntry],
    inventory_paths: &BTreeSet<String>,
) -> BTreeSet<String> {
    artifacts
        .iter()
        .filter(|entry| {
            entry.edges().iter().any(|edge| {
                edge.kind == EdgeKind::Supported
                    && edge.target.as_ref().is_some_and(|target| {
                        !inventory_paths.contains(target)
                            && !entry.plan_warnings().iter().any(|warning| {
                                warning.code == WarningCode::MissingEdgeTarget
                                    && warning.path.as_deref() == Some(target)
                            })
                    })
            })
        })
        .map(|entry| entry.source().to_string())
        .collect()
}

fn embedded_owners_by_path(
    owners: &[EmbeddedArtifactOwner],
) -> BTreeMap<String, Vec<&EmbeddedArtifactOwner>> {
    owners.iter().fold(BTreeMap::new(), |mut grouped, owner| {
        grouped
            .entry(normalize_artifact_path(&owner.artifact))
            .or_insert_with(Vec::new)
            .push(owner);
        grouped
    })
}

fn embedded_owner_is_outside(owner: &EmbeddedArtifactOwner, target: &PlanTarget) -> bool {
    match target {
        PlanTarget::Container { .. } => !owner.inside_subtree,
        PlanTarget::Document { path } => normalize_artifact_path(&owner.root) != *path,
    }
}

fn append_container_destination_blocker(
    target: &PlanTarget,
    state: &ContainerDestinationState,
    destination_root: &str,
    blockers: &mut Vec<PlanBlocker>,
) {
    if !matches!(target, PlanTarget::Container { .. }) {
        return;
    }
    let code = match state {
        ContainerDestinationState::Available => return,
        ContainerDestinationState::Conflict => BlockerCode::DestinationConflict,
        ContainerDestinationState::Symlink => BlockerCode::SymlinkArtifact,
    };
    blockers.push(PlanBlocker::new(code, Some(destination_root)));
}

/// Compute a container's preferred human-readable destination before storage reconciliation.
///
/// The short id is the authoritative collision-resistant prefix. It gains a
/// slug suffix only when the issue carries exactly one type label and exactly
/// one label in that type's configured membership namespace; every other shape,
/// including a membership value that normalizes to nothing, resolves to the
/// bare short-id directory.
pub fn preferred_container_destination_root(
    issue: &Issue,
    hierarchy: &HierarchyConfig,
    archive_root: &str,
) -> String {
    let issue_types = issue
        .labels
        .iter()
        .filter_map(|label| type_value_of(label))
        .collect::<Vec<_>>();
    let strategic_value = (issue_types.len() == 1)
        .then(|| hierarchy.get_membership_namespace(issue_types[0]))
        .flatten()
        .and_then(|namespace| {
            let values = issue
                .labels
                .iter()
                .filter_map(|label| label.split_once(':'))
                .filter_map(|(candidate, value)| (candidate == namespace).then_some(value))
                .collect::<Vec<_>>();
            (values.len() == 1).then(|| values[0])
        });
    let short_id = container_short_id(&issue.id);
    let directory = strategic_value
        .and_then(archive_container_slug)
        .map(|slug| format!("{short_id}-{slug}"))
        .unwrap_or(short_id);
    join_path(archive_root, &directory)
}

/// Normalize one label value into a bounded path component.
///
/// This is the crate's single slug normalizer for archive directory names. It
/// is crate-visible rather than module-private so that every resolver naming
/// such a directory, including one in a sibling module, derives its slug from
/// this implementation instead of a second copy (`@/inv/single-source-prose`).
/// Lowercases Unicode alphanumeric characters, collapses every other run into a
/// single `-`, and bounds the result at 48 characters so an arbitrarily long
/// label value cannot name an unwieldy directory. Returns `None` when the value
/// holds no alphanumeric character and therefore names nothing, leaving the
/// caller with the bare short-id directory.
pub(crate) fn archive_container_slug(value: &str) -> Option<String> {
    const MAX_CHARS: usize = 48;

    let mut slug = String::new();
    let mut separator_pending = false;
    for character in value.chars() {
        if character.is_alphanumeric() {
            if separator_pending && !slug.is_empty() {
                slug.push('-');
            }
            slug.extend(character.to_lowercase());
            separator_pending = false;
        } else {
            separator_pending = !slug.is_empty();
        }
    }
    let bounded = slug.chars().take(MAX_CHARS).collect::<String>();
    let bounded = bounded.trim_end_matches('-');
    (!bounded.is_empty()).then(|| bounded.to_string())
}

fn target_document_path(target: &PlanTarget) -> Option<String> {
    match target {
        PlanTarget::Document { path } => Some(path.clone()),
        PlanTarget::Container { .. } => None,
    }
}

fn container_short_id(id: &IssueId) -> String {
    id.short_id().to_string()
}

/// Where one plan publishes each repository-relative source beneath its
/// destination root.
///
/// Sources inside a configured development root retain only their path relative
/// to that policy boundary, which preserves the relative layout among them.
/// Flat managed areas never participate. Document targets retain every source
/// component beneath the development root because they have no container owner
/// in their destination root.
///
/// A container target also removes a matching `<issue-scoped-area>/<container>/`
/// owner prefix, since the destination root already records that ownership —
/// but only when every relative edge between inventoried artifacts still
/// resolves afterwards. The removal moves the owner's artifacts relative to
/// their siblings, and publication copies bytes unchanged, so an edge crossing
/// the owner directory would break; such a plan keeps the owner directory.
///
/// An empty development root represents the repository root. Sources outside a
/// non-empty development root have no destination because classification
/// retains them.
#[derive(Debug, Clone, Copy)]
pub struct ArchiveLayout<'a> {
    destination_root: &'a str,
    policy: &'a ArtifactClassificationPolicy,
    removes_owner: bool,
}

impl<'a> ArchiveLayout<'a> {
    /// Decide the layout for `artifacts`, the plan's complete inventory.
    pub fn new(
        target: &'a PlanTarget,
        destination_root: &'a str,
        policy: &'a ArtifactClassificationPolicy,
        artifacts: &[ArtifactPlanEntry],
    ) -> Self {
        let removing = Self {
            destination_root,
            policy,
            removes_owner: matches!(target, PlanTarget::Container { .. }),
        };
        let inventoried = artifacts
            .iter()
            .map(ArtifactPlanEntry::source)
            .collect::<BTreeSet<_>>();
        let preserved = artifacts.iter().all(|parent| {
            parent
                .edges()
                .iter()
                .filter(|edge| {
                    edge.kind == EdgeKind::Supported
                        && edge.resolution_mode == EdgeResolutionMode::Relative
                })
                .filter_map(|edge| edge.target.as_deref().map(|target| (edge, target)))
                .filter(|(_, target)| inventoried.contains(target))
                .all(|(edge, target)| {
                    match (
                        removing.destination(parent.source()),
                        removing.destination(target),
                    ) {
                        (Some(parent), Some(target)) => {
                            resolve_relative_reference(&parent, &edge.reference) == target
                        }
                        _ => true,
                    }
                })
        });
        Self {
            removes_owner: removing.removes_owner && preserved,
            ..removing
        }
    }

    /// The destination of `source`, or `None` outside the development root.
    pub fn destination(&self, source: &str) -> Option<String> {
        let development_root = &self.policy.development_root;
        let source = normalize_artifact_path(source);
        let relative = if development_root.is_empty() {
            source.as_str()
        } else if source == development_root.as_str() {
            return Some(normalize_artifact_path(self.destination_root));
        } else {
            source.strip_prefix(&format!("{development_root}/"))?
        };
        let relative = if self.removes_owner {
            strip_matching_container_owner(
                self.destination_root,
                development_root,
                &self.policy.issue_scoped_areas,
                relative,
            )
        } else {
            relative.to_string()
        };
        Some(join_path(self.destination_root, &relative))
    }
}

/// The repository path a relative `reference` written in the document at
/// `parent` names, ignoring any query string or fragment.
fn resolve_relative_reference(parent: &str, reference: &str) -> String {
    let reference_path = reference.split(['?', '#']).next().unwrap_or_default();
    let parent_dir = Path::new(parent).parent().unwrap_or(Path::new(""));
    normalize_artifact_path(&parent_dir.join(reference_path).to_string_lossy())
}

/// Remove the redundant owner from one exact configured issue-scoped area.
///
/// Areas may contain multiple components. Only the component immediately
/// beneath a complete configured area participates; matching-looking directories
/// in flat managed areas and repeated third-party names deeper in an artifact,
/// such as `vendor/reveal.js/reveal.js`, remain ordinary structure.
fn strip_matching_container_owner(
    destination_root: &str,
    development_root: &str,
    issue_scoped_areas: &[String],
    relative: &str,
) -> String {
    let destination_root = normalize_artifact_path(destination_root);
    let Some(container) = destination_root.rsplit('/').next() else {
        return relative.to_string();
    };
    issue_scoped_areas
        .iter()
        .filter_map(|area| development_relative_area(development_root, area))
        .find_map(|area| {
            let beneath_area = if area.is_empty() {
                relative
            } else {
                relative.strip_prefix(&format!("{area}/"))?
            };
            let artifact = beneath_area.strip_prefix(&format!("{container}/"))?;
            Some(join_path(&area, artifact))
        })
        .unwrap_or_else(|| relative.to_string())
}

/// Express one exact configured area relative to the development boundary.
fn development_relative_area(development_root: &str, area: &str) -> Option<String> {
    let development_root = normalize_artifact_path(development_root);
    let area = normalize_artifact_path(area);
    if area.is_empty() {
        None
    } else if development_root.is_empty() {
        Some(area)
    } else if area == development_root {
        Some(String::new())
    } else {
        area.strip_prefix(&format!("{development_root}/"))
            .map(str::to_string)
    }
}

fn join_path(left: &str, right: &str) -> String {
    normalize_artifact_path(&format!("{left}/{right}"))
}

/// Component-aware containment for repository-relative policy roots.
pub fn contains_path(root: &str, candidate: &str) -> bool {
    let root = normalize_artifact_path(root);
    let candidate = normalize_artifact_path(candidate);
    !root.is_empty()
        && (candidate == root
            || candidate
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with('/')))
}

fn normalized_paths(paths: Vec<String>) -> Vec<String> {
    paths
        .into_iter()
        .map(|path| normalize_artifact_path(&path))
        .filter(|path| !path.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn merge<T: Clone>(existing: &[T], additional: impl IntoIterator<Item = T>) -> Vec<T> {
    existing.iter().cloned().chain(additional).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::type_taxonomy::HierarchyConfig;
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn container_id() -> IssueId {
        crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12)
    }

    fn identity(bytes: &[u8]) -> ContentIdentity {
        ContentIdentity::from_bytes(bytes)
    }

    fn owner(issue: crate::domain::identity::IssueId, state: State, inside: bool) -> ArtifactOwner {
        ArtifactOwner {
            issue,
            document_index: 0,
            state,
            archived_from: None,
            inside_subtree: inside,
            pinned: false,
            selected_for_relink: false,
        }
    }

    fn archived_owner(
        issue: crate::domain::identity::IssueId,
        archived_from: Option<State>,
        inside: bool,
    ) -> ArtifactOwner {
        ArtifactOwner {
            archived_from,
            ..owner(issue, State::Archived, inside)
        }
    }

    #[test]
    fn test_owner_effective_terminality_preserves_archived_origin() {
        // An owner archived from a terminal state is effectively terminal (its
        // documents can be archived); one archived from a non-terminal state, or
        // is not — it still counts as an active owner.
        assert!(
            archived_owner(crate::test_utils::issue_id(175), Some(State::Done), true)
                .is_effectively_terminal()
        );
        assert!(archived_owner(
            crate::test_utils::issue_id(175),
            Some(State::Rejected),
            true
        )
        .is_effectively_terminal());
        assert!(!archived_owner(
            crate::test_utils::issue_id(175),
            Some(State::InProgress),
            true
        )
        .is_effectively_terminal());
        assert!(
            owner(crate::test_utils::issue_id(175), State::Done, true).is_effectively_terminal()
        );
        assert!(
            !owner(crate::test_utils::issue_id(175), State::InProgress, true)
                .is_effectively_terminal()
        );
    }

    #[test]
    fn test_destination_source_conflicts_groups_conflicting_sources_deterministically() {
        let sources = BTreeSet::from([
            "./dev/active/plan.md".to_string(),
            "dev/active/plan.md".to_string(),
        ]);
        let target = PlanTarget::Document {
            path: "dev/active/plan.md".to_string(),
        };
        let policy = ArtifactClassificationPolicy::configured("dev", vec![], vec![], "archive");

        assert_eq!(
            destination_source_conflicts(
                &ArchiveLayout::new(&target, "archive/container", &policy, &[]),
                &sources,
            ),
            BTreeMap::from([(
                "archive/container/active/plan.md".to_string(),
                vec![
                    "./dev/active/plan.md".to_string(),
                    "dev/active/plan.md".to_string(),
                ],
            )]),
        );
    }

    #[test]
    fn test_archive_layout_destination_strips_matching_owners_in_configured_issue_scoped_areas() {
        let policy = ArtifactClassificationPolicy::configured("dev", vec![], vec![], "dev/archive")
            .with_issue_scoped_areas(vec!["dev/active".into(), "dev/presentations".into()]);
        let cases = [
            (
                "dev/active/abcdef12-platform/plan.md",
                "dev/archive/abcdef12-platform/active/plan.md",
            ),
            (
                "dev/presentations/abcdef12-platform/vendor/reveal.js/reveal.js",
                "dev/archive/abcdef12-platform/presentations/vendor/reveal.js/reveal.js",
            ),
        ];

        cases.into_iter().for_each(|(source, expected)| {
            assert_eq!(
                ArchiveLayout::new(
                    &PlanTarget::Container {
                        id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                    },
                    "dev/archive/abcdef12-platform",
                    &policy,
                    &[]
                )
                .destination(source),
                Some(expected.to_string()),
            );
        });
    }

    #[test]
    fn test_archive_layout_destination_preserves_nonmatching_owner_directories_and_document_paths()
    {
        let container = PlanTarget::Container {
            id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
        };
        let policy = ArtifactClassificationPolicy::configured("dev", vec![], vec![], "dev/archive")
            .with_issue_scoped_areas(vec!["dev/active".into(), "dev/presentations".into()]);
        for source in [
            "dev/active/fedcba98-other/plan.md",
            "dev/presentations/fedcba98-other/talk.html",
        ] {
            let relative = source.strip_prefix("dev/").unwrap();
            assert_eq!(
                ArchiveLayout::new(&container, "dev/archive/abcdef12-platform", &policy, &[])
                    .destination(source),
                Some(format!("dev/archive/abcdef12-platform/{relative}")),
            );
        }

        assert_eq!(
            ArchiveLayout::new(
                &PlanTarget::Document {
                    path: "dev/active/archive/plan.md".to_string(),
                },
                "dev/archive",
                &policy,
                &[]
            )
            .destination("dev/active/archive/plan.md"),
            Some("dev/archive/active/archive/plan.md".to_string()),
            "document archival must not interpret a matching directory as a container owner",
        );
    }

    #[test]
    fn test_destination_source_conflicts_detects_nested_issue_scoped_owner_collisions() {
        let sources = BTreeSet::from([
            "dev/team/active/abcdef12-platform/plan.md".to_string(),
            "dev/team/active/plan.md".to_string(),
        ]);
        let target = PlanTarget::Container {
            id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
        };
        let policy = ArtifactClassificationPolicy::configured(
            "dev",
            vec!["dev/team/active".into()],
            vec![],
            "dev/archive",
        )
        .with_issue_scoped_areas(vec!["./dev/team/active/".into()]);

        assert_eq!(
            destination_source_conflicts(
                &ArchiveLayout::new(&target, "dev/archive/abcdef12-platform", &policy, &[]),
                &sources,
            ),
            BTreeMap::from([(
                "dev/archive/abcdef12-platform/team/active/plan.md".to_string(),
                vec![
                    "dev/team/active/abcdef12-platform/plan.md".to_string(),
                    "dev/team/active/plan.md".to_string(),
                ],
            )]),
        );

        let artifacts = sources
            .iter()
            .map(|source| {
                explicit(
                    source,
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )
            })
            .collect();
        let facts = locations(&[
            (
                "dev/team/active/abcdef12-platform/plan.md",
                ArtifactLocation::Regular(identity(b"owned")),
                ArtifactLocation::Missing,
            ),
            (
                "dev/team/active/plan.md",
                ArtifactLocation::Regular(identity(b"flat")),
                ArtifactLocation::Missing,
            ),
        ]);
        let plan = classify_artifacts(
            ArtifactClassificationInventory::new(target, artifacts, Vec::new()),
            "dev/archive/abcdef12-platform",
            policy,
            facts,
        )
        .unwrap();
        assert!(!plan.eligible());
        assert_eq!(
            plan.blockers()
                .iter()
                .filter(|blocker| blocker.code == BlockerCode::DestinationConflict)
                .filter_map(|blocker| blocker.path.clone())
                .collect::<BTreeSet<_>>(),
            sources,
            "classification must block every source collapsed by owner stripping",
        );
    }

    #[test]
    fn test_container_destination_preserves_matching_directory_in_flat_managed_area() {
        let source = "dev/generated/abcdef12-platform/report.md";
        let policy = ArtifactClassificationPolicy::from_documentation(Some(&DocumentationConfig {
            development_root: Some("dev".to_string()),
            managed_paths: Some(vec!["dev/generated".to_string()]),
            archive_root: Some("dev/archive".to_string()),
            permanent_paths: Some(Vec::new()),
            issue_scoped_areas: Some(vec!["dev/team/active".to_string()]),
            citation_scan_roots: Some(Vec::new()),
        }));
        let plan = classify_artifacts(
            ArtifactClassificationInventory::new(
                PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                },
                vec![explicit(
                    source,
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )],
                Vec::new(),
            ),
            "dev/archive/abcdef12-platform",
            policy,
            locations(&[(
                source,
                ArtifactLocation::Regular(identity(b"flat")),
                ArtifactLocation::Missing,
            )]),
        )
        .unwrap();

        assert_eq!(
            entry(&plan, source).destination(),
            Some("dev/archive/abcdef12-platform/generated/abcdef12-platform/report.md"),
            "a matching-looking directory in a flat managed area is artifact structure",
        );
    }

    #[test]
    fn test_container_destination_strips_owner_in_normalized_nested_issue_scoped_area() {
        let source = "dev/team/active/abcdef12-platform/plan.md";
        let policy = ArtifactClassificationPolicy::from_documentation(Some(&DocumentationConfig {
            development_root: Some("./dev/".to_string()),
            managed_paths: Some(vec!["dev/team/active".to_string()]),
            archive_root: Some("dev/archive".to_string()),
            permanent_paths: Some(Vec::new()),
            issue_scoped_areas: Some(vec!["./dev/team/active/".to_string()]),
            citation_scan_roots: Some(Vec::new()),
        }));
        let plan = classify_artifacts(
            ArtifactClassificationInventory::new(
                PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                },
                vec![explicit(
                    source,
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )],
                Vec::new(),
            ),
            "dev/archive/abcdef12-platform",
            policy,
            locations(&[(
                source,
                ArtifactLocation::Regular(identity(b"nested")),
                ArtifactLocation::Missing,
            )]),
        )
        .unwrap();

        assert_eq!(
            entry(&plan, source).destination(),
            Some("dev/archive/abcdef12-platform/team/active/plan.md"),
        );
    }

    /// The relative reference a document at `parent` writes to reach `target`.
    fn relative_reference(parent: &str, target: &str) -> String {
        let parent_dir = parent.split('/').collect::<Vec<_>>();
        let parent_dir = &parent_dir[..parent_dir.len() - 1];
        let target = target.split('/').collect::<Vec<_>>();
        let shared = parent_dir
            .iter()
            .zip(&target)
            .take_while(|(left, right)| left == right)
            .count();
        std::iter::repeat_n("..", parent_dir.len() - shared)
            .chain(target[shared..].iter().copied())
            .collect::<Vec<_>>()
            .join("/")
    }

    /// Classify a container plan over present `sources`, each owned inside the
    /// subtree, with a relative edge for every `(parent, target)` pair.
    fn plan_with_relative_edges(sources: &[&str], edges: &[(&str, &str)]) -> ArtifactPlan {
        plan_with_owned_relative_edges(sources, &[], edges)
    }

    /// [`plan_with_relative_edges`], where each source in `shared` also has an
    /// owner outside the subtree, so the plan copies it rather than moving it.
    fn plan_with_owned_relative_edges(
        sources: &[&str],
        shared: &[&str],
        edges: &[(&str, &str)],
    ) -> ArtifactPlan {
        let present = sources
            .iter()
            .map(|source| {
                (
                    *source,
                    ArtifactLocation::Regular(identity(source.as_bytes())),
                    ArtifactLocation::Missing,
                )
            })
            .collect::<Vec<_>>();
        container(
            sources
                .iter()
                .map(|source| {
                    let owners =
                        std::iter::once(owner(crate::test_utils::issue_id(176), State::Done, true))
                            .chain(shared.contains(source).then(|| {
                                owner(crate::test_utils::issue_id(177), State::Done, false)
                            }))
                            .collect();
                    explicit(source, owners).with_edges(
                        edges
                            .iter()
                            .filter(|(parent, _)| parent == source)
                            .map(|(parent, target)| {
                                edge(
                                    &relative_reference(parent, target),
                                    target,
                                    EdgeResolutionMode::Relative,
                                )
                            })
                            .collect(),
                    )
                })
                .collect(),
            locations(&present),
        )
    }

    #[test]
    fn test_classify_artifacts_keeps_the_owner_directory_when_collapsing_it_breaks_a_relative_edge()
    {
        let owned = "dev/active/abcdef12/classification.md";
        let sibling = "dev/active/220cab0b/design.md";
        let plan = plan_with_relative_edges(&[owned, sibling], &[(sibling, owned)]);

        assert!(plan.eligible(), "{:?}", plan.blockers());
        validate_proposed_layout(&plan).expect("an eligible plan's layout resolves");
        assert_eq!(
            entry(&plan, owned).destination(),
            Some("dev/archive/abcdef12/active/abcdef12/classification.md")
        );
        // Without the crossing edge the owner directory collapses as usual.
        let plan = plan_with_relative_edges(&[owned, sibling], &[]);
        assert_eq!(
            entry(&plan, owned).destination(),
            Some("dev/archive/abcdef12/active/classification.md")
        );
    }

    #[test]
    fn test_classify_artifacts_blocks_a_plan_the_proposed_layout_check_rejects() {
        let parent = "dev/active/abcdef12/plan.md";
        let target = "dev/active/other/notes.md";
        let present = [parent, target].map(|source| {
            (
                source,
                ArtifactLocation::Regular(identity(source.as_bytes())),
                ArtifactLocation::Missing,
            )
        });
        // The reference does not reach the target it names from any layout.
        let plan = container(
            vec![
                explicit(
                    parent,
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )
                .with_edges(vec![edge(
                    "../elsewhere.md",
                    target,
                    EdgeResolutionMode::Relative,
                )]),
                explicit(
                    target,
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                ),
            ],
            locations(&present),
        );

        assert!(validate_proposed_layout(&plan).is_err());
        assert!(plan.blockers().contains(&PlanBlocker::new(
            BlockerCode::UnpreservableLayout,
            Some(parent),
        )));
        assert!(!plan.eligible());
    }

    proptest! {
        #[test]
        fn test_classify_artifacts_eligible_plan_always_passes_the_proposed_layout_check(
            chosen in prop::collection::btree_set(0usize..6, 1..=6),
            shared in prop::collection::btree_set(0usize..6, 0..=6),
            edges in prop::collection::vec((0usize..6, 0usize..6), 0..8),
        ) {
            const POOL: [&str; 6] = [
                "dev/active/abcdef12/plan.md",
                "dev/active/abcdef12/sub/notes.md",
                "dev/active/220cab0b/design.md",
                "dev/active/220cab0b/deep/data.md",
                "dev/active/notes.md",
                "dev/active/abcdef12.md",
            ];
            let sources = chosen.iter().map(|index| POOL[*index]).collect::<Vec<_>>();
            let edges = edges
                .into_iter()
                .filter(|(parent, target)| {
                    parent != target && chosen.contains(parent) && chosen.contains(target)
                })
                .map(|(parent, target)| (POOL[parent], POOL[target]))
                .collect::<Vec<_>>();
            let shared = shared
                .intersection(&chosen)
                .map(|index| POOL[*index])
                .collect::<Vec<_>>();
            let plan = plan_with_owned_relative_edges(&sources, &shared, &edges);
            prop_assert!(plan.eligible(), "{:?}", plan.blockers());
            prop_assert!(validate_proposed_layout(&plan).is_ok(), "{:?}", validate_proposed_layout(&plan));
        }
    }

    proptest! {
        #[test]
        fn test_archive_layout_destination_strips_any_configured_development_root(
            development_segments in prop::collection::vec("[a-z][a-z0-9-]{0,7}", 1..4),
            relative_segments in prop::collection::vec("[a-z][a-z0-9.-]{0,9}", 1..6),
            archive_segments in prop::collection::vec("[a-z][a-z0-9-]{0,7}", 1..4),
        ) {
            let development_root = development_segments.join("/");
            let relative = relative_segments.join("/");
            let archive_root = archive_segments.join("/");
            let destination_root = format!("{archive_root}/container");
            let source = format!("./{development_root}/{relative}");
            let policy = ArtifactClassificationPolicy::configured(
                format!("./{development_root}/"),
                vec![],
                vec![],
                "archive",
            );

            prop_assert_eq!(
                ArchiveLayout::new(&PlanTarget::Document { path: source.clone() }, &format!("./{destination_root}/"), &policy, &[]).destination(&source),
                Some(format!("{destination_root}/{relative}")),
            );
        }

        #[test]
        fn test_archive_layout_destination_is_injective_for_normalized_relative_paths(
            development_root in "[a-z][a-z0-9-]{0,7}",
            left in prop::collection::vec("[a-z][a-z0-9.-]{0,9}", 1..6),
            right in prop::collection::vec("[a-z][a-z0-9.-]{0,9}", 1..6),
        ) {
            let left = left.join("/");
            let right = right.join("/");
            prop_assume!(left != right);
            let target = PlanTarget::Document { path: format!("{development_root}/{left}") };
            let policy = ArtifactClassificationPolicy::configured(
                development_root.clone(),
                vec![],
                vec![],
                "archives",
            );

            prop_assert_ne!(
                ArchiveLayout::new(&target, "archives/container", &policy, &[]).destination(&format!("{development_root}/{left}")),
                ArchiveLayout::new(&target, "archives/container", &policy, &[]).destination(&format!("{development_root}/{right}")),
            );
        }
    }

    fn explicit(path: &str, owners: Vec<ArtifactOwner>) -> ArtifactPlanEntry {
        ArtifactPlanEntry::new(path, ArtifactVersion::WorkingTree, ArtifactAction::Retain)
            .with_provenance(vec![ArtifactProvenance::Explicit])
            .with_owners(owners)
    }

    fn embedded(path: &str) -> ArtifactPlanEntry {
        ArtifactPlanEntry::new(path, ArtifactVersion::WorkingTree, ArtifactAction::Retain)
            .with_provenance(vec![ArtifactProvenance::Embedded])
    }

    fn edge(reference: &str, target: &str, mode: EdgeResolutionMode) -> ArtifactEdge {
        ArtifactEdge {
            reference: reference.to_string(),
            target: Some(target.to_string()),
            kind: EdgeKind::Supported,
            resolution_mode: mode,
        }
    }

    fn directory(scope: ArtifactListingScope, entries: &[&str]) -> ArtifactEvidence {
        ArtifactEvidence::Directory {
            scope,
            entries: entries.iter().map(|entry| (*entry).to_string()).collect(),
        }
    }

    #[test]
    fn test_destination_resolution_uses_marker_preferred_root_and_duplicate_evidence() {
        let cases = [
            (
                vec!["archive/abcdef12-old"],
                vec![("archive/abcdef12-old", format!(" \n{}\t", container_id()))],
                "archive/abcdef12-old",
                Vec::new(),
            ),
            (Vec::new(), Vec::new(), "archive/abcdef12-new", Vec::new()),
            (
                vec!["archive/abcdef12-a", "archive/abcdef12-b"],
                vec![
                    ("archive/abcdef12-a", container_id().to_string()),
                    ("archive/abcdef12-b", container_id().to_string()),
                ],
                "archive/abcdef12-a",
                vec!["archive/abcdef12-a", "archive/abcdef12-b"],
            ),
        ];
        for (children, markers, expected, conflicts) in cases {
            let mut evidence = ArtifactEvidenceMap::from([(
                "archive".to_string(),
                directory(ArtifactListingScope::ImmediateChildren, &children),
            )]);
            children.iter().for_each(|child| {
                evidence.insert(
                    (*child).to_string(),
                    directory(ArtifactListingScope::MetadataOnly, &[]),
                );
            });
            markers.into_iter().for_each(|(root, owner)| {
                evidence.insert(
                    container_marker_path(root),
                    ArtifactEvidence::File(owner.into_bytes()),
                );
            });
            let resolved = resolve_container_destination(
                "archive",
                "archive/abcdef12-new",
                &container_id(),
                &evidence,
            )
            .unwrap();
            assert_eq!(resolved.destination_root, expected);
            assert_eq!(resolved.conflicting_roots, conflicts);
        }
    }

    #[test]
    fn test_resolve_container_destination_does_not_adopt_an_unmarked_bare_short_id_directory() {
        let issue = container_issue("Mutable title", &["type:epic", "epic:artifact-archival"]);
        let preferred =
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive");
        let bare = format!("archive/{}", container_short_id(&issue.id));
        assert_ne!(
            preferred, bare,
            "the precondition needs a preferred name the bare directory does not already carry"
        );
        let evidence = ArtifactEvidenceMap::from([
            (
                "archive".to_string(),
                directory(ArtifactListingScope::ImmediateChildren, &[bare.as_str()]),
            ),
            (
                bare.clone(),
                directory(ArtifactListingScope::MetadataOnly, &[]),
            ),
            (container_marker_path(&bare), ArtifactEvidence::Missing),
        ]);
        let resolved =
            resolve_container_destination("archive", &preferred, &issue.id, &evidence).unwrap();
        assert_eq!(resolved.destination_root, preferred);
        assert!(resolved.conflicting_roots.is_empty());
    }

    #[test]
    fn test_destination_resolution_rejects_missing_immediate_child_evidence() {
        let evidence = ArtifactEvidenceMap::from([(
            "archive".into(),
            directory(ArtifactListingScope::ImmediateChildren, &["archive/unread"]),
        )]);
        assert_eq!(
            resolve_container_destination("archive", "archive/new", &container_id(), &evidence,),
            Err(ArtifactEvidenceError::IncompleteEvidence {
                path: "archive/unread".into(),
            })
        );
    }

    #[test]
    fn test_destination_resolution_rejects_missing_directory_marker_evidence() {
        let evidence = ArtifactEvidenceMap::from([
            (
                "archive".into(),
                directory(
                    ArtifactListingScope::ImmediateChildren,
                    &["archive/unmarked"],
                ),
            ),
            (
                "archive/unmarked".into(),
                directory(ArtifactListingScope::MetadataOnly, &[]),
            ),
        ]);
        assert_eq!(
            resolve_container_destination("archive", "archive/new", &container_id(), &evidence,),
            Err(ArtifactEvidenceError::IncompleteEvidence {
                path: container_marker_path("archive/unmarked"),
            })
        );
    }

    #[test]
    fn test_evidence_listing_scopes_are_not_interchangeable() {
        let resolution = ArtifactEvidenceMap::from([(
            "archive".into(),
            directory(ArtifactListingScope::RecursiveFiles, &[]),
        )]);
        assert!(resolve_container_destination(
            "archive",
            "archive/new",
            &container_id(),
            &resolution
        )
        .is_err());

        let artifact = embedded("docs/a.md");
        let evidence = ArtifactEvidenceMap::from([
            ("docs/a.md".into(), ArtifactEvidence::File(b"a".to_vec())),
            ("archive/docs/a.md".into(), ArtifactEvidence::Missing),
            (
                "archive".into(),
                directory(ArtifactListingScope::ImmediateChildren, &[]),
            ),
        ]);
        assert!(classification_facts_from_evidence(
            &PlanTarget::Container {
                id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12)
            },
            "archive",
            &[artifact],
            &ArtifactClassificationPolicy::configured("", vec![], vec![], "archive"),
            Vec::new(),
            &evidence,
        )
        .is_err());
    }

    #[test]
    fn test_classification_evidence_derives_locations_and_recursive_occupancy() {
        let artifact = embedded("docs/a.md");
        for (entries, expected) in [
            (
                vec!["archive/docs/a.md"],
                ContainerDestinationState::Available,
            ),
            (
                vec!["archive/docs/a.md", "archive/foreign"],
                ContainerDestinationState::Conflict,
            ),
        ] {
            let evidence = ArtifactEvidenceMap::from([
                (
                    "docs/a.md".into(),
                    ArtifactEvidence::File(b"source".to_vec()),
                ),
                (
                    "archive/docs/a.md".into(),
                    ArtifactEvidence::File(b"mirror".to_vec()),
                ),
                (
                    "archive".into(),
                    directory(ArtifactListingScope::RecursiveFiles, &entries),
                ),
                (container_marker_path("archive"), ArtifactEvidence::Missing),
            ]);
            let facts = classification_facts_from_evidence(
                &PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                },
                "archive",
                std::slice::from_ref(&artifact),
                &ArtifactClassificationPolicy::configured("", vec![], vec![], "archive"),
                Vec::new(),
                &evidence,
            )
            .unwrap();
            assert_eq!(facts.container_destination, expected);
            assert_eq!(
                facts.locations["docs/a.md"],
                ArtifactLocationFacts {
                    source: ArtifactLocation::Regular(identity(b"source")),
                    destination: ArtifactLocation::Regular(identity(b"mirror")),
                }
            );
        }
    }

    #[test]
    fn test_classification_evidence_maps_each_destination_to_its_outcome() {
        let artifact = embedded("docs/a.md");
        let owner_marker =
            |owner: &IssueId| ArtifactEvidence::File(owner.as_str().as_bytes().to_vec());
        let cases = [
            (
                "absent",
                ArtifactEvidence::Missing,
                None,
                ContainerDestinationState::Available,
            ),
            (
                "owned by target",
                directory(ArtifactListingScope::RecursiveFiles, &["archive/foreign"]),
                Some(owner_marker(&container_id())),
                ContainerDestinationState::Available,
            ),
            (
                "owned by other",
                directory(ArtifactListingScope::RecursiveFiles, &[]),
                Some(owner_marker(&crate::test_utils::issue_id(1))),
                ContainerDestinationState::Conflict,
            ),
            (
                "symlink root",
                ArtifactEvidence::Symlink,
                None,
                ContainerDestinationState::Symlink,
            ),
            (
                "symlink marker",
                directory(ArtifactListingScope::RecursiveFiles, &[]),
                Some(ArtifactEvidence::Symlink),
                ContainerDestinationState::Symlink,
            ),
            (
                "file in place of directory",
                ArtifactEvidence::File(b"x".to_vec()),
                None,
                ContainerDestinationState::Conflict,
            ),
        ];
        for (name, root, marker, expected) in cases {
            let mut evidence = ArtifactEvidenceMap::from([
                ("docs/a.md".into(), ArtifactEvidence::File(b"a".to_vec())),
                ("archive/docs/a.md".into(), ArtifactEvidence::Missing),
                ("archive".into(), root),
            ]);
            if let Some(marker) = marker {
                evidence.insert(container_marker_path("archive"), marker);
            }
            let facts = classification_facts_from_evidence(
                &PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                },
                "archive",
                std::slice::from_ref(&artifact),
                &ArtifactClassificationPolicy::configured("", vec![], vec![], "archive"),
                Vec::new(),
                &evidence,
            )
            .unwrap();
            assert_eq!(facts.container_destination, expected, "{name}");
        }
    }

    #[test]
    fn test_classification_evidence_rejects_uninspected_location() {
        let artifact = embedded("docs/a.md");
        let evidence = ArtifactEvidenceMap::from([(
            "docs/a.md".into(),
            ArtifactEvidence::File(b"a".to_vec()),
        )]);
        assert_eq!(
            classification_facts_from_evidence(
                &PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12)
                },
                "archive",
                &[artifact],
                &ArtifactClassificationPolicy::configured("", vec![], vec![], "archive"),
                Vec::new(),
                &evidence,
            ),
            Err(ArtifactEvidenceError::IncompleteEvidence {
                path: "archive/docs/a.md".into(),
            })
        );
    }

    #[test]
    fn test_classify_artifacts_rejects_working_tree_entry_without_location_facts() {
        let result = classify_artifacts(
            ArtifactClassificationInventory::new(
                PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                },
                vec![explicit(
                    "dev/active/doc.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )],
                Vec::new(),
            ),
            "archive/abcdef12",
            default_policy(),
            no_facts(),
        );
        assert_eq!(
            result,
            Err(PlanError::Evidence(
                ArtifactEvidenceError::IncompleteEvidence {
                    path: "dev/active/doc.md".into(),
                }
            ))
        );
    }

    #[test]
    fn test_validate_proposed_layout_rejects_broken_relative_edge() {
        let content_identity = identity(b"root");
        let parent = ArtifactPlanEntry::new(
            "fixtures/root.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(content_identity.clone())
        .with_destination("archive/fixtures/root.md")
        .with_edges(vec![edge(
            "target.png",
            "fixtures/target.png",
            EdgeResolutionMode::Relative,
        )])
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/root.md".into(),
            content_identity,
        }]);
        let target = ArtifactPlanEntry::new(
            "fixtures/target.png",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        );
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "fixtures/root.md".into(),
            },
            "archive",
            PolicyStatus::Configured,
            vec![parent, target],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let error = validate_proposed_layout(&plan).expect_err("broken edge must be rejected");
        assert!(matches!(
            error,
            ProposedLayoutViolation::EdgeUnavailable {
                ref reference,
                ref parent,
                ref resolved,
                ..
            } if reference == "target.png"
                && parent == "fixtures/root.md"
                && resolved == "archive/fixtures/target.png"
        ));
    }

    #[test]
    fn test_validate_proposed_layout_accepts_a_relative_edge_into_an_artifact_retained_in_place() {
        // A relocation may not be defeated by a target the plan is not allowed
        // to reproduce. The exemption keys on the evidence naming that reason,
        // so an otherwise identical retained target still rejects the layout.
        let layout = |evidence: Vec<EvidenceCode>| {
            let content_identity = identity(b"plan");
            let parent = ArtifactPlanEntry::new(
                "dev/active/plan.md",
                ArtifactVersion::WorkingTree,
                ArtifactAction::Move,
            )
            .with_content_identity(content_identity.clone())
            .with_destination("dev/archive/abcdef12/active/plan.md")
            .with_edges(vec![edge(
                "../../README.md",
                "README.md",
                EdgeResolutionMode::Relative,
            )])
            .with_pending_deletions(vec![PendingDeletion {
                source: "dev/active/plan.md".into(),
                content_identity,
            }]);
            let target = ArtifactPlanEntry::new(
                "README.md",
                ArtifactVersion::WorkingTree,
                ArtifactAction::Retain,
            )
            .with_evidence(evidence);
            let plan = ArtifactPlan::new(
                PlanTarget::Container {
                    id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
                },
                "dev/archive/abcdef12",
                PolicyStatus::Configured,
                vec![parent, target],
                Vec::new(),
                Vec::new(),
            )
            .unwrap();
            validate_proposed_layout(&plan)
        };

        assert!(layout(vec![EvidenceCode::OutsideDevelopmentRoot]).is_ok());
        assert!(layout(vec![EvidenceCode::PermanentPath]).is_ok());
        assert!(layout(vec![EvidenceCode::ActiveOwner]).is_err());
    }

    #[test]
    fn test_validate_proposed_layout_accepts_relative_edge_with_anchor_fragment() {
        // A relative reference carrying an anchor fragment (e.g. Markdown's
        // `target.png#quick-start`) must resolve against the same location as
        // the equivalent fragment-free reference once both parent and target
        // land at their moved destinations.
        let content_identity = identity(b"root");
        let parent = ArtifactPlanEntry::new(
            "fixtures/root.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(content_identity.clone())
        .with_destination("archive/fixtures/root.md")
        .with_edges(vec![edge(
            "target.png#quick-start",
            "fixtures/target.png",
            EdgeResolutionMode::Relative,
        )])
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/root.md".into(),
            content_identity,
        }]);
        let target_identity = identity(b"target");
        let target = ArtifactPlanEntry::new(
            "fixtures/target.png",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(target_identity.clone())
        .with_destination("archive/fixtures/target.png")
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/target.png".into(),
            content_identity: target_identity,
        }]);
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "fixtures/root.md".into(),
            },
            "archive",
            PolicyStatus::Configured,
            vec![parent, target],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        validate_proposed_layout(&plan).expect("anchor fragment must resolve like the bare path");
    }

    #[test]
    fn test_validate_proposed_layout_accepts_relative_edge_with_query_string() {
        // A relative reference carrying a query string must resolve the same
        // way as the equivalent query-free reference.
        let content_identity = identity(b"root");
        let parent = ArtifactPlanEntry::new(
            "fixtures/root.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(content_identity.clone())
        .with_destination("archive/fixtures/root.md")
        .with_edges(vec![edge(
            "target.png?raw=true",
            "fixtures/target.png",
            EdgeResolutionMode::Relative,
        )])
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/root.md".into(),
            content_identity,
        }]);
        let target_identity = identity(b"target");
        let target = ArtifactPlanEntry::new(
            "fixtures/target.png",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(target_identity.clone())
        .with_destination("archive/fixtures/target.png")
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/target.png".into(),
            content_identity: target_identity,
        }]);
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "fixtures/root.md".into(),
            },
            "archive",
            PolicyStatus::Configured,
            vec![parent, target],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        validate_proposed_layout(&plan).expect("query string must resolve like the bare path");
    }

    #[test]
    fn test_validate_proposed_layout_accepts_root_relative_edge_with_anchor_fragment_and_query_string(
    ) {
        // Root-relative dependencies are retained at their source path rather
        // than moved (see `test_root_relative_edge_retains_dependency_at_source`);
        // an anchor fragment and query string on such a reference must still
        // strip cleanly against that retained location.
        let parent = ArtifactPlanEntry::new(
            "dev/active/page.html",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        )
        .with_edges(vec![edge(
            "/dev/active/theme.css?v=2#top",
            "dev/active/theme.css",
            EdgeResolutionMode::RootRelative,
        )]);
        let target = ArtifactPlanEntry::new(
            "dev/active/theme.css",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        );
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "dev/active/page.html".into(),
            },
            "archive",
            PolicyStatus::Configured,
            vec![parent, target],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        validate_proposed_layout(&plan)
            .expect("root-relative fragment and query string must resolve like the bare path");
    }

    #[test]
    fn test_validate_proposed_layout_rejects_broken_edge_with_anchor_fragment_and_reports_reference_verbatim(
    ) {
        // A path component that genuinely fails to resolve after the move
        // must still fail even when the reference also carries an anchor
        // fragment: fragment-stripping must not paper over a real mismatch.
        // The reported reference must retain the fragment exactly as written.
        let content_identity = identity(b"root");
        let parent = ArtifactPlanEntry::new(
            "fixtures/root.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(content_identity.clone())
        .with_destination("archive/fixtures/root.md")
        .with_edges(vec![edge(
            "target.png#quick-start",
            "fixtures/target.png",
            EdgeResolutionMode::Relative,
        )])
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/root.md".into(),
            content_identity,
        }]);
        let target = ArtifactPlanEntry::new(
            "fixtures/target.png",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        );
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "fixtures/root.md".into(),
            },
            "archive",
            PolicyStatus::Configured,
            vec![parent, target],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let error = validate_proposed_layout(&plan)
            .expect_err("broken path component must still be rejected");
        assert!(matches!(
            error,
            ProposedLayoutViolation::EdgeUnavailable {
                ref reference,
                ref parent,
                ref resolved,
                ..
            } if reference == "target.png#quick-start"
                && parent == "fixtures/root.md"
                && resolved == "archive/fixtures/target.png"
        ));
    }

    fn locations(
        paths: &[(&str, ArtifactLocation, ArtifactLocation)],
    ) -> ArtifactClassificationFacts {
        ArtifactClassificationFacts {
            locations: paths
                .iter()
                .map(|(path, source, destination)| {
                    (
                        (*path).to_string(),
                        ArtifactLocationFacts {
                            source: source.clone(),
                            destination: destination.clone(),
                        },
                    )
                })
                .collect(),
            ..no_facts()
        }
    }

    fn no_facts() -> ArtifactClassificationFacts {
        ArtifactClassificationFacts {
            locations: BTreeMap::new(),
            embedded_owners: Vec::new(),
            container_destination: ContainerDestinationState::Available,
        }
    }

    fn default_policy() -> ArtifactClassificationPolicy {
        ArtifactClassificationPolicy::configured(
            "dev",
            vec!["dev/active".into()],
            vec!["docs".into()],
            "dev/archive",
        )
        .with_issue_scoped_areas(vec!["dev/active".into()])
    }

    fn classify_with_policy(
        policy: ArtifactClassificationPolicy,
        target: PlanTarget,
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        let destination_root = match &target {
            PlanTarget::Container { id } => {
                join_path(&policy.archive_root, &container_short_id(id))
            }
            PlanTarget::Document { .. } => normalize_artifact_path(&policy.archive_root),
        };
        classify_artifacts(
            ArtifactClassificationInventory::new(target, artifacts, Vec::new()),
            &destination_root,
            policy,
            facts,
        )
        .unwrap()
    }

    fn classify(
        target: PlanTarget,
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        classify_with_policy(default_policy(), target, artifacts, facts)
    }

    fn container_with_policy(
        policy: ArtifactClassificationPolicy,
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        classify_with_policy(
            policy,
            PlanTarget::Container {
                id: crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12),
            },
            artifacts,
            facts,
        )
    }

    fn container(
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        container_with_policy(default_policy(), artifacts, facts)
    }

    fn present_source(path: &str) -> ArtifactClassificationFacts {
        locations(&[(
            path,
            ArtifactLocation::Regular(identity(path.as_bytes())),
            ArtifactLocation::Missing,
        )])
    }

    fn selected_root(path: &str) -> Vec<ArtifactPlanEntry> {
        vec![explicit(
            path,
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )]
    }

    fn raises_unmanaged_selected_root(plan: &ArtifactPlan) -> bool {
        plan.blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::UnmanagedSelectedRoot)
    }

    fn entry<'a>(plan: &'a ArtifactPlan, source: &str) -> &'a ArtifactPlanEntry {
        plan.artifacts()
            .iter()
            .find(|entry| entry.source() == source)
            .unwrap()
    }

    #[test]
    fn test_contains_path_uses_components_not_string_prefixes() {
        assert!(contains_path("dev/active", "dev/active/plan.md"));
        assert!(contains_path("dev/active/", "./dev/active"));
        assert!(!contains_path("dev/active", "dev/active-other/plan.md"));
    }

    fn archive_hierarchy() -> HierarchyConfig {
        HierarchyConfig::new(
            HashMap::from([("epic".to_string(), 1)]),
            HashMap::from([("epic".to_string(), "epic".to_string())]),
        )
        .unwrap()
    }

    fn container_issue(title: &str, labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue(title.into(), String::new());
        issue.id = crate::test_utils::issue_id(0x3456_7890_abcd_ef12_3456_7890_abcd_ef12);
        issue.labels = labels.iter().map(|label| (*label).to_string()).collect();
        issue
    }

    fn bare_short_id_root() -> String {
        format!("archive/{}", container_id().short_id())
    }

    #[test]
    fn test_preferred_container_destination_root_uses_the_single_membership_label_value_as_slug() {
        let issue = container_issue(
            "A title that may change",
            &["type:epic", "epic:artifact-archival"],
        );

        assert_eq!(
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive"),
            format!("{}-artifact-archival", bare_short_id_root())
        );
    }

    #[test]
    fn test_preferred_container_destination_root_uses_the_bare_short_id_for_ambiguous_or_absent_membership_labels(
    ) {
        // The fixture title is slug-worthy in every shape below, so a
        // title-derived name stays distinguishable from the bare short id.
        let ambiguous = [
            vec!["type:epic", "type:task", "epic:artifact-archival"],
            vec!["type:epic", "epic:first", "epic:second"],
            vec!["type:epic"],
        ];

        ambiguous.into_iter().for_each(|labels| {
            assert_eq!(
                preferred_container_destination_root(
                    &container_issue("Stable Title Fallback", &labels),
                    &archive_hierarchy(),
                    "archive",
                ),
                bare_short_id_root(),
                "{labels:?} names no single membership value"
            );
        });
    }

    #[test]
    fn test_preferred_container_destination_root_ignores_the_title_for_labelled_and_ambiguous_issues(
    ) {
        let titles = ["Stable Title Fallback", "Entirely Renamed Container"];
        let shapes = [
            vec!["type:epic", "epic:artifact-archival"],
            vec!["type:epic"],
        ];

        shapes.into_iter().for_each(|labels| {
            let roots = titles
                .iter()
                .map(|title| {
                    preferred_container_destination_root(
                        &container_issue(title, &labels),
                        &archive_hierarchy(),
                        "archive",
                    )
                })
                .collect::<Vec<_>>();
            assert!(
                roots.windows(2).all(|pair| pair[0] == pair[1]),
                "renaming the issue must not rename its destination: {roots:?}"
            );
            assert!(
                titles
                    .iter()
                    .flat_map(|title| title.split_whitespace())
                    .all(|word| !roots[0].contains(&word.to_lowercase())),
                "no title word may reach the destination name: {}",
                roots[0]
            );
        });
    }

    #[test]
    fn test_archive_container_slug_is_unicode_safe_bounded_and_absent_without_alphanumerics() {
        assert_eq!(
            archive_container_slug("Résumé Δοκιμή !!!").as_deref(),
            Some("résumé-δοκιμή")
        );
        assert_eq!(
            archive_container_slug("Platform/Archive_V2").as_deref(),
            Some("platform-archive-v2")
        );
        assert_eq!(archive_container_slug("///"), None);
        assert_eq!(archive_container_slug("@/._-/._-"), None);
        assert_eq!(
            archive_container_slug(&"a".repeat(80)).map(|slug| slug.chars().count()),
            Some(48)
        );
        assert!(archive_container_slug(&format!("{}-", "a".repeat(48)))
            .is_some_and(|slug| !slug.ends_with('-')));
    }

    #[test]
    fn test_preferred_container_destination_root_uses_the_bare_short_id_for_a_slugless_membership_value(
    ) {
        // An item-address label value built only from punctuation segments is
        // format-valid yet normalizes to nothing, leaving no slug to carry.
        let issue = container_issue("Stable Title Fallback", &["type:epic", "epic:@/._-/._-"]);

        assert_eq!(
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive"),
            bare_short_id_root()
        );
    }

    #[test]
    fn test_classifier_moves_selected_terminal_container_root() {
        let source = identity(b"plan");
        let plan = container(
            vec![explicit(
                "dev/active/plan.md",
                vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
            )],
            locations(&[(
                "dev/active/plan.md",
                ArtifactLocation::Regular(source.clone()),
                ArtifactLocation::Missing,
            )]),
        );
        let artifact = entry(&plan, "dev/active/plan.md");
        assert_eq!(artifact.action(), ArtifactAction::Move);
        assert_eq!(
            artifact.destination(),
            Some("dev/archive/abcdef12/active/plan.md")
        );
        assert_eq!(artifact.reference_changes().len(), 1);
        assert_eq!(artifact.pending_deletions().len(), 1);
        assert_eq!(artifact.content_identity(), Some(&source));
    }

    #[test]
    fn test_classifier_copies_for_active_or_outside_owner_without_relinking_them() {
        let plan = container(
            vec![explicit(
                "dev/active/shared.md",
                vec![
                    owner(crate::test_utils::issue_id(178), State::Done, true),
                    owner(crate::test_utils::issue_id(179), State::Done, false),
                    owner(crate::test_utils::issue_id(180), State::InProgress, true),
                ],
            )],
            locations(&[(
                "dev/active/shared.md",
                ArtifactLocation::Regular(identity(b"shared")),
                ArtifactLocation::Missing,
            )]),
        );
        let artifact = entry(&plan, "dev/active/shared.md");
        assert_eq!(artifact.action(), ArtifactAction::Copy);
        assert_eq!(artifact.reference_changes().len(), 1);
        assert!(artifact.evidence().contains(&EvidenceCode::OutsideOwner));
        assert!(artifact.evidence().contains(&EvidenceCode::ActiveOwner));
    }

    #[test]
    fn test_relative_edge_to_dependency_copies_when_dependency_needs_source() {
        let parent = explicit(
            "dev/active/page.html",
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )
        .with_edges(vec![edge(
            "../shared/figure.png",
            "dev/shared/figure.png",
            EdgeResolutionMode::Relative,
        )]);
        let facts = locations(&[
            (
                "dev/active/page.html",
                ArtifactLocation::Regular(identity(b"page")),
                ArtifactLocation::Missing,
            ),
            (
                "dev/shared/figure.png",
                ArtifactLocation::Regular(identity(b"figure")),
                ArtifactLocation::Missing,
            ),
        ]);
        let plan = container(vec![parent, embedded("dev/shared/figure.png")], facts);
        let dependency = entry(&plan, "dev/shared/figure.png");
        assert_eq!(dependency.action(), ArtifactAction::Copy);
        assert!(dependency.evidence().contains(&EvidenceCode::UnmanagedPath));
    }

    #[test]
    fn test_relative_edge_moves_managed_dependency_when_nothing_needs_source() {
        let parent = explicit(
            "dev/active/page.html",
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )
        .with_edges(vec![edge(
            "theme.css",
            "dev/active/theme.css",
            EdgeResolutionMode::Relative,
        )]);
        let facts = locations(&[
            (
                "dev/active/page.html",
                ArtifactLocation::Regular(identity(b"page")),
                ArtifactLocation::Missing,
            ),
            (
                "dev/active/theme.css",
                ArtifactLocation::Regular(identity(b"theme")),
                ArtifactLocation::Missing,
            ),
        ]);
        let plan = container(vec![parent, embedded("dev/active/theme.css")], facts);
        let dependency = entry(&plan, "dev/active/theme.css");
        assert_eq!(dependency.action(), ArtifactAction::Move);
        assert_eq!(dependency.pending_deletions().len(), 1);
    }

    #[test]
    fn test_root_relative_edge_retains_dependency_at_source() {
        let parent = explicit(
            "dev/active/page.html",
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )
        .with_edges(vec![edge(
            "/dev/active/theme.css",
            "dev/active/theme.css",
            EdgeResolutionMode::RootRelative,
        )]);
        let facts = locations(&[
            (
                "dev/active/page.html",
                ArtifactLocation::Regular(identity(b"page")),
                ArtifactLocation::Missing,
            ),
            (
                "dev/active/theme.css",
                ArtifactLocation::Regular(identity(b"theme")),
                ArtifactLocation::Missing,
            ),
        ]);
        let plan = container(vec![parent, embedded("dev/active/theme.css")], facts);
        assert_eq!(
            entry(&plan, "dev/active/theme.css").action(),
            ArtifactAction::Retain
        );
    }

    #[test]
    fn test_relative_dependency_under_archive_root_copies_and_retains_source() {
        let parent = explicit(
            "dev/active/page.html",
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )
        .with_edges(vec![edge(
            "../archive/theme.css",
            "dev/archive/theme.css",
            EdgeResolutionMode::Relative,
        )]);
        let facts = locations(&[
            (
                "dev/active/page.html",
                ArtifactLocation::Regular(identity(b"page")),
                ArtifactLocation::Missing,
            ),
            (
                "dev/archive/theme.css",
                ArtifactLocation::Regular(identity(b"theme")),
                ArtifactLocation::Missing,
            ),
        ]);
        let plan = container(vec![parent, embedded("dev/archive/theme.css")], facts);
        let dependency = entry(&plan, "dev/archive/theme.css");
        assert_eq!(dependency.action(), ArtifactAction::Copy);
        assert!(dependency
            .evidence()
            .contains(&EvidenceCode::ArchivedSource));
        assert!(dependency.pending_deletions().is_empty());
    }

    #[test]
    fn test_archive_root_direct_root_retains_without_destination_or_relink() {
        let plan = container(
            vec![explicit(
                "dev/archive/old.md",
                vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
            )],
            locations(&[(
                "dev/archive/old.md",
                ArtifactLocation::Regular(identity(b"old")),
                ArtifactLocation::Missing,
            )]),
        );
        let artifact = entry(&plan, "dev/archive/old.md");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
        assert!(artifact.already_archived());
        assert!(artifact.reference_changes().is_empty());
    }

    #[test]
    fn test_selected_root_inside_development_root_matching_no_area_blocks_the_plan() {
        let plan = container(
            selected_root("dev/scratch/root.md"),
            present_source("dev/scratch/root.md"),
        );
        assert!(raises_unmanaged_selected_root(&plan));
        assert!(!plan.eligible());
        let artifact = entry(&plan, "dev/scratch/root.md");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
    }

    #[test]
    fn test_selected_root_outside_development_root_is_retained_at_its_source_without_a_destination()
    {
        let plan = container(
            selected_root("scripts/install.sh"),
            present_source("scripts/install.sh"),
        );
        assert!(!raises_unmanaged_selected_root(&plan));
        assert!(plan.eligible());
        let artifact = entry(&plan, "scripts/install.sh");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
        assert!(artifact
            .evidence()
            .contains(&EvidenceCode::OutsideDevelopmentRoot));
        assert!(artifact.pending_deletions().is_empty());
    }

    #[test]
    fn test_classify_artifacts_retains_every_selected_root_outside_the_development_root_without_blocking_the_plan(
    ) {
        // Source, script, agent-asset, and repository-root paths all lie
        // outside the configured development root, so each is left where it is
        // and none defeats the plan.
        for source in [
            "crates/jit/src/main.rs",
            "scripts/install-jit.sh",
            ".agents/skills/jit-manage/SKILL.md",
            "README.md",
        ] {
            let plan = container(selected_root(source), present_source(source));
            assert!(
                !raises_unmanaged_selected_root(&plan),
                "{source} raised an unmanaged-selected-root blocker"
            );
            assert!(plan.eligible(), "{source} left the plan ineligible");
            let artifact = entry(&plan, source);
            assert_eq!(artifact.action(), ArtifactAction::Retain, "{source}");
            assert_eq!(artifact.destination(), None, "{source}");
            assert!(artifact.pending_deletions().is_empty(), "{source}");
        }
    }

    #[test]
    fn test_classify_entry_suppresses_the_relink_for_a_retained_owner_outside_the_development_root()
    {
        // The owner alone — unpinned, inside the resolved subtree, terminal —
        // would otherwise earn a relink, but the source lies outside the
        // development root, so classification always retains it (`:805-806`)
        // and writes no destination. The relink and its reference change must
        // not survive a `Retain` (`@/issue/8e071e18/decision/D-14`).
        let plan = container(
            selected_root("scripts/install.sh"),
            present_source("scripts/install.sh"),
        );
        let artifact = entry(&plan, "scripts/install.sh");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert!(artifact.reference_changes().is_empty());
        assert!(artifact
            .owners()
            .iter()
            .all(|owner| !owner.selected_for_relink));
    }

    #[test]
    fn test_classify_entry_suppresses_the_relink_for_a_blocked_owner() {
        // A destination conflict blocks the artifact even though its terminal
        // owner would otherwise earn a relink. `Block`, like `Retain`, writes
        // no destination, so the relink must not survive either.
        let plan = container(
            vec![explicit(
                "dev/active/b/file.md",
                vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
            )],
            locations(&[(
                "dev/active/b/file.md",
                ArtifactLocation::Regular(identity(b"source")),
                ArtifactLocation::Regular(identity(b"different")),
            )]),
        );
        let artifact = entry(&plan, "dev/active/b/file.md");
        assert_eq!(artifact.action(), ArtifactAction::Block);
        assert!(artifact.reference_changes().is_empty());
        assert!(artifact
            .owners()
            .iter()
            .all(|owner| !owner.selected_for_relink));
    }

    #[test]
    fn test_classify_entry_keeps_the_relink_for_a_copied_artifacts_terminal_owner() {
        // A second, still-active owner forces the classifier to keep the
        // source alongside the destination it also needs, so the shared
        // artifact copies rather than moves. `Copy` writes the destination
        // the relink names (REQ-03), so the terminal owner's relink must
        // survive even though the source stays too.
        let plan = container(
            vec![explicit(
                "dev/active/shared.md",
                vec![
                    owner(crate::test_utils::issue_id(181), State::Done, true),
                    owner(crate::test_utils::issue_id(180), State::InProgress, true),
                ],
            )],
            locations(&[(
                "dev/active/shared.md",
                ArtifactLocation::Regular(identity(b"shared")),
                ArtifactLocation::Missing,
            )]),
        );
        let artifact = entry(&plan, "dev/active/shared.md");
        assert_eq!(artifact.action(), ArtifactAction::Copy);
        let mirror = ArchiveLayout::new(
            plan.target(),
            plan.destination_root(),
            &default_policy(),
            &[],
        )
        .destination("dev/active/shared.md")
        .unwrap();
        assert_eq!(
            artifact.reference_changes(),
            &[ReferenceChange {
                issue: crate::test_utils::issue_id(181),
                document_index: 0,
                from_path: "dev/active/shared.md".to_string(),
                to_path: mirror,
            }]
        );
        let terminal_owner = artifact
            .owners()
            .iter()
            .find(|owner| owner.issue == crate::test_utils::issue_id(181))
            .unwrap();
        assert!(terminal_owner.selected_for_relink);
    }

    #[test]
    fn test_permanent_area_retains_its_source_whether_inside_or_outside_the_development_root() {
        let policy = || {
            ArtifactClassificationPolicy::configured(
                "dev",
                vec!["dev/active".into()],
                vec!["dev/vision".into(), "docs".into()],
                "dev/archive",
            )
        };
        for source in ["dev/vision/charter.md", "docs/reference/storage-format.md"] {
            let plan =
                container_with_policy(policy(), selected_root(source), present_source(source));
            let artifact = entry(&plan, source);
            assert_eq!(artifact.action(), ArtifactAction::Retain);
            assert_eq!(artifact.destination(), None);
            assert!(artifact.pending_deletions().is_empty());
            assert!(artifact.reference_changes().is_empty());
            assert!(plan.blockers().is_empty());
        }
    }

    #[test]
    fn test_permanent_area_with_an_existing_differing_destination_raises_no_conflict() {
        // A re-run over an archive that already holds a stale copy.
        let source = "dev/vision/charter.md";
        let policy = ArtifactClassificationPolicy::configured(
            "dev",
            vec!["dev/active".into()],
            vec!["dev/vision".into()],
            "dev/archive",
        );
        let plan = container_with_policy(
            policy,
            selected_root(source),
            locations(&[(
                source,
                ArtifactLocation::Regular(identity(b"live")),
                ArtifactLocation::Regular(identity(b"frozen")),
            )]),
        );
        assert_eq!(entry(&plan, source).action(), ArtifactAction::Retain);
        assert!(plan.blockers().is_empty());
        assert!(entry(&plan, source).blockers().is_empty());
    }

    #[test]
    fn test_selected_root_classification_follows_the_configured_development_root() {
        let source = "workspace/notes/plan.md";
        let inside = container_with_policy(
            ArtifactClassificationPolicy::configured(
                "workspace",
                vec!["workspace/active".into()],
                vec![],
                "workspace/archive",
            ),
            selected_root(source),
            present_source(source),
        );
        assert!(raises_unmanaged_selected_root(&inside));
        assert_eq!(entry(&inside, source).destination(), None);

        let outside = container_with_policy(
            ArtifactClassificationPolicy::configured(
                "dev",
                vec!["dev/active".into()],
                vec![],
                "dev/archive",
            ),
            selected_root(source),
            present_source(source),
        );
        assert!(!raises_unmanaged_selected_root(&outside));
        assert_eq!(entry(&outside, source).action(), ArtifactAction::Retain);
        assert_eq!(entry(&outside, source).destination(), None);
    }

    #[test]
    fn test_classify_artifacts_relocates_the_same_sources_whether_or_not_an_out_of_root_artifact_joins_the_graph(
    ) {
        // A multi-hop chain inside the development root: the selected root
        // links a note and the note links a figure. Admitting a repository-root
        // artifact that the same selected root links must leave every one of
        // those relocations untouched.
        let chain = || {
            vec![
                explicit(
                    "dev/active/plan.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )
                .with_edges(vec![edge(
                    "notes.md",
                    "dev/active/notes.md",
                    EdgeResolutionMode::Relative,
                )]),
                embedded("dev/active/notes.md").with_edges(vec![edge(
                    "figure.png",
                    "dev/active/figure.png",
                    EdgeResolutionMode::Relative,
                )]),
                embedded("dev/active/figure.png"),
            ]
        };
        let chain_facts = || {
            locations(&[
                (
                    "dev/active/plan.md",
                    ArtifactLocation::Regular(identity(b"plan")),
                    ArtifactLocation::Missing,
                ),
                (
                    "dev/active/notes.md",
                    ArtifactLocation::Regular(identity(b"notes")),
                    ArtifactLocation::Missing,
                ),
                (
                    "dev/active/figure.png",
                    ArtifactLocation::Regular(identity(b"figure")),
                    ArtifactLocation::Missing,
                ),
            ])
        };
        let relocating = |plan: &ArtifactPlan| {
            plan.artifacts()
                .iter()
                .filter(|entry| entry.action() == ArtifactAction::Move)
                .map(|entry| entry.source().to_string())
                .collect::<BTreeSet<_>>()
        };

        let without = container(chain(), chain_facts());
        let mut artifacts = chain();
        artifacts[0] = artifacts[0].clone().with_edges(vec![
            edge(
                "notes.md",
                "dev/active/notes.md",
                EdgeResolutionMode::Relative,
            ),
            edge("../../README.md", "README.md", EdgeResolutionMode::Relative),
        ]);
        artifacts.push(embedded("README.md"));
        let mut facts = chain_facts();
        facts.locations.insert(
            "README.md".into(),
            ArtifactLocationFacts {
                source: ArtifactLocation::Regular(identity(b"readme")),
                destination: ArtifactLocation::Missing,
            },
        );
        let with = container(artifacts, facts);

        // The chain relocates whole, and the out-of-root artifact neither joins
        // it nor perturbs it.
        assert!(relocating(&without).contains("dev/active/figure.png"));
        assert_eq!(relocating(&without), relocating(&with));
        assert_eq!(entry(&with, "README.md").action(), ArtifactAction::Retain);
        assert_eq!(entry(&with, "README.md").destination(), None);
    }

    #[test]
    fn test_managed_root_outside_development_root_is_retained_rather_than_relocated() {
        // The development-root boundary admits no area carve-out: a managed
        // root the development root does not contain is still left where it is,
        // so no configured area can schedule a source outside that root for
        // relocation.
        let plan = container_with_policy(
            ArtifactClassificationPolicy::configured(
                "dev",
                vec!["fixtures".into()],
                vec![],
                "archive",
            ),
            selected_root("fixtures/root.md"),
            present_source("fixtures/root.md"),
        );
        let artifact = entry(&plan, "fixtures/root.md");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
        assert!(artifact
            .evidence()
            .contains(&EvidenceCode::OutsideDevelopmentRoot));
        assert!(artifact.pending_deletions().is_empty());
    }

    #[test]
    fn test_unconfigured_development_root_keeps_blocking_every_unmatched_selected_root() {
        let plan = container_with_policy(
            ArtifactClassificationPolicy::configured(
                "",
                vec!["dev/active".into()],
                vec![],
                "dev/archive",
            ),
            selected_root("scripts/install.sh"),
            present_source("scripts/install.sh"),
        );
        assert!(raises_unmanaged_selected_root(&plan));
        assert_eq!(entry(&plan, "scripts/install.sh").destination(), None);
    }

    #[test]
    fn test_repository_embedded_outside_owner_forces_copy() {
        let mut facts = locations(&[(
            "dev/active/figure.png",
            ArtifactLocation::Regular(identity(b"figure")),
            ArtifactLocation::Missing,
        )]);
        facts.embedded_owners.push(EmbeddedArtifactOwner {
            artifact: "dev/active/figure.png".into(),
            root: "dev/active/outside.md".into(),
            issue: crate::test_utils::issue_id(179),
            state: State::Done,
            archived_from: None,
            inside_subtree: false,
        });
        let plan = container(
            vec![explicit(
                "dev/active/figure.png",
                vec![owner(crate::test_utils::issue_id(178), State::Done, true)],
            )],
            facts,
        );
        let artifact = entry(&plan, "dev/active/figure.png");
        assert_eq!(artifact.action(), ArtifactAction::Copy);
        assert!(artifact.evidence().contains(&EvidenceCode::OutsideOwner));
    }

    #[test]
    fn test_destination_identity_adopts_or_conflicts_without_flattening_paths() {
        let same = identity(b"same");
        let plan = container(
            vec![
                explicit(
                    "dev/active/a/file.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                ),
                explicit(
                    "dev/active/b/file.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                ),
            ],
            locations(&[
                (
                    "dev/active/a/file.md",
                    ArtifactLocation::Regular(same.clone()),
                    ArtifactLocation::Regular(same),
                ),
                (
                    "dev/active/b/file.md",
                    ArtifactLocation::Regular(identity(b"source")),
                    ArtifactLocation::Regular(identity(b"different")),
                ),
            ]),
        );
        let adopted = entry(&plan, "dev/active/a/file.md");
        assert!(adopted.already_archived());
        assert_eq!(adopted.action(), ArtifactAction::Move);
        assert_eq!(adopted.pending_deletions().len(), 1);
        let conflict = entry(&plan, "dev/active/b/file.md");
        assert_eq!(conflict.action(), ArtifactAction::Block);
        assert!(conflict
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));
        assert_ne!(adopted.destination(), conflict.destination());
    }

    #[test]
    fn test_missing_root_uses_existing_mirror_or_blocks_when_both_absent() {
        let plan = container(
            vec![
                explicit(
                    "dev/active/adopt.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )
                .with_blockers(vec![PlanBlocker::new(
                    BlockerCode::MissingSource,
                    Some("dev/active/adopt.md"),
                )]),
                explicit(
                    "dev/active/missing.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                ),
            ],
            locations(&[
                (
                    "dev/active/adopt.md",
                    ArtifactLocation::Missing,
                    ArtifactLocation::Regular(identity(b"published")),
                ),
                (
                    "dev/active/missing.md",
                    ArtifactLocation::Missing,
                    ArtifactLocation::Missing,
                ),
            ]),
        );
        assert!(entry(&plan, "dev/active/adopt.md").already_archived());
        assert!(!entry(&plan, "dev/active/adopt.md")
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::MissingSource));
        assert!(entry(&plan, "dev/active/missing.md")
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::MissingSource));
    }

    #[test]
    fn test_missing_embedded_target_adds_no_destination_constraint_or_blocker() {
        let parent = explicit(
            "dev/active/page.md",
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )
        .with_edges(vec![edge(
            "missing.png",
            "dev/active/missing.png",
            EdgeResolutionMode::Relative,
        )])
        .with_plan_warnings(vec![crate::domain::artifact_plan::PlanWarning::new(
            WarningCode::MissingEdgeTarget,
            Some("dev/active/missing.png"),
        )]);
        let plan = container(
            vec![parent, embedded("dev/active/missing.png")],
            locations(&[
                (
                    "dev/active/page.md",
                    ArtifactLocation::Regular(identity(b"page")),
                    ArtifactLocation::Missing,
                ),
                (
                    "dev/active/missing.png",
                    ArtifactLocation::Missing,
                    ArtifactLocation::Missing,
                ),
            ]),
        );
        let missing = entry(&plan, "dev/active/missing.png");
        assert_eq!(missing.action(), ArtifactAction::Retain);
        assert!(missing.blockers().is_empty());
        assert_eq!(missing.destination(), None);
    }

    #[test]
    fn test_document_target_requires_all_direct_and_embedded_owners_terminal() {
        let mut facts = locations(&[(
            "dev/active/doc.md",
            ArtifactLocation::Regular(identity(b"doc")),
            ArtifactLocation::Missing,
        )]);
        facts.embedded_owners.push(EmbeddedArtifactOwner {
            artifact: "dev/active/doc.md".into(),
            root: "dev/active/active-parent.md".into(),
            issue: crate::test_utils::issue_id(180),
            state: State::InProgress,
            archived_from: None,
            inside_subtree: false,
        });
        let plan = classify(
            PlanTarget::Document {
                path: "dev/active/doc.md".into(),
            },
            vec![explicit(
                "dev/active/doc.md",
                vec![owner(crate::test_utils::issue_id(182), State::Done, false)],
            )],
            facts,
        );
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DocumentNonTerminalOwner));
        assert!(entry(&plan, "dev/active/doc.md")
            .reference_changes()
            .is_empty());
    }

    #[test]
    fn test_document_target_ignores_active_embedded_owner_outside_selected_closure() {
        let mut facts = locations(&[(
            "dev/active/doc.md",
            ArtifactLocation::Regular(identity(b"doc")),
            ArtifactLocation::Missing,
        )]);
        facts.embedded_owners.push(EmbeddedArtifactOwner {
            artifact: "dev/active/unrelated.png".into(),
            root: "dev/active/unrelated.md".into(),
            issue: crate::test_utils::issue_id(180),
            state: State::InProgress,
            archived_from: None,
            inside_subtree: false,
        });
        let plan = classify(
            PlanTarget::Document {
                path: "dev/active/doc.md".into(),
            },
            vec![explicit(
                "dev/active/doc.md",
                vec![owner(crate::test_utils::issue_id(182), State::Done, false)],
            )],
            facts,
        );
        assert!(!plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DocumentNonTerminalOwner));
        assert_eq!(
            entry(&plan, "dev/active/doc.md").reference_changes().len(),
            1
        );
        assert_eq!(
            entry(&plan, "dev/active/doc.md").action(),
            ArtifactAction::Move
        );
    }

    #[test]
    fn test_document_target_treats_its_own_embedded_closure_as_selected() {
        let parent = explicit(
            "dev/active/doc.md",
            vec![owner(crate::test_utils::issue_id(182), State::Done, false)],
        )
        .with_edges(vec![edge(
            "figure.png",
            "dev/active/figure.png",
            EdgeResolutionMode::Relative,
        )]);
        let mut facts = locations(&[
            (
                "dev/active/doc.md",
                ArtifactLocation::Regular(identity(b"doc")),
                ArtifactLocation::Missing,
            ),
            (
                "dev/active/figure.png",
                ArtifactLocation::Regular(identity(b"figure")),
                ArtifactLocation::Missing,
            ),
        ]);
        facts.embedded_owners.push(EmbeddedArtifactOwner {
            artifact: "dev/active/figure.png".into(),
            root: "dev/active/doc.md".into(),
            issue: crate::test_utils::issue_id(182),
            state: State::Done,
            archived_from: None,
            inside_subtree: false,
        });
        let plan = classify(
            PlanTarget::Document {
                path: "dev/active/doc.md".into(),
            },
            vec![parent, embedded("dev/active/figure.png")],
            facts,
        );
        assert_eq!(
            entry(&plan, "dev/active/doc.md").action(),
            ArtifactAction::Move
        );
        assert_eq!(
            entry(&plan, "dev/active/figure.png").action(),
            ArtifactAction::Move
        );
    }

    #[test]
    fn test_container_marker_conflicts_and_symlink_paths_block_structurally() {
        let mut facts = locations(&[(
            "dev/active/doc.md",
            ArtifactLocation::Symlink,
            ArtifactLocation::Missing,
        )]);
        facts.container_destination = ContainerDestinationState::Conflict;
        let plan = container(
            vec![explicit(
                "dev/active/doc.md",
                vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
            )],
            facts,
        );
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));
        assert!(entry(&plan, "dev/active/doc.md")
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));
    }

    #[test]
    fn test_container_destination_outcomes_raise_their_own_blockers() {
        let blocker_codes = |state| {
            let mut facts = present_source("dev/active/doc.md");
            facts.container_destination = state;
            container(
                vec![explicit(
                    "dev/active/doc.md",
                    vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
                )],
                facts,
            )
            .blockers()
            .iter()
            .map(|blocker| blocker.code)
            .collect::<Vec<_>>()
        };
        assert!(blocker_codes(ContainerDestinationState::Available).is_empty());
        assert_eq!(
            blocker_codes(ContainerDestinationState::Conflict),
            [BlockerCode::DestinationConflict]
        );
        assert_eq!(
            blocker_codes(ContainerDestinationState::Symlink),
            [BlockerCode::SymlinkArtifact]
        );
    }

    #[test]
    fn test_non_regular_source_and_destination_use_distinct_blockers() {
        let plan = container(
            vec![explicit(
                "dev/active",
                vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
            )],
            locations(&[(
                "dev/active",
                ArtifactLocation::Unsupported,
                ArtifactLocation::Unsupported,
            )]),
        );
        let artifact = entry(&plan, "dev/active");
        assert_eq!(artifact.action(), ArtifactAction::Block);
        assert!(artifact.blockers().iter().any(|blocker| {
            blocker.code == BlockerCode::UnsupportedArtifactType
                && blocker.path.as_deref() == Some("dev/active")
        }));
        assert!(artifact.blockers().iter().any(|blocker| {
            blocker.code == BlockerCode::DestinationConflict
                && blocker.path.as_deref() == Some("dev/archive/abcdef12/active")
        }));
    }

    #[test]
    fn test_markerless_unaccounted_directory_is_target_conflict() {
        let mut facts = locations(&[(
            "dev/active/doc.md",
            ArtifactLocation::Regular(identity(b"doc")),
            ArtifactLocation::Missing,
        )]);
        facts.container_destination = ContainerDestinationState::Conflict;
        let plan = container(
            vec![explicit(
                "dev/active/doc.md",
                vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
            )],
            facts,
        );
        assert!(plan.blockers().iter().any(|blocker| {
            blocker.code == BlockerCode::DestinationConflict
                && blocker.path.as_deref() == Some("dev/archive/abcdef12")
        }));
    }

    #[test]
    fn test_supported_edge_missing_from_inventory_blocks_unpreservable_layout() {
        let parent = explicit(
            "dev/active/page.html",
            vec![owner(crate::test_utils::issue_id(176), State::Done, true)],
        )
        .with_edges(vec![edge(
            "theme.css",
            "dev/active/theme.css",
            EdgeResolutionMode::Relative,
        )]);
        let plan = container(
            vec![parent],
            locations(&[(
                "dev/active/page.html",
                ArtifactLocation::Regular(identity(b"page")),
                ArtifactLocation::Missing,
            )]),
        );
        assert!(entry(&plan, "dev/active/page.html")
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::UnpreservableLayout));
    }

    #[test]
    fn test_pinned_entry_retains_without_working_tree_constraints() {
        let pinned = ArtifactPlanEntry::new(
            "dev/active/history.md",
            ArtifactVersion::pinned("a".repeat(40)).unwrap(),
            ArtifactAction::Retain,
        )
        .with_provenance(vec![ArtifactProvenance::Explicit])
        .with_owners(vec![ArtifactOwner {
            pinned: true,
            ..owner(crate::test_utils::issue_id(176), State::Done, true)
        }]);
        let plan = container(vec![pinned], no_facts());
        let artifact = entry(&plan, "dev/active/history.md");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
        assert!(artifact
            .evidence()
            .contains(&EvidenceCode::PinnedHistorical));
    }
}
