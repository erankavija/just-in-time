//! Pure edge-aware classification for dependency-aware artifact archival.
//!
//! Callers inspect the repository and supply immutable location and ownership
//! facts. This module applies policy, per-reference selection, and the
//! destination/source constraint calculus without performing I/O.

use crate::config::DocumentationConfig;
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArtifactAction, ArtifactEdge, ArtifactOwner, ArtifactPlan,
    ArtifactPlanEntry, ArtifactProvenance, ArtifactVersion, BlockerCode, ContentIdentity, EdgeKind,
    EdgeResolutionMode, EvidenceCode, PendingDeletion, PlanBlocker, PlanError, PlanTarget,
    PolicyStatus, ReferenceChange, WarningCode,
};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::Issue;
use crate::domain::State;
use crate::domain::SHORT_ID_LENGTH;
use crate::labels::type_value_of;
use std::collections::{BTreeMap, BTreeSet};

/// Explicit documentation policy used by the classifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactClassificationPolicy {
    /// Whether every mutation-authorizing policy field was explicitly set.
    pub status: PolicyStatus,
    /// Repository-relative roots eligible for archival.
    pub managed_paths: Vec<String>,
    /// Repository-relative roots whose sources must remain available.
    pub permanent_paths: Vec<String>,
    /// Repository-relative destination root.
    pub archive_root: String,
}

impl ArtifactClassificationPolicy {
    /// Build policy from authored configuration while retaining its explicitness status.
    pub fn from_documentation(documentation: Option<&DocumentationConfig>) -> Self {
        Self {
            status: PolicyStatus::from_documentation(documentation),
            managed_paths: documentation
                .map(DocumentationConfig::managed_paths)
                .unwrap_or_default(),
            permanent_paths: documentation
                .map(DocumentationConfig::permanent_paths)
                .unwrap_or_default(),
            archive_root: documentation
                .map(DocumentationConfig::archive_root)
                .unwrap_or_default(),
        }
        .normalized()
    }

    /// Construct an explicit configured policy, primarily for embedded callers and tests.
    pub fn configured(
        managed_paths: Vec<String>,
        permanent_paths: Vec<String>,
        archive_root: impl Into<String>,
    ) -> Self {
        Self {
            status: PolicyStatus::Configured,
            managed_paths,
            permanent_paths,
            archive_root: archive_root.into(),
        }
        .normalized()
    }

