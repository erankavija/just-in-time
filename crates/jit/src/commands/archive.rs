//! Unified dependency-aware archive planning and coordinated execution.

use super::CommandExecutor;
use super::{with_mutation_session, SessionStep};
use crate::domain::artifact_classifier::{
    artifact_destination_root, artifact_mirror_destination, classification_facts_from_evidence,
    classify_artifacts, preferred_container_destination_root,
    resolve_container_destination as derive_container_destination, ArtifactClassificationInventory,
    ArtifactClassificationPolicy, ArtifactLocation, ArtifactLocationFacts,
};
use crate::domain::artifact_discovery::{
    discover_archive_artifacts as derive_archive_artifacts, expand_artifact_closure,
    ArtifactClosure, ArtifactClosureState, ArtifactEvidence, ArtifactEvidenceMap,
    ArtifactListingScope,
};
use crate::domain::artifact_execution::ArchiveExecutionResult;
use crate::domain::artifact_inventory::{
    inventory_explicit_roots, pinned_root_requests, ExplicitRootTarget, PinnedRootEvidence,
    PinnedRootEvidenceMap,
};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArchiveCandidates, ArtifactPlan, BlockerCode, PlanBlocker, PlanTarget,
};
#[cfg(test)]
use crate::domain::artifact_plan::{ArtifactAction, WarningCode};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{Event, Issue};
use crate::storage::{
    collect_artifact_classification_facts, discover_archive_artifacts,
    resolve_container_destination, validate_repo_relative_path, GitRevisionResolver, IssueStore,
    JsonFileStorage, RepositoryMutationSession, RepositoryStateStoreError,
};
use anyhow::{bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
enum ArchiveTarget<'a> {
    Document(&'a str),
    Container(&'a str),
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
) -> Vec<String> {
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

fn remap_archived_references(issues: &mut [Issue], destination_root: &str) {
    let prefix = format!("{}/", normalize_artifact_path(destination_root));
    issues.iter_mut().for_each(|issue| {
        issue
            .documents
            .iter_mut()
            .filter(|document| document.commit.is_none())
            .for_each(|document| {
                let normalized = normalize_artifact_path(&document.path);
                if let Some(source) = normalized.strip_prefix(&prefix) {
                    document.path = source.to_string();
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
    events: &[Event],
) {
    let covered = events
        .iter()
        .filter_map(|event| match event {
            Event::ArtifactArchiveExecuted {
                target: event_target,
                destination_root: event_root,
                publications,
                ..
            } if event_target == target && event_root == destination_root => Some(publications),
            _ => None,
        })
        .flatten()
        .map(|publication| {
            (
                publication.destination.clone(),
                publication.content_identity.clone(),
            )
        })
        .collect::<BTreeMap<_, _>>();

    locations.iter_mut().for_each(|(source_path, location)| {
        if let (ArtifactLocation::Regular(source), ArtifactLocation::Regular(destination)) =
            (&location.source, &location.destination)
        {
            let differs = source != destination;
            let mirror = artifact_mirror_destination(destination_root, source_path);
            // Only a prior archive record turns differing source bytes into an
            // edited residue; an unrelated occupied destination still blocks.
            if differs
                && covered
                    .get(&mirror)
                    .is_some_and(|identity| identity == destination)
            {
                location.source = ArtifactLocation::Regular(destination.clone());
            }
        }
    });
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Fully evaluate every effectively terminal configured non-leaf container
    /// without mutation.
    pub fn archive_candidates(&self) -> Result<ArchiveCandidates> {
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let issues = self.storage.list_issues()?;
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
    pub fn preview_archive_container(&self, id: &str) -> Result<ArtifactPlan> {
        self.plan_archive_target(ArchiveTarget::Container(id))
    }

    fn plan_archive_target(&self, target: ArchiveTarget<'_>) -> Result<ArtifactPlan> {
        let config = self.config_manager.load()?;
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let issues = self.storage.list_issues()?;
        let layout = self.require_layout()?;
        let repo_root = layout.worktree_root();
        let resolver = GitRevisionResolver::new(repo_root);

        let root_container_id = match target {
            ArchiveTarget::Document(_) => None,
            ArchiveTarget::Container(id) => Some(self.storage.resolve_issue_id(id)?),
        };
        let policy =
            ArtifactClassificationPolicy::from_documentation(config.documentation.as_ref());
        let target_for_layout = match &target {
            ArchiveTarget::Document(path) => PlanTarget::Document {
                path: normalize_artifact_path(path),
            },
            ArchiveTarget::Container(_) => PlanTarget::Container {
                id: root_container_id
                    .as_deref()
                    .context("container id was not resolved")?
                    .into(),
            },
        };
        let legacy_root = artifact_destination_root(&target_for_layout, &policy.archive_root);
        let (destination_root, destination_conflicts) = match root_container_id.as_deref() {
            Some(container_id) => {
                let issue = issues
                    .iter()
                    .find(|issue| issue.id == container_id)
                    .context("resolved archive container is missing from issue inventory")?;
                let preferred_root =
                    preferred_container_destination_root(issue, &hierarchy, &policy.archive_root);
                if policy.archive_root.is_empty() {
                    (preferred_root, Vec::new())
                } else {
                    let resolved = resolve_container_destination(
                        &self.storage,
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
        remap_archived_references(&mut inventory_issues, &destination_root);
        let explicit_target = match &target {
            ArchiveTarget::Document(path) => ExplicitRootTarget::Document(path),
            ArchiveTarget::Container(_) => ExplicitRootTarget::Container(
                root_container_id
                    .as_deref()
                    .context("container id was not resolved")?,
            ),
        };
        let pinned = inventory_issues_pinned_evidence(
            pinned_root_requests(&inventory_issues, &hierarchy, explicit_target)?,
            &resolver,
        );
        let inventory =
            inventory_explicit_roots(&inventory_issues, &hierarchy, explicit_target, &pinned)?;
        let (discovered, embedded_owners) =
            discover_archive_artifacts(&self.storage, inventory, &issues)?;
        let plan_target = discovered.target;
        let artifacts = discovered.artifacts;
        let mut blockers = discovered.blockers;
        blockers.extend(
            destination_conflicts
                .into_iter()
                .map(|root| PlanBlocker::new(BlockerCode::DestinationConflict, Some(root))),
        );

        if let Some(container_id) = root_container_id.as_deref() {
            // Coupled retirement (`jit:45a140ae`): artifact archival requires an
            // effectively terminal container (Done/Rejected, or already Archived
            // from one of those for an idempotent rerun). An Archived container
            // retired from a non-terminal state, or any active state, is blocked.
            if issues
                .iter()
                .find(|issue| issue.id == container_id)
                .is_some_and(|issue| !issue.is_effectively_terminal())
            {
                blockers.push(PlanBlocker::new(
                    BlockerCode::NonTerminalTarget,
                    None::<String>,
                ));
            }
        }

        let mut facts = collect_artifact_classification_facts(
            &self.storage,
            &plan_target,
            &destination_root,
            &artifacts,
            &policy,
            embedded_owners,
        )?;
        let archive_events = self.storage.read_artifact_archive_events()?;
        apply_recorded_residue_identities(
            &mut facts.locations,
            &plan_target,
            &destination_root,
            &archive_events,
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

fn capture_more(
    session: &mut dyn RepositoryMutationSession,
    image: &mut crate::repository_state::RepositoryImage,
    paths: impl IntoIterator<Item = crate::repository_state::VirtualPath>,
    listings: impl IntoIterator<Item = crate::repository_state::VirtualPath>,
) -> Result<bool> {
    let paths = paths.into_iter().collect::<BTreeSet<_>>();
    let listings = listings.into_iter().collect::<BTreeSet<_>>();
    if paths.iter().all(|path| {
        image
            .capture_spec()
            .paths()
            .any(|captured| captured == path)
    }) && listings
        .iter()
        .all(|path| image.capture_spec().listings().contains(path))
    {
        return Ok(true);
    }
    let mut spec = image.capture_spec().clone();
    spec.discover_paths(paths)?;
    for listing in listings {
        spec.discover_listing(listing)?;
    }
    match session.capture(spec) {
        Ok(next) if next.has_stable_overlap(image) => {
            *image = next;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(RepositoryStateStoreError::RetryableConflict { .. }) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn capture_pinned_more(
    session: &mut dyn RepositoryMutationSession,
    image: &mut crate::repository_state::RepositoryImage,
    requests: &BTreeSet<(String, String)>,
) -> Result<bool> {
    if requests
        .iter()
        .all(|request| image.pinned_evidence().contains_key(request))
    {
        return Ok(true);
    }
    let mut spec = image.capture_spec().clone();
    for (revision, path) in requests {
        spec.discover_pinned(revision.clone(), path.clone())?;
    }
    match session.capture(spec) {
        Ok(next) if next.has_stable_overlap(image) => {
            *image = next;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(RepositoryStateStoreError::RetryableConflict { .. }) => Ok(false),
        Err(error) => Err(error.into()),
    }
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
        RepositoryEntry::File { bytes, .. } => ArtifactEvidence::File(bytes.clone()),
        RepositoryEntry::Symlink { .. } => ArtifactEvidence::Symlink,
        RepositoryEntry::Unsupported { .. } => ArtifactEvidence::Unsupported,
        RepositoryEntry::Directory { .. } => {
            let entries = match scope {
                ArtifactListingScope::MetadataOnly => Vec::new(),
                ArtifactListingScope::ImmediateChildren => image
                    .listing_fingerprints()
                    .get(&vpath)
                    .context("archive directory listing was not captured")?
                    .children()
                    .keys()
                    .map(|name| normalize_artifact_path(&format!("{path}/{name}")))
                    .collect(),
                ArtifactListingScope::RecursiveFiles => image
                    .entries()
                    .iter()
                    .filter(|(candidate, entry)| {
                        image.layout().resolve(candidate).is_ok_and(|physical| {
                            physical.starts_with(image.layout().worktree_root().join(path))
                        }) && !matches!(
                            entry,
                            RepositoryEntry::Directory { .. } | RepositoryEntry::Absent
                        )
                    })
                    .map(|(candidate, _)| {
                        Ok(image
                            .layout()
                            .resolve(candidate)?
                            .strip_prefix(image.layout().worktree_root())?
                            .to_string_lossy()
                            .replace('\\', "/"))
                    })
                    .collect::<Result<Vec<_>>>()?,
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
        if validate_repo_relative_path(path).is_err() {
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
        if !capture_more(session, image, requested.values().cloned(), [])? {
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
    mut image: crate::repository_state::RepositoryImage,
    root: &str,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    use crate::repository_state::RepositoryEntry;
    let root_path = image_path(image.layout(), root)?;
    if !image.entries().contains_key(&root_path)
        && !capture_more(session, &mut image, [root_path.clone()], [])?
    {
        return Ok(None);
    }
    match image.entry(&root_path)? {
        RepositoryEntry::Absent
        | RepositoryEntry::File { .. }
        | RepositoryEntry::Symlink { .. }
        | RepositoryEntry::Unsupported { .. } => return Ok(Some(image)),
        RepositoryEntry::Directory { .. } => {}
    }
    if !image.listing_fingerprints().contains_key(&root_path)
        && !capture_more(session, &mut image, [], [root_path.clone()])?
    {
        return Ok(None);
    }
    let root_physical = image.layout().resolve(&root_path)?;
    loop {
        let mut paths = BTreeSet::new();
        for (directory, listing) in image.listing_fingerprints() {
            if image
                .layout()
                .resolve(directory)?
                .starts_with(&root_physical)
            {
                for name in listing.children().keys() {
                    let child = image
                        .layout()
                        .classify_and_canonicalize(image.layout().resolve(directory)?.join(name))?;
                    if !image.entries().contains_key(&child) {
                        paths.insert(child);
                    }
                }
            }
        }
        if !paths.is_empty() {
            if !capture_more(session, &mut image, paths, [])? {
                return Ok(None);
            }
            continue;
        }
        let listings = image
            .entries()
            .iter()
            .filter(|(path, entry)| {
                matches!(entry, RepositoryEntry::Directory { .. })
                    && image
                        .layout()
                        .resolve(path)
                        .is_ok_and(|physical| physical.starts_with(&root_physical))
                    && !image.listing_fingerprints().contains_key(path)
            })
            .map(|(path, _)| path.clone())
            .collect::<BTreeSet<_>>();
        if listings.is_empty() {
            return Ok(Some(image));
        }
        if !capture_more(session, &mut image, [], listings)? {
            return Ok(None);
        }
    }
}

impl CommandExecutor<JsonFileStorage> {
    fn capture_archive_plan(
        &self,
        session: &mut dyn RepositoryMutationSession,
        target: ArchiveTarget<'_>,
    ) -> Result<Option<(crate::repository_state::RepositoryImage, ArtifactPlan)>> {
        let Some(mut image) = self.capture_proposed_base_without_documents(session)? else {
            return Ok(None);
        };
        let config = crate::repository_state::assemble_config(&image)?;
        let namespaces = crate::config_manager::namespaces_from_config(&config);
        let hierarchy = crate::repository_state::hierarchy_config(&namespaces);
        let issues = super::captured_active_issues(&image)?;
        let root_id = match target {
            ArchiveTarget::Document(_) => None,
            ArchiveTarget::Container(id) => Some(super::resolve_issue_from_capture(&issues, id)?),
        };
        let policy =
            ArtifactClassificationPolicy::from_documentation(config.documentation.as_ref());
        let plan_target = match target {
            ArchiveTarget::Document(path) => PlanTarget::Document {
                path: normalize_artifact_path(path),
            },
            ArchiveTarget::Container(_) => PlanTarget::Container {
                id: root_id
                    .as_deref()
                    .context("container id was not resolved")?
                    .into(),
            },
        };
        let legacy = artifact_destination_root(&plan_target, &policy.archive_root);
        let mut conflicts = Vec::new();
        let destination = if let Some(id) = root_id.as_deref() {
            let issue = issues
                .iter()
                .find(|issue| issue.id == id)
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
                    if !capture_more(session, &mut image, [], [archive_path.clone()])? {
                        return Ok(None);
                    }
                    image
                        .listing_fingerprints()
                        .get(&archive_path)
                        .context("archive root listing was not captured")?
                        .children()
                        .keys()
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
                if !capture_more(session, &mut image, child_paths, [])? {
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
                if !capture_more(session, &mut image, markers, [])? {
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
        remap_archived_references(&mut inventory_issues, &destination);
        let explicit = match target {
            ArchiveTarget::Document(path) => ExplicitRootTarget::Document(path),
            ArchiveTarget::Container(_) => ExplicitRootTarget::Container(
                root_id
                    .as_deref()
                    .context("container id was not resolved")?,
            ),
        };
        let pinned_requests = pinned_root_requests(&inventory_issues, &hierarchy, explicit)?;
        if !capture_pinned_more(session, &mut image, &pinned_requests)? {
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
        let mut state = ArtifactClosureState::new(roots);
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
            derive_archive_artifacts(inventory, &issues, &evidence, &parsed)?;
        let mut blockers = discovered.blockers;
        blockers.extend(
            conflicts
                .into_iter()
                .map(|root| PlanBlocker::new(BlockerCode::DestinationConflict, Some(root))),
        );
        if let Some(id) = root_id.as_deref() {
            if issues
                .iter()
                .find(|issue| issue.id == id)
                .is_some_and(|issue| !issue.is_effectively_terminal())
            {
                blockers.push(PlanBlocker::new(
                    BlockerCode::NonTerminalTarget,
                    None::<String>,
                ));
            }
        }
        let artifacts = discovered.artifacts;
        let mut paths = artifacts
            .iter()
            .filter(|a| !a.version().is_pinned())
            .flat_map(|a| {
                [
                    a.source().to_string(),
                    artifact_mirror_destination(&destination, a.source()),
                ]
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
        apply_recorded_residue_identities(
            &mut facts.locations,
            &plan_target,
            &destination,
            &crate::repository_state::captured_archive_events(&image)?,
        );
        let plan = classify_artifacts(
            ArtifactClassificationInventory::new(plan_target, artifacts, blockers)
                .with_destination_root(destination),
            policy,
            facts,
        )?;
        Ok(Some((image, plan)))
    }

    /// Execute a freshly recomputed document archive plan under one write guard.
    pub fn execute_archive_document(&self, path: &str) -> Result<ArchiveExecutionResult> {
        self.execute_archive_transaction(ArchiveTarget::Document(path))
    }

    /// Execute a freshly recomputed container archive plan under one write guard.
    pub fn execute_archive_container(&self, id: &str) -> Result<ArchiveExecutionResult> {
        self.execute_archive_transaction(ArchiveTarget::Container(id))
    }

    fn execute_archive_transaction(
        &self,
        target: ArchiveTarget<'_>,
    ) -> Result<ArchiveExecutionResult> {
        let layout = self.require_layout()?;
        let context = crate::repository_state::MutationContext::production();
        with_mutation_session(&self.storage, &layout, "archive execution", |session| {
            let Some((image, plan)) = self.capture_archive_plan(session, target)? else {
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
                crate::repository_state::finalize_archive_execution(&image, &context, &plan)?;
            if materialization.delta().actions().is_empty() {
                return Ok(SessionStep::Done(result));
            }
            Ok(SessionStep::Apply(materialization, result))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, State};
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::storage::JsonFileStorage;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    fn executor(repo: &TempDir, storage: JsonFileStorage) -> CommandExecutor<JsonFileStorage> {
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        CommandExecutor::new(storage).with_layout(layout)
    }

    /// Seed an exact file-backed issue precondition without exercising command mutation behavior.
    fn seed_archive_issue_precondition(storage: &JsonFileStorage, issue: Issue) {
        let index_path = storage.root().join("index.json");
        let mut index = crate::repository_state::RepositoryIndex::parse(
            &fs::read(&index_path).expect("initialized repository index"),
        )
        .expect("valid repository index");
        index.all_ids.push(issue.id.clone());
        index.all_ids.sort();
        index.all_ids.dedup();
        index.deleted_ids.retain(|id| id != &issue.id);
        fs::write(
            storage
                .root()
                .join("issues")
                .join(format!("{}.json", issue.id)),
            crate::repository_state::serialize_issue(&issue).expect("serialize fixture issue"),
        )
        .unwrap();
        fs::write(
            index_path,
            index.to_pretty_bytes().expect("serialize fixture index"),
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
            |id: &str, state: State, archived_from: Option<State>, issue_type: Option<&str>| {
                let mut issue = crate::domain::types::fixture_issue(id.to_string(), String::new());
                issue.id = id.to_string();
                issue.state = state;
                issue.archived_from = archived_from;
                issue.labels = issue_type
                    .map(|kind| vec![format!("type:{kind}")])
                    .unwrap_or_default();
                issue
            };
        let issues = vec![
            issue("done-container", State::Done, None, Some("portfolio")),
            issue(
                "rejected-container",
                State::Rejected,
                None,
                Some("portfolio"),
            ),
            issue(
                "active-container",
                State::InProgress,
                None,
                Some("portfolio"),
            ),
            // Archived from a terminal state is effectively terminal, so it is a
            // reconcilable candidate — same as the direct archive path accepts.
            issue(
                "archived-from-done",
                State::Archived,
                Some(State::Done),
                Some("portfolio"),
            ),
            // Archived from a non-terminal state (and a legacy record with no
            // recorded origin) is not effectively terminal, so it is excluded.
            issue(
                "archived-from-active",
                State::Archived,
                Some(State::InProgress),
                Some("portfolio"),
            ),
            issue("archived-legacy", State::Archived, None, Some("portfolio")),
            issue("done-leaf", State::Done, None, Some("unit")),
            issue("done-unknown", State::Done, None, Some("epic")),
            issue("done-untyped", State::Done, None, None),
        ];

        assert_eq!(
            effectively_terminal_container_ids(&issues, &hierarchy),
            vec!["archived-from-done", "done-container", "rejected-container"]
        );
    }

    fn executable_document_repo(
        owner_count: usize,
        content: &str,
    ) -> (TempDir, CommandExecutor<JsonFileStorage>, Vec<String>) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), content).unwrap();
        let ids = (0..owner_count)
            .map(|index| {
                let mut issue =
                    crate::domain::types::fixture_issue(format!("Owner {index}"), String::new());
                issue.state = State::Done;
                let mut document = DocumentReference::new("fixtures/root.md".into());
                document.assets.push(crate::document::Asset {
                    original_path: "asset.png".into(),
                    resolved_path: Some("fixtures/asset.png".into()),
                    asset_type: crate::document::AssetType::Missing,
                    mime_type: None,
                    content_hash: None,
                    is_shared: false,
                });
                issue.documents = vec![document];
                let id = issue.id.clone();
                seed_archive_issue_precondition(&storage, issue);
                id
            })
            .collect();
        let executor = executor(&repo, storage);
        (repo, executor, ids)
    }

    #[test]
    fn test_execute_document_commits_event_before_delete_and_noop_rerun_is_stable() {
        let (repo, executor, ids) = executable_document_repo(1, "# Root\n");
        let first = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(first.event_appended);
        assert_eq!(first.deleted_sources, vec!["fixtures/root.md"]);
        assert!(!repo.path().join("fixtures/root.md").exists());
        assert_eq!(
            fs::read(repo.path().join("archive/fixtures/root.md")).unwrap(),
            b"# Root\n"
        );
        let issue = executor.storage.load_issue(&ids[0]).unwrap();
        assert_eq!(issue.documents[0].path, "archive/fixtures/root.md");
        assert!(issue.documents[0].assets.is_empty());
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );

        let second = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(!second.event_appended);
        assert!(second.publications.is_empty());
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_execute_non_directory_archive_root_is_blocked_without_descending() {
        let (repo, executor, _) = executable_document_repo(1, "source");
        fs::write(repo.path().join("archive"), b"occupied").unwrap();

        assert!(executor
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert_eq!(fs::read(repo.path().join("archive")).unwrap(), b"occupied");
        assert!(repo.path().join("fixtures/root.md").exists());
    }

    #[test]
    fn test_execute_archive_root_nested_under_data_uses_data_paths() {
        let (repo, executor, _) = executable_document_repo(1, "source");
        fs::write(
            repo.path().join(".jit/config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \".jit/archive\"\n",
        )
        .unwrap();

        executor
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

        let (repo, executor, _) = executable_document_repo(1, "source");
        fs::remove_file(repo.path().join("fixtures/root.md")).unwrap();
        fs::write(repo.path().join("outside.md"), b"outside").unwrap();
        symlink("../outside.md", repo.path().join("fixtures/root.md")).unwrap();

        assert!(executor
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

        let (repo, executor, id) = configured_repo();
        fs::write(
            repo.path().join(".jit/config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"safe/archive\"\n\n[type_hierarchy]\ntypes = { epic = 1, task = 2 }\n[type_hierarchy.label_associations]\nepic = \"epic\"\n",
        )
        .unwrap();
        fs::create_dir(repo.path().join("outside")).unwrap();
        symlink("outside", repo.path().join("safe")).unwrap();

        assert!(executor.execute_archive_container(&id).is_err());
        assert!(fs::read_dir(repo.path().join("outside"))
            .unwrap()
            .next()
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn test_execute_preserves_executable_source_mode() {
        use std::os::unix::fs::PermissionsExt;

        let (repo, executor, _) = executable_document_repo(1, "source");
        fs::set_permissions(
            repo.path().join("fixtures/root.md"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();

        executor
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
        let (_repo, executor, _) = executable_document_repo(0, "guard contention");
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
                .send(executor.execute_archive_document("fixtures/root.md"))
                .unwrap();
        });
        assert!(done_receiver
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err());
        release_sender.send(()).unwrap();
        holder.join().unwrap();
        assert!(done_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap()
            .is_ok());
        worker.join().unwrap();
    }

    #[test]
    fn test_execute_document_allows_managed_zero_owner_and_refuses_active_owner() {
        let (repo, executor, _) = executable_document_repo(0, "zero owner");
        let result = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(result
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::NoOwner));
        assert!(repo.path().join("archive/fixtures/root.md").exists());

        let (active_repo, active_executor, ids) = executable_document_repo(1, "active");
        let mut issue = active_executor.storage.load_issue(&ids[0]).unwrap();
        issue.state = State::InProgress;
        seed_archive_issue_precondition(&active_executor.storage, issue);
        assert!(active_executor
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert!(active_repo.path().join("fixtures/root.md").exists());
        assert!(!active_repo.path().join("archive/fixtures/root.md").exists());
    }

    #[test]
    fn test_archive_event_reader_accepts_final_and_multiple_nested_brace_torn_prefixes() {
        let (repo, executor, _) = executable_document_repo(0, "event tails");
        let events_path = repo.path().join(".jit/events.jsonl");
        let nested_torn = concat!(
            "{\"type\":\"artifact_archive_executed\",",
            "\"target\":{\"kind\":\"document\",\"path\":\"fixtures/root.md\"}"
        );
        fs::write(&events_path, nested_torn).unwrap();
        assert!(executor
            .storage
            .read_artifact_archive_events()
            .unwrap()
            .is_empty());

        let valid = Event::draft_artifact_archive_executed(
            PlanTarget::Document {
                path: "fixtures/root.md".into(),
            },
            "archive".into(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            true,
        );
        let valid_json = serde_json::to_string(&valid).unwrap();
        fs::write(&events_path, format!("{nested_torn}\n{valid_json}\n")).unwrap();
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );
        fs::write(
            &events_path,
            format!("{nested_torn}\n{{\"type\":\"issue_created\"\n{nested_torn}\n{valid_json}\n"),
        )
        .unwrap();
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );
        fs::write(&events_path, format!("{{not-json}}\n{valid_json}\n")).unwrap();
        assert!(executor.storage.read_artifact_archive_events().is_err());

        let retired_document_event = concat!(
            "{\"type\":\"document_",
            "archived\",\"id\":\"historical\",",
            "\"timestamp\":\"2025-01-01T00:00:00Z\",",
            "\"source\":\"dev/old.md\",\"destination\":\"dev/archive/old.md\",",
            "\"category\":\"design\",\"issues_updated\":1}"
        );
        fs::write(
            &events_path,
            format!("{retired_document_event}\n{valid_json}\n"),
        )
        .unwrap();
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1,
            "unrelated historical event variants must not block archive recovery"
        );

        fs::write(
            &events_path,
            format!("{{\"type\":\"artifact_archive_executed\"}}\n{valid_json}\n"),
        )
        .unwrap();
        assert!(executor.storage.read_artifact_archive_events().is_err());

        fs::write(&events_path, format!("{{}}\n{valid_json}\n")).unwrap();
        assert!(executor.storage.read_artifact_archive_events().is_err());
    }

    #[test]
    fn test_execution_preserves_positive_relative_and_root_relative_multi_edge_layout() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"shared\"]\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::create_dir(repo.path().join("shared")).unwrap();
        fs::write(
            repo.path().join("fixtures/root.md"),
            "[child](child.md) [shared](/shared/global.md)",
        )
        .unwrap();
        fs::write(repo.path().join("fixtures/child.md"), "child").unwrap();
        fs::write(repo.path().join("shared/global.md"), "shared").unwrap();
        let executor = executor(&repo, storage);

        let result = executor
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
        let (repo, executor, ids) = executable_document_repo(1, "identical");
        fs::create_dir(repo.path().join("archive")).unwrap();
        fs::create_dir(repo.path().join("archive/fixtures")).unwrap();
        fs::write(repo.path().join("archive/fixtures/root.md"), b"identical").unwrap();
        let result = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(result.publications.iter().any(|publication| {
            publication.destination == "archive/fixtures/root.md" && publication.adopted
        }));
        assert_eq!(
            executor.storage.load_issue(&ids[0]).unwrap().documents[0].path,
            "archive/fixtures/root.md"
        );

        let (conflict_repo, conflict_executor, _) = executable_document_repo(1, "source bytes");
        fs::create_dir_all(conflict_repo.path().join("archive/fixtures")).unwrap();
        fs::write(
            conflict_repo.path().join("archive/fixtures/root.md"),
            b"foreign bytes",
        )
        .unwrap();
        assert!(conflict_executor
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
    fn test_publication_only_execution_records_one_event() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(repo.path().join("docs")).unwrap();
        fs::write(repo.path().join("docs/permanent.md"), b"permanent").unwrap();
        let executor = executor(&repo, storage);
        let result = executor
            .execute_archive_document("docs/permanent.md")
            .unwrap();
        assert!(result.event_appended);
        assert_eq!(result.publications.len(), 1);
        assert!(result.reference_changes.is_empty());
        assert!(result.planned_deletions.is_empty());
        assert!(repo.path().join("docs/permanent.md").exists());
        assert!(repo.path().join("archive/docs/permanent.md").exists());
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_preexisting_identical_permanent_copy_is_adopted_once_without_deletion() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir_all(repo.path().join("archive/docs")).unwrap();
        fs::create_dir(repo.path().join("docs")).unwrap();
        fs::write(repo.path().join("docs/permanent.md"), b"permanent").unwrap();
        fs::write(repo.path().join("archive/docs/permanent.md"), b"permanent").unwrap();
        let executor = executor(&repo, storage);

        let first = executor
            .execute_archive_document("docs/permanent.md")
            .unwrap();
        assert!(first.event_appended);
        assert!(first.planned_deletions.is_empty());
        assert!(first.deleted_sources.is_empty());
        assert!(first.publications.iter().any(|publication| {
            publication.destination == "archive/docs/permanent.md" && publication.adopted
        }));
        assert!(repo.path().join("docs/permanent.md").exists());
        let second = executor
            .execute_archive_document("docs/permanent.md")
            .unwrap();
        assert!(!second.event_appended);
        assert!(second.publications.is_empty());
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn test_container_execution_creates_marker_and_inverse_mirror_rerun_is_noop() {
        let (repo, executor, id) = configured_repo();
        let first = executor.execute_archive_container(&id).unwrap();
        let destination_root = format!("archive/{}-archive-fixture", &id[..8]);
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
        let event_count = executor
            .storage
            .read_artifact_archive_events()
            .unwrap()
            .len();

        let second = executor.execute_archive_container(&id).unwrap();
        assert!(!second.event_appended);
        assert!(second.publications.is_empty());
        assert!(second.reference_changes.is_empty());
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            event_count
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

        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n\n[type_hierarchy]\ntypes = { epic = 1, task = 2 }\n[type_hierarchy.label_associations]\nepic = \"epic\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), b"shared").unwrap();
        git(repo.path(), &["init", "-q"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test"]);
        git(repo.path(), &["add", "fixtures/root.md"]);
        git(repo.path(), &["commit", "-qm", "fixture"]);
        let commit = git(repo.path(), &["rev-parse", "HEAD"]);

        let mut container = crate::domain::types::fixture_issue("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec!["type:epic".into()];
        container.documents = vec![
            DocumentReference::new("fixtures/root.md".into()),
            DocumentReference::at_commit("fixtures/root.md".into(), commit),
        ];
        let container_id = container.id.clone();
        seed_archive_issue_precondition(&storage, container);
        let mut outside =
            crate::domain::types::fixture_issue("Outside active".into(), String::new());
        outside.state = State::InProgress;
        outside.labels = vec!["type:task".into()];
        outside.documents = vec![DocumentReference::new("fixtures/root.md".into())];
        let outside_id = outside.id.clone();
        seed_archive_issue_precondition(&storage, outside);

        let executor = executor(&repo, storage);
        let result = executor.execute_archive_container(&container_id).unwrap();
        let destination = format!("archive/{}-container/fixtures/root.md", &container_id[..8]);
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

    fn configured_repo() -> (TempDir, CommandExecutor<JsonFileStorage>, String) {
        let repo = TempDir::new().unwrap();
        let jit = repo.path().join(".jit");
        let storage = JsonFileStorage::new(&jit);
        fs::create_dir_all(&jit).unwrap();
        fs::write(
            jit.join("config.toml"),
            r#"
[documentation]
managed_paths = ["fixtures"]
permanent_paths = ["docs"]
archive_root = "archive"

[type_hierarchy]
types = { epic = 1, task = 2 }
[type_hierarchy.label_associations]
epic = "epic"
"#,
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();

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

        let mut epic = crate::domain::types::fixture_issue("Archive fixture".into(), String::new());
        epic.state = State::Done;
        epic.labels = vec!["type:epic".into(), "epic:archive-fixture".into()];
        epic.documents = [
            "fixtures/readme.md",
            "fixtures/bundle/index.html",
            "fixtures/bundle/theme/base.css",
            "fixtures/data.csv",
            "fixtures/bundle/image.png",
            "fixtures/bundle/image.svg",
        ]
        .into_iter()
        .map(|path| DocumentReference::new(path.into()))
        .collect();
        let id = epic.id.clone();
        seed_archive_issue_precondition(&storage, epic);
        let executor = executor(&repo, storage);
        (repo, executor, id)
    }

    #[test]
    fn test_preview_container_is_deterministic_complete_and_non_mutating() {
        let (repo, executor, id) = configured_repo();
        let events_before = fs::read(repo.path().join(".jit/events.jsonl")).unwrap();
        let issues_before = executor.storage.list_issues().unwrap();

        let first = executor.preview_archive_container(&id).unwrap();
        let second = executor.preview_archive_container(&id).unwrap();

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
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root().join("issues")).unwrap();
        fs::write(
            storage.root().join("index.json"),
            crate::repository_state::RepositoryIndex::default()
                .to_pretty_bytes()
                .unwrap(),
        )
        .unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout.clone());
        let unconfigured = executor.preview_archive_document("root.csv").unwrap();
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
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\".\"]\n",
        )
        .unwrap();
        let incomplete = CommandExecutor::new(storage)
            .with_layout(layout)
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
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root().join("issues")).unwrap();
        fs::write(
            storage.root().join("index.json"),
            crate::repository_state::RepositoryIndex::default()
                .to_pretty_bytes()
                .unwrap(),
        )
        .unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
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
                storage.root().join("config.toml"),
                format!("[documentation]\n{authored}\n"),
            )
            .unwrap();
            let plan = CommandExecutor::new(storage.clone())
                .with_layout(layout.clone())
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
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = []\npermanent_paths = []\narchive_root = \"\"\n",
        )
        .unwrap();
        let configured = CommandExecutor::new(storage)
            .with_layout(layout)
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

    #[test]
    fn test_preview_non_terminal_container_and_foreign_destination_are_blocked() {
        let (repo, executor, id) = configured_repo();
        let mut issue = executor.storage.load_issue(&id).unwrap();
        issue.state = State::InProgress;
        seed_archive_issue_precondition(&executor.storage, issue);
        let destination = repo.path().join("archive").join(&id[..8]);
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join(".jit-container"), "foreign-container-id\n").unwrap();

        let plan = executor.preview_archive_container(&id[..8]).unwrap();
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
        let markerless = executor.preview_archive_container(&id).unwrap();
        assert!(markerless
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::DestinationConflict));
    }

    #[cfg(unix)]
    #[test]
    fn test_preview_blocks_symlink_artifact_without_following_it() {
        use std::os::unix::fs::symlink;

        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("real.md"), "[referent](secret.html)").unwrap();
        fs::write(repo.path().join("fixtures/secret.html"), "referent-only").unwrap();
        symlink(
            repo.path().join("real.md"),
            repo.path().join("fixtures/link.md"),
        )
        .unwrap();

        let plan = executor(&repo, storage)
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

        let regular_repo = TempDir::new().unwrap();
        let regular_storage = JsonFileStorage::new(regular_repo.path().join(".jit"));
        fs::create_dir_all(regular_storage.root()).unwrap();
        fs::write(
            regular_storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&regular_repo, regular_storage.clone())
            .initialize_fresh_repository(regular_repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(regular_repo.path().join("fixtures")).unwrap();
        fs::create_dir(regular_repo.path().join("real-archive")).unwrap();
        fs::write(regular_repo.path().join("fixtures/root.md"), "root").unwrap();
        symlink(
            regular_repo.path().join("real-archive"),
            regular_repo.path().join("archive"),
        )
        .unwrap();
        let destination_symlink = executor(&regular_repo, regular_storage)
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        assert!(destination_symlink.artifacts()[0]
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));

        let owner_repo = TempDir::new().unwrap();
        let owner_storage = JsonFileStorage::new(owner_repo.path().join(".jit"));
        fs::create_dir_all(owner_storage.root()).unwrap();
        fs::write(
            owner_storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&owner_repo, owner_storage.clone())
            .initialize_fresh_repository(owner_repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
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
            crate::domain::types::fixture_issue("Active outside owner".into(), String::new());
        outside.state = State::InProgress;
        outside.documents = vec![DocumentReference::new("docs/outside.md".into())];
        seed_archive_issue_precondition(&owner_storage, outside);

        let owner_plan = executor(&owner_repo, owner_storage)
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

    // === Coupled retirement workflow (jit:45a140ae) ===

    /// A repo with one container issue in `state` owning `fixtures/root.md`.
    /// Returns the repo, executor, and the container's full id.
    fn executable_container_repo(
        state: State,
        content: &str,
    ) -> (TempDir, CommandExecutor<JsonFileStorage>, String) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir_all(storage.root()).unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        executor(&repo, storage.clone())
            .initialize_fresh_repository(repo.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), content).unwrap();
        let mut container = crate::domain::types::fixture_issue("Container".into(), String::new());
        container.state = state;
        container.labels = vec!["type:epic".to_string()];
        container.documents = vec![DocumentReference::new("fixtures/root.md".into())];
        let id = container.id.clone();
        seed_archive_issue_precondition(&storage, container);
        let executor = executor(&repo, storage);
        (repo, executor, id)
    }

    fn state_change_count(executor: &CommandExecutor<JsonFileStorage>, id: &str) -> usize {
        executor
            .storage
            .read_events()
            .unwrap()
            .iter()
            .filter(|event| matches!(event, Event::IssueStateChanged { issue_id, to: State::Archived, .. } if issue_id == id))
            .count()
    }

    #[test]
    fn test_container_archival_retires_container_to_archived() {
        let (repo, executor, id) = executable_container_repo(State::Done, "# Root\n");
        let result = executor.execute_archive_container(&id).unwrap();
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
        let (_repo, executor, id) = executable_container_repo(State::Rejected, "# Root\n");
        executor.execute_archive_container(&id).unwrap();
        let container = executor.storage.load_issue(&id).unwrap();
        assert_eq!(container.state, State::Archived);
        assert_eq!(container.archived_from, Some(State::Rejected));
    }

    #[test]
    fn test_non_terminal_container_archival_refused_with_guidance() {
        let (repo, executor, id) = executable_container_repo(State::InProgress, "# Root\n");
        let error = executor
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
        let (_repo, executor, id) = executable_container_repo(State::Done, "# Root\n");
        executor.execute_archive_container(&id).unwrap();
        // The container is now Archived-from-Done, still effectively terminal, so a
        // rerun is eligible and reconciles to a no-op rather than erroring.
        let rerun = executor.execute_archive_container(&id).unwrap();
        assert!(!rerun.event_appended, "a fully-archived rerun is a no-op");
        let container = executor.storage.load_issue(&id).unwrap();
        assert_eq!(container.state, State::Archived);
        assert_eq!(container.archived_from, Some(State::Done));
        // The retirement transition is not re-emitted on the no-op rerun.
        assert_eq!(state_change_count(&executor, &id), 1);
    }

    #[test]
    fn test_independently_archived_terminal_owner_does_not_block_shared_document() {
        // A document shared by a Done owner and a descendant independently
        // archived FROM a terminal state: the archived owner is effectively
        // terminal, so it is not an active owner and the document still moves.
        let (_repo, executor, _) = executable_document_repo(1, "shared doc");
        let mut archived_owner =
            crate::domain::types::fixture_issue("Independently archived".into(), String::new());
        archived_owner.state = State::Archived;
        archived_owner.archived_from = Some(State::Done);
        archived_owner.documents = vec![DocumentReference::new("fixtures/root.md".into())];
        seed_archive_issue_precondition(&executor.storage, archived_owner.clone());

        let plan = executor
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
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        let artifact = &plan.artifacts()[0];
        assert!(artifact
            .evidence()
            .contains(&crate::domain::artifact_plan::EvidenceCode::ActiveOwner));
    }
}
