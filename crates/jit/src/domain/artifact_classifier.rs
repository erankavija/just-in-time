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
    PlanWarning, PolicyStatus, ReferenceChange, WarningCode,
};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::Issue;
use crate::domain::State;
use crate::domain::SHORT_ID_LENGTH;
use crate::labels::type_value_of;
use anyhow::{anyhow, bail};
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
/// repository-relative source: a permanent root retains its source beside the
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
    /// Repository-relative destination root.
    pub archive_root: String,
    /// Repository-relative roots whose file text the in-content citation scan reads.
    pub citation_scan_roots: Vec<String>,
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
            archive_root: documentation
                .map(DocumentationConfig::archive_root)
                .unwrap_or_default(),
            citation_scan_roots: documentation
                .map(DocumentationConfig::citation_scan_roots)
                .unwrap_or_default(),
        }
        .normalized()
    }

    /// Construct an explicit configured policy, primarily for embedded callers and tests.
    ///
    /// The policy declares no citation scan universe, so no in-content citation
    /// warning is derived until [`with_citation_scan_roots`](Self::with_citation_scan_roots)
    /// supplies one. Authored configuration reaches the universe through
    /// [`from_documentation`](Self::from_documentation), which carries the
    /// documented fallback.
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
            archive_root: archive_root.into(),
            citation_scan_roots: Vec::new(),
        }
        .normalized()
    }

    /// Declare the repository-relative roots the in-content citation scan reads.
    ///
    /// The roots need not lie under the development root: a citation that a move
    /// breaks lives wherever the repository writes it.
    pub fn with_citation_scan_roots(mut self, roots: Vec<String>) -> Self {
        self.citation_scan_roots = normalized_paths(roots);
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

    /// Whether a configured permanent root mirrors this source while retaining it.
    ///
    /// Permanence is a property of an area the archive wants a frozen snapshot
    /// of while readers keep the live file, so it schedules a destination and
    /// leaves the source in place. It says nothing about the development-root
    /// boundary: a source outside that root is retained by
    /// [`retains_outside_development_root`](Self::retains_outside_development_root)
    /// whether or not a permanent root also claims it.
    fn is_permanent(&self, path: &str) -> bool {
        self.permanent_paths
            .iter()
            .any(|root| contains_path(root, path))
    }

    /// Whether this source lies outside the development root and is therefore
    /// left exactly where it is.
    ///
    /// Such a source — a source file, a script, an agent asset, a
    /// repository-root document — is not the repository's development record,
    /// so an archive neither relocates nor duplicates it: the plan schedules no
    /// destination for it and admits no descendant of it, so one linked
    /// repository-root hub no longer draws the set it cites into the plan
    /// (`@/issue/8e071e18/decision/D-14`).
    ///
    /// The boundary admits no area carve-out. A managed or permanent root the
    /// development root does not contain is retained just the same, so no
    /// configured area can schedule a source outside that root for relocation.
    /// The archive root is the one path this leaves alone, because a source
    /// already beneath it is a published mirror rather than a working-tree
    /// artifact and carries its own already-archived treatment.
    pub fn retains_outside_development_root(&self, path: &str) -> bool {
        !self.is_archived(path) && self.is_outside_development_root(path)
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
        self.archive_root = normalize_artifact_path(&self.archive_root);
        self.citation_scan_roots = normalized_paths(self.citation_scan_roots);
        self
    }
}

/// Filesystem state captured by the storage boundary for one path.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ArtifactLocation {
    /// The caller did not inspect this path. Production planners should avoid this state.
    #[default]
    Unknown,
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    pub issue: String,
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

/// Existing state of a container's short-id destination directory.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ContainerDestinationState {
    /// No directory exists yet.
    #[default]
    Absent,
    /// Its `.jit-container` marker names this exact full container id.
    OwnedByTarget,
    /// Its marker names another full container id.
    OwnedByOther(String),
    /// It has no marker and every existing entry is accounted for by this plan.
    MarkerlessAccounted,
    /// It has no marker and contains at least one unaccounted entry.
    MarkerlessWithUnaccountedEntries,
    /// The directory path or a traversed component is a symbolic link.
    Symlink,
}

/// Text of the files the citation scan universe reached, keyed by normalized
/// repository-relative citing path.
///
/// Acquired at the storage and planning boundary, where the rest of the plan
/// evidence is gathered, and read here to derive in-content citation warnings.
/// A file the scan could not read as UTF-8 text is absent rather than
/// represented, because the scan is advisory and a plan is produced without it.
pub type CitationScanEvidence = BTreeMap<String, String>;

/// All I/O-derived facts consumed by pure classification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArtifactClassificationFacts {
    /// Facts keyed by normalized repository-relative source path.
    pub locations: BTreeMap<String, ArtifactLocationFacts>,
    /// Embedded owners from every issue-linked working-tree document, not just the target.
    pub embedded_owners: Vec<EmbeddedArtifactOwner>,
    /// Marker/occupancy state for a container destination directory.
    pub container_destination: ContainerDestinationState,
    /// Scanned file text backing in-content citation warnings. Empty when the
    /// caller derives facts from a closed evidence snapshot instead of the
    /// planning boundary's scan, which leaves every other classification
    /// outcome unchanged.
    pub citations: CitationScanEvidence,
}

/// Marker-backed destination chosen from a closed evidence snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedContainerDestination {
    /// The frozen marker-backed, legacy, or newly preferred destination root.
    pub destination_root: String,
    /// Every marker-backed root when duplicate ownership makes execution unsafe.
    pub conflicting_roots: Vec<String>,
}

/// Stable failures while resolving a container destination from closed evidence.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArtifactEvidenceError {
    /// A path implied by complete evidence was not itself captured.
    #[error("archive evidence is incomplete for {path}")]
    IncompleteEvidence { path: String },
    /// The archive root was captured with a listing that cannot prove its children.
    #[error("archive evidence for {path} is not an immediate-child listing")]
    WrongListingScope { path: String },
    /// The legacy destination does not identify an archive root.
    #[error("container archive destination has no archive root")]
    MissingArchiveRoot,
}

/// Resolve marker ownership, legacy fallback, and preferred placement from closed evidence.
pub fn resolve_container_destination(
    preferred_root: &str,
    legacy_root: &str,
    container_id: &str,
    evidence: &ArtifactEvidenceMap,
) -> Result<ResolvedContainerDestination, ArtifactEvidenceError> {
    let archive_root = Path::new(legacy_root)
        .parent()
        .ok_or(ArtifactEvidenceError::MissingArchiveRoot)?;
    let archive_root = normalize_artifact_path(&archive_root.to_string_lossy());
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
            let marker = format!("{child}/.jit-container");
            if matches!(
                required_resolution_evidence(evidence, &marker)?,
                ArtifactEvidence::File(owner) if owner.trim_ascii() == container_id.as_bytes()
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
        destination_root: if matches!(
            required_resolution_evidence(evidence, legacy_root)?,
            ArtifactEvidence::Missing
        ) {
            preferred_root
        } else {
            legacy_root
        }
        .to_string(),
        conflicting_roots: Vec::new(),
    })
}

fn required_resolution_evidence<'a>(
    evidence: &'a ArtifactEvidenceMap,
    path: &str,
) -> Result<&'a ArtifactEvidence, ArtifactEvidenceError> {
    evidence
        .get(path)
        .ok_or_else(|| ArtifactEvidenceError::IncompleteEvidence {
            path: path.to_string(),
        })
}

