//! Unified dependency-aware archive planning and coordinated execution.

use super::CommandExecutor;
use super::{capture_or_retry, with_mutation_session, SessionStep};
use crate::domain::artifact_classifier::{
    artifact_destination_root, classification_facts_from_evidence, classify_artifacts,
    preferred_container_destination_root,
    resolve_container_destination as derive_container_destination, ArchiveLayout,
    ArtifactClassificationInventory, ArtifactClassificationPolicy, ArtifactLocation,
    ArtifactLocationFacts,
};
use crate::domain::artifact_discovery::{
    discover_archive_artifacts as derive_archive_artifacts, expand_artifact_closure,
    recorded_archived_copies, ArtifactClosure, ArtifactClosureState, ArtifactEvidence,
    ArtifactEvidenceMap, ArtifactListingScope,
};
use crate::domain::artifact_execution::{
    ArchiveEvidenceRecord, ArchiveExecution, ArchiveExecutionResult,
};
use crate::domain::artifact_inventory::{
    inventory_explicit_roots, member_derived_documents, member_document_directories,
    pinned_root_requests, ExplicitRootTarget, PinnedRootEvidence, PinnedRootEvidenceMap,
};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArchiveCandidates, ArtifactPlan, BlockerCode, PlanBlocker, PlanTarget,
};
#[cfg(test)]
use crate::domain::artifact_plan::{ArtifactAction, WarningCode};
use crate::domain::identity::IssueId;
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::Issue;
use crate::storage::artifact_planning::inspect_artifact_evidence;
use crate::storage::{
    collect_artifact_classification_facts, discover_archive_artifacts,
    resolve_container_destination, validate_repo_relative_path, GitRevisionResolver, IssueStore,
    JsonFileStorage, RepositoryMutationSession,
};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
enum ArchiveTarget<'a> {
    Document(&'a str),
    Container(&'a IssueId),
}

/// Archive and snapshot-export operations of a [`CommandExecutor`].
pub struct ArchiveOps<'a, S: IssueStore> {
    pub(super) exec: &'a CommandExecutor<S>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// The archive and snapshot-export family.
    pub fn archive(&self) -> ArchiveOps<'_, S> {
        ArchiveOps { exec: self }
    }
}

/// Ids of configured non-leaf containers that are archive candidates: those that
/// are effectively terminal ([`Issue::is_effectively_terminal`]) — Done, Rejected,
/// or already Archived from one of those. This is the same predicate the direct
/// archive path gates coupled retirement on (`jit:45a140ae`), so an
/// Archived-from-terminal container the direct path would reconcile also appears
/// as a candidate.
fn effectively_terminal_container_ids(
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
) -> Vec<IssueId> {
    let leaf_level = hierarchy.types().map(|(_, level)| *level).max();
    let mut ids = issues
        .iter()
        .filter(|issue| issue.is_effectively_terminal())
        .filter(|issue| {
            crate::labels::type_label_value(&issue.labels)
                .and_then(|type_name| hierarchy.get_level(type_name))
                .zip(leaf_level)
                .is_some_and(|(level, leaf)| level < leaf)
        })
        .map(|issue| issue.id.clone())
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn remap_archived_references(issues: &mut [Issue], destination_root: &str, development_root: &str) {
    let prefix = format!("{}/", normalize_artifact_path(destination_root));
    let development_root = normalize_artifact_path(development_root);
    issues.iter_mut().for_each(|issue| {
        issue
            .documents
            .iter_mut()
            .filter(|document| document.commit.is_none())
            .for_each(|document| {
                let normalized = normalize_artifact_path(&document.path);
                if let Some(relative) = normalized.strip_prefix(&prefix) {
                    document.path = if development_root.is_empty() {
                        relative.to_string()
                    } else {
                        normalize_artifact_path(&format!("{development_root}/{relative}"))
                    };
                }
            });
    });
}

fn inventory_issues_pinned_evidence(
    requests: impl IntoIterator<Item = (String, String)>,
    resolver: &GitRevisionResolver,
) -> PinnedRootEvidenceMap {
    requests
        .into_iter()
        .map(|(revision, path)| {
            let key = (revision.clone(), path.clone());
            let evidence = resolver
                .read_pinned_path(&revision, &path)
                .map(|read| PinnedRootEvidence::Resolved(read.version().clone()))
                .unwrap_or(PinnedRootEvidence::Unavailable);
            (key, evidence)
        })
        .collect()
}

fn apply_recorded_residue_identities(
    locations: &mut BTreeMap<String, ArtifactLocationFacts>,
    target: &PlanTarget,
    destination_root: &str,
    executions: &[ArchiveExecution],
) {
    let covered = executions
        .iter()
        .filter(|execution| {
            &execution.target == target && execution.destination_root == destination_root
        })
        .flat_map(|execution| &execution.publications)
        .filter_map(|publication| {
            publication.source.as_ref().map(|source| {
                (
                    normalize_artifact_path(source),
                    publication.content_identity.clone(),
                )
            })
        })
        .collect::<BTreeMap<_, _>>();

    locations.iter_mut().for_each(|(source_path, location)| {
        if let (ArtifactLocation::Regular(source), ArtifactLocation::Regular(destination)) =
            (&location.source, &location.destination)
        {
            let differs = source != destination;
            // Only a prior archive record turns differing source bytes into an
            // edited residue; an unrelated occupied destination still blocks.
            // Source is the stable publication identity across destination-layout
            // cutovers, while target and destination root scope the execution set.
            if differs
                && covered
                    .get(&normalize_artifact_path(source_path))
                    .is_some_and(|identity| identity == destination)
            {
                location.source = ArtifactLocation::Regular(destination.clone());
            }
        }
    });
}

impl<S: IssueStore> ArchiveOps<'_, S> {
    /// Fully evaluate every effectively terminal configured non-leaf container
    /// without mutation.
    pub fn archive_candidates(&self) -> Result<ArchiveCandidates> {
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
        let issues = self.exec.storage.list_issues()?;
        let plans = effectively_terminal_container_ids(&issues, &hierarchy)
            .into_iter()
            .map(|id| self.plan_archive_target(ArchiveTarget::Container(&id)))
            .collect::<Result<Vec<_>>>()?;
        Ok(ArchiveCandidates::new(plans))
    }

    /// Build a complete non-mutating plan for one arbitrary repository document.
    pub fn preview_archive_document(&self, path: &str) -> Result<ArtifactPlan> {
        self.plan_archive_target(ArchiveTarget::Document(path))
    }

    /// Build a complete non-mutating plan for one resolved container subtree.
    pub fn preview_archive_container(&self, id: &IssueId) -> Result<ArtifactPlan> {
        self.plan_archive_target(ArchiveTarget::Container(id))
    }

    fn plan_archive_target(&self, target: ArchiveTarget<'_>) -> Result<ArtifactPlan> {
        let config = self.exec.config_manager.load()?;
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.exec.storage)?;
        let issues = self.exec.storage.list_issues()?;
        let layout = self.exec.require_layout()?;
        let repo_root = layout.worktree_root();
        let resolver = GitRevisionResolver::new(repo_root);

        let root_container_id = match target {
            ArchiveTarget::Document(_) => None,
            ArchiveTarget::Container(id) => Some(id.clone()),
        };
        let policy =
            ArtifactClassificationPolicy::from_documentation(config.documentation.as_ref());
        let target_for_layout = match &target {
            ArchiveTarget::Document(path) => PlanTarget::Document {
                path: normalize_artifact_path(path),
            },
            ArchiveTarget::Container(_) => PlanTarget::Container {
                id: root_container_id
                    .as_ref()
                    .context("container id was not resolved")?
                    .clone(),
            },
        };
        let legacy_root = artifact_destination_root(&target_for_layout, &policy.archive_root);
        let (destination_root, destination_conflicts) = match root_container_id.as_ref() {
            Some(container_id) => {
                let issue = issues
                    .iter()
                    .find(|issue| &issue.id == container_id)
                    .context("resolved archive container is missing from issue inventory")?;
                let preferred_root =
                    preferred_container_destination_root(issue, &hierarchy, &policy.archive_root);
                if policy.archive_root.is_empty() {
                    (preferred_root, Vec::new())
                } else {
                    let resolved = resolve_container_destination(
                        &self.exec.storage,
                        &preferred_root,
                        &legacy_root,
                        container_id,
                    )?;
                    (resolved.destination_root, resolved.conflicting_roots)
                }
            }
            None => (legacy_root, Vec::new()),
        };
        // Feed inverse-mirror sources to inventory while retaining the real
        // durable issue records for exact apply/observe decisions below.
        let mut inventory_issues = issues.clone();
        remap_archived_references(
            &mut inventory_issues,
            &destination_root,
            &policy.development_root,
        );
        let explicit_target = match &target {
            ArchiveTarget::Document(path) => ExplicitRootTarget::Document(path),
            ArchiveTarget::Container(_) => ExplicitRootTarget::Container(
                root_container_id
                    .as_ref()
                    .context("container id was not resolved")?,
            ),
        };
        let pinned = inventory_issues_pinned_evidence(
            pinned_root_requests(&inventory_issues, &hierarchy, explicit_target)?,
            &resolver,
        );
        let inventory =
            inventory_explicit_roots(&inventory_issues, &hierarchy, explicit_target, &pinned)?;
        let inventory = match root_container_id.as_ref() {
            Some(root) => {
                let documentation = config.documentation.clone().unwrap_or_default();
                let files = member_document_directories(
                    &inventory_issues,
                    &hierarchy,
                    root,
                    &documentation,
                )
                .into_iter()
                .map(|directory| {
                    match inspect_artifact_evidence(
                        &self.exec.storage,
                        &directory,
                        ArtifactListingScope::RecursiveFiles,
                    )? {
                        ArtifactEvidence::Directory { entries, .. } => Ok(entries),
                        _ => Ok(Vec::new()),
                    }
                })
                .collect::<Result<Vec<_>>>()?
                .concat();
                inventory.with_unlinked_roots(member_derived_documents(
                    &inventory_issues,
                    &hierarchy,
                    root,
                    &files,
                    &documentation,
                ))?
            }
            None => inventory,
        };
        let archive_evidence = self
            .exec
            .storage
            .read_archive_evidence_record(&target_for_layout)?
            .unwrap_or_else(|| ArchiveEvidenceRecord {
                target: target_for_layout.clone(),
                executions: Vec::new(),
            });
        if archive_evidence.target != target_for_layout
            || archive_evidence
                .executions
                .iter()
                .any(|execution| execution.target != archive_evidence.target)
        {
            bail!("Archive evidence record does not match target {target_for_layout:?}");
        }
        let archive_executions = archive_evidence.executions;
        let (discovered, embedded_owners) = discover_archive_artifacts(
            &self.exec.storage,
            inventory,
            &issues,
            &policy,
            recorded_archived_copies(&archive_executions, &target_for_layout),
        )?;
        let plan_target = discovered.target;
        let artifacts = discovered.artifacts;
        let mut blockers = discovered.blockers;
        blockers.extend(
            destination_conflicts
                .into_iter()
                .map(|root| PlanBlocker::new(BlockerCode::DestinationConflict, Some(root))),
        );

        if let Some(container_id) = root_container_id.as_ref() {
            // Coupled retirement (`jit:45a140ae`): artifact archival requires an
            // effectively terminal container (Done/Rejected, or already Archived
            // from one of those for an idempotent rerun). An Archived container
            // retired from a non-terminal state, or any active state, is blocked.
            if issues
                .iter()
                .find(|issue| &issue.id == container_id)
                .is_some_and(|issue| !issue.is_effectively_terminal())
            {
                blockers.push(PlanBlocker::new(
                    BlockerCode::NonTerminalTarget,
                    None::<String>,
                ));
            }
        }

        let mut facts = collect_artifact_classification_facts(
            &self.exec.storage,
            &plan_target,
            &destination_root,
            &artifacts,
            &policy,
            embedded_owners,
        )?;
        apply_recorded_residue_identities(
            &mut facts.locations,
            &plan_target,
            &destination_root,
            &archive_executions,
        );
        classify_artifacts(
            ArtifactClassificationInventory::new(plan_target, artifacts, blockers)
                .with_destination_root(destination_root),
            policy,
            facts,
        )
        .map_err(Into::into)
    }
}

