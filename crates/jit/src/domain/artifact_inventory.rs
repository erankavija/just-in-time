//! Deterministic inventory of issue-linked artifact roots.
//!
//! This module establishes explicit roots and their repository-wide direct
//! owners before embedded-edge discovery or action classification. Container
//! membership follows the resolved hierarchy's `children` relation, never a
//! raw dependency closure. Git and filesystem access enter through
//! [`PinnedRootResolver`], keeping hierarchy, grouping, and owner association
//! deterministic and independently testable.

use crate::domain::artifact_plan::{
    normalize_artifact_path, ArtifactAction, ArtifactPlanEntry, ArtifactProvenance,
    ArtifactVersion, BlockerCode, PlanBlocker, PlanError, PlanTarget, PlanWarning, WarningCode,
};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{Issue, State};
use crate::graph::hierarchy::resolve_hierarchy;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// One explicit-root inventory target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitRootTarget<'a> {
    /// A container already resolved to its full durable issue id.
    Container(&'a str),
    /// An arbitrary repository-relative working-tree path.
    Document(&'a str),
}

/// Storage boundary used to validate and canonicalize pinned document roots.
///
/// Implementations must resolve `revision` to a canonical full commit OID and
/// successfully read `path` at that commit. Returning success without the read
/// would weaken the inventory's `pinned-read-failed` guarantee.
pub trait PinnedRootResolver {
    type Error;

    /// Resolve the revision, read the historical root, and return its canonical
    /// pinned version. No working-tree fallback is permitted.
    fn resolve_and_read(&self, revision: &str, path: &str) -> Result<ArtifactVersion, Self::Error>;
}

/// Versioned explicit roots and target-level failures ready to feed a plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitRootInventory {
    target: PlanTarget,
    member_ids: Vec<String>,
    artifacts: Vec<ArtifactPlanEntry>,
    blockers: Vec<PlanBlocker>,
}

impl ExplicitRootInventory {
    /// Normalized plan target represented by this inventory.
    pub fn target(&self) -> &PlanTarget {
        &self.target
    }

    /// Canonically ordered resolved-hierarchy members, including the root.
    pub fn member_ids(&self) -> &[String] {
        &self.member_ids
    }

    /// Canonically ordered `(path, version)` explicit-root entries.
    pub fn artifacts(&self) -> &[ArtifactPlanEntry] {
        &self.artifacts
    }

    /// Target-level pinned-resolution or pinned-read failures.
    pub fn blockers(&self) -> &[PlanBlocker] {
        &self.blockers
    }

    /// Consume the inventory into fields accepted directly by
    /// [`crate::domain::artifact_plan::ArtifactPlan::new`].
    pub fn into_plan_parts(self) -> (PlanTarget, Vec<ArtifactPlanEntry>, Vec<PlanBlocker>) {
        (self.target, self.artifacts, self.blockers)
    }
}

/// Failures in deterministic inventory construction itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryError {
    /// The supplied full container id is absent from the repository-wide input.
    ContainerNotFound(String),
    /// A resolver claimed success with a working-tree or malformed version.
    NonPinnedResolution { revision: String, path: String },
    /// The produced entry violated the artifact-plan model.
    InvalidPlanEntry(PlanError),
}

