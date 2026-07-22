//! Pure finalization of one dependency-aware archive execution.

use super::mutation::finalize_delta;
use super::{
    ExpectedPreimage, FileMode, MaterializationIntent, MaterializationPlan, MutationContext,
    MutationIntent, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    VirtualPath,
};
use crate::domain::artifact_execution::{ArchiveExecutionResult, ArchivePublication};
use crate::domain::artifact_plan::{
    ArtifactAction, ArtifactPlan, ContentIdentity, PlanTarget, PlanWarning, ReferenceChange,
    WarningCode,
};
use crate::domain::{Event, Issue, State};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};

const OWNER: &str = "archive-execution";

/// Close a captured archive plan into one recoverable repository transaction.
pub fn finalize_archive_execution(
    image: &RepositoryImage,
    context: &MutationContext,
    plan: &ArtifactPlan,
) -> Result<(MaterializationPlan, ArchiveExecutionResult)> {
    let artifacts = plan.executable_artifacts()?;
    crate::domain::artifact_classifier::validate_proposed_layout(plan)?;
    let prior = captured_archive_events(image)?;
    let (covered_publications, covered_changes) =
        archive_coverage(&prior, plan.target(), plan.destination_root());
    let mut actions = Vec::new();
    let mut publications = Vec::new();
    let mut created_directories = BTreeSet::new();

    for artifact in artifacts.iter().filter(|artifact| {
        matches!(
            artifact.action(),
            ArtifactAction::Move | ArtifactAction::Copy
        )
    }) {
        let destination = artifact
            .destination()
            .context("archive publication has no destination")?;
        let identity = artifact
            .content_identity()
            .context("archive publication has no content identity")?;
        let destination_path = archive_path(image, destination)?;
        let (bytes, source_mode) = if artifact.already_archived() {
            match image.entry(&destination_path)? {
                RepositoryEntry::File { bytes, mode, .. } => (bytes.clone(), *mode),
                _ => bail!("archived destination is not an ordinary file: {destination}"),
            }
        } else {
            let source = archive_path(image, artifact.source())?;
            match image.entry(&source)? {
                RepositoryEntry::File { bytes, mode, .. } => (bytes.clone(), *mode),
                _ => bail!(
                    "archive source is not an ordinary file: {}",
                    artifact.source()
                ),
            }
        };
        identity_matches(identity, &bytes)?;
        match image.entry(&destination_path)? {
            RepositoryEntry::Absent => {
                create_ancestors(
                    image,
                    &destination_path,
                    &mut created_directories,
                    &mut actions,
                )?;
                actions.push(RepositoryAction::WriteFile {
                    path: destination_path,
                    owner: OWNER.into(),
                    expected: ExpectedPreimage::Absent,
                    bytes,
                    mode: source_mode,
                });
                publications.push(ArchivePublication {
                    source: Some(artifact.source().into()),
                    destination: destination.into(),
                    content_identity: identity.clone(),
                    adopted: false,
                });
            }
            RepositoryEntry::File { bytes, .. } => {
                identity_matches(identity, bytes)?;
                if !covered_publications
                    .iter()
                    .any(|(path, found)| path == destination && found == identity)
                {
                    publications.push(ArchivePublication {
                        source: Some(artifact.source().into()),
                        destination: destination.into(),
                        content_identity: identity.clone(),
                        adopted: true,
                    });
                }
            }
            _ => bail!("archive destination has an unsafe occupant: {destination}"),
        }
    }

    if let PlanTarget::Container { id } = plan.target() {
        let destination = format!("{}/.jit-container", plan.destination_root());
        let bytes = format!("{id}\n").into_bytes();
        let identity = ContentIdentity::from_bytes(&bytes);
        let path = archive_path(image, &destination)?;
        match image.entry(&path)? {
            RepositoryEntry::Absent => {
                create_ancestors(image, &path, &mut created_directories, &mut actions)?;
                actions.push(RepositoryAction::WriteFile {
                    path,
                    owner: OWNER.into(),
                    expected: ExpectedPreimage::Absent,
                    bytes,
                    mode: FileMode::Regular,
                });
                publications.push(ArchivePublication {
                    source: None,
                    destination,
                    content_identity: identity,
                    adopted: false,
                });
            }
            RepositoryEntry::File { bytes, .. } => {
                identity_matches(&identity, bytes)?;
                if !covered_publications
                    .iter()
                    .any(|(path, found)| path == &destination && found == &identity)
                {
                    publications.push(ArchivePublication {
                        source: None,
                        destination,
                        content_identity: identity,
                        adopted: true,
                    });
                }
            }
            _ => bail!("container marker has an unsafe occupant: {destination}"),
        }
    }

    let mut issues = BTreeMap::<String, Issue>::new();
    let mut event_changes = Vec::new();
    let mut observed_uncovered = false;
    for change in artifacts
        .iter()
        .flat_map(|artifact| artifact.reference_changes())
    {
        let issue = captured_issue(image, &change.issue, &mut issues)?;
        let document = issue
            .documents
            .get_mut(change.document_index)
            .ok_or_else(|| {
                anyhow!(
                    "planned document index {} is absent on issue {}",
                    change.document_index,
                    change.issue
                )
            })?;
        if document.commit.is_some() {
            bail!(
                "planned relink points at pinned issue document: {}",
                change.issue
            );
        }
        let actual = crate::domain::artifact_plan::normalize_artifact_path(&document.path);
        if actual == change.from_path {
            document.path = change.to_path.clone();
            document.assets.clear();
            event_changes.push(change.clone());
        } else if actual == change.to_path {
            if !covered_changes.contains(change) {
                event_changes.push(change.clone());
                observed_uncovered = true;
            }
        } else {
            bail!(
                "planned relink no longer matches issue {} document {}",
                change.issue,
                change.document_index
            );
        }
    }

    let mut state_event = None;
    if let PlanTarget::Container { id } = plan.target() {
        let issue = captured_issue(image, id, &mut issues)?;
        if issue.state != State::Archived {
            let from = issue.state;
            issue.archived_from = Some(from);
            issue.state = State::Archived;
            state_event = Some(Event::draft_issue_state_changed(
                id.clone(),
                from,
                State::Archived,
            ));
        }
    }
    let mut changed_issue_ids = Vec::new();
    for (id, issue) in &issues {
        let path = VirtualPath::data(format!("issues/{id}.json"))?;
        let RepositoryEntry::File { bytes, .. } = image.entry(&path)? else {
            bail!("captured issue is not an ordinary file: {id}");
        };
        if crate::repository_state::serialize_issue(issue)?.as_slice() != bytes {
            changed_issue_ids.push(id.clone());
        }
    }

    event_changes.sort_by(|a, b| {
        (&a.issue, a.document_index, &a.from_path, &a.to_path).cmp(&(
            &b.issue,
            b.document_index,
            &b.from_path,
            &b.to_path,
        ))
    });
    event_changes.dedup();
    let planned_deletions = artifacts
        .iter()
        .flat_map(|artifact| artifact.pending_deletions().iter().cloned())
        .collect::<Vec<_>>();
    let mut deletions = Vec::new();
    let mut execution_warnings = Vec::new();
    for deletion in &planned_deletions {
        let path = archive_path(image, &deletion.source)?;
        match image.entry(&path)? {
            RepositoryEntry::Absent => {}
            RepositoryEntry::File { bytes, .. } => {
                if ContentIdentity::from_bytes(bytes) == deletion.content_identity {
                    actions.push(RepositoryAction::DeleteFile {
                        path,
                        owner: OWNER.into(),
                        expected: ExpectedPreimage::of(
                            image.entry(&archive_path(image, &deletion.source)?)?,
                        ),
                    });
                    deletions.push(deletion.clone());
                } else {
                    execution_warnings.push(PlanWarning::new(
                        WarningCode::DeletionFailed,
                        Some(&deletion.source),
                    ));
                }
            }
            _ => bail!(
                "archive deletion source has an unsafe occupant: {}",
                deletion.source
            ),
        }
    }

    let reconciling =
        publications.iter().any(|publication| publication.adopted) || observed_uncovered;
    let event_needed =
        !publications.is_empty() || !event_changes.is_empty() || !deletions.is_empty();
    let mut intents = changed_issue_ids
        .iter()
        .map(|id| -> Result<_> {
            Ok(MutationIntent::UpdateIssue {
                issue: Box::new(
                    issues
                        .get(id)
                        .with_context(|| format!("changed archive issue {id} was not captured"))?
                        .clone(),
                ),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    if event_needed {
        intents.push(MutationIntent::RecordEvent {
            phase: 2,
            event: Box::new(Event::draft_artifact_archive_executed(
                plan.target().clone(),
                plan.destination_root().into(),
                publications.clone(),
                event_changes.clone(),
                planned_deletions.clone(),
                reconciling,
            )),
        });
    }
    if let Some(event) = state_event {
        intents.push(MutationIntent::RecordEvent {
            phase: 3,
            event: Box::new(event),
        });
    }
    let record_actions = finalize_delta(image.layout(), image, context, &intents)?
        .actions()
        .to_vec();
    let mut all_actions = actions;
    all_actions.extend(record_actions);
    let delta = RepositoryDelta::new(image.layout(), all_actions)?;
    let materialization = MaterializationPlan::new(
        image,
        &context.repository_seed(&intents)?,
        &MaterializationIntent::SemanticMutation,
        delta,
    )?;
    let mut warnings = plan
        .warnings()
        .iter()
        .cloned()
        .chain(artifacts.iter().flat_map(|a| a.warnings().iter().cloned()))
        .chain(execution_warnings)
        .collect::<Vec<_>>();
    warnings.sort_by(|left, right| {
        (left.code.as_str(), left.path.as_deref())
            .cmp(&(right.code.as_str(), right.path.as_deref()))
    });
    warnings.dedup();
    let result = ArchiveExecutionResult {
        schema_version: 1,
        target: plan.target().clone(),
        destination_root: plan.destination_root().into(),
        publications,
        reference_changes: event_changes,
        planned_deletions,
        deleted_sources: deletions.into_iter().map(|d| d.source).collect(),
        warnings,
        event_appended: event_needed,
        reconciling,
    };
    Ok((materialization, result))
}

fn identity_matches(identity: &ContentIdentity, bytes: &[u8]) -> Result<()> {
    if &ContentIdentity::from_bytes(bytes) != identity {
        bail!("artifact content identity changed after planning");
    }
    Ok(())
}

fn create_ancestors(
    image: &RepositoryImage,
    path: &VirtualPath,
    created: &mut BTreeSet<VirtualPath>,
    actions: &mut Vec<RepositoryAction>,
) -> Result<()> {
    let mut ancestors = Vec::new();
    let mut relative = path.relative().as_path().parent();
    while let Some(parent) = relative {
        if parent.as_os_str().is_empty() {
            break;
        }
        ancestors.push(VirtualPath::from_root(
            path.root_class(),
            super::RootRelativePath::parse(parent)?,
        )?);
        relative = parent.parent();
    }
    for ancestor in ancestors.into_iter().rev() {
        match image.entry(&ancestor)? {
            RepositoryEntry::Absent if created.insert(ancestor.clone()) => {
                actions.push(RepositoryAction::CreateDirectory {
                    path: ancestor,
                    owner: OWNER.into(),
                    expected: ExpectedPreimage::Absent,
                })
            }
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => {}
            _ => bail!("archive ancestor has an unsafe occupant: {ancestor:?}"),
        }
    }
    Ok(())
}

fn archive_path(image: &RepositoryImage, path: &str) -> Result<VirtualPath> {
    Ok(image
        .layout()
        .classify_and_canonicalize(image.layout().worktree_root().join(path))?)
}

fn captured_issue<'a>(
    image: &RepositoryImage,
    id: &str,
    issues: &'a mut BTreeMap<String, Issue>,
) -> Result<&'a mut Issue> {
    if !issues.contains_key(id) {
        let path = VirtualPath::data(format!("issues/{id}.json"))?;
        let bytes = image
            .file_bytes(&path)?
            .ok_or_else(|| anyhow!("captured issue is absent: {id}"))?;
        let issue: Issue = serde_json::from_slice(bytes)
            .with_context(|| format!("invalid captured issue {id}"))?;
        if issue.id != id {
            bail!(
                "captured issue {id} contains mismatched embedded id {}",
                issue.id
            );
        }
        issues.insert(id.into(), issue);
    }
    issues
        .get_mut(id)
        .ok_or_else(|| anyhow!("captured issue cache lost {id}"))
}

pub(crate) fn captured_archive_events(image: &RepositoryImage) -> Result<Vec<Event>> {
    let bytes = image
        .file_bytes(&VirtualPath::data("events.jsonl")?)?
        .unwrap_or_default();
    Ok(crate::domain::parse_known_events(std::str::from_utf8(
        bytes,
    )?)?)
}

fn archive_coverage(
    events: &[Event],
    target: &PlanTarget,
    root: &str,
) -> (Vec<(String, ContentIdentity)>, Vec<ReferenceChange>) {
    events
        .iter()
        .filter_map(|event| match event {
            Event::ArtifactArchiveExecuted {
                target: found,
                destination_root,
                publications,
                reference_changes,
                ..
            } if found == target && destination_root == root => {
                Some((publications, reference_changes))
            }
            _ => None,
        })
        .fold(
            (Vec::new(), Vec::new()),
            |(mut publications, mut changes), (found_publications, found_changes)| {
                publications.extend(
                    found_publications
                        .iter()
                        .map(|p| (p.destination.clone(), p.content_identity.clone())),
                );
                changes.extend(found_changes.iter().cloned());
                (publications, changes)
            },
        )
}