/// Grow `image` by the requests `add` puts in its spec, reading only those;
/// `false` is a retryable conflict.
fn capture_more(
    session: &mut dyn RepositoryMutationSession,
    image: &mut crate::repository_state::RepositoryImage,
    add: impl FnOnce(&mut crate::repository_state::CaptureSpec) -> Result<()>,
) -> Result<bool> {
    let mut spec = image.capture_spec().clone();
    add(&mut spec)?;
    if &spec == image.capture_spec() {
        return Ok(true);
    }
    match capture_or_retry(session.extend(image.clone(), spec))? {
        Some(next) => {
            *image = next;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// [`capture_more`] of `paths`.
fn capture_paths(
    session: &mut dyn RepositoryMutationSession,
    image: &mut crate::repository_state::RepositoryImage,
    paths: impl IntoIterator<Item = crate::repository_state::VirtualPath>,
) -> Result<bool> {
    capture_more(session, image, |spec| Ok(spec.discover_paths(paths)?))
}

fn image_path(
    layout: &crate::repository_state::RepositoryLayout,
    path: &str,
) -> Result<crate::repository_state::VirtualPath> {
    Ok(layout.classify_and_canonicalize(layout.worktree_root().join(path))?)
}

fn image_evidence(
    image: &crate::repository_state::RepositoryImage,
    path: &str,
    scope: ArtifactListingScope,
) -> Result<ArtifactEvidence> {
    use crate::repository_state::RepositoryEntry;
    let vpath = image_path(image.layout(), path)?;
    Ok(match image.entry(&vpath)? {
        RepositoryEntry::Absent => ArtifactEvidence::Missing,
        RepositoryEntry::File { content, .. } => ArtifactEvidence::File(content.bytes().to_vec()),
        RepositoryEntry::FileTail { .. } => {
            return Err(crate::repository_state::CaptureError::TailOnlyCapture(vpath).into())
        }
        RepositoryEntry::FileKind { .. } => {
            return Err(crate::repository_state::CaptureError::KindOnlyCapture(vpath).into())
        }
        RepositoryEntry::Symlink { .. } => ArtifactEvidence::Symlink,
        RepositoryEntry::Unsupported { .. } => ArtifactEvidence::Unsupported,
        RepositoryEntry::Directory { .. } => {
            let entries = match scope {
                ArtifactListingScope::MetadataOnly => Vec::new(),
                ArtifactListingScope::ImmediateChildren => image
                    .directory_listings()
                    .get(&vpath)
                    .context("archive directory listing was not captured")?
                    .names()
                    .iter()
                    .map(|name| normalize_artifact_path(&format!("{path}/{name}")))
                    .collect(),
                // The two recursive scopes differ only in whether a descended
                // directory is named as well, so they share one traversal
                // rather than diverging into two filters that could drift.
                ArtifactListingScope::RecursiveFiles | ArtifactListingScope::RecursiveEntries => {
                    let root = image.layout().worktree_root().join(path);
                    let name_directories = matches!(scope, ArtifactListingScope::RecursiveEntries);
                    image
                        .entries()
                        .iter()
                        .filter(|(candidate, entry)| {
                            image.layout().resolve(candidate).is_ok_and(|physical| {
                                physical.starts_with(&root) && physical != root
                            }) && match entry {
                                RepositoryEntry::Absent => false,
                                RepositoryEntry::Directory { .. } => name_directories,
                                _ => true,
                            }
                        })
                        .map(|(candidate, _)| {
                            Ok(image
                                .layout()
                                .resolve(candidate)?
                                .strip_prefix(image.layout().worktree_root())?
                                .to_string_lossy()
                                .replace('\\', "/"))
                        })
                        .collect::<Result<Vec<_>>>()?
                }
            };
            ArtifactEvidence::Directory { scope, entries }
        }
    })
}

fn capture_artifact_evidence(
    session: &mut dyn RepositoryMutationSession,
    image: &mut crate::repository_state::RepositoryImage,
    paths: impl IntoIterator<Item = String>,
    evidence: &mut ArtifactEvidenceMap,
) -> Result<bool> {
    let paths = paths.into_iter().collect::<BTreeSet<_>>();
    for path in &paths {
        if validate_repo_relative_path(path).is_err()
            || crate::domain::relative_path::RootRelativePath::parse(path).is_err()
        {
            evidence.insert(path.clone(), ArtifactEvidence::InvalidPath);
        }
    }
    loop {
        let mut requested = BTreeMap::new();
        let unresolved = paths
            .iter()
            .filter(|path| !evidence.contains_key(path.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for path in &unresolved {
            let mut prefix = std::path::PathBuf::new();
            for component in std::path::Path::new(path).components() {
                prefix.push(component);
                let prefix = prefix.to_string_lossy().replace('\\', "/");
                let Some(fact) = evidence.get(&prefix).cloned() else {
                    requested.insert(prefix.clone(), image_path(image.layout(), &prefix)?);
                    break;
                };
                if prefix == *path {
                    break;
                }
                let descendant = match fact {
                    ArtifactEvidence::Directory { .. } | ArtifactEvidence::Missing => None,
                    ArtifactEvidence::Symlink => Some(ArtifactEvidence::Symlink),
                    ArtifactEvidence::InvalidPath => Some(ArtifactEvidence::InvalidPath),
                    ArtifactEvidence::File(_) | ArtifactEvidence::Unsupported => {
                        Some(ArtifactEvidence::Unsupported)
                    }
                };
                if let Some(descendant) = descendant {
                    evidence.insert(path.clone(), descendant);
                    break;
                }
            }
        }
        if requested.is_empty() {
            return Ok(true);
        }
        if !capture_paths(session, image, requested.values().cloned())? {
            return Ok(false);
        }
        for (path, _) in requested {
            evidence.insert(
                path.clone(),
                image_evidence(image, &path, ArtifactListingScope::MetadataOnly)?,
            );
        }
    }
}

fn capture_directory_tree(
    session: &mut dyn RepositoryMutationSession,
    image: crate::repository_state::RepositoryImage,
    root: &str,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    use crate::repository_state::RepositoryEntry;
    let root_path = image_path(image.layout(), root)?;
    let root_physical = image.layout().resolve(&root_path)?;
    let beneath = |image: &crate::repository_state::RepositoryImage,
                   path: &crate::repository_state::VirtualPath| {
        image
            .layout()
            .resolve(path)
            .is_ok_and(|physical| physical.starts_with(&root_physical))
    };
    super::grow_discovering(session, image, |image, spec| {
        let mut paths = vec![root_path.clone()];
        for (directory, listing) in image.directory_listings() {
            if beneath(image, directory) {
                for name in listing.names().iter() {
                    let physical = image.layout().resolve(directory)?.join(name);
                    paths.push(image.layout().classify_and_canonicalize(physical)?);
                }
            }
        }
        paths.retain(|path| !spec.contains_path(path));
        spec.discover_paths(paths)?;
        Ok(spec.discover_listings(
            image
                .entries()
                .iter()
                .filter(|(path, entry)| {
                    matches!(entry, RepositoryEntry::Directory { .. }) && beneath(image, path)
                })
                .map(|(path, _)| path.clone()),
        )?)
    })
}

impl ArchiveOps<'_, JsonFileStorage> {
    fn capture_archive_plan(
        &self,
        session: &mut dyn RepositoryMutationSession,
        target: ArchiveTarget<'_>,
    ) -> Result<
        Option<(
            crate::repository_state::RepositoryImage,
            ArtifactPlan,
            ArchiveEvidenceRecord,
        )>,
    > {
        use crate::repository_state::{ArchiveExecutionError, MutationError, ProducerError};

        let Some(mut image) = self.exec.capture_archive_base(session)? else {
            return Ok(None);
        };
        let config = crate::repository_state::assemble_config(&image)?;
        let namespaces = crate::config_manager::namespaces_from_config(&config);
        let hierarchy =
            crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&namespaces)?;
        let issues = super::captured_active_issues(&image)?;
        let root_id = match target {
            ArchiveTarget::Document(_) => None,
            ArchiveTarget::Container(id) => Some(super::require_issue_in_capture(&issues, id)?),
        };
        let policy =
            ArtifactClassificationPolicy::from_documentation(config.documentation.as_ref());
        let plan_target = match target {
            ArchiveTarget::Document(path) => PlanTarget::Document {
                path: normalize_artifact_path(path),
            },
            ArchiveTarget::Container(_) => PlanTarget::Container {
                id: root_id
                    .as_ref()
                    .context("container id was not resolved")?
                    .clone(),
            },
        };
        let evidence_path =
            crate::repository_state::VirtualPath::archive_evidence_record(&plan_target);
        if !capture_more(session, &mut image, |spec| {
            spec.discover_paths(evidence_path.ancestor_directories()?)?;
            Ok(spec.discover_paths([evidence_path.clone()])?)
        })? {
            return Ok(None);
        }
        let archive_evidence = match image
            .file_bytes(&evidence_path)
            .map_err(ProducerError::from)
            .map_err(ArchiveExecutionError::Producer)?
        {
            Some(bytes) => serde_json::from_slice::<ArchiveEvidenceRecord>(bytes)
                .map_err(|error| MutationError::MalformedRecord {
                    path: evidence_path.clone(),
                    reason: error.to_string(),
                })
                .map_err(ArchiveExecutionError::Mutation)?,
            None => ArchiveEvidenceRecord {
                target: plan_target.clone(),
                executions: Vec::new(),
            },
        };
        if archive_evidence.target != plan_target
            || archive_evidence
                .executions
                .iter()
                .any(|prior| prior.target != archive_evidence.target)
        {
            return Err(
                ArchiveExecutionError::Mutation(MutationError::MalformedRecord {
                    path: evidence_path,
                    reason: "archive evidence contains another target".into(),
                })
                .into(),
            );
        }
        let legacy = artifact_destination_root(&plan_target, &policy.archive_root);
        let mut conflicts = Vec::new();
        let destination = if let Some(id) = root_id.as_ref() {
            let issue = issues
                .iter()
                .find(|issue| &issue.id == id)
                .context("captured container missing")?;
            let preferred =
                preferred_container_destination_root(issue, &hierarchy, &policy.archive_root);
            if policy.archive_root.is_empty() {
                preferred
            } else {
                let archive_root = std::path::Path::new(&legacy)
                    .parent()
                    .context("archive root missing")?
                    .to_string_lossy()
                    .replace('\\', "/");
                let archive_path = image_path(image.layout(), &archive_root)?;
                let mut root_evidence = ArtifactEvidenceMap::new();
                if !capture_artifact_evidence(
                    session,
                    &mut image,
                    [archive_root.clone()],
                    &mut root_evidence,
                )? {
                    return Ok(None);
                }
                let archive_is_directory = matches!(
                    root_evidence.get(&archive_root),
                    Some(ArtifactEvidence::Directory { .. })
                );
                let children = if archive_is_directory {
                    if !capture_more(session, &mut image, |spec| {
                        Ok(spec.discover_listing(archive_path.clone())?)
                    })? {
                        return Ok(None);
                    }
                    image
                        .directory_listings()
                        .get(&archive_path)
                        .context("archive root listing was not captured")?
                        .names()
                        .iter()
                        .map(|name| normalize_artifact_path(&format!("{archive_root}/{name}")))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                let mut child_paths = children
                    .iter()
                    .map(|path| image_path(image.layout(), path))
                    .collect::<Result<Vec<_>>>()?;
                if archive_is_directory {
                    child_paths.push(image_path(image.layout(), &legacy)?);
                }
                if !capture_paths(session, &mut image, child_paths)? {
                    return Ok(None);
                }
                let mut markers = Vec::new();
                for path in &children {
                    let child = image_path(image.layout(), path)?;
                    if matches!(
                        image.entry(&child)?,
                        crate::repository_state::RepositoryEntry::Directory { .. }
                    ) {
                        markers.push(image_path(
                            image.layout(),
                            &format!("{path}/.jit-container"),
                        )?);
                    }
                }
                if !capture_paths(session, &mut image, markers)? {
                    return Ok(None);
                }
                let mut evidence = ArtifactEvidenceMap::from([
                    (
                        archive_root.clone(),
                        if archive_is_directory {
                            image_evidence(
                                &image,
                                &archive_root,
                                ArtifactListingScope::ImmediateChildren,
                            )?
                        } else {
                            root_evidence
                                .remove(&archive_root)
                                .context("archive root evidence was not derived")?
                        },
                    ),
                    (
                        legacy.clone(),
                        if archive_is_directory {
                            image_evidence(&image, &legacy, ArtifactListingScope::MetadataOnly)?
                        } else {
                            ArtifactEvidence::Missing
                        },
                    ),
                ]);
                for child in children {
                    let fact = image_evidence(&image, &child, ArtifactListingScope::MetadataOnly)?;
                    if matches!(fact, ArtifactEvidence::Directory { .. }) {
                        let marker = format!("{child}/.jit-container");
                        evidence.insert(
                            marker.clone(),
                            image_evidence(&image, &marker, ArtifactListingScope::MetadataOnly)?,
                        );
                    }
                    evidence.insert(child, fact);
                }
                let resolved = derive_container_destination(&preferred, &legacy, id, &evidence)?;
                conflicts = resolved.conflicting_roots;
                resolved.destination_root
            }
        } else {
            legacy.clone()
        };
        let mut inventory_issues = issues.clone();
        remap_archived_references(
            &mut inventory_issues,
            &destination,
            &policy.development_root,
        );
        let explicit = match target {
            ArchiveTarget::Document(path) => ExplicitRootTarget::Document(path),
            ArchiveTarget::Container(_) => ExplicitRootTarget::Container(
                root_id.as_ref().context("container id was not resolved")?,
            ),
        };
        let pinned_requests = pinned_root_requests(&inventory_issues, &hierarchy, explicit)?;
        if !capture_more(session, &mut image, |spec| {
            pinned_requests
                .iter()
                .try_for_each(|(revision, path)| spec.discover_pinned(revision, path))
                .map_err(Into::into)
        })? {
            return Ok(None);
        }
        let pinned = pinned_requests
            .into_iter()
            .map(|request| -> Result<_> {
                let captured = image.pinned_evidence().get(&request).with_context(|| {
                    format!(
                        "captured pinned evidence is missing for {} at {}",
                        request.1, request.0
                    )
                })?;
                let fact = match captured.commit_oid() {
                    Some(oid) => PinnedRootEvidence::Resolved(
                        crate::domain::artifact_plan::ArtifactVersion::pinned(oid)?,
                    ),
                    None if captured.unavailable_reason().is_some() => {
                        PinnedRootEvidence::Unavailable
                    }
                    None => bail!(
                        "captured pinned evidence is malformed for {} at {}",
                        request.1,
                        request.0
                    ),
                };
                Ok((request, fact))
            })
            .collect::<Result<_>>()?;
        let inventory = inventory_explicit_roots(&inventory_issues, &hierarchy, explicit, &pinned)?;
        let inventory = match root_id.as_ref() {
            Some(root) => {
                let documentation = config.documentation.clone().unwrap_or_default();
                let mut files = Vec::new();
                for directory in
                    member_document_directories(&inventory_issues, &hierarchy, root, &documentation)
                {
                    let Some(next) = capture_directory_tree(session, image, &directory)? else {
                        return Ok(None);
                    };
                    image = next;
                    if let ArtifactEvidence::Directory { entries, .. } =
                        image_evidence(&image, &directory, ArtifactListingScope::RecursiveFiles)?
                    {
                        files.extend(entries);
                    }
                }
                inventory.with_unlinked_roots(member_derived_documents(
                    &inventory_issues,
                    &hierarchy,
                    root,
                    &files,
                    &documentation,
                ))?
            }
            None => inventory,
        };
        let roots = inventory
            .artifacts()
            .iter()
            .filter(|entry| !entry.version().is_pinned())
            .map(|entry| entry.source().to_string())
            .chain(
                issues
                    .iter()
                    .flat_map(|issue| issue.documents.iter())
                    .filter(|doc| doc.commit.is_none())
                    .map(|doc| normalize_artifact_path(&doc.path)),
            )
            .collect::<BTreeSet<_>>();
        let mut evidence = ArtifactEvidenceMap::new();
        let archive_executions = &archive_evidence.executions;
        let mut state = ArtifactClosureState::new(roots)
            .with_archived_copies(recorded_archived_copies(archive_executions, &plan_target));
        let parsed = loop {
            match expand_artifact_closure(state, &evidence) {
                ArtifactClosure::Complete(parsed) => break parsed,
                ArtifactClosure::Needs {
                    paths,
                    state: next_state,
                } => {
                    if !capture_artifact_evidence(
                        session,
                        &mut image,
                        paths.iter().cloned(),
                        &mut evidence,
                    )? {
                        return Ok(None);
                    }
                    state = next_state;
                }
            }
        };
        let (discovered, embedded) =
            derive_archive_artifacts(inventory, &issues, &evidence, &parsed, &policy)?;
        let mut blockers = discovered.blockers;
        blockers.extend(
            conflicts
                .into_iter()
                .map(|root| PlanBlocker::new(BlockerCode::DestinationConflict, Some(root))),
        );
        if let Some(id) = root_id.as_ref() {
            if issues
                .iter()
                .find(|issue| &issue.id == id)
                .is_some_and(|issue| !issue.is_effectively_terminal())
            {
                blockers.push(PlanBlocker::new(
                    BlockerCode::NonTerminalTarget,
                    None::<String>,
                ));
            }
        }
        let artifacts = discovered.artifacts;
        let layout = ArchiveLayout::new(&plan_target, &destination, &policy, &artifacts);
        let mut paths = artifacts
            .iter()
            .filter(|a| !a.version().is_pinned())
            .flat_map(|a| {
                std::iter::once(a.source().to_string()).chain(layout.destination(a.source()))
            })
            .collect::<BTreeSet<_>>();
        if matches!(plan_target, PlanTarget::Container { .. }) {
            paths.insert(format!("{destination}/.jit-container"));
        }
        let ancestors = paths
            .iter()
            .flat_map(|path| {
                std::path::Path::new(path)
                    .ancestors()
                    .skip(1)
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .collect::<BTreeSet<_>>();
        paths.extend(ancestors);
        if !capture_artifact_evidence(session, &mut image, paths.iter().cloned(), &mut evidence)? {
            return Ok(None);
        }
        if matches!(plan_target, PlanTarget::Container { .. })
            && matches!(
                evidence.get(&destination),
                Some(ArtifactEvidence::Directory { .. })
            )
        {
            let Some(next) = capture_directory_tree(session, image, &destination)? else {
                return Ok(None);
            };
            image = next;
            evidence.insert(
                destination.clone(),
                image_evidence(&image, &destination, ArtifactListingScope::RecursiveFiles)?,
            );
        }
        let mut facts = classification_facts_from_evidence(
            &plan_target,
            &destination,
            &artifacts,
            &policy,
            embedded,
            &evidence,
        )?;
        // Citation warnings are advisory and no write derives from them, so the
        // scan reads the files directly, one at a time, rather than charging
        // them to the guarded image and its capture budget.
        facts.citations = crate::storage::collect_citation_scan_evidence(
            &self.exec.storage,
            &policy.citation_scan_roots,
            artifacts
                .iter()
                .map(|artifact| artifact.source().to_string()),
        )?;
        apply_recorded_residue_identities(
            &mut facts.locations,
            &plan_target,
            &destination,
            archive_executions,
        );
        let plan = classify_artifacts(
            ArtifactClassificationInventory::new(plan_target, artifacts, blockers)
                .with_destination_root(destination),
            policy,
            facts,
        )?;
        Ok(Some((image, plan, archive_evidence)))
    }

    /// Execute a freshly recomputed document archive plan under one write guard.
    pub fn execute_archive_document(&self, path: &str) -> Result<ArchiveExecutionResult> {
        self.execute_archive_target(ArchiveTarget::Document(path))
    }

    /// Execute a freshly recomputed container archive plan under one write guard.
    pub fn execute_archive_container(&self, id: &IssueId) -> Result<ArchiveExecutionResult> {
        self.execute_archive_target(ArchiveTarget::Container(id))
    }

    fn execute_archive_target(&self, target: ArchiveTarget<'_>) -> Result<ArchiveExecutionResult> {
        let layout = self.exec.require_layout()?;
        let context = super::production_mutation_context();
        with_mutation_session(
            &self.exec.storage,
            &layout,
            "archive execution",
            |session| {
                let Some((image, plan, evidence)) = self.capture_archive_plan(session, target)?
                else {
                    return Ok(SessionStep::Retry);
                };
                if let Some((code, guidance)) = plan
                    .blockers()
                    .iter()
                    .chain(
                        plan.artifacts()
                            .iter()
                            .flat_map(|artifact| artifact.blockers()),
                    )
                    .find_map(|blocker| {
                        blocker
                            .code
                            .guidance()
                            .map(|guidance| (blocker.code, guidance))
                    })
                {
                    bail!("cannot archive target: {} — {guidance}", code.as_str());
                }
                let (materialization, result) =
                    crate::repository_state::finalize_archive_execution(
                        &image, &context, &plan, evidence,
                    )?;
                if materialization.delta().actions().is_empty() {
                    return Ok(SessionStep::Done(result));
                }
                Ok(SessionStep::Apply(materialization, result))
            },
        )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_apply_recorded_residue_identities_scopes_coverage_to_target_and_root() {
        let target = PlanTarget::Document {
            path: "a.md".into(),
        };
        let executions = [
            archive_execution(target.clone(), "arc", &[("src/x.md", "arc/x.md")]),
            archive_execution(target.clone(), "elsewhere", &[("src/y.md", "other/y.md")]),
        ];
        let bytes = |text: &[u8]| {
            ArtifactLocation::Regular(crate::domain::artifact_plan::ContentIdentity::from_bytes(
                text,
            ))
        };
        let facts = |source: &[u8], destination: &[u8]| ArtifactLocationFacts {
            source: bytes(source),
            destination: bytes(destination),
        };
        let mut locations = BTreeMap::from([
            ("src/x.md".to_string(), facts(b"edited", b"arc/x.md")),
            ("src/y.md".to_string(), facts(b"edited", b"other/y.md")),
        ]);
        let before = locations.clone();
        apply_recorded_residue_identities(&mut locations, &target, "arc", &executions);
        assert_ne!(locations["src/x.md"], before["src/x.md"]);
        assert_eq!(locations["src/y.md"], before["src/y.md"]);
    }

    use super::*;
    use crate::domain::artifact_plan::EvidenceCode;
    use crate::domain::identity::IssueId;
    use crate::domain::{EventBody, State};
    use crate::storage::JsonFileStorage;
    use crate::test_taxonomy::test_taxonomy;
    use crate::test_utils::archive_execution;
    use crate::test_utils::{document_reference, document_reference_at};
    use std::collections::HashMap;
    use std::fs;

    fn stored_archive_executions<S: IssueStore>(storage: &S) -> Vec<ArchiveExecution> {
        crate::domain::artifact_execution::archive_executions(&storage.read_events().unwrap())
    }

    #[test]
    fn test_archive_candidates_propagates_invalid_hierarchy_error() {
        let (repo, executor) =
            crate::test_utils::setup_test_repo_with_config(&test_taxonomy().config_fragment())
                .unwrap();
        let config = crate::test_taxonomy::invalid_hierarchy_config_fragment();
        std::fs::write(repo.path().join(".jit/config.toml"), config).unwrap();

        let error = executor
            .archive()
            .archive_candidates()
            .expect_err("invalid hierarchy must fail archive candidate planning");

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
    }

    /// Candidates are every effectively terminal non-leaf container, each fully
    /// evaluated, and reporting them leaves issues and events intact.
    #[test]
    fn test_archive_candidates_are_complete_and_non_mutating() {
        let storage = crate::storage::InMemoryStorage::new();
        let archive_root = format!(
            "{}/archive",
            storage.root().file_name().unwrap().to_string_lossy()
        );
        let seed = |title: &str, state: State, kind: &str| {
            let mut issue = crate::test_utils::fixture_issue(title.to_string(), String::new());
            issue.state = state;
            issue.archived_from = (state == State::Archived).then_some(State::InProgress);
            issue.labels = vec![format!("type:{kind}")];
            let id = issue.id.clone();
            crate::commands::test_helpers::seed_issue(&storage, issue);
            id
        };
        let rejected = seed("Rejected portfolio", State::Rejected, "portfolio");
        let done = seed("Done portfolio", State::Done, "portfolio");
        seed("Done leaf", State::Done, "unit");
        seed("Archived portfolio", State::Archived, "portfolio");
        let config = format!(
            "[documentation]\nmanaged_paths = []\npermanent_paths = []\narchive_root = \"{archive_root}\"\n\n[type_hierarchy]\ntypes = {{ portfolio = 2, unit = 7 }}\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n"
        );
        let executor = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );
        let issues_before = executor.storage.list_issues().unwrap();
        let events_before = executor.storage.read_events().unwrap();

        let report = executor.archive().archive_candidates().unwrap();

        let target_ids = report
            .candidates()
            .iter()
            .map(|plan| match plan.target() {
                PlanTarget::Container { id } => id.clone(),
                _ => panic!("candidate target must be a container"),
            })
            .collect::<Vec<_>>();
        let mut expected = vec![done, rejected];
        expected.sort();
        assert_eq!(target_ids, expected);
        assert!(report.candidates().iter().all(|plan| {
            plan.count() == 0
                && plan.policy_status() == crate::domain::artifact_plan::PolicyStatus::Configured
        }));
        assert_eq!(executor.storage.list_issues().unwrap(), issues_before);
        assert_eq!(executor.storage.read_events().unwrap(), events_before);
    }

    #[test]
    fn test_archive_candidates_ignore_unparseable_event_log() {
        let storage = crate::storage::InMemoryStorage::new();
        let mut issue =
            crate::test_utils::fixture_issue("Archived portfolio".into(), String::new());
        issue.state = State::Done;
        issue.labels = vec!["type:portfolio".into()];
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let target = PlanTarget::Container { id: id.clone() };
        storage.add_repository_bytes(
            crate::repository_state::VirtualPath::archive_evidence_record(&target),
            &serde_json::to_vec(&ArchiveEvidenceRecord {
                target,
                executions: Vec::new(),
            })
            .unwrap(),
        );
        storage.add_repository_bytes(
            crate::repository_state::VirtualPath::EVENTS,
            b"{not-json}\n",
        );
        let config = format!(
            "[documentation]\nmanaged_paths = []\npermanent_paths = []\narchive_root = \"{}\"\n\n[type_hierarchy]\ntypes = {{ portfolio = 2, unit = 7 }}\n\n[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
            storage.root().file_name().unwrap().to_string_lossy()
        );
        let executor = crate::commands::test_helpers::memory_executor_declaring(
            storage,
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&config),
                ..Default::default()
            },
        );

        let report = executor.archive().archive_candidates().unwrap();

        assert_eq!(report.candidates().len(), 1);
        assert_eq!(
            report.candidates()[0].target(),
            &PlanTarget::Container { id }
        );
    }

    #[test]
    fn test_preview_archive_document_rearchive_uses_evidence_with_unparseable_event_log() {
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let source = "fixtures/root.md";
        let destination = "archive/fixtures/root.md";
        fs::create_dir_all(repo.path().join("fixtures")).unwrap();
        fs::create_dir_all(repo.path().join("archive/fixtures")).unwrap();
        fs::write(repo.path().join(source), b"edited residue\n").unwrap();
        fs::write(repo.path().join(destination), b"original\n").unwrap();

        let target = PlanTarget::Document {
            path: source.into(),
        };
        let execution = ArchiveExecution {
            target: target.clone(),
            destination_root: "archive".into(),
            publications: vec![crate::domain::artifact_execution::ArchivePublication {
                source: Some(source.into()),
                destination: destination.into(),
                content_identity: crate::domain::artifact_plan::ContentIdentity::from_bytes(
                    b"original\n",
                ),
                adopted: false,
            }],
            reference_changes: Vec::new(),
            planned_deletions: Vec::new(),
            reconciling: false,
        };
        let event = crate::test_utils::stamped_event(
            EventBody::draft_artifact_archive_executed(execution.clone()),
            "historical-archive",
            chrono::DateTime::UNIX_EPOCH,
        );
        let evidence_path = crate::repository_state::VirtualPath::archive_evidence_record(&target);
        let evidence_file = repo
            .path()
            .join(".jit")
            .join(evidence_path.relative().as_str());
        fs::create_dir_all(evidence_file.parent().unwrap()).unwrap();
        fs::write(
            evidence_file,
            serde_json::to_vec(&ArchiveEvidenceRecord {
                target,
                executions: vec![execution],
            })
            .unwrap(),
        )
        .unwrap();
        let events_path = repo.path().join(".jit/events.jsonl");
        let mut valid_event = serde_json::to_vec(&event).unwrap();
        valid_event.push(b'\n');
        fs::write(&events_path, valid_event).unwrap();

        let before = executor.archive().preview_archive_document(source).unwrap();
        assert!(before.eligible());
        assert_eq!(before.count(), 1);
        assert_eq!(before.artifacts()[0].source(), source);

        fs::write(&events_path, b"{not-json}\n").unwrap();
        let after = executor.archive().preview_archive_document(source).unwrap();

        assert_eq!(after, before);
    }

    /// Seed an exact file-backed issue precondition without exercising command mutation behavior.
    fn seed_archive_issue_precondition(storage: &JsonFileStorage, issue: Issue) {
        let issue_path = storage
            .repository_layout()
            .unwrap()
            .resolve(&crate::repository_state::VirtualPath::issue_record(
                &issue.id,
            ))
            .unwrap();
        fs::write(
            issue_path,
            crate::repository_state::serialize_issue(&issue).expect("serialize fixture issue"),
        )
        .unwrap();
    }

    #[test]
    fn test_effectively_terminal_container_ids_use_live_non_leaf_levels_and_effective_terminality()
    {
        let hierarchy = HierarchyConfig::new(
            HashMap::from([("portfolio".to_string(), 2), ("unit".to_string(), 7)]),
            HashMap::new(),
        )
        .unwrap();
        let issue =
            |id: &IssueId, state: State, archived_from: Option<State>, issue_type: Option<&str>| {
                let mut issue = crate::test_utils::fixture_issue(id.to_string(), String::new());
                issue.id = id.clone();
                issue.state = state;
                issue.archived_from = archived_from;
                issue.labels = issue_type
                    .map(|kind| vec![format!("type:{kind}")])
                    .unwrap_or_default();
                issue
            };
        let issues = vec![
            issue(
                &crate::test_utils::issue_id(0x481e_5d93_bcbd_8d3a_8296_04ea_3073_ec9d),
                State::Done,
                None,
                Some("portfolio"),
            ),
            issue(
                &crate::test_utils::issue_id(0x8274_5bc5_ab45_4bcd_c906_31e7_c81c_a1b2),
                State::Rejected,
                None,
                Some("portfolio"),
            ),
            issue(
                &crate::test_utils::issue_id(0x2115_5819_bd02_0be9_e12b_79b2_58a2_e44c),
                State::InProgress,
                None,
                Some("portfolio"),
            ),
            // Archived from a terminal state is effectively terminal, so it is a
            // reconcilable candidate — same as the direct archive path accepts.
            issue(
                &crate::test_utils::issue_id(0x4078_5d9c_b05c_90bf_43cf_08c2_8f2e_6a1f),
                State::Archived,
                Some(State::Done),
                Some("portfolio"),
            ),
            // Archived from a non-terminal state is not effectively terminal, so
            // it is excluded.
            issue(
                &crate::test_utils::issue_id(0xb561_527e_9bdf_3a3c_1258_a630_ef24_90cc),
                State::Archived,
                Some(State::InProgress),
                Some("portfolio"),
            ),
            issue(
                &crate::test_utils::issue_id(0x3f8_54fa_b8d7_66a8_b066_27fe_907a_9f01),
                State::Done,
                None,
                Some("unit"),
            ),
            issue(
                &crate::test_utils::issue_id(0x1cd3_507f_a34c_9168_6888_506d_6493_8dc3),
                State::Done,
                None,
                Some("epic"),
            ),
            issue(
                &crate::test_utils::issue_id(0xc7e4_5c20_9543_1654_446a_06ac_748f_d8b6),
                State::Done,
                None,
                None,
            ),
        ];

        let mut expected = vec![
            issues[0].id.clone(),
            issues[1].id.clone(),
            issues[3].id.clone(),
        ];
        expected.sort();
        assert_eq!(
            effectively_terminal_container_ids(&issues, &hierarchy),
            expected
        );
    }

    #[test]
    fn test_execute_document_commits_event_before_delete_and_noop_rerun_is_stable() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "# Root\n").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];
        let id = owner.id.clone();
        seed_archive_issue_precondition(&executor.storage, owner);
        let first = executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(first.event_appended);
        assert_eq!(first.deleted_sources, vec!["fixtures/root.md"]);
        assert!(!repo.path().join("fixtures/root.md").exists());
        assert_eq!(
            fs::read(repo.path().join("archive/fixtures/root.md")).unwrap(),
            b"# Root\n"
        );
        let issue = executor.storage.load_issue(&id).unwrap();
        assert_eq!(issue.documents[0].path, "archive/fixtures/root.md");
        assert!(issue.documents[0].assets.is_empty());
        assert_eq!(stored_archive_executions(&executor.storage).len(), 1);

        let second = executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(!second.event_appended);
        assert!(second.publications.is_empty());
        assert_eq!(stored_archive_executions(&executor.storage).len(), 1);
    }

    #[test]
    fn test_execute_archive_document_rearchive_preserves_evidence_with_unparseable_event_log() {
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let source = "fixtures/root.md";
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join(source), b"# Original\n").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner".into(), String::new());
        owner.state = State::Done;
        owner.documents = vec![document_reference(source)];
        seed_archive_issue_precondition(&executor.storage, owner);
        let first = executor.archive().execute_archive_document(source).unwrap();
        fs::write(repo.path().join(source), b"# Edited residue\n").unwrap();

        let before = executor.archive().execute_archive_document(source).unwrap();
        assert!(before.publications.is_empty());
        assert!(before.reference_changes.is_empty());
        assert_eq!(before.planned_deletions, first.planned_deletions);
        assert!(before.deleted_sources.is_empty());
        assert!(before
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::DeletionFailed));

        fs::write(
            repo.path().join(".jit/events.jsonl"),
            b"unparseable event\n",
        )
        .unwrap();
        let after = executor.archive().execute_archive_document(source).unwrap();

        assert_eq!(after, before);
        assert_eq!(
            fs::read(repo.path().join(source)).unwrap(),
            b"# Edited residue\n"
        );
    }

    #[test]
    fn test_execute_reports_preview_citations_without_capturing_the_scanned_files() {
        use crate::storage::RepositoryStateStore as _;
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\ncitation_scan_roots = [\"notes\"]\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("fixtures")).unwrap();
        fs::create_dir_all(repo.path().join("notes")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "# Root\n").unwrap();
        fs::write(
            repo.path().join("notes/cite.md"),
            "See `fixtures/root.md`.\n",
        )
        .unwrap();
        let mut issue = crate::test_utils::fixture_issue("Owner".into(), String::new());
        issue.state = State::Done;
        issue.documents = vec![document_reference("fixtures/root.md")];
        seed_archive_issue_precondition(&executor.storage, issue);
        let citations = |warnings: Vec<crate::domain::artifact_plan::PlanWarning>| {
            warnings
                .into_iter()
                .filter(|warning| {
                    warning.code == crate::domain::artifact_plan::WarningCode::MovingPathCitation
                })
                .collect::<Vec<_>>()
        };

        let preview = executor
            .archive()
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        let mut session = executor
            .storage
            .open_mutation_session(executor.require_layout().unwrap())
            .unwrap();
        let (image, _, _) = executor
            .archive()
            .capture_archive_plan(
                session.as_mut(),
                ArchiveTarget::Document("fixtures/root.md"),
            )
            .unwrap()
            .expect("an uncontended capture converges");
        drop(session);
        let cited = image_path(image.layout(), "notes/cite.md").unwrap();
        assert!(
            !image.capture_spec().contains_path(&cited),
            "the citing file stays out of the guarded image and its budget"
        );
        let previewed = citations(
            preview
                .artifacts()
                .iter()
                .flat_map(|artifact| artifact.warnings().iter().cloned())
                .collect(),
        );
        assert!(!previewed.is_empty(), "the fixture cites the moving source");
        let executed = executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert_eq!(citations(executed.warnings), previewed);
    }

    #[test]
    fn test_execute_non_directory_archive_root_is_blocked_without_descending() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "source").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];

        seed_archive_issue_precondition(&executor.storage, owner);
        fs::write(repo.path().join("archive"), b"occupied").unwrap();

        assert!(executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert_eq!(fs::read(repo.path().join("archive")).unwrap(), b"occupied");
        assert!(repo.path().join("fixtures/root.md").exists());
    }

    #[test]
    fn test_execute_archive_root_nested_under_data_uses_data_paths() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "source").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];

        seed_archive_issue_precondition(&executor.storage, owner);
        fs::write(
            repo.path().join(".jit/config.toml"),
            "[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \".jit/archive\"\n",
        )
        .unwrap();

        executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert_eq!(
            fs::read(repo.path().join(".jit/archive/fixtures/root.md")).unwrap(),
            b"source"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_execute_symlink_source_is_blocked_without_following_it() {
        use std::os::unix::fs::symlink;

        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "source").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];

        seed_archive_issue_precondition(&executor.storage, owner);
        fs::remove_file(repo.path().join("fixtures/root.md")).unwrap();
        fs::write(repo.path().join("outside.md"), b"outside").unwrap();
        symlink("../outside.md", repo.path().join("fixtures/root.md")).unwrap();

        assert!(executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert_eq!(
            fs::read(repo.path().join("outside.md")).unwrap(),
            b"outside"
        );
        assert!(!repo.path().join("archive/fixtures/root.md").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_execute_nested_archive_root_symlink_is_blocked_without_descending() {
        use std::os::unix::fs::symlink;

        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("fixtures/bundle/theme")).unwrap();
        fs::write(
            repo.path().join("fixtures/readme.md"),
            "[page](bundle/index.html) [external](https://example.com)",
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/index.html"),
            r#"<link href="theme/base.css"><img src="image.svg">"#,
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/theme/base.css"),
            r#"@import "nested.css"; body { background: url("../../image.png"); }"#,
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/bundle/theme/nested.css"), "a{}").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.svg"), "<svg/>").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.png"), b"png").unwrap();
        fs::write(repo.path().join("fixtures/data.csv"), "a,b\n1,2\n").unwrap();

        let mut epic = crate::test_utils::fixture_issue("Archive fixture".into(), String::new());
        epic.state = State::Done;
        let epic_type = taxonomy.type_at_level(1);
        let epic_namespace = taxonomy
            .label_associations
            .get(epic_type)
            .expect("taxonomy declares a membership namespace for level one");
        epic.labels = vec![
            format!("type:{epic_type}"),
            format!("{epic_namespace}:archive-fixture"),
        ];
        epic.documents = [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ]
        .into_iter()
        .map(document_reference)
        .collect();
        let id = epic.id.clone();
        seed_archive_issue_precondition(&executor.storage, epic);
        let config_path = repo.path().join(".jit/config.toml");
        let config = fs::read_to_string(&config_path).unwrap().replace(
            "archive_root = \"archive\"",
            "archive_root = \"safe/archive\"",
        );
        fs::write(config_path, config).unwrap();
        fs::create_dir(repo.path().join("outside")).unwrap();
        symlink("outside", repo.path().join("safe")).unwrap();

        assert!(executor.archive().execute_archive_container(&id).is_err());
        assert!(fs::read_dir(repo.path().join("outside"))
            .unwrap()
            .next()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_execute_preserves_executable_source_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "source").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];

        seed_archive_issue_precondition(&executor.storage, owner);
        fs::set_permissions(
            repo.path().join("fixtures/root.md"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert_eq!(
            fs::metadata(repo.path().join("archive/fixtures/root.md"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[test]
    fn test_execute_waits_for_competing_repository_write_guard() {
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(_repo.path().join("fixtures")).unwrap();
        fs::write(_repo.path().join("fixtures/root.md"), "guard contention").unwrap();
        let holder_storage = executor.storage.clone();
        let (held_sender, held_receiver) = std::sync::mpsc::channel();
        let (release_sender, release_receiver) = std::sync::mpsc::channel();
        let holder = std::thread::spawn(move || {
            let _guard = holder_storage.acquire_repo_write_lock().unwrap();
            held_sender.send(()).unwrap();
            release_receiver.recv().unwrap();
        });
        held_receiver.recv().unwrap();

        let (done_sender, done_receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            done_sender
                .send(
                    executor
                        .archive()
                        .execute_archive_document("fixtures/root.md"),
                )
                .unwrap();
        });
        assert!(done_receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        release_sender.send(()).unwrap();
        holder.join().unwrap();
        // Join before receiving: the worker sends its outcome as its last act, so
        // a joined worker has already sent and the receive completes without
        // waiting. What remains blocking is the join, and a join that never
        // returns is the archive deadlocked behind a released guard rather than a
        // thread the host was slow to schedule.
        worker.join().unwrap();
        assert!(done_receiver.recv().unwrap().is_ok());
    }

    #[test]
    fn test_execute_document_allows_managed_zero_owner_and_refuses_active_owner() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "zero owner").unwrap();
        let result = executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::NoOwner));
        assert!(repo.path().join("archive/fixtures/root.md").exists());

        let (active_repo, active_executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(active_repo.path().join("fixtures")).unwrap();
        fs::write(active_repo.path().join("fixtures/root.md"), "active").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];
        let active_id = owner.id.clone();
        seed_archive_issue_precondition(&active_executor.storage, owner);
        let mut issue = active_executor.storage.load_issue(&active_id).unwrap();
        issue.state = State::InProgress;
        seed_archive_issue_precondition(&active_executor.storage, issue);
        assert!(active_executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert!(active_repo.path().join("fixtures/root.md").exists());
        assert!(!active_repo.path().join("archive/fixtures/root.md").exists());
    }

    #[test]
    fn test_execution_preserves_positive_relative_and_root_relative_multi_edge_layout() {
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::create_dir(repo.path().join("shared")).unwrap();
        fs::write(
            repo.path().join("fixtures/root.md"),
            "[child](child.md) [shared](/shared/global.md)",
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/child.md"), "child").unwrap();
        fs::write(repo.path().join("shared/global.md"), "shared").unwrap();
        let result = executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(result.event_appended);
        assert!(repo.path().join("archive/fixtures/root.md").exists());
        assert!(repo.path().join("archive/fixtures/child.md").exists());
        assert!(repo.path().join("shared/global.md").exists());
        assert!(!repo.path().join("archive/shared/global.md").exists());
    }

    #[test]
    fn test_identical_destination_is_adopted_but_differing_destination_is_never_overwritten() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "identical").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];
        let id = owner.id.clone();
        seed_archive_issue_precondition(&executor.storage, owner);
        fs::create_dir(repo.path().join("archive")).unwrap();
        fs::create_dir(repo.path().join("archive/fixtures")).unwrap();
        fs::write(repo.path().join("archive/fixtures/root.md"), b"identical").unwrap();
        let result = executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(result.publications.iter().any(|publication| {
            publication.destination == "archive/fixtures/root.md" && publication.adopted
        }));
        assert_eq!(
            executor.storage.load_issue(&id).unwrap().documents[0].path,
            "archive/fixtures/root.md"
        );

        let (conflict_repo, conflict_executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(conflict_repo.path().join("fixtures")).unwrap();
        fs::write(
            conflict_repo.path().join("fixtures/root.md"),
            "source bytes",
        )
        .unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];

        seed_archive_issue_precondition(&conflict_executor.storage, owner);
        fs::create_dir_all(conflict_repo.path().join("archive/fixtures")).unwrap();
        fs::write(
            conflict_repo.path().join("archive/fixtures/root.md"),
            b"foreign bytes",
        )
        .unwrap();
        assert!(conflict_executor
            .archive()
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert_eq!(
            fs::read(conflict_repo.path().join("archive/fixtures/root.md")).unwrap(),
            b"foreign bytes"
        );
        assert_eq!(
            fs::read(conflict_repo.path().join("fixtures/root.md")).unwrap(),
            b"source bytes"
        );
    }

    #[test]
    fn test_permanent_document_archive_writes_no_copy_and_leaves_the_source() {
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"dev\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"dev/guides\"]\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("dev/guides")).unwrap();
        fs::write(repo.path().join("dev/guides/permanent.md"), b"permanent").unwrap();
        let result = executor
            .archive()
            .execute_archive_document("dev/guides/permanent.md")
            .unwrap();
        assert!(!result.event_appended);
        assert!(result.publications.is_empty());
        assert!(result.reference_changes.is_empty());
        assert!(result.planned_deletions.is_empty());
        assert_eq!(
            fs::read(repo.path().join("dev/guides/permanent.md")).unwrap(),
            b"permanent"
        );
        assert!(!repo.path().join("archive/guides/permanent.md").exists());
    }

    #[test]
    fn test_permanent_document_archive_ignores_a_differing_preexisting_copy() {
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"dev\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"dev/guides\"]\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("dev/guides")).unwrap();
        fs::write(repo.path().join("dev/guides/permanent.md"), b"permanent").unwrap();
        fs::create_dir_all(repo.path().join("archive/guides")).unwrap();
        fs::write(repo.path().join("archive/guides/permanent.md"), b"frozen").unwrap();
        let result = executor
            .archive()
            .execute_archive_document("dev/guides/permanent.md")
            .unwrap();
        assert!(!result.event_appended);
        assert!(result.publications.is_empty());
        assert_eq!(
            fs::read(repo.path().join("archive/guides/permanent.md")).unwrap(),
            b"frozen"
        );
        assert_eq!(
            fs::read(repo.path().join("dev/guides/permanent.md")).unwrap(),
            b"permanent"
        );
        assert!(stored_archive_executions(&executor.storage).is_empty());
    }

    #[test]
    fn test_container_execution_creates_marker_and_inverse_mirror_rerun_is_noop() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("fixtures/bundle/theme")).unwrap();
        fs::write(
            repo.path().join("fixtures/readme.md"),
            "[page](bundle/index.html) [external](https://example.com)",
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/index.html"),
            r#"<link href="theme/base.css"><img src="image.svg">"#,
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/theme/base.css"),
            r#"@import "nested.css"; body { background: url("../../image.png"); }"#,
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/bundle/theme/nested.css"), "a{}").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.svg"), "<svg/>").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.png"), b"png").unwrap();
        fs::write(repo.path().join("fixtures/data.csv"), "a,b\n1,2\n").unwrap();

        let mut epic = crate::test_utils::fixture_issue("Archive fixture".into(), String::new());
        epic.state = State::Done;
        let epic_type = taxonomy.type_at_level(1);
        let epic_namespace = taxonomy
            .label_associations
            .get(epic_type)
            .expect("taxonomy declares a membership namespace for level one");
        epic.labels = vec![
            format!("type:{epic_type}"),
            format!("{epic_namespace}:archive-fixture"),
        ];
        epic.documents = [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ]
        .into_iter()
        .map(document_reference)
        .collect();
        let id = epic.id.clone();
        seed_archive_issue_precondition(&executor.storage, epic);
        let first = executor.archive().execute_archive_container(&id).unwrap();
        let destination_root = format!("archive/{}-archive-fixture", id.short_id());
        assert_eq!(
            fs::read_to_string(repo.path().join(&destination_root).join(".jit-container")).unwrap(),
            format!("{id}\n")
        );
        assert!(first.publications.iter().any(|publication| {
            publication.source.is_none()
                && publication.destination == format!("{destination_root}/.jit-container")
        }));
        let issue = executor.storage.load_issue(&id).unwrap();
        assert!(issue
            .documents
            .iter()
            .all(|document| document.path.starts_with(&destination_root)));
        let event_count = stored_archive_executions(&executor.storage).len();

        let second = executor.archive().execute_archive_container(&id).unwrap();
        assert!(!second.event_appended);
        assert!(second.publications.is_empty());
        assert!(second.reference_changes.is_empty());
        assert_eq!(
            stored_archive_executions(&executor.storage).len(),
            event_count
        );
    }

    #[test]
    fn test_container_preview_and_execution_share_owner_relative_destinations_across_areas() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"workspace\"\nmanaged_paths = [\"workspace/active\", \"workspace/presentations\"]\nissue_scoped_areas = [\"workspace/active\", \"workspace/presentations\"]\npermanent_paths = []\narchive_root = \"workspace/archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();

        let mut container =
            crate::test_utils::fixture_issue("Owned archive layout".into(), String::new());
        container.state = State::Done;
        let container_type = taxonomy.type_at_level(1);
        let container_namespace = taxonomy
            .label_associations
            .get(container_type)
            .expect("taxonomy declares a membership namespace for level one");
        container.labels = vec![
            format!("type:{container_type}"),
            format!("{container_namespace}:platform-archive"),
        ];
        let owner = format!("{}-platform-archive", container.short_id());
        let sources = [
            format!("workspace/active/{owner}/plan.md"),
            format!("workspace/presentations/{owner}/vendor/reveal.js/reveal.js"),
            "workspace/presentations/another-owner/notes.md".to_string(),
        ];
        sources.iter().for_each(|source| {
            let path = repo.path().join(source);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        });
        container.documents = sources.iter().map(document_reference).collect();
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        let destination_root = format!("workspace/archive/{owner}");
        let expected = [
            (
                sources[0].as_str(),
                format!("{destination_root}/active/plan.md"),
            ),
            (
                sources[1].as_str(),
                format!("{destination_root}/presentations/vendor/reveal.js/reveal.js"),
            ),
            (
                sources[2].as_str(),
                format!("{destination_root}/presentations/another-owner/notes.md"),
            ),
        ];

        let preview = executor.archive().preview_archive_container(&id).unwrap();
        expected.iter().for_each(|(source, destination)| {
            let artifact = preview
                .artifacts()
                .iter()
                .find(|artifact| artifact.source() == *source)
                .unwrap();
            assert_eq!(artifact.destination(), Some(destination.as_str()));
        });

        let execution = executor.archive().execute_archive_container(&id).unwrap();
        expected.iter().for_each(|(source, destination)| {
            assert!(execution.publications.iter().any(|publication| {
                publication.source.as_deref() == Some(*source)
                    && publication.destination == *destination
            }));
            assert!(repo.path().join(destination).exists());
            assert!(!repo.path().join(source).exists());
        });
    }

    #[test]
    fn test_container_archive_moves_derived_documents_beside_linked_documents() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"workspace\"\nmanaged_paths = [\"workspace/active\"]\nissue_scoped_areas = [\"workspace/active\"]\npermanent_paths = []\narchive_root = \"workspace/archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        let mut container = crate::test_utils::fixture_issue("Derived".into(), String::new());
        container.state = State::Done;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        let owned = format!("workspace/active/{}", container.short_id());
        let linked = format!("{owned}/linked.md");
        let derived = format!("{owned}/nested/derived.md");
        [&linked, &derived].iter().for_each(|source| {
            let path = repo.path().join(source);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, source).unwrap();
        });
        container.documents = vec![document_reference(&linked)];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container.clone());
        let destination_root = format!("workspace/archive/{}", container.short_id());

        let preview = executor.archive().preview_archive_container(&id).unwrap();
        for (source, destination) in [
            (&linked, format!("{destination_root}/active/linked.md")),
            (
                &derived,
                format!("{destination_root}/active/nested/derived.md"),
            ),
        ] {
            let matching = preview
                .artifacts()
                .iter()
                .filter(|artifact| artifact.source() == source.as_str())
                .collect::<Vec<_>>();
            assert_eq!(matching.len(), 1, "{source} is listed once");
            assert_eq!(matching[0].destination(), Some(destination.as_str()));
        }

        executor.archive().execute_archive_container(&id).unwrap();
        assert!(!repo.path().join(&derived).exists());
        assert!(repo
            .path()
            .join(format!("{destination_root}/active/nested/derived.md"))
            .exists());
        assert_eq!(
            executor.storage.load_issue(&id).unwrap().documents.len(),
            1,
            "a derived document never becomes a link"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_container_relinks_only_terminal_inside_unpinned_owner() {
        fn git(repo: &std::path::Path, args: &[&str]) -> String {
            let output = std::process::Command::new("git")
                .args(args)
                .current_dir(repo)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        }

        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), b"shared").unwrap();
        crate::test_cli::init_git_repository(repo.path()).unwrap();
        git(repo.path(), &["add", "fixtures/root.md"]);
        git(repo.path(), &["commit", "-qm", "fixture"]);
        let commit = git(repo.path(), &["rev-parse", "HEAD"]);

        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![
            document_reference("fixtures/root.md"),
            document_reference_at("fixtures/root.md", commit),
        ];
        let container_id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        let mut outside = crate::test_utils::fixture_issue("Outside active".into(), String::new());
        outside.state = State::InProgress;
        outside.labels = vec![format!("type:{}", taxonomy.type_at_level(4))];
        outside.documents = vec![document_reference("fixtures/root.md")];
        let outside_id = outside.id.clone();
        seed_archive_issue_precondition(&executor.storage, outside);

        let result = executor
            .archive()
            .execute_archive_container(&container_id)
            .unwrap();
        let destination = format!("archive/{}/fixtures/root.md", container_id.short_id());
        let container = executor.storage.load_issue(&container_id).unwrap();
        let outside = executor.storage.load_issue(&outside_id).unwrap();
        assert_eq!(container.documents[0].path, destination);
        assert_eq!(container.documents[1].path, "fixtures/root.md");
        assert!(container.documents[1].commit.is_some());
        assert_eq!(outside.documents[0].path, "fixtures/root.md");
        assert!(repo.path().join("fixtures/root.md").exists());
        assert!(repo.path().join(&container.documents[0].path).exists());
        assert_eq!(result.reference_changes.len(), 1);
        assert!(result.planned_deletions.is_empty());
    }

    #[test]
    fn test_preview_container_is_deterministic_complete_and_non_mutating() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("fixtures/bundle/theme")).unwrap();
        fs::write(
            repo.path().join("fixtures/readme.md"),
            "[page](bundle/index.html) [external](https://example.com)",
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/index.html"),
            r#"<link href="theme/base.css"><img src="image.svg">"#,
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/theme/base.css"),
            r#"@import "nested.css"; body { background: url("../../image.png"); }"#,
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/bundle/theme/nested.css"), "a{}").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.svg"), "<svg/>").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.png"), b"png").unwrap();
        fs::write(repo.path().join("fixtures/data.csv"), "a,b\n1,2\n").unwrap();

        let mut epic = crate::test_utils::fixture_issue("Archive fixture".into(), String::new());
        epic.state = State::Done;
        let epic_type = taxonomy.type_at_level(1);
        let epic_namespace = taxonomy
            .label_associations
            .get(epic_type)
            .expect("taxonomy declares a membership namespace for level one");
        epic.labels = vec![
            format!("type:{epic_type}"),
            format!("{epic_namespace}:archive-fixture"),
        ];
        epic.documents = [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ]
        .into_iter()
        .map(document_reference)
        .collect();
        let id = epic.id.clone();
        seed_archive_issue_precondition(&executor.storage, epic);
        let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();
        let issues_before = executor.storage.list_issues().unwrap();

        let first = executor.archive().preview_archive_container(&id).unwrap();
        let second = executor.archive().preview_archive_container(&id).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.schema_version(), 1);
        assert!(first.eligible());
        for path in [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ] {
            assert!(first
                .artifacts()
                .iter()
                .any(|artifact| artifact.source() == path));
        }
        for path in [
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ] {
            assert_eq!(
                first
                    .artifacts()
                    .iter()
                    .find(|artifact| artifact.source() == path)
                    .unwrap()
                    .format(),
                None,
                "opaque root {path} must remain inventoried with null format"
            );
        }
        assert!(!repo.path().join("archive").exists());
        assert_eq!(executor.storage.list_issues().unwrap(), issues_before);
        assert_eq!(
            fs::read(repo.path().join(".jit/events.jsonl")).unwrap(),
            events_before
        );
    }

    #[test]
    fn test_preview_policy_statuses_remain_distinct_and_ineligible() {
        let (repo, executor) =
            crate::test_utils::setup_test_repo_with_config(&test_taxonomy().config_fragment())
                .unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let unconfigured = executor
            .archive()
            .preview_archive_document("root.csv")
            .unwrap();
        assert_eq!(
            unconfigured.policy_status(),
            crate::domain::artifact_plan::PolicyStatus::Unconfigured
        );
        assert!(!unconfigured.eligible());
        assert!(unconfigured
            .blockers()
            .iter()
            .any(|b| b.code == BlockerCode::PolicyUnconfigured));

        fs::write(
            repo.path().join(".jit/config.toml"),
            "[documentation]\nmanaged_paths = [\".\"]\n",
        )
        .unwrap();
        let incomplete = executor
            .archive()
            .preview_archive_document("root.csv")
            .unwrap();
        assert_eq!(
            incomplete.policy_status(),
            crate::domain::artifact_plan::PolicyStatus::Incomplete
        );
        assert!(!incomplete.eligible());
        assert!(incomplete
            .blockers()
            .iter()
            .any(|b| b.code == BlockerCode::PolicyIncomplete));
    }

    #[test]
    fn test_preview_every_partial_policy_is_incomplete_and_explicit_empty_is_configured() {
        let (repo, executor) =
            crate::test_utils::setup_test_repo_with_config(&test_taxonomy().config_fragment())
                .unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let fields = [
            "managed_paths = []",
            "permanent_paths = []",
            "archive_root = \"\"",
        ];

        for mask in 0..7 {
            let authored = fields
                .iter()
                .enumerate()
                .filter(|(index, _)| mask & (1 << index) != 0)
                .map(|(_, field)| *field)
                .collect::<Vec<_>>()
                .join("\n");
            fs::write(
                repo.path().join(".jit/config.toml"),
                format!("[documentation]\n{authored}\n"),
            )
            .unwrap();
            let plan = executor
                .archive()
                .preview_archive_document("root.csv")
                .unwrap();
            assert_eq!(
                plan.policy_status(),
                crate::domain::artifact_plan::PolicyStatus::Incomplete,
                "mask {mask} must not receive accessor defaults"
            );
            assert!(!plan.eligible());
        }

        fs::write(
            repo.path().join(".jit/config.toml"),
            "[documentation]\nmanaged_paths = []\npermanent_paths = []\narchive_root = \"\"\n",
        )
        .unwrap();
        let configured = executor
            .archive()
            .preview_archive_document("root.csv")
            .unwrap();
        assert_eq!(
            configured.policy_status(),
            crate::domain::artifact_plan::PolicyStatus::Configured
        );
        assert!(!configured.blockers().iter().any(|blocker| matches!(
            blocker.code,
            BlockerCode::PolicyIncomplete | BlockerCode::PolicyUnconfigured
        )));
    }

    /// Guards the deliberately unbounded closure expansion in
    /// [`crate::domain::artifact_discovery::expand_artifact_closure`].
    ///
    /// The only claim on `workspace/active/target.md` from outside its own
    /// document arrives along `workspace/notes/other.md` -> `HUB.md` ->
    /// `docs/relay.md` -> the target: a chain that leaves the development root,
    /// stays out for two hops, and re-enters. That claim is what makes the
    /// artifact a copy instead of a relocation, and the plan's own walk cannot
    /// see it, because the plan stops at `HUB.md` by design.
    ///
    /// Bounding evidence acquisition at the development root — an inviting
    /// optimisation, since the plan discards those descendants anyway — deletes
    /// the chain and with it the evidence, and this test fails with the
    /// artifact relocating. `docs/relay.md` is reachable only through the hub
    /// on purpose, so the test also fails if only the ownership walk is bounded.
    #[test]
    fn test_document_preview_copies_an_artifact_whose_only_outside_owner_arrives_through_an_out_of_root_hub(
    ) {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"workspace\"\nmanaged_paths = [\"workspace/active\"]\npermanent_paths = []\narchive_root = \"workspace/archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        for (path, content) in [
            ("workspace/active/target.md", "# Target\n"),
            ("workspace/notes/other.md", "[hub](../../HUB.md)\n"),
            ("HUB.md", "[relay](docs/relay.md)\n"),
            ("docs/relay.md", "[target](../workspace/active/target.md)\n"),
        ] {
            let file = repo.path().join(path);
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, content).unwrap();
        }
        for path in ["workspace/active/target.md", "workspace/notes/other.md"] {
            let mut issue =
                crate::test_utils::fixture_issue(format!("Owner of {path}"), String::new());
            issue.state = State::Done;
            issue.documents = vec![document_reference(path)];
            seed_archive_issue_precondition(&executor.storage, issue);
        }

        let plan = executor
            .archive()
            .preview_archive_document("workspace/active/target.md")
            .unwrap();
        let artifact = plan
            .artifacts()
            .iter()
            .find(|artifact| artifact.source() == "workspace/active/target.md")
            .unwrap();
        assert_eq!(
            artifact.action(),
            ArtifactAction::Copy,
            "an artifact another document still reaches must keep its source"
        );
        assert!(
            artifact.evidence().contains(&EvidenceCode::OutsideOwner),
            "the claim reaching the target through the hub is why it keeps it"
        );
        assert!(artifact.pending_deletions().is_empty());
        // The hub bounds the plan even though it does not bound the ownership
        // chain: nothing reached only through it joins the artifacts.
        assert!(plan
            .artifacts()
            .iter()
            .all(|artifact| artifact.source() == "workspace/active/target.md"));
    }

    #[test]
    fn test_preview_document_classifies_selected_roots_by_the_configured_development_root() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"workspace\"\nmanaged_paths = [\"workspace/active\"]\npermanent_paths = []\narchive_root = \"workspace/archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("scripts")).unwrap();
        fs::create_dir_all(repo.path().join("workspace/active")).unwrap();
        fs::create_dir_all(repo.path().join("workspace/scratch")).unwrap();
        fs::write(repo.path().join("scripts/install.sh"), "#!/bin/sh\n").unwrap();
        fs::write(repo.path().join("workspace/active/target.md"), "# Target\n").unwrap();
        fs::write(repo.path().join("workspace/scratch/notes.md"), "# Notes\n").unwrap();
        for path in [
            "scripts/install.sh",
            "workspace/active/target.md",
            "workspace/scratch/notes.md",
        ] {
            let mut issue =
                crate::test_utils::fixture_issue(format!("Owner of {path}"), String::new());
            issue.state = State::Done;
            issue.documents = vec![document_reference(path)];
            seed_archive_issue_precondition(&executor.storage, issue);
        }

        let outside = executor
            .archive()
            .preview_archive_document("scripts/install.sh")
            .unwrap();
        assert!(outside
            .blockers()
            .iter()
            .all(|blocker| blocker.code != BlockerCode::UnmanagedSelectedRoot));
        assert!(outside.eligible());
        let artifact = outside
            .artifacts()
            .iter()
            .find(|artifact| artifact.source() == "scripts/install.sh")
            .unwrap();
        assert_eq!(artifact.action(), ArtifactAction::Retain);
        assert_eq!(artifact.destination(), None);
        assert!(artifact.pending_deletions().is_empty());

        let inside = executor
            .archive()
            .preview_archive_document("workspace/scratch/notes.md")
            .unwrap();
        assert!(inside
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::UnmanagedSelectedRoot));
        assert!(!inside.eligible());
    }

    #[test]
    fn test_execute_document_uses_path_relative_to_a_non_default_development_root_and_relinks() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"workspace\"\nmanaged_paths = [\"workspace/active\"]\npermanent_paths = []\narchive_root = \"workspace/archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("scripts")).unwrap();
        fs::create_dir_all(repo.path().join("workspace/active")).unwrap();
        fs::create_dir_all(repo.path().join("workspace/scratch")).unwrap();
        fs::write(repo.path().join("scripts/install.sh"), "#!/bin/sh\n").unwrap();
        fs::write(repo.path().join("workspace/active/target.md"), "# Target\n").unwrap();
        fs::write(repo.path().join("workspace/scratch/notes.md"), "# Notes\n").unwrap();
        for path in [
            "scripts/install.sh",
            "workspace/active/target.md",
            "workspace/scratch/notes.md",
        ] {
            let mut issue =
                crate::test_utils::fixture_issue(format!("Owner of {path}"), String::new());
            issue.state = State::Done;
            issue.documents = vec![document_reference(path)];
            seed_archive_issue_precondition(&executor.storage, issue);
        }

        let result = executor
            .archive()
            .execute_archive_document("workspace/active/target.md")
            .unwrap();

        assert!(result.event_appended);
        assert!(repo
            .path()
            .join("workspace/archive/active/target.md")
            .exists());
        assert!(!repo.path().join("workspace/active/target.md").exists());
        assert!(executor.storage.list_issues().unwrap().iter().any(|issue| {
            issue.documents.iter().any(|document| {
                document.commit.is_none() && document.path == "workspace/archive/active/target.md"
            })
        }));
    }

    #[test]
    fn test_execute_document_reconciles_edited_residue_from_pre_cutover_event_destination() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"workspace\"\nmanaged_paths = [\"workspace/active\"]\npermanent_paths = []\narchive_root = \"workspace/archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("scripts")).unwrap();
        fs::create_dir_all(repo.path().join("workspace/active")).unwrap();
        fs::create_dir_all(repo.path().join("workspace/scratch")).unwrap();
        fs::write(repo.path().join("scripts/install.sh"), "#!/bin/sh\n").unwrap();
        fs::write(repo.path().join("workspace/active/target.md"), "# Target\n").unwrap();
        fs::write(repo.path().join("workspace/scratch/notes.md"), "# Notes\n").unwrap();
        for path in [
            "scripts/install.sh",
            "workspace/active/target.md",
            "workspace/scratch/notes.md",
        ] {
            let mut issue =
                crate::test_utils::fixture_issue(format!("Owner of {path}"), String::new());
            issue.state = State::Done;
            issue.documents = vec![document_reference(path)];
            seed_archive_issue_precondition(&executor.storage, issue);
        }
        let source = "workspace/active/target.md";
        let canonical_destination = "workspace/archive/active/target.md";
        let historical_destination = "workspace/archive/workspace/active/target.md";
        fs::create_dir_all(repo.path().join("workspace/archive/active")).unwrap();
        fs::write(repo.path().join(canonical_destination), b"# Target\n").unwrap();
        fs::write(repo.path().join(source), b"# Edited residue\n").unwrap();

        let historical_event = crate::test_utils::stamped_event(
            EventBody::draft_artifact_archive_executed(ArchiveExecution {
                target: PlanTarget::Document {
                    path: source.into(),
                },
                destination_root: "workspace/archive".into(),
                publications: vec![crate::domain::artifact_execution::ArchivePublication {
                    source: Some(source.into()),
                    destination: historical_destination.into(),
                    content_identity: crate::domain::artifact_plan::ContentIdentity::from_bytes(
                        b"# Target\n",
                    ),
                    adopted: false,
                }],
                reference_changes: Vec::new(),
                planned_deletions: Vec::new(),
                reconciling: false,
            }),
            "historical-archive",
            chrono::DateTime::UNIX_EPOCH,
        );
        let events_path = repo.path().join(".jit/events.jsonl");
        let EventBody::ArtifactArchiveExecuted(execution) = historical_event.body() else {
            panic!("historical event must be an archive execution");
        };
        let execution = execution.clone();
        let evidence_path =
            crate::repository_state::VirtualPath::archive_evidence_record(&execution.target);
        let evidence_file = repo
            .path()
            .join(".jit")
            .join(evidence_path.relative().as_str());
        fs::create_dir_all(evidence_file.parent().unwrap()).unwrap();
        fs::write(
            evidence_file,
            serde_json::to_vec(&ArchiveEvidenceRecord {
                target: execution.target.clone(),
                executions: vec![execution],
            })
            .unwrap(),
        )
        .unwrap();
        let mut event_bytes = fs::read(&events_path).unwrap_or_default();
        event_bytes.extend(serde_json::to_vec(&historical_event).unwrap());
        event_bytes.push(b'\n');
        fs::write(&events_path, event_bytes).unwrap();

        let result = executor.archive().execute_archive_document(source).unwrap();

        assert!(result.event_appended);
        assert!(result.reconciling);
        assert!(result.publications.iter().any(|publication| {
            publication.source.as_deref() == Some(source)
                && publication.destination == canonical_destination
                && publication.adopted
        }));
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::DeletionFailed));
        assert_eq!(
            fs::read(repo.path().join(canonical_destination)).unwrap(),
            b"# Target\n"
        );
        assert_eq!(
            fs::read(repo.path().join(source)).unwrap(),
            b"# Edited residue\n"
        );
        assert!(executor.storage.list_issues().unwrap().iter().any(|issue| {
            issue
                .documents
                .iter()
                .any(|document| document.commit.is_none() && document.path == canonical_destination)
        }));
        assert!(
            executor
                .storage
                .read_events()
                .unwrap()
                .contains(&historical_event),
            "the immutable historical event must retain its pre-cutover destination"
        );
    }

    #[test]
    fn test_preview_non_terminal_container_and_foreign_destination_are_blocked() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(repo.path().join("fixtures/bundle/theme")).unwrap();
        fs::write(
            repo.path().join("fixtures/readme.md"),
            "[page](bundle/index.html) [external](https://example.com)",
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/index.html"),
            r#"<link href="theme/base.css"><img src="image.svg">"#,
        )
        .unwrap();
        fs::write(
            repo.path().join("fixtures/bundle/theme/base.css"),
            r#"@import "nested.css"; body { background: url("../../image.png"); }"#,
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/bundle/theme/nested.css"), "a{}").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.svg"), "<svg/>").unwrap();
        fs::write(repo.path().join("fixtures/bundle/image.png"), b"png").unwrap();
        fs::write(repo.path().join("fixtures/data.csv"), "a,b\n1,2\n").unwrap();

        let mut epic = crate::test_utils::fixture_issue("Archive fixture".into(), String::new());
        epic.state = State::Done;
        let epic_type = taxonomy.type_at_level(1);
        let epic_namespace = taxonomy
            .label_associations
            .get(epic_type)
            .expect("taxonomy declares a membership namespace for level one");
        epic.labels = vec![
            format!("type:{epic_type}"),
            format!("{epic_namespace}:archive-fixture"),
        ];
        epic.documents = [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ]
        .into_iter()
        .map(document_reference)
        .collect();
        let id = epic.id.clone();
        seed_archive_issue_precondition(&executor.storage, epic);
        let mut issue = executor.storage.load_issue(&id).unwrap();
        issue.state = State::InProgress;
        seed_archive_issue_precondition(&executor.storage, issue);
        let destination = repo.path().join("archive").join(id.short_id());
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join(".jit-container"), "foreign-container-id\n").unwrap();

        let plan = executor.archive().preview_archive_container(&id).unwrap();
        assert!(!plan.eligible());
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::NonTerminalTarget));
        assert!(plan
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));

        fs::remove_file(destination.join(".jit-container")).unwrap();
        fs::write(destination.join("foreign.txt"), "not in the plan").unwrap();
        let markerless = executor.archive().preview_archive_container(&id).unwrap();
        assert!(markerless
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));
    }

    #[cfg(unix)]
    #[test]
    fn test_preview_blocks_symlink_artifact_without_following_it() {
        use std::os::unix::fs::symlink;

        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("real.md"), "[referent](secret.html)").unwrap();
        fs::write(repo.path().join("fixtures/secret.html"), "referent-only").unwrap();
        symlink(
            repo.path().join("real.md"),
            repo.path().join("fixtures/link.md"),
        )
        .unwrap();

        let plan = executor
            .archive()
            .preview_archive_document("fixtures/link.md")
            .unwrap();
        let artifact = plan
            .artifacts()
            .iter()
            .find(|artifact| artifact.source() == "fixtures/link.md")
            .unwrap();
        assert_eq!(plan.artifacts().len(), 1);
        assert!(artifact.edges().is_empty());
        assert_eq!(
            artifact.action(),
            crate::domain::artifact_plan::ArtifactAction::Block
        );
        assert!(artifact
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));

        let (regular_repo, regular_executor) =
            crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(regular_repo.path().join("fixtures")).unwrap();
        fs::create_dir(regular_repo.path().join("real-archive")).unwrap();
        fs::write(regular_repo.path().join("fixtures/root.md"), "root").unwrap();
        symlink(
            regular_repo.path().join("real-archive"),
            regular_repo.path().join("archive"),
        )
        .unwrap();
        let destination_symlink = regular_executor
            .archive()
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        assert!(destination_symlink.artifacts()[0]
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));

        let (owner_repo, owner_executor) =
            crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir_all(owner_repo.path().join("docs")).unwrap();
        fs::create_dir_all(owner_repo.path().join("fixtures")).unwrap();
        fs::create_dir_all(owner_repo.path().join("referents")).unwrap();
        fs::write(
            owner_repo.path().join("referents/outside.md"),
            "[selected](../fixtures/selected.md)",
        )
        .unwrap();
        fs::write(owner_repo.path().join("fixtures/selected.md"), "selected").unwrap();
        symlink(
            owner_repo.path().join("referents/outside.md"),
            owner_repo.path().join("docs/outside.md"),
        )
        .unwrap();
        let mut outside =
            crate::test_utils::fixture_issue("Active outside owner".into(), String::new());
        outside.state = State::InProgress;
        outside.documents = vec![document_reference("docs/outside.md")];
        seed_archive_issue_precondition(&owner_executor.storage, outside);

        let owner_plan = owner_executor
            .archive()
            .preview_archive_document("fixtures/selected.md")
            .unwrap();
        let selected = &owner_plan.artifacts()[0];
        assert_eq!(
            selected.action(),
            crate::domain::artifact_plan::ArtifactAction::Move
        );
        assert!(!selected.evidence().iter().any(|evidence| matches!(
            evidence,
            crate::domain::artifact_plan::EvidenceCode::OutsideOwner
                | crate::domain::artifact_plan::EvidenceCode::ActiveOwner
        )));
    }

    #[test]
    fn test_preview_container_directory_link_target_contributes_no_artifact_and_stays_eligible() {
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir_all(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/index.md"), "[section](section/)").unwrap();
        let taxonomy = test_taxonomy();
        let mut epic =
            crate::test_utils::fixture_issue("Directory link container".into(), String::new());
        epic.state = State::Done;
        let epic_type = taxonomy.type_at_level(1);
        let epic_namespace = taxonomy
            .label_associations
            .get(epic_type)
            .expect("taxonomy declares a membership namespace for level one");
        epic.labels = vec![
            format!("type:{epic_type}"),
            format!("{epic_namespace}:directory-link"),
        ];
        epic.documents = vec![document_reference("fixtures/index.md")];
        let id = epic.id.clone();
        seed_archive_issue_precondition(&executor.storage, epic);
        fs::create_dir_all(repo.path().join("fixtures/section")).unwrap();
        fs::write(repo.path().join("fixtures/section/child.md"), "child").unwrap();

        let plan = executor.archive().preview_archive_container(&id).unwrap();

        assert!(plan
            .artifacts()
            .iter()
            .all(|artifact| artifact.source() != "fixtures/section"));
        assert!(plan
            .blockers()
            .iter()
            .chain(
                plan.artifacts()
                    .iter()
                    .flat_map(crate::domain::artifact_plan::ArtifactPlanEntry::blockers)
            )
            .all(|blocker| blocker.code != BlockerCode::UnsupportedArtifactType));
        assert!(plan.eligible());
    }

    #[cfg(unix)]
    #[test]
    fn test_preview_container_symlinked_directory_link_target_still_blocks_as_symlink_artifact() {
        use std::os::unix::fs::symlink;

        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir_all(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/index.md"), "[section](section/)").unwrap();
        let taxonomy = test_taxonomy();
        let mut epic =
            crate::test_utils::fixture_issue("Directory link container".into(), String::new());
        epic.state = State::Done;
        let epic_type = taxonomy.type_at_level(1);
        let epic_namespace = taxonomy
            .label_associations
            .get(epic_type)
            .expect("taxonomy declares a membership namespace for level one");
        epic.labels = vec![
            format!("type:{epic_type}"),
            format!("{epic_namespace}:directory-link"),
        ];
        epic.documents = vec![document_reference("fixtures/index.md")];
        let id = epic.id.clone();
        seed_archive_issue_precondition(&executor.storage, epic);
        fs::create_dir_all(repo.path().join("referents/section")).unwrap();
        symlink(
            repo.path().join("referents/section"),
            repo.path().join("fixtures/section"),
        )
        .unwrap();

        let plan = executor.archive().preview_archive_container(&id).unwrap();

        assert!(plan
            .artifacts()
            .iter()
            .find(|artifact| artifact.source() == "fixtures/section")
            .expect("a symlinked target stays an artifact")
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));
        assert!(!plan.eligible());
    }

    // === Coupled retirement workflow (jit:45a140ae) ===

    fn state_change_count(executor: &CommandExecutor<JsonFileStorage>, id: &IssueId) -> usize {
        executor
            .storage
            .read_events()
            .unwrap()
            .iter()
            .filter(|event| matches!(event.body(), EventBody::IssueStateChanged { issue_id, to: State::Archived, .. } if issue_id == id))
            .count()
    }

    #[test]
    fn test_container_archival_retires_container_to_archived() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "# Root\n").unwrap();
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![document_reference("fixtures/root.md")];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        let result = executor.archive().execute_archive_container(&id).unwrap();
        assert!(result.event_appended);
        // The document was relocated out of its source into the archive mirror
        // (a container uses its own destination subtree, so assert on the source).
        assert!(!repo.path().join("fixtures/root.md").exists());
        assert!(!result.publications.is_empty());
        // The container is retired into Archived, recording its terminal origin.
        let container = executor.storage.load_issue(&id).unwrap();
        assert_eq!(container.state, State::Archived);
        assert_eq!(container.archived_from, Some(State::Done));
        // The retirement appended an issue_state_changed into Archived.
        assert_eq!(state_change_count(&executor, &id), 1);
    }

    #[test]
    fn test_rejected_container_archival_retires_from_rejected() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "# Root\n").unwrap();
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::Rejected;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![document_reference("fixtures/root.md")];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        executor.archive().execute_archive_container(&id).unwrap();
        let container = executor.storage.load_issue(&id).unwrap();
        assert_eq!(container.state, State::Archived);
        assert_eq!(container.archived_from, Some(State::Rejected));
    }

    #[test]
    fn test_non_terminal_container_archival_refused_with_guidance() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "# Root\n").unwrap();
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::InProgress;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![document_reference("fixtures/root.md")];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        let error = executor
            .archive()
            .execute_archive_container(&id)
            .expect_err("archiving a non-terminal container must be refused");
        let rendered = format!("{error:#}");
        assert!(
            rendered.contains("non-terminal-target") && rendered.contains("terminal container"),
            "diagnostic must explain the permitted next action: {rendered}"
        );
        // Nothing moved and the container is unchanged.
        assert!(repo.path().join("fixtures/root.md").exists());
        assert!(!repo.path().join("archive/fixtures/root.md").exists());
        let container = executor.storage.load_issue(&id).unwrap();
        assert_eq!(container.state, State::InProgress);
        assert_eq!(container.archived_from, None);
    }

    #[test]
    fn test_container_archival_rerun_is_idempotent_noop() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), "# Root\n").unwrap();
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![document_reference("fixtures/root.md")];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        executor.archive().execute_archive_container(&id).unwrap();
        // The container is now Archived-from-Done, still effectively terminal, so a
        // rerun is eligible and reconciles to a no-op rather than erroring.
        let rerun = executor.archive().execute_archive_container(&id).unwrap();
        assert!(!rerun.event_appended, "a fully-archived rerun is a no-op");
        let container = executor.storage.load_issue(&id).unwrap();
        assert_eq!(container.state, State::Archived);
        assert_eq!(container.archived_from, Some(State::Done));
        // The retirement transition is not re-emitted on the no-op rerun.
        assert_eq!(state_change_count(&executor, &id), 1);
    }

    #[test]
    fn test_container_archival_rerun_keeps_a_copy_an_archived_document_still_cites() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(
            repo.path().join("fixtures/root.md"),
            "See [b](/fixtures/b.md).\n",
        )
        .unwrap();
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![document_reference("fixtures/root.md")];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);
        fs::write(repo.path().join("fixtures/b.md"), "# B\n").unwrap();
        let mut container = executor.storage.load_issue(&id).unwrap();
        container
            .documents
            .push(document_reference("fixtures/b.md"));
        seed_archive_issue_precondition(&executor.storage, container);

        let first = executor.archive().execute_archive_container(&id).unwrap();
        assert_eq!(first.deleted_sources, vec!["fixtures/root.md"]);
        // The archived root still cites b at its source path, so b stays there
        // through every rerun.
        let rerun = executor.archive().execute_archive_container(&id).unwrap();
        assert!(!rerun.event_appended, "{:?}", rerun.deleted_sources);
        assert!(repo.path().join("fixtures/b.md").exists());
    }

    #[test]
    fn test_archive_reports_a_link_target_the_layout_cannot_represent_as_a_blocker() {
        let taxonomy = test_taxonomy();
        let config = format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            taxonomy.config_fragment()
        );
        let (repo, executor) = crate::test_utils::setup_test_repo_with_config(&config).unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(
            repo.path().join("fixtures/root.md"),
            "See [item](my_module::item).\n",
        )
        .unwrap();
        let mut container = crate::test_utils::fixture_issue("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec![format!("type:{}", taxonomy.type_at_level(1))];
        container.documents = vec![document_reference("fixtures/root.md")];
        let id = container.id.clone();
        seed_archive_issue_precondition(&executor.storage, container);

        let preview = executor.archive().preview_archive_container(&id).unwrap();
        assert!(preview
            .artifacts()
            .iter()
            .flat_map(|artifact| artifact.blockers())
            .any(|blocker| blocker.code == BlockerCode::RepositoryEscape));
        // Execution reaches the same plan rather than failing on the path.
        let error = executor
            .archive()
            .execute_archive_container(&id)
            .unwrap_err();
        assert!(error.to_string().contains("not eligible"), "{error}");
    }

    #[test]
    fn test_independently_archived_terminal_owner_does_not_block_shared_document() {
        // A document shared by a Done owner and a descendant independently
        // archived FROM a terminal state: the archived owner is effectively
        // terminal, so it is not an active owner and the document still moves.
        let (_repo, executor) = crate::test_utils::setup_test_repo_with_config(&format!(
            "{}\n[documentation]\ndevelopment_root = \"\"\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
            test_taxonomy().config_fragment()
        ))
        .unwrap();
        fs::create_dir(_repo.path().join("fixtures")).unwrap();
        fs::write(_repo.path().join("fixtures/root.md"), "shared doc").unwrap();
        let mut owner = crate::test_utils::fixture_issue("Owner 0".into(), String::new());
        owner.state = State::Done;
        let mut document = document_reference("fixtures/root.md");
        document.assets.push(crate::document::Asset {
            original_path: "asset.png".into(),
            resolved_path: Some("fixtures/asset.png".into()),
            asset_type: crate::document::AssetType::Missing,
            mime_type: None,
            content_hash: None,
            is_shared: false,
        });
        owner.documents = vec![document];

        seed_archive_issue_precondition(&executor.storage, owner);
        let mut archived_owner =
            crate::test_utils::fixture_issue("Independently archived".into(), String::new());
        archived_owner.state = State::Archived;
        archived_owner.archived_from = Some(State::Done);
        archived_owner.documents = vec![document_reference("fixtures/root.md")];
        seed_archive_issue_precondition(&executor.storage, archived_owner.clone());

        let plan = executor
            .archive()
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        let artifact = &plan.artifacts()[0];
        assert_eq!(artifact.action(), ArtifactAction::Move);
        assert!(!artifact
            .evidence()
            .contains(&crate::domain::artifact_plan::EvidenceCode::ActiveOwner));

        // Contrast: the same descendant archived from a NON-terminal state is an
        // active owner, so the shared source is retained instead of moved.
        let mut parked = executor.storage.load_issue(&archived_owner.id).unwrap();
        parked.archived_from = Some(State::InProgress);
        seed_archive_issue_precondition(&executor.storage, parked);
        let plan = executor
            .archive()
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        let artifact = &plan.artifacts()[0];
        assert!(artifact
            .evidence()
            .contains(&crate::domain::artifact_plan::EvidenceCode::ActiveOwner));
    }
}