/// Derive source, mirror, and container occupancy facts from closed evidence.
///
/// A closed evidence snapshot carries the paths classification names, not the
/// text of the citation scan universe, so the derived facts declare no citation
/// evidence and the resulting plan reports no in-content citation warning. The
/// planning boundary supplies that evidence separately.
pub fn classification_facts_from_evidence(
    target: &PlanTarget,
    destination_root: &str,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    embedded_owners: Vec<EmbeddedArtifactOwner>,
    evidence: &ArtifactEvidenceMap,
) -> anyhow::Result<ArtifactClassificationFacts> {
    let inspect_destinations = !policy.archive_root.is_empty();
    let locations = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .map(|artifact| {
            let source = artifact.source();
            let destination = artifact_mirror_destination(destination_root, source);
            Ok((
                source.to_string(),
                ArtifactLocationFacts {
                    source: location_from_evidence(source, evidence)?,
                    destination: if inspect_destinations {
                        location_from_evidence(&destination, evidence)?
                    } else {
                        ArtifactLocation::Missing
                    },
                },
            ))
        })
        .collect::<anyhow::Result<BTreeMap<_, _>>>()?;
    let container_destination = match target {
        PlanTarget::Container { id } if inspect_destinations => {
            container_destination_from_evidence(destination_root, id, artifacts, evidence)?
        }
        _ => ContainerDestinationState::Absent,
    };
    Ok(ArtifactClassificationFacts {
        locations,
        embedded_owners,
        container_destination,
        citations: CitationScanEvidence::new(),
    })
}

fn location_from_evidence(
    path: &str,
    evidence: &ArtifactEvidenceMap,
) -> anyhow::Result<ArtifactLocation> {
    Ok(match required_evidence(evidence, path)? {
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
    container_id: &str,
    artifacts: &[ArtifactPlanEntry],
    evidence: &ArtifactEvidenceMap,
) -> anyhow::Result<ContainerDestinationState> {
    let entries = match required_evidence(evidence, destination_root)? {
        ArtifactEvidence::Missing => return Ok(ContainerDestinationState::Absent),
        ArtifactEvidence::Symlink => return Ok(ContainerDestinationState::Symlink),
        ArtifactEvidence::File(_)
        | ArtifactEvidence::Unsupported
        | ArtifactEvidence::InvalidPath => {
            return Ok(ContainerDestinationState::MarkerlessWithUnaccountedEntries)
        }
        ArtifactEvidence::Directory { scope, entries } => {
            if *scope != ArtifactListingScope::RecursiveFiles {
                bail!("archive evidence for {destination_root} is not a recursive listing");
            }
            entries
        }
    };
    let marker_path = format!("{destination_root}/.jit-container");
    match required_evidence(evidence, &marker_path)? {
        ArtifactEvidence::Symlink => return Ok(ContainerDestinationState::Symlink),
        ArtifactEvidence::File(owner) => {
            let owner = std::str::from_utf8(owner.trim_ascii())
                .map_err(|_| anyhow!("container marker is not valid UTF-8: {marker_path}"))?;
            return Ok(if owner == container_id {
                ContainerDestinationState::OwnedByTarget
            } else {
                ContainerDestinationState::OwnedByOther(owner.to_string())
            });
        }
        ArtifactEvidence::Unsupported
        | ArtifactEvidence::Directory { .. }
        | ArtifactEvidence::InvalidPath => {
            return Ok(ContainerDestinationState::MarkerlessWithUnaccountedEntries)
        }
        ArtifactEvidence::Missing => {}
    }
    let accounted = artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
        .map(|artifact| artifact_mirror_destination(destination_root, artifact.source()))
        .collect::<BTreeSet<_>>();
    Ok(if entries.iter().all(|entry| accounted.contains(entry)) {
        ContainerDestinationState::MarkerlessAccounted
    } else {
        ContainerDestinationState::MarkerlessWithUnaccountedEntries
    })
}

fn required_evidence<'a>(
    evidence: &'a ArtifactEvidenceMap,
    path: &str,
) -> anyhow::Result<&'a ArtifactEvidence> {
    evidence
        .get(path)
        .ok_or_else(|| anyhow!("archive evidence is missing for {path}"))
}

/// Inventory data accepted from recursive discovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClassificationInventory {
    pub target: PlanTarget,
    pub artifacts: Vec<ArtifactPlanEntry>,
    pub blockers: Vec<PlanBlocker>,
    destination_root: Option<String>,
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
            destination_root: None,
        }
    }

    /// Canonically ordered discovered artifact entries.
    pub fn artifacts(&self) -> &[ArtifactPlanEntry] {
        &self.artifacts
    }

    /// Use the destination previously resolved by the storage boundary.
    pub fn with_destination_root(mut self, destination_root: impl Into<String>) -> Self {
        self.destination_root = Some(destination_root.into());
        self
    }
}