impl fmt::Display for InventoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContainerNotFound(id) => write!(formatter, "container not found: {id}"),
            Self::NonPinnedResolution { revision, path } => write!(
                formatter,
                "pinned resolver returned a non-pinned version for {revision} at {path}"
            ),
            Self::InvalidPlanEntry(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for InventoryError {}

impl From<PlanError> for InventoryError {
    fn from(error: PlanError) -> Self {
        Self::InvalidPlanEntry(error)
    }
}

#[derive(Debug, Clone)]
struct DirectOwner {
    issue: String,
    document_index: usize,
    state: State,
    inside_subtree: bool,
}

#[derive(Debug, Clone)]
enum CachedResolution {
    Resolved(ArtifactVersion),
    Failed,
}

/// Inventory explicit roots for a container or arbitrary document target.
///
/// `issues` must be the repository-wide issue set. That single universe is
/// used both to resolve hierarchy membership and to associate every direct
/// `DocumentReference` owner. Pinned references are included only after
/// `resolver` both canonicalizes and reads them; failures add a
/// `pinned-read-failed` blocker and never attach the owner to a working-tree
/// entry.
pub fn inventory_explicit_roots<R: PinnedRootResolver>(
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    target: ExplicitRootTarget<'_>,
    resolver: &R,
) -> Result<ExplicitRootInventory, InventoryError> {
    let (plan_target, member_ids, explicit_paths, force_working_tree) = match target {
        ExplicitRootTarget::Container(root) => {
            if !issues.iter().any(|issue| issue.id == root) {
                return Err(InventoryError::ContainerNotFound(root.to_string()));
            }
            let members = resolved_subtree_members(issues, hierarchy, root);
            let paths = issues
                .iter()
                .filter(|issue| members.contains(&issue.id))
                .flat_map(|issue| issue.documents.iter())
                .map(|document| normalize_artifact_path(&document.path))
                .collect();
            (
                PlanTarget::Container {
                    id: root.to_string(),
                },
                members,
                paths,
                false,
            )
        }
        ExplicitRootTarget::Document(path) => {
            let path = normalize_artifact_path(path);
            (
                PlanTarget::Document { path: path.clone() },
                BTreeSet::new(),
                BTreeSet::from([path]),
                true,
            )
        }
    };

    let mut owners_by_identity: BTreeMap<(String, ArtifactVersion), Vec<DirectOwner>> =
        BTreeMap::new();
    let mut matching_reference_count: BTreeMap<String, usize> = explicit_paths
        .iter()
        .cloned()
        .map(|path| (path, 0))
        .collect();
    let mut blockers = Vec::new();
    let mut resolution_cache: BTreeMap<(String, String), CachedResolution> = BTreeMap::new();

    if force_working_tree {
        explicit_paths.iter().for_each(|path| {
            owners_by_identity
                .entry((path.clone(), ArtifactVersion::WorkingTree))
                .or_default();
        });
    }

    let mut repository_issues = issues.iter().collect::<Vec<_>>();
    repository_issues.sort_by(|left, right| left.id.cmp(&right.id));
    for issue in repository_issues {
        for (document_index, document) in issue.documents.iter().enumerate() {
            let path = normalize_artifact_path(&document.path);
            if !explicit_paths.contains(&path) {
                continue;
            }
            *matching_reference_count.entry(path.clone()).or_default() += 1;
            let owner = DirectOwner {
                issue: issue.id.clone(),
                document_index,
                state: issue.state,
                inside_subtree: member_ids.contains(&issue.id),
            };

            match document.commit.as_deref() {
                None => owners_by_identity
                    .entry((path, ArtifactVersion::WorkingTree))
                    .or_default()
                    .push(owner),
                Some(revision) => {
                    let cache_key = (revision.to_string(), path.clone());
                    let resolved = resolution_cache.entry(cache_key).or_insert_with(|| {
                        match resolver.resolve_and_read(revision, &path) {
                            Ok(version) => CachedResolution::Resolved(version),
                            Err(_) => CachedResolution::Failed,
                        }
                    });
                    match resolved {
                        CachedResolution::Resolved(version) if version.is_pinned() => {
                            owners_by_identity
                                .entry((path, version.clone()))
                                .or_default()
                                .push(owner);
                        }
                        CachedResolution::Resolved(_) => {
                            return Err(InventoryError::NonPinnedResolution {
                                revision: revision.to_string(),
                                path,
                            });
                        }
                        CachedResolution::Failed => blockers
                            .push(PlanBlocker::new(BlockerCode::PinnedReadFailed, Some(path))),
                    }
                }
            }
        }
    }

    let mut artifacts = owners_by_identity
        .into_iter()
        .map(|((path, version), owners)| {
            let no_owner = matching_reference_count.get(&path).copied().unwrap_or(0) == 0;
            let owners = owners
                .into_iter()
                .map(|owner| crate::domain::artifact_plan::ArtifactOwner {
                    issue: owner.issue,
                    document_index: owner.document_index,
                    state: owner.state,
                    inside_subtree: owner.inside_subtree,
                    pinned: version.is_pinned(),
                    selected_for_relink: false,
                })
                .collect();
            let warnings = no_owner
                .then(|| PlanWarning::new(WarningCode::NoOwner, Some(path.clone())))
                .into_iter()
                .collect();
            let mut entry = ArtifactPlanEntry::new(path, version, ArtifactAction::Retain)
                .with_provenance(vec![ArtifactProvenance::Explicit])
                .with_owners(owners)
                .with_warnings(warnings);
            entry.normalize()?;
            Ok(entry)
        })
        .collect::<Result<Vec<_>, InventoryError>>()?;
    artifacts.sort_by_key(ArtifactPlanEntry::identity);

    blockers.sort_by(|left, right| {
        (left.code.as_str(), left.path.as_deref())
            .cmp(&(right.code.as_str(), right.path.as_deref()))
    });
    blockers.dedup();

    Ok(ExplicitRootInventory {
        target: plan_target,
        member_ids: member_ids.into_iter().collect(),
        artifacts,
        blockers,
    })
}

fn resolved_subtree_members(
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    root: &str,
) -> BTreeSet<String> {
    let issue_refs = issues.iter().collect::<Vec<_>>();
    let resolution = resolve_hierarchy(&issue_refs, hierarchy);
    let mut members = BTreeSet::new();
    let mut pending = vec![root.to_string()];

    while let Some(id) = pending.pop() {
        if !members.insert(id.clone()) {
            continue;
        }
        pending.extend(resolution.children(&id).iter().cloned());
    }
    members
}