    fn normalized(mut self) -> Self {
        self.managed_paths = normalized_paths(self.managed_paths);
        self.permanent_paths = normalized_paths(self.permanent_paths);
        self.archive_root = normalize_artifact_path(&self.archive_root);
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

/// All I/O-derived facts consumed by pure classification.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArtifactClassificationFacts {
    /// Facts keyed by normalized repository-relative source path.
    pub locations: BTreeMap<String, ArtifactLocationFacts>,
    /// Embedded owners from every issue-linked working-tree document, not just the target.
    pub embedded_owners: Vec<EmbeddedArtifactOwner>,
    /// Marker/occupancy state for a container destination directory.
    pub container_destination: ContainerDestinationState,
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
    let archived_source = contains_path(&policy.archive_root, &source);
    let permanent = policy
        .permanent_paths
        .iter()
        .any(|root| contains_path(root, &source));
    let managed = policy
        .managed_paths
        .iter()
        .any(|root| contains_path(root, &source));
    let explicit = entry.provenance().contains(&ArtifactProvenance::Explicit);
    let embedded = entry.provenance().contains(&ArtifactProvenance::Embedded);

    let owners = select_direct_owners(
        entry.owners(),
        target,
        document_all_terminal,
        archived_source,
    );
    let selected_owners = owners
        .iter()
        .filter(|owner| owner.selected_for_relink)
        .collect::<Vec<_>>();
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
    let unmanaged_embedded = embedded && !managed && !permanent && !archived_source;

    let mut evidence = entry.evidence().to_vec();
    evidence.extend(
        [
            (permanent, EvidenceCode::PermanentPath),
            (outside_owner, EvidenceCode::OutsideOwner),
            (active_owner, EvidenceCode::ActiveOwner),
            (unmanaged_embedded, EvidenceCode::UnmanagedPath),
            (archived_source, EvidenceCode::ArchivedSource),
        ]
        .into_iter()
        .filter_map(|(present, evidence)| present.then_some(evidence)),
    );

    let wants_destination = needs_destination.contains(&source);
    let needs_source = archived_source
        || permanent
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

    let reference_changes = if archived_source && explicit {
        Vec::new()
    } else {
        selected_owners
            .into_iter()
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
            let archived = contains_path(&policy.archive_root, source);
            let permanent = policy
                .permanent_paths
                .iter()
                .any(|root| contains_path(root, source));
            let managed = policy
                .managed_paths
                .iter()
                .any(|root| contains_path(root, source));
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

fn propagate_relative_destinations(
    artifacts: &[ArtifactPlanEntry],
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
/// An unambiguous label in the issue type's configured membership namespace
/// takes precedence. Missing or ambiguous type/membership labels fall back to
/// the title, while the short id remains the authoritative collision-resistant
/// prefix.
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
    let slug = archive_container_slug(strategic_value.unwrap_or(&issue.title));
    join_path(
        archive_root,
        &format!("{}-{slug}", container_short_id(&issue.id)),
    )
}

/// Normalize user-authored label or title text into one bounded path component.
pub fn archive_container_slug(value: &str) -> String {
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
    if bounded.is_empty() {
        "container".to_string()
    } else {
        bounded.to_string()
    }
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
    use crate::domain::Issue;
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

    fn classify(
        target: PlanTarget,
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        classify_artifacts(
            ArtifactClassificationInventory::new(target, artifacts, Vec::new()),
            ArtifactClassificationPolicy::configured(
                vec!["dev/active".into()],
                vec!["docs".into()],
                "dev/archive",
            ),
            facts,
        )
        .unwrap()
    }

    fn container(
        artifacts: Vec<ArtifactPlanEntry>,
        facts: ArtifactClassificationFacts,
    ) -> ArtifactPlan {
        classify(
            PlanTarget::Container {
                id: CONTAINER.into(),
            },
            artifacts,
            facts,
        )
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

    #[test]
    fn test_preferred_container_destination_uses_unambiguous_strategic_label_slug() {
        let mut issue = Issue::new("A title that may change".into(), String::new());
        issue.id = CONTAINER.into();
        issue.labels = vec!["type:epic".into(), "epic:artifact-archival".into()];

        assert_eq!(
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive"),
            "archive/abcdef12-artifact-archival"
        );
    }

    #[test]
    fn test_preferred_container_destination_falls_back_to_title_for_missing_or_ambiguous_label() {
        let mut issue = Issue::new("Stable Title Fallback".into(), String::new());
        issue.id = CONTAINER.into();
        issue.labels = vec!["type:epic".into()];

        assert_eq!(
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive"),
            "archive/abcdef12-stable-title-fallback"
        );

        issue.labels = vec![
            "type:epic".into(),
            "epic:first".into(),
            "epic:second".into(),
        ];

        assert_eq!(
            preferred_container_destination_root(&issue, &archive_hierarchy(), "archive"),
            "archive/abcdef12-stable-title-fallback"
        );
    }

    #[test]
    fn test_archive_container_slug_is_unicode_safe_bounded_and_nonempty() {
        assert_eq!(archive_container_slug("Résumé Δοκιμή !!!"), "résumé-δοκιμή");
        assert_eq!(
            archive_container_slug("Platform/Archive_V2"),
            "platform-archive-v2"
        );
        assert_eq!(archive_container_slug("///"), "container");
        assert_eq!(archive_container_slug(&"a".repeat(80)).chars().count(), 48);
        assert!(!archive_container_slug(&format!("{}-", "a".repeat(48))).ends_with('-'));
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
    fn test_unmanaged_root_blocks_but_unmanaged_embedded_never_moves() {
        let plan = container(
            vec![explicit(
                "misc/root.md",
                vec![owner("i", State::Done, true)],
            )],
            locations(&[(
                "misc/root.md",
                ArtifactLocation::Regular(identity(b"root")),
                ArtifactLocation::Missing,
            )]),
        );
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::UnmanagedSelectedRoot));
        assert!(!plan.eligible());
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