/// Apply repository ownership, reference selection, and the edge-aware action calculus.
pub fn classify_artifacts(
    inventory: ArtifactClassificationInventory,
    policy: ArtifactClassificationPolicy,
    facts: ArtifactClassificationFacts,
) -> Result<ArtifactPlan, PlanError> {
    let destination_root = inventory
        .destination_root
        .clone()
        .unwrap_or_else(|| artifact_destination_root(&inventory.target, &policy.archive_root));
    let mut plan_blockers = inventory.blockers;
    append_container_destination_blocker(
        &inventory.target,
        &facts.container_destination,
        &destination_root,
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
    let edge_source_constraints = edge_source_constraints(
        &inventory.artifacts,
        &facts.locations,
        &paths,
        &needs_destination,
    );

    let artifacts = inventory
        .artifacts
        .into_iter()
        .map(|entry| {
            classify_entry(
                entry,
                &inventory.target,
                &policy,
                &facts.locations,
                &embedded_owners,
                document_all_terminal,
                &needs_destination,
                &edge_source_constraints,
                &unpreservable_parents,
                &destination_root,
            )
            .map(|entry| cite_moving_path(entry, &facts.citations))
        })
        .collect::<Result<Vec<_>, PlanError>>()?;

    ArtifactPlan::new(
        inventory.target,
        destination_root,
        policy.status,
        artifacts,
        plan_blockers,
        Vec::new(),
    )
}

/// Verify that every supported edge still resolves to an available path after execution.
///
/// An edge into an artifact the plan retains outside the development root
/// imposes no constraint. Such a target is a fixed point the archive may
/// neither relocate nor reproduce (`@/issue/8e071e18/decision/D-14`), so
/// demanding that it stay reachable from every proposed parent location would
/// forbid relocating anything that cites it — the layout cannot be preserved,
/// and the plan already records the retention as
/// [`EvidenceCode::OutsideDevelopmentRoot`]. A relative citation of such a
/// target no longer resolves from the archived copy, which is the cost the
/// decision accepts against duplicating the repository into every archive.
pub fn validate_proposed_layout(
    plan: &ArtifactPlan,
) -> Result<(), crate::repository_state::ProducerError> {
    let by_source = plan
        .artifacts()
        .iter()
        .map(|artifact| (artifact.source(), artifact))
        .collect::<BTreeMap<_, _>>();
    for parent in plan.artifacts() {
        for edge in parent
            .edges()
            .iter()
            .filter(|edge| edge.kind == EdgeKind::Supported)
        {
            let Some(target_source) = edge.target.as_deref() else {
                continue;
            };
            if parent.warnings().iter().any(|warning| {
                warning.code == WarningCode::MissingEdgeTarget
                    && warning.path.as_deref() == Some(target_source)
            }) {
                continue;
            }
            let target = by_source.get(target_source).ok_or_else(|| {
                crate::repository_state::ProducerError::ProposedLayoutTargetAbsent {
                    target: target_source.to_string(),
                }
            })?;
            if target
                .evidence()
                .contains(&EvidenceCode::OutsideDevelopmentRoot)
            {
                continue;
            }
            let available = proposed_available_paths(target);
            // Resolve on the reference's path component only: discovery strips
            // the query string and fragment before computing a target (see
            // `resolve_reference` in `artifact_discovery.rs`), but `available`
            // never carries one either, so matching the raw reference here
            // would reject every anchor- or query-bearing edge.
            let reference_path = edge.reference.split(['?', '#']).next().unwrap_or_default();
            for parent_path in proposed_available_paths(parent) {
                let resolved = match edge.resolution_mode {
                    EdgeResolutionMode::Relative => {
                        let parent_dir = Path::new(&parent_path).parent().unwrap_or(Path::new(""));
                        normalize_artifact_path(&parent_dir.join(reference_path).to_string_lossy())
                    }
                    EdgeResolutionMode::RootRelative => {
                        normalize_artifact_path(reference_path.trim_start_matches('/'))
                    }
                    EdgeResolutionMode::External => continue,
                };
                if !available.contains(&resolved) {
                    return Err(
                        crate::repository_state::ProducerError::ProposedLayoutEdgeUnavailable {
                            reference: edge.reference.clone(),
                            parent: parent.source().to_string(),
                            resolved,
                            available: available.iter().cloned().collect(),
                        },
                    );
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
    edge_source_constraints: &BTreeSet<String>,
    unpreservable_parents: &BTreeSet<String>,
    destination_root: &str,
) -> Result<ArtifactPlanEntry, PlanError> {
    if entry.version().is_pinned() {
        let evidence = merge(entry.evidence(), [EvidenceCode::PinnedHistorical]);
        return Ok(entry
            .with_action(ArtifactAction::Retain)
            .with_evidence(evidence));
    }

    let source = entry.source().to_string();
    let location = locations.get(&source).cloned().unwrap_or_default();
    let archived_source = policy.is_archived(&source);
    let permanent = policy.is_permanent(&source);
    let outside_development_root = policy.retains_outside_development_root(&source);
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

    // A source outside the development root never earns a destination, so the
    // pair below always selects `Retain` for it.
    let wants_destination = needs_destination.contains(&source);
    let needs_source = archived_source
        || permanent
        || outside_development_root
        || outside_owner
        || active_owner
        || unselected_unpinned
        || unmanaged_embedded
        || edge_source_constraints.contains(&source)
        || !wants_destination;
    let mut action = match (wants_destination, needs_source) {
        (true, true) => ArtifactAction::Copy,
        (true, false) => ArtifactAction::Move,
        (false, _) => ArtifactAction::Retain,
    };

    let mirror = artifact_mirror_destination(destination_root, &source);
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
            (ArtifactLocation::Missing, ArtifactLocation::Regular(_)) if explicit => true,
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

    if embedded_missing {
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

/// Warn about every in-content citation the relocation of `entry` would break.
///
/// Only a relocating artifact earns warnings: an artifact whose source stays
/// where it is leaves every citation of that path resolvable, so it contributes
/// none. The warnings carry no action and no blocker, leaving the plan's
/// eligibility untouched.
fn cite_moving_path(
    entry: ArtifactPlanEntry,
    citations: &CitationScanEvidence,
) -> ArtifactPlanEntry {
    if entry.action() != ArtifactAction::Move {
        return entry;
    }
    let warnings = merge(
        entry.warnings(),
        moving_path_citation_warnings(entry.source(), citations),
    );
    entry.with_warnings(warnings)
}

/// One [`WarningCode::MovingPathCitation`] warning per citation of `source` in
/// the text of a scanned file.
///
/// Each warning names the citing file with the citation's 1-based line and
/// column, spelled `<citing path>:<line>:<column>`. The position both locates a
/// citation that structured document edges cannot see — a code comment, an
/// inline code span, a shell-script line — and keeps every citation a distinct
/// warning under the plan's code-and-path warning identity, so repeated
/// citations in one file are each reported.
fn moving_path_citation_warnings(
    source: &str,
    citations: &CitationScanEvidence,
) -> Vec<PlanWarning> {
    citations
        .iter()
        .flat_map(|(citing, text)| {
            text.lines().enumerate().flat_map(move |(index, line)| {
                citation_columns(line, source).map(move |column| {
                    PlanWarning::new(
                        WarningCode::MovingPathCitation,
                        Some(format!("{citing}:{}:{column}", index + 1)),
                    )
                })
            })
        })
        .collect()
}

/// The 1-based character column of every citation of `source` in `line`, in
/// ascending order.
///
/// The column is counted in characters, so text in any script is located where
/// its reader sees it. An empty `source` matches only empty text, which cites
/// nothing.
///
/// The search resumes past the end of each occurrence it examines. That loses
/// no citation: an occurrence starting inside another is preceded by the cited
/// path's own opening characters, which name path content, so
/// [`is_whole_path_citation`] rejects it.
fn citation_columns<'a>(line: &'a str, source: &'a str) -> impl Iterator<Item = usize> + 'a {
    line.match_indices(source)
        .filter(move |(offset, matched)| {
            !matched.is_empty() && is_whole_path_citation(line, *offset, source)
        })
        .map(move |(offset, _)| line[..offset].chars().count() + 1)
}

/// Whether the occurrence of `source` at byte `offset` in `line` cites that path
/// whole, rather than sitting inside a longer one.
///
/// A longer path reaches the occurrence through a directory name, whether it
/// stands above the cited path (`dev/archive/8e071e18/dev/active/plan.md`, the
/// destination every relocation publishes) or merely ends with the cited path's
/// first segment (`mydev/active/plan.md`), and it extends past the occurrence
/// through a longer name (`dev/active/plan.mdx`). Repository filenames can use
/// nearly every non-NUL, non-slash character, so a small allow-list of filename
/// characters cannot reliably find that boundary. Instead, an occurrence is a
/// citation only when prose or code syntax delimits it.
///
/// Punctuation carrying no name leaves the citation whole, which is how the
/// citations prose and code actually write are kept: a quote or bracket around
/// the path, the `-` of a shell default (`${OUT:-dev/active/plan.md}`), a
/// sentence period after it, and a relative `./` or `../` before it, which adds
/// no directory and marks a citation the same relocation breaks. A period is
/// only sentence punctuation when what follows cannot continue a filename.
fn is_whole_path_citation(line: &str, offset: usize, source: &str) -> bool {
    let prefix = &line[..offset];
    let suffix = &line[offset + source.len()..];
    citation_start(prefix).is_some_and(|start| match start {
        CitationStart::Delimited => is_citation_end(suffix),
        CitationStart::ShellDefault => is_shell_default_end(suffix),
    })
}

/// The syntax that introduces a standalone path citation.
#[derive(Clone, Copy)]
enum CitationStart {
    Delimited,
    ShellDefault,
}

/// Whether text before a path finishes with syntax that introduces a citation.
///
/// Relative `./` and `../` prefixes still name the same repository-relative
/// artifact. A shell default is a citation only when its valid parameter name,
/// opening `${`, and closing `}` prove the source occupies the whole default.
fn citation_start(prefix: &str) -> Option<CitationStart> {
    shell_default_start(prefix)
        .then_some(CitationStart::ShellDefault)
        .or_else(|| relative_prefix_start(prefix))
        .or_else(|| {
            (prefix.is_empty() || prefix.chars().last().is_some_and(is_citation_delimiter))
                .then_some(CitationStart::Delimited)
        })
}

/// Whether `prefix` ends with a standalone `${NAME:-` shell-default introducer.
fn shell_default_start(prefix: &str) -> bool {
    prefix.strip_suffix(":-").is_some_and(|before_default| {
        before_default
            .rsplit_once("${")
            .is_some_and(|(before_open, name)| {
                is_shell_variable_name(name)
                    && (before_open.is_empty()
                        || before_open
                            .chars()
                            .last()
                            .is_some_and(is_citation_delimiter))
            })
    })
}

/// Whether `name` is a portable shell variable name.
fn is_shell_variable_name(name: &str) -> bool {
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first == '_' || first.is_ascii_alphabetic())
        && characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
}

/// Whether `suffix` closes a shell default without extending its value.
fn is_shell_default_end(suffix: &str) -> bool {
    suffix.strip_prefix('}').is_some_and(|after_default| {
        !after_default.starts_with('}') && is_citation_end(after_default)
    })
}

/// Whether text after a path starts with syntax that closes a citation.
fn is_citation_end(suffix: &str) -> bool {
    suffix.is_empty()
        || suffix.chars().next().is_some_and(is_citation_delimiter)
        || is_sentence_period(suffix)
}

/// Whether `suffix` begins with a sentence period rather than a filename extension.
fn is_sentence_period(suffix: &str) -> bool {
    suffix.strip_prefix('.').is_some_and(|after_period| {
        after_period.is_empty()
            || after_period
                .chars()
                .next()
                .is_some_and(is_sentence_period_follower)
    })
}

/// Whether `character` can follow sentence-ending punctuation without extending a path.
fn is_sentence_period_follower(character: char) -> bool {
    character.is_whitespace() || matches!(character, '\'' | '"' | '`' | ')' | ']' | '}' | '>')
}

/// Whether `prefix` ends in one or more repository-relative path introducers.
fn relative_prefix_start(prefix: &str) -> Option<CitationStart> {
    ["../", "./"].into_iter().find_map(|relative| {
        prefix.strip_suffix(relative).and_then(|before| {
            (!before.ends_with('.'))
                .then_some(())
                .and_then(|()| citation_start(before))
        })
    })
}

/// Whether `character` delimits a path citation in prose or code.
///
/// This intentionally lists syntax rather than filename characters. A character
/// not listed here is allowed in a repository filename and therefore keeps an
/// adjacent source occurrence inside a longer path.
fn is_citation_delimiter(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '\'' | '"'
                | '`'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | ','
                | ';'
                | '!'
                | '?'
        )
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
            // A root outside the development root is retained where it is: it
            // earns no destination, and its absence from the configured areas
            // is the policy's intent rather than a mistyped area entry.
            if policy.retains_outside_development_root(source) {
                return None;
            }
            let archived = policy.is_archived(source);
            let permanent = policy.is_permanent(source);
            let managed = policy.is_managed(source);
            let selected = match target {
                PlanTarget::Container { .. } => entry.owners().iter().any(|owner| {
                    owner.inside_subtree && owner.is_effectively_terminal() && !owner.pinned
                }),
                PlanTarget::Document { .. } => document_all_terminal,
            };
            if selected && !archived && !permanent && !managed {
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
/// A target the policy retains outside the development root is excluded: it
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
            .filter_map(|(_, edge)| present_target(edge, locations, paths))
            .filter(|target| !policy.retains_outside_development_root(target))
            .filter(|target| !needs_destination.contains(*target))
            .cloned()
            .collect::<BTreeSet<_>>();
        if additional.is_empty() {
            break;
        }
        needs_destination.extend(additional);
    }
}

fn edge_source_constraints(
    artifacts: &[ArtifactPlanEntry],
    locations: &BTreeMap<String, ArtifactLocationFacts>,
    paths: &BTreeSet<String>,
    needs_destination: &BTreeSet<String>,
) -> BTreeSet<String> {
    artifact_edges(artifacts)
        .into_iter()
        .filter(|(parent, edge)| {
            edge.resolution_mode == EdgeResolutionMode::RootRelative
                || !needs_destination.contains(*parent)
        })
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
                            && !entry.warnings().iter().any(|warning| {
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
    let blocked = matches!(
        state,
        ContainerDestinationState::OwnedByOther(_)
            | ContainerDestinationState::MarkerlessWithUnaccountedEntries
    );
    if blocked {
        blockers.push(PlanBlocker::new(
            BlockerCode::DestinationConflict,
            Some(destination_root),
        ));
    }
    if matches!(state, ContainerDestinationState::Symlink) {
        blockers.push(PlanBlocker::new(
            BlockerCode::SymlinkArtifact,
            Some(destination_root),
        ));
    }
}

/// Compute the stable target destination root used by planning and storage inspection.
pub fn artifact_destination_root(target: &PlanTarget, archive_root: &str) -> String {
    match target {
        PlanTarget::Container { id } => join_path(archive_root, &container_short_id(id)),
        PlanTarget::Document { .. } => normalize_artifact_path(archive_root),
    }
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

fn container_short_id(id: &str) -> String {
    id.chars().take(SHORT_ID_LENGTH).collect()
}

/// Mirror one repository-relative source beneath a target destination root.
pub fn artifact_mirror_destination(destination_root: &str, source: &str) -> String {
    join_path(destination_root, source)
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

    const CONTAINER: &str = "abcdef12-3456-7890-abcd-ef1234567890";

    fn identity(bytes: &[u8]) -> ContentIdentity {
        ContentIdentity::from_bytes(bytes)
    }

    fn owner(issue: &str, state: State, inside: bool) -> ArtifactOwner {
        ArtifactOwner {
            issue: issue.to_string(),
            document_index: 0,
            state,
            archived_from: None,
            inside_subtree: inside,
            pinned: false,
            selected_for_relink: false,
        }
    }

    fn archived_owner(issue: &str, archived_from: Option<State>, inside: bool) -> ArtifactOwner {
        ArtifactOwner {
            archived_from,
            ..owner(issue, State::Archived, inside)
        }
    }

    #[test]
    fn test_owner_effective_terminality_preserves_archived_origin() {
        // An owner archived from a terminal state is effectively terminal (its
        // documents can be archived); one archived from a non-terminal state, or
        // a legacy archived owner with no recorded origin, is not — it still
        // counts as an active owner.
        assert!(archived_owner("owner", Some(State::Done), true).is_effectively_terminal());
        assert!(archived_owner("owner", Some(State::Rejected), true).is_effectively_terminal());
        assert!(!archived_owner("owner", Some(State::InProgress), true).is_effectively_terminal());
        assert!(!archived_owner("owner", None, true).is_effectively_terminal());
        assert!(owner("owner", State::Done, true).is_effectively_terminal());
        assert!(!owner("owner", State::InProgress, true).is_effectively_terminal());
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
    fn test_destination_resolution_uses_marker_freeze_legacy_fallback_and_duplicate_evidence() {
        let cases = [
            (
                vec!["archive/abcdef12-old"],
                vec![("archive/abcdef12-old", format!(" \n{CONTAINER}\t"))],
                false,
                "archive/abcdef12-old",
                Vec::new(),
            ),
            (Vec::new(), Vec::new(), true, "archive/abcdef12", Vec::new()),
            (
                vec!["archive/abcdef12-a", "archive/abcdef12-b"],
                vec![
                    ("archive/abcdef12-a", CONTAINER.to_string()),
                    ("archive/abcdef12-b", CONTAINER.to_string()),
                ],
                false,
                "archive/abcdef12-a",
                vec!["archive/abcdef12-a", "archive/abcdef12-b"],
            ),
        ];
        for (children, markers, legacy_exists, expected, conflicts) in cases {
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
                    format!("{root}/.jit-container"),
                    ArtifactEvidence::File(owner.into_bytes()),
                );
            });
            evidence
                .entry("archive/abcdef12".into())
                .or_insert_with(|| {
                    if legacy_exists {
                        directory(ArtifactListingScope::MetadataOnly, &[])
                    } else {
                        ArtifactEvidence::Missing
                    }
                });
            let resolved = resolve_container_destination(
                "archive/abcdef12-new",
                "archive/abcdef12",
                CONTAINER,
                &evidence,
            )
            .unwrap();
            assert_eq!(resolved.destination_root, expected);
            assert_eq!(resolved.conflicting_roots, conflicts);
        }
    }

    #[test]
    fn test_resolve_container_destination_adopts_a_preexisting_identifier_only_directory_without_relocating_it(
    ) {
        let issue = container_issue("Mutable title", &["type:epic", "epic:artifact-archival"]);
        let preferred =
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive");
        let legacy = artifact_destination_root(
            &PlanTarget::Container {
                id: issue.id.clone(),
            },
            "archive",
        );
        assert_ne!(
            preferred, legacy,
            "the precondition needs a preferred name the legacy directory does not already carry"
        );

        // The identifier-only directory exists and carries no ownership marker.
        let occupied = ArtifactEvidenceMap::from([
            (
                "archive".to_string(),
                directory(ArtifactListingScope::ImmediateChildren, &[legacy.as_str()]),
            ),
            (
                legacy.clone(),
                directory(ArtifactListingScope::MetadataOnly, &[]),
            ),
            (
                format!("{legacy}/.jit-container"),
                ArtifactEvidence::Missing,
            ),
        ]);
        let adopted =
            resolve_container_destination(&preferred, &legacy, &issue.id, &occupied).unwrap();
        assert_eq!(adopted.destination_root, legacy);
        assert!(adopted.conflicting_roots.is_empty());
        assert!(
            matches!(
                occupied.get(&adopted.destination_root),
                Some(ArtifactEvidence::Directory { .. })
            ),
            "adoption resolves onto the directory that already exists, so nothing relocates"
        );

        // The same container against an archive root without that directory
        // takes the preferred branch, so the adoption above is the legacy
        // branch rather than a coincidence of the expected name.
        let vacant = ArtifactEvidenceMap::from([
            (
                "archive".to_string(),
                directory(ArtifactListingScope::ImmediateChildren, &[]),
            ),
            (legacy.clone(), ArtifactEvidence::Missing),
        ]);
        assert_eq!(
            resolve_container_destination(&preferred, &legacy, &issue.id, &vacant)
                .unwrap()
                .destination_root,
            preferred
        );
    }

    #[test]
    fn test_destination_resolution_rejects_missing_immediate_child_evidence() {
        let evidence = ArtifactEvidenceMap::from([
            (
                "archive".into(),
                directory(ArtifactListingScope::ImmediateChildren, &["archive/unread"]),
            ),
            ("archive/abcdef12".into(), ArtifactEvidence::Missing),
        ]);
        assert_eq!(
            resolve_container_destination("archive/new", "archive/abcdef12", CONTAINER, &evidence,),
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
            ("archive/abcdef12".into(), ArtifactEvidence::Missing),
        ]);
        assert_eq!(
            resolve_container_destination("archive/new", "archive/abcdef12", CONTAINER, &evidence,),
            Err(ArtifactEvidenceError::IncompleteEvidence {
                path: "archive/unmarked/.jit-container".into(),
            })
        );
    }

    #[test]
    fn test_evidence_listing_scopes_are_not_interchangeable() {
        let resolution = ArtifactEvidenceMap::from([
            (
                "archive".into(),
                directory(ArtifactListingScope::RecursiveFiles, &[]),
            ),
            ("archive/abcdef12".into(), ArtifactEvidence::Missing),
        ]);
        assert!(resolve_container_destination(
            "archive/new",
            "archive/abcdef12",
            CONTAINER,
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
                id: CONTAINER.into()
            },
            "archive",
            &[artifact],
            &ArtifactClassificationPolicy::configured("dev", vec![], vec![], "archive"),
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
                ContainerDestinationState::MarkerlessAccounted,
            ),
            (
                vec!["archive/docs/a.md", "archive/foreign"],
                ContainerDestinationState::MarkerlessWithUnaccountedEntries,
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
                ("archive/.jit-container".into(), ArtifactEvidence::Missing),
            ]);
            let facts = classification_facts_from_evidence(
                &PlanTarget::Container {
                    id: CONTAINER.into(),
                },
                "archive",
                std::slice::from_ref(&artifact),
                &ArtifactClassificationPolicy::configured("dev", vec![], vec![], "archive"),
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
            crate::repository_state::ProducerError::ProposedLayoutEdgeUnavailable {
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
    fn test_validate_proposed_layout_accepts_a_relative_edge_into_an_artifact_retained_outside_the_development_root(
    ) {
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
            .with_destination("dev/archive/abcdef12/dev/active/plan.md")
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
                    id: CONTAINER.into(),
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
        assert!(layout(vec![EvidenceCode::PermanentPath]).is_err());
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
            crate::repository_state::ProducerError::ProposedLayoutEdgeUnavailable {
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
            ..ArtifactClassificationFacts::default()
        }
    }

    fn default_policy() -> ArtifactClassificationPolicy {
        ArtifactClassificationPolicy::configured(
            "dev",
            vec!["dev/active".into()],
            vec!["docs".into()],
            "dev/archive",
        )
    }

    fn classify_with_policy(
        policy: ArtifactClassificationPolicy,
        target: PlanTarget,
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        classify_artifacts(
            ArtifactClassificationInventory::new(target, artifacts, Vec::new()),
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
                id: CONTAINER.into(),
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
        vec![explicit(path, vec![owner("i", State::Done, true)])]
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
        let mut issue = crate::domain::types::fixture_issue(title.into(), String::new());
        issue.id = CONTAINER.into();
        issue.labels = labels.iter().map(|label| (*label).to_string()).collect();
        issue
    }

    fn bare_short_id_root() -> String {
        format!("archive/{}", &CONTAINER[..SHORT_ID_LENGTH])
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
                vec![owner("i", State::Done, true)],
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
            Some("dev/archive/abcdef12/dev/active/plan.md")
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
                    owner("inside", State::Done, true),
                    owner("outside", State::Done, false),
                    owner("active", State::InProgress, true),
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
        let parent = explicit("dev/active/page.html", vec![owner("i", State::Done, true)])
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
        let parent = explicit("dev/active/page.html", vec![owner("i", State::Done, true)])
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
        let parent = explicit("dev/active/page.html", vec![owner("i", State::Done, true)])
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
        let parent = explicit("dev/active/page.html", vec![owner("i", State::Done, true)])
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
                vec![owner("i", State::Done, true)],
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
                vec![owner("i", State::Done, true)],
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
                    owner("terminal", State::Done, true),
                    owner("active", State::InProgress, true),
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
        let mirror = artifact_mirror_destination(plan.destination_root(), "dev/active/shared.md");
        assert_eq!(
            artifact.reference_changes(),
            &[ReferenceChange {
                issue: "terminal".to_string(),
                document_index: 0,
                from_path: "dev/active/shared.md".to_string(),
                to_path: mirror,
            }]
        );
        let terminal_owner = artifact
            .owners()
            .iter()
            .find(|owner| owner.issue == "terminal")
            .unwrap();
        assert!(terminal_owner.selected_for_relink);
    }

    #[test]
    fn test_permanent_area_copies_inside_the_development_root_and_retains_outside_it() {
        // Permanence and the development-root boundary are independent: a
        // permanent area inside the root is mirrored while its source stays,
        // and the same policy leaves a permanent area outside the root alone.
        let policy = || {
            ArtifactClassificationPolicy::configured(
                "dev",
                vec!["dev/active".into()],
                vec!["dev/vision".into(), "docs".into()],
                "dev/archive",
            )
        };
        let inside = "dev/vision/charter.md";
        let plan = container_with_policy(policy(), selected_root(inside), present_source(inside));
        let artifact = entry(&plan, inside);
        assert_eq!(artifact.action(), ArtifactAction::Copy);
        assert_eq!(
            artifact.destination(),
            Some(artifact_mirror_destination(plan.destination_root(), inside).as_str())
        );
        assert!(artifact.evidence().contains(&EvidenceCode::PermanentPath));
        assert!(artifact.pending_deletions().is_empty());

        let outside = "docs/reference/storage-format.md";
        let plan = container_with_policy(policy(), selected_root(outside), present_source(outside));
        let artifact = entry(&plan, outside);
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
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
                explicit("dev/active/plan.md", vec![owner("i", State::Done, true)]).with_edges(
                    vec![edge(
                        "notes.md",
                        "dev/active/notes.md",
                        EdgeResolutionMode::Relative,
                    )],
                ),
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
            issue: "outside".into(),
            state: State::Done,
            archived_from: None,
            inside_subtree: false,
        });
        let plan = container(
            vec![explicit(
                "dev/active/figure.png",
                vec![owner("inside", State::Done, true)],
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
                explicit("dev/active/a/file.md", vec![owner("i", State::Done, true)]),
                explicit("dev/active/b/file.md", vec![owner("i", State::Done, true)]),
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
                explicit("dev/active/adopt.md", vec![owner("i", State::Done, true)]).with_blockers(
                    vec![PlanBlocker::new(
                        BlockerCode::MissingSource,
                        Some("dev/active/adopt.md"),
                    )],
                ),
                explicit("dev/active/missing.md", vec![owner("i", State::Done, true)]),
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
        let parent = explicit("dev/active/page.md", vec![owner("i", State::Done, true)])
            .with_edges(vec![edge(
                "missing.png",
                "dev/active/missing.png",
                EdgeResolutionMode::Relative,
            )])
            .with_warnings(vec![crate::domain::artifact_plan::PlanWarning::new(
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
            issue: "active".into(),
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
                vec![owner("done", State::Done, false)],
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
            issue: "active".into(),
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
                vec![owner("done", State::Done, false)],
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
        let parent = explicit("dev/active/doc.md", vec![owner("done", State::Done, false)])
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
            issue: "done".into(),
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
        facts.container_destination = ContainerDestinationState::OwnedByOther("foreign".into());
        let plan = container(
            vec![explicit(
                "dev/active/doc.md",
                vec![owner("i", State::Done, true)],
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
    fn test_non_regular_source_and_destination_use_distinct_blockers() {
        let plan = container(
            vec![explicit("dev/active", vec![owner("i", State::Done, true)])],
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
                && blocker.path.as_deref() == Some("dev/archive/abcdef12/dev/active")
        }));
    }

    #[test]
    fn test_markerless_unaccounted_directory_is_target_conflict() {
        let mut facts = locations(&[(
            "dev/active/doc.md",
            ArtifactLocation::Regular(identity(b"doc")),
            ArtifactLocation::Missing,
        )]);
        facts.container_destination = ContainerDestinationState::MarkerlessWithUnaccountedEntries;
        let plan = container(
            vec![explicit(
                "dev/active/doc.md",
                vec![owner("i", State::Done, true)],
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
        let parent = explicit("dev/active/page.html", vec![owner("i", State::Done, true)])
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

    /// The path whose relocation the citation fixture below would break.
    const CITED_SOURCE: &str = "dev/active/plan.md";

    /// Scanned file text covering the citation kinds the in-content census in
    /// `dev/archive/8e071e18-dev-artifact-layout/dev/active/8e071e18-investigation.md`
    /// measured: a production Rust module
    /// doc comment, an adopter-facing inline code span, a shell-script comment
    /// beside an executable default, and one file citing the path twice on a
    /// single line. The last entry is one of the census's verified
    /// non-citations, an illustrative JSON sample naming a different path.
    fn citation_fixture() -> [(&'static str, &'static str); 5] {
        [
            (
                "crates/jit/src/storage/claim_coordinator.rs",
                "//! Claim coordination.\n//! See design doc: `dev/active/plan.md` - \"Claim Acquisition\"\n",
            ),
            (
                "docs/tutorials/parallel-work-worktrees.md",
                "## Further reading\n\n- Design document: `dev/active/plan.md`\n",
            ),
            (
                "scripts/benchmark-rust-build.sh",
                "#!/usr/bin/env bash\n# Outputs (default dev/active/plan.md)\nOUT=\"${BENCH_OUT:-dev/active/plan.md}\"\n",
            ),
            (
                "dev/studies/tooling.md",
                "Both `dev/active/plan.md` and `dev/active/plan.md` cover it.\n",
            ),
            (
                "docs/concepts/core-model.md",
                "An illustrative sample: {\"path\": \"dev/design/auth-design.md\"}\n",
            ),
        ]
    }

    fn citations(files: &[(&str, &str)]) -> CitationScanEvidence {
        files
            .iter()
            .map(|(path, text)| ((*path).to_string(), (*text).to_string()))
            .collect()
    }

    fn scanned_plan(owners: Vec<ArtifactOwner>, files: &[(&str, &str)]) -> ArtifactPlan {
        container(
            vec![explicit(CITED_SOURCE, owners)],
            ArtifactClassificationFacts {
                citations: citations(files),
                ..present_source(CITED_SOURCE)
            },
        )
    }

    fn cited_locations(artifact: &ArtifactPlanEntry) -> Vec<(String, usize, usize)> {
        artifact
            .warnings()
            .iter()
            .filter(|warning| warning.code == WarningCode::MovingPathCitation)
            .map(|warning| {
                let location = warning
                    .path
                    .as_deref()
                    .expect("a citation warning names where it was found");
                let mut parts = location.rsplitn(3, ':');
                let column = parts.next().and_then(|part| part.parse().ok());
                let line = parts.next().and_then(|part| part.parse().ok());
                match (parts.next(), line, column) {
                    (Some(citing), Some(line), Some(column)) => (citing.to_string(), line, column),
                    _ => panic!("{location} does not name a citing path with its position"),
                }
            })
            .collect()
    }

    fn scanned_text<'a>(files: &[(&'a str, &'a str)], citing: &str) -> &'a str {
        files
            .iter()
            .find(|(path, _)| *path == citing)
            .map(|(_, text)| *text)
            .unwrap_or_else(|| panic!("{citing} is not one of the scanned files"))
    }

    /// The scanned line a warning points at, with the cited path's own column
    /// range resolved, so a test can inspect what surrounds the occurrence.
    fn cited_line<'a>(files: &[(&'a str, &'a str)], location: &(String, usize, usize)) -> &'a str {
        let (citing, line, _) = location;
        scanned_text(files, citing)
            .lines()
            .nth(line - 1)
            .unwrap_or_else(|| panic!("{citing} has no line {line}"))
    }

    /// Whether the scanned text really cites `cited` where `location` says.
    ///
    /// Reading the named line and skipping the reported column's worth of
    /// characters must land on the cited path, which is what makes a reported
    /// position a citation the reader can follow.
    fn reads_back(files: &[(&str, &str)], location: &(String, usize, usize), cited: &str) -> bool {
        let (_, _, column) = location;
        cited_line(files, location)
            .chars()
            .skip(column - 1)
            .collect::<String>()
            .starts_with(cited)
    }

    #[test]
    fn test_classify_artifacts_warns_once_per_occurrence_of_a_moving_artifact_path_in_scanned_text()
    {
        let files = citation_fixture();
        let plan = scanned_plan(vec![owner("i", State::Done, true)], &files);
        let artifact = entry(&plan, CITED_SOURCE);
        assert_eq!(artifact.action(), ArtifactAction::Move);
        let located = cited_locations(artifact);

        // Every warning points at a real occurrence: reading the file it names
        // from the line and column it names yields the moving artifact's path.
        for location in &located {
            let (citing, line, column) = location;
            assert!(
                reads_back(&files, location, CITED_SOURCE),
                "{citing}:{line}:{column} does not locate a citation"
            );
        }
        // Every citation earns exactly one warning, counted over the fixture
        // text independently of the scan, so a file citing the path twice on one
        // line is reported twice and a file citing another path is silent.
        for (citing, text) in &files {
            assert_eq!(
                located.iter().filter(|(cited, ..)| cited == citing).count(),
                occurrences(text, CITED_SOURCE),
                "{citing} earns one warning per citation"
            );
        }
    }

    /// Every occurrence of `cited` in `text`, whether or not it stands alone.
    ///
    /// The count comes from a candidate start at every character of the text,
    /// which shares no search with the scan under test: the scan skips ahead
    /// through the text as it matches, while this examines each position on its
    /// own and so cannot inherit a skipped position from it. It counts warnings
    /// only for text whose every occurrence of `cited` names that path whole,
    /// which is how the fixtures above cite it.
    fn occurrences(text: &str, cited: &str) -> usize {
        text.char_indices()
            .filter(|(offset, _)| text[*offset..].starts_with(cited))
            .count()
    }

    fn scanned_plan_for(
        policy: ArtifactClassificationPolicy,
        source: &str,
        files: &[(&str, &str)],
    ) -> ArtifactPlan {
        container_with_policy(
            policy,
            vec![explicit(source, vec![owner("i", State::Done, true)])],
            ArtifactClassificationFacts {
                citations: citations(files),
                ..present_source(source)
            },
        )
    }

    /// A container plan relocating several present sources, scanning `files`.
    fn scanned_plan_for_sources(sources: &[&str], files: &[(&str, &str)]) -> ArtifactPlan {
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
                .map(|source| explicit(source, vec![owner("i", State::Done, true)]))
                .collect(),
            ArtifactClassificationFacts {
                citations: citations(files),
                ..locations(&present)
            },
        )
    }

    /// The destination a plan publishes for `source`, read from a plan of that
    /// same artifact, so a fixture cites the path an operator repoints to
    /// instead of a copy of the destination rule.
    fn planned_destination(source: &str) -> String {
        let plan = scanned_plan_for(default_policy(), source, &[]);
        let artifact = entry(&plan, source);
        assert_eq!(artifact.action(), ArtifactAction::Move);
        artifact
            .destination()
            .expect("a moving artifact publishes a destination")
            .to_string()
    }

    #[test]
    fn test_classify_artifacts_reports_no_citation_warning_for_a_citation_repointed_to_the_archived_destination(
    ) {
        let destination = planned_destination(CITED_SOURCE);
        // The shape that makes this more than a spelling difference: a
        // destination is the destination root followed by the source path, so
        // the repointed citation still holds the source path inside it.
        assert!(
            destination.contains(CITED_SOURCE),
            "a destination carries the source path, which is what makes the repointed citation ambiguous"
        );
        let text = format!("Superseded, see `{destination}` for the design.\n");
        let files = [("dev/studies/tooling.md", text.as_str())];

        let plan = scanned_plan_for(default_policy(), CITED_SOURCE, &files);
        let artifact = entry(&plan, CITED_SOURCE);

        assert_eq!(artifact.action(), ArtifactAction::Move);
        assert!(
            cited_locations(artifact).is_empty(),
            "a citation naming the destination is already repointed and leaves the report"
        );
    }

    #[test]
    fn test_classify_artifacts_reports_a_citation_of_only_the_source_path_at_the_column_it_begins()
    {
        let line = format!("See `{CITED_SOURCE}` for the design.");
        let text = format!("{line}\n");
        let files = [("dev/studies/tooling.md", text.as_str())];

        let plan = scanned_plan_for(default_policy(), CITED_SOURCE, &files);
        let located = cited_locations(entry(&plan, CITED_SOURCE));

        // The one citation is reported where the cited path begins, located
        // independently of the scan by examining each character position of the
        // line on its own.
        let begins = line
            .char_indices()
            .position(|(offset, _)| line[offset..].starts_with(CITED_SOURCE))
            .map(|index| index + 1)
            .expect("the fixture must cite the path for the column to mean anything");
        assert_eq!(
            located
                .iter()
                .map(|(_, _, column)| *column)
                .collect::<Vec<_>>(),
            vec![begins]
        );
        assert!(reads_back(&files, &located[0], CITED_SOURCE));
    }

    #[test]
    fn test_classify_artifacts_warns_once_for_a_line_citing_one_moving_source_and_another_moving_destination(
    ) {
        let repointed = "dev/active/design.md";
        let destination = planned_destination(repointed);
        let text = format!("Superseded `{CITED_SOURCE}`, now `{destination}`.\n");
        let files = [("dev/studies/tooling.md", text.as_str())];

        let plan = scanned_plan_for_sources(&[CITED_SOURCE, repointed], &files);

        // Both artifacts relocate, so both read the same line and the reported
        // citation is a property of the text rather than of which artifact the
        // plan happens to carry.
        assert!(plan
            .artifacts()
            .iter()
            .all(|artifact| artifact.action() == ArtifactAction::Move));
        let located = plan
            .artifacts()
            .iter()
            .flat_map(cited_locations)
            .collect::<Vec<_>>();
        assert_eq!(located.len(), 1, "{text} cites one path that still moves");
        assert!(
            reads_back(&files, &located[0], CITED_SOURCE),
            "the reported citation is the one naming a source path"
        );
    }

    /// Whether the one occurrence of the cited path in `line` cites it.
    fn cites(line: &str) -> bool {
        let offset = line
            .find(CITED_SOURCE)
            .expect("the fixture must carry the cited path");
        is_whole_path_citation(line, offset, CITED_SOURCE)
    }

    #[test]
    fn test_is_whole_path_citation_separates_the_cited_path_from_a_longer_path_around_it() {
        // Text naming the path and nothing more. What sits against it carries
        // no name of its own: the delimiters prose and code write, a shell
        // default's `-`, a sentence period, a relative prefix that adds no
        // directory.
        for line in [
            format!("See `{CITED_SOURCE}` for the design."),
            format!("See [the design]({CITED_SOURCE})."),
            format!("OUT=\"${{PLAN_OUT:-{CITED_SOURCE}}}\""),
            format!("The design lives in {CITED_SOURCE}."),
            format!("See ./{CITED_SOURCE} from the repository root."),
            format!("See ../{CITED_SOURCE} from a nested directory."),
        ] {
            assert!(cites(&line), "{line} cites the path");
        }
        // Text whose path continues past the occurrence in either direction.
        for line in [
            format!("Mirrored at `dev/archive/abcdef12/{CITED_SOURCE}`."),
            format!("Vendored at `my{CITED_SOURCE}`."),
            format!("Rendered as `{CITED_SOURCE}x`."),
            format!("Superseded by `{CITED_SOURCE}-old`."),
            format!("Stored at `{CITED_SOURCE}.bak`."),
            format!("Stored at `other.{CITED_SOURCE}`."),
            format!("Stored at `.../{CITED_SOURCE}`."),
        ] {
            assert!(!cites(&line), "{line} names a longer path");
        }
    }

    #[test]
    fn test_is_whole_path_citation_recognizes_only_closed_standalone_shell_defaults() {
        for line in [
            format!("OUT=\"${{PLAN_OUT:-{CITED_SOURCE}}}\""),
            format!("${{_PLAN_OUT9:-{CITED_SOURCE}}}"),
            format!("OUT=\"${{PLAN_OUT:-./{CITED_SOURCE}}}\""),
        ] {
            assert!(cites(&line), "{line} is a shell default naming the path");
        }

        for line in [
            format!("notes:-{CITED_SOURCE}"),
            format!("notes:-./{CITED_SOURCE}"),
            format!("${{9PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN-OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}"),
            format!("prefix${{PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}}}suffix"),
            format!("${{PLAN_OUT:-notes:-{CITED_SOURCE}}}"),
        ] {
            assert!(
                !cites(&line),
                "{line} is not a standalone shell default citation"
            );
        }
    }

    #[test]
    fn test_classify_artifacts_ignores_shell_default_lookalikes_that_do_not_name_the_source() {
        let text = [
            format!("notes:-{CITED_SOURCE}"),
            format!("${{9PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}"),
            format!("prefix${{PLAN_OUT:-{CITED_SOURCE}}}"),
            format!("${{PLAN_OUT:-{CITED_SOURCE}}}suffix"),
        ]
        .join("\n");
        let files = [("scripts/archive.sh", text.as_str())];
        let plan = scanned_plan_for(default_policy(), CITED_SOURCE, &files);

        assert!(
            cited_locations(entry(&plan, CITED_SOURCE)).is_empty(),
            "raw, malformed, and concatenated shell-default lookalikes do not cite the source"
        );
    }

    #[test]
    fn test_is_whole_path_citation_rejects_a_source_inside_a_path_with_non_alphanumeric_names() {
        let line = "See `other+dev/active/design.md` for the alternate design.";
        let source = "dev/active/design.md";
        let offset = line
            .find(source)
            .expect("the longer path must contain the source path");

        assert!(
            !is_whole_path_citation(line, offset, source),
            "a plus belongs to the preceding filename, not citation syntax"
        );
    }

    proptest! {
        #[test]
        fn test_is_whole_path_citation_rejects_path_safe_characters_adjacent_to_the_source(
            before in any::<u8>().prop_map(char::from).prop_filter(
                "a non-delimiter filename character",
                |character| *character != '\0' && *character != '/' && !is_citation_delimiter(*character),
            ),
            after in any::<u8>().prop_map(char::from).prop_filter(
                "a non-delimiter filename character",
                |character| *character != '\0' && *character != '/' && !is_citation_delimiter(*character),
            ),
        ) {
            let prefixed = format!("`{before}{CITED_SOURCE}`");
            let suffixed = format!("`{CITED_SOURCE}{after}continued`");

            prop_assert!(!cites(&prefixed), "{before:?} extends the path before its source");
            prop_assert!(!cites(&suffixed), "{after:?} extends the path after its source");
        }

        #[test]
        fn test_is_whole_path_citation_accepts_syntax_delimiters(
            delimiter in any::<u8>().prop_map(char::from).prop_filter(
                "a citation delimiter",
                |character| is_citation_delimiter(*character),
            ),
        ) {
            let line = format!("{delimiter}{CITED_SOURCE}{delimiter}");

            prop_assert!(cites(&line), "{delimiter:?} delimits a standalone citation");
        }

        #[test]
        fn test_is_whole_path_citation_accepts_a_period_only_before_sentence_context(
            follower in any::<u8>().prop_map(char::from).prop_filter(
                "a sentence-period follower",
                |character| is_sentence_period_follower(*character),
            ),
        ) {
            let line = format!("{CITED_SOURCE}.{follower}");

            prop_assert!(cites(&line), "a period before {follower:?} ends the citation");
        }
    }

    #[test]
    fn test_classify_artifacts_reports_no_citation_warning_for_a_longer_path_ending_with_the_cited_path(
    ) {
        // Every way a longer path can carry the cited path inside it: a root
        // above it, a root above it named outside ASCII, a directory whose name
        // merely ends with its first segment, a longer name past its end, and
        // text chaining the path into itself.
        let cases = [
            (
                CITED_SOURCE,
                format!("Mirrored at `docs/mirror/{CITED_SOURCE}`.\n"),
            ),
            (
                CITED_SOURCE,
                format!("Mirrored at `äänet/{CITED_SOURCE}`.\n"),
            ),
            (CITED_SOURCE, format!("Vendored at `my{CITED_SOURCE}`.\n")),
            (CITED_SOURCE, format!("Stored at `{CITED_SOURCE}.bak`.\n")),
            (CITED_SOURCE, format!("Stored at `other.{CITED_SOURCE}`.\n")),
            (CITED_SOURCE, format!("Rendered as `{CITED_SOURCE}x`.\n")),
            (
                "dev/active/dev",
                "`dev/active/dev/active/dev`\n".to_string(),
            ),
        ];

        for (source, text) in cases {
            assert!(
                text.contains(source),
                "{text} must carry {source} inside it for the case to bite"
            );
            let files = [("dev/studies/tooling.md", text.as_str())];
            let plan = scanned_plan_for(default_policy(), source, &files);
            let artifact = entry(&plan, source);

            assert_eq!(artifact.action(), ArtifactAction::Move);
            assert!(
                cited_locations(artifact).is_empty(),
                "{text} names a longer path and does not cite {source}"
            );
        }
    }

    #[test]
    fn test_classify_artifacts_reports_a_character_column_for_a_citation_among_multibyte_text() {
        // A repository whose roots are not ASCII: the cited path and the prose
        // ahead of it are both multi-byte, so a column counted in bytes would
        // overshoot the one the text reads back at.
        let policy = ArtifactClassificationPolicy::configured(
            "äänet",
            vec!["äänet/käynnissä".into()],
            vec!["docs".into()],
            "äänet/arkisto",
        );
        let source = "äänet/käynnissä/ääni.md";
        let files = [(
            "äänet/tutkimus/muistio.md",
            "# Ääni\n\nÄänitys: `äänet/käynnissä/ääni.md` on päivitetty.\n",
        )];
        let plan = scanned_plan_for(policy, source, &files);
        let artifact = entry(&plan, source);
        assert_eq!(artifact.action(), ArtifactAction::Move);
        let located = cited_locations(artifact);

        assert_eq!(located.len(), occurrences(files[0].1, source));
        assert!(located
            .iter()
            .all(|location| reads_back(&files, location, source)));
    }

    #[test]
    fn test_classify_artifacts_reports_citations_in_code_comments_inline_code_spans_and_shell_scripts(
    ) {
        let files = citation_fixture();
        let plan = scanned_plan(vec![owner("i", State::Done, true)], &files);
        let located = cited_locations(entry(&plan, CITED_SOURCE));
        let reported = |citing: &str| {
            located
                .iter()
                .filter(|(cited, ..)| cited == citing)
                .collect::<Vec<_>>()
        };

        // A production module doc comment, which no document edge records.
        let comment = reported("crates/jit/src/storage/claim_coordinator.rs");
        assert!(comment
            .iter()
            .all(|location| cited_line(&files, location).trim_start().starts_with("//!")));
        assert_eq!(comment.len(), 1);

        // An adopter-facing citation inside a markdown inline code span.
        let span = reported("docs/tutorials/parallel-work-worktrees.md");
        assert!(span.iter().all(|location| {
            let (_, _, column) = location;
            let line = cited_line(&files, location);
            let before = line.chars().take(column - 1).collect::<String>();
            let after = line.chars().skip(column - 1 + CITED_SOURCE.chars().count());
            before.ends_with('`') && after.clone().next() == Some('`')
        }));
        assert_eq!(span.len(), 1);

        // A shell script, in a comment and in an executable default alike.
        let script = reported("scripts/benchmark-rust-build.sh");
        let commented = script
            .iter()
            .filter(|location| cited_line(&files, location).trim_start().starts_with('#'))
            .count();
        assert_eq!(commented, 1);
        assert_eq!(script.len() - commented, 1);

        // A verified non-citation naming a different path stays silent.
        assert!(reported("docs/concepts/core-model.md").is_empty());
    }

    #[test]
    fn test_classify_artifacts_reports_no_citation_warning_for_an_artifact_the_plan_leaves_in_place(
    ) {
        // One citing file and one artifact path across every arm, so the
        // classified action is the only difference: the relocating arm is the
        // control proving the fixture really cites the path, and the arms whose
        // source stays where the citation points report nothing.
        let files = [("dev/studies/tooling.md", "See `dev/active/plan.md`.\n")];
        let arms = [
            (
                vec![owner("inside", State::Done, true)],
                ArtifactAction::Move,
            ),
            (
                vec![
                    owner("inside", State::Done, true),
                    owner("outside", State::Done, false),
                ],
                ArtifactAction::Copy,
            ),
            (
                vec![owner("active", State::InProgress, true)],
                ArtifactAction::Retain,
            ),
        ];

        for (owners, expected) in arms {
            let plan = scanned_plan(owners, &files);
            let artifact = entry(&plan, CITED_SOURCE);
            assert_eq!(artifact.action(), expected);
            assert_eq!(
                cited_locations(artifact).is_empty(),
                expected != ArtifactAction::Move,
                "a {expected:?} artifact reports citations only when its source relocates"
            );
        }
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
            ..owner("i", State::Done, true)
        }]);
        let plan = container(vec![pinned], ArtifactClassificationFacts::default());
        let artifact = entry(&plan, "dev/active/history.md");
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
        assert!(artifact
            .evidence()
            .contains(&EvidenceCode::PinnedHistorical));
    }
}
