//! Pure finalization of one dependency-aware archive execution.

use super::mutation::finalize_delta;
use super::{
    ExpectedPreimage, FileMode, MaterializationPlan, MutationContext, MutationError,
    MutationIntent, ProducerError, RepositoryAction, RepositoryActionOwner, RepositoryDelta,
    RepositoryEntry, RepositoryImage, RepositoryStateError, VirtualPath,
};
use crate::domain::artifact_execution::{
    ArchiveEvidenceRecord, ArchiveExecution, ArchiveExecutionResult, ArchivePublication,
};
use crate::domain::artifact_plan::{
    ArtifactAction, ArtifactPlan, ContentIdentity, PlanError, PlanTarget, PlanWarning,
    ReferenceChange, WarningCode,
};
use crate::domain::identity::IssueId;
use crate::domain::{EventBody, Issue, State};
use std::collections::{BTreeMap, BTreeSet};

/// Close a captured archive plan into one materialization plan, which
/// publishes as one redo record. `evidence` is the target record decoded by
/// the planner from the same image.
pub fn finalize_archive_execution(
    image: &RepositoryImage,
    context: &MutationContext,
    plan: &ArtifactPlan,
    mut evidence: ArchiveEvidenceRecord,
) -> Result<(MaterializationPlan, ArchiveExecutionResult), RepositoryStateError> {
    let artifacts = plan
        .executable_artifacts()
        .map_err(ArchiveExecutionError::Plan)?;
    crate::domain::artifact_classifier::validate_proposed_layout(plan)
        .map_err(|violation| ArchiveExecutionError::Producer(violation.into()))?;
    let (covered_publications, covered_changes) =
        archive_coverage(&evidence.executions, plan.target(), plan.destination_root());
    let mut actions = Vec::new();
    let mut publications = Vec::new();
    let mut created_directories = BTreeSet::new();

    for artifact in artifacts.iter().filter(|artifact| {
        matches!(
            artifact.action(),
            ArtifactAction::Move | ArtifactAction::Copy
        )
    }) {
        let destination = artifact.destination().ok_or_else(|| {
            ArchiveExecutionError::MissingDestination(artifact.source().to_string())
        })?;
        let identity = artifact.content_identity().ok_or_else(|| {
            ArchiveExecutionError::MissingContentIdentity(artifact.source().to_string())
        })?;
        let destination_path = archive_path(image, destination)?;
        let (bytes, source_mode) = if artifact.already_archived() {
            match captured_entry(image, &destination_path)? {
                RepositoryEntry::File { content, mode } => (content.bytes().to_vec(), *mode),
                _ => {
                    return Err(ArchiveExecutionError::NonFileSource {
                        role: "archived destination",
                        path: destination.to_string(),
                    }
                    .into())
                }
            }
        } else {
            let source = archive_path(image, artifact.source())?;
            match captured_entry(image, &source)? {
                RepositoryEntry::File { content, mode } => (content.bytes().to_vec(), *mode),
                _ => {
                    return Err(ArchiveExecutionError::NonFileSource {
                        role: "archive source",
                        path: artifact.source().to_string(),
                    }
                    .into())
                }
            }
        };
        identity_matches(identity, &bytes, artifact.source())?;
        match captured_entry(image, &destination_path)? {
            RepositoryEntry::Absent => {
                create_ancestors(
                    image,
                    &destination_path,
                    &mut created_directories,
                    &mut actions,
                )?;
                actions.push(RepositoryAction::WriteFile {
                    path: destination_path,
                    owner: RepositoryActionOwner::ArchiveExecution,
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
            RepositoryEntry::File { content, .. } => {
                identity_matches(identity, content.bytes(), destination)?;
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
            _ => {
                return Err(ArchiveExecutionError::UnsafeOccupant {
                    role: "archive destination",
                    path: destination.to_string(),
                }
                .into())
            }
        }
    }

    if let PlanTarget::Container { id } = plan.target() {
        let destination = format!("{}/.jit-container", plan.destination_root());
        let bytes = format!("{id}\n").into_bytes();
        let identity = ContentIdentity::from_bytes(&bytes);
        let path = archive_path(image, &destination)?;
        match captured_entry(image, &path)? {
            RepositoryEntry::Absent => {
                create_ancestors(image, &path, &mut created_directories, &mut actions)?;
                actions.push(RepositoryAction::WriteFile {
                    path,
                    owner: RepositoryActionOwner::ArchiveExecution,
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
            RepositoryEntry::File { content, .. } => {
                identity_matches(&identity, content.bytes(), &destination)?;
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
            _ => {
                return Err(ArchiveExecutionError::UnsafeOccupant {
                    role: "container marker",
                    path: destination.clone(),
                }
                .into())
            }
        }
    }

    let mut issues = BTreeMap::<IssueId, Issue>::new();
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
            .ok_or_else(|| ArchiveExecutionError::MissingPlannedDocument {
                issue: change.issue.clone(),
                document_index: change.document_index,
            })?;
        if document.commit.is_some() {
            return Err(ArchiveExecutionError::RelinkTargetsPinned(change.issue.clone()).into());
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
            return Err(ArchiveExecutionError::RelinkStale {
                issue: change.issue.clone(),
                document_index: change.document_index,
            }
            .into());
        }
    }

    let mut state_event = None;
    if let PlanTarget::Container { id } = plan.target() {
        let issue = captured_issue(image, id, &mut issues)?;
        if issue.state != State::Archived {
            let from = issue.state;
            issue.archived_from = Some(from);
            issue.state = State::Archived;
            state_event = Some(EventBody::draft_issue_state_changed(
                id.clone(),
                from,
                State::Archived,
            ));
        }
    }
    let mut changed_issue_ids = Vec::new();
    for (id, issue) in &issues {
        let path = VirtualPath::issue_record(id);
        let RepositoryEntry::File { content, .. } = captured_entry(image, &path)? else {
            return Err(ArchiveExecutionError::NonFileSource {
                role: "captured issue",
                path: path.relative().as_str().to_string(),
            }
            .into());
        };
        if crate::repository_state::serialize_issue(issue)
            .map_err(ArchiveExecutionError::Mutation)?
            .as_slice()
            != content.bytes()
        {
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
        match captured_entry(image, &path)? {
            RepositoryEntry::Absent => {}
            RepositoryEntry::File { content, .. } => {
                if ContentIdentity::from_bytes(content.bytes()) == deletion.content_identity {
                    actions.push(RepositoryAction::DeleteFile {
                        path,
                        owner: RepositoryActionOwner::ArchiveExecution,
                        expected: ExpectedPreimage::of(captured_entry(
                            image,
                            &archive_path(image, &deletion.source)?,
                        )?),
                    });
                    deletions.push(deletion.clone());
                } else {
                    execution_warnings.push(PlanWarning::new(
                        WarningCode::DeletionFailed,
                        Some(&deletion.source),
                    ));
                }
            }
            _ => {
                return Err(ArchiveExecutionError::UnsafeOccupant {
                    role: "archive deletion source",
                    path: deletion.source.clone(),
                }
                .into())
            }
        }
    }

    let reconciling =
        publications.iter().any(|publication| publication.adopted) || observed_uncovered;
    let event_needed =
        !publications.is_empty() || !event_changes.is_empty() || !deletions.is_empty();
    let mut intents = changed_issue_ids
        .iter()
        .map(|id| -> Result<_, ArchiveExecutionError> {
            Ok(MutationIntent::UpdateIssue {
                issue: Box::new(
                    issues
                        .get(id)
                        .ok_or_else(|| ArchiveExecutionError::MissingChangedIssue(id.clone()))?
                        .clone(),
                ),
            })
        })
        .collect::<Result<Vec<_>, ArchiveExecutionError>>()?;
    if event_needed {
        let execution = ArchiveExecution {
            target: plan.target().clone(),
            destination_root: plan.destination_root().into(),
            publications: publications.clone(),
            reference_changes: event_changes.clone(),
            planned_deletions: planned_deletions.clone(),
            reconciling,
        };
        let path = VirtualPath::archive_evidence_record(plan.target());
        let entry = captured_entry(image, &path)?;
        evidence.executions.push(execution.clone());
        create_ancestors(image, &path, &mut created_directories, &mut actions)?;
        actions.push(RepositoryAction::WriteFile {
            path,
            owner: RepositoryActionOwner::ArchiveExecution,
            expected: ExpectedPreimage::of(entry),
            bytes: super::canonical_json(&evidence, true)
                .map_err(MutationError::from)
                .map_err(ArchiveExecutionError::Mutation)?,
            mode: FileMode::Regular,
        });
        intents.push(MutationIntent::RecordEvent {
            phase: 2,
            event: Box::new(EventBody::draft_artifact_archive_executed(execution)),
        });
    }
    if let Some(event) = state_event {
        intents.push(MutationIntent::RecordEvent {
            phase: 3,
            event: Box::new(event),
        });
    }
    let record_actions = finalize_delta(image.layout(), image, context, &intents, None)
        .map_err(ArchiveExecutionError::Mutation)?
        .actions()
        .to_vec();
    let mut all_actions = actions;
    all_actions.extend(record_actions);
    let delta = RepositoryDelta::new(image.layout(), all_actions)?;
    let materialization = MaterializationPlan::new(image, delta);
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

fn identity_matches(
    identity: &ContentIdentity,
    bytes: &[u8],
    path: &str,
) -> Result<(), ArchiveExecutionError> {
    if &ContentIdentity::from_bytes(bytes) != identity {
        return Err(ArchiveExecutionError::ContentIdentityChanged(
            path.to_string(),
        ));
    }
    Ok(())
}

fn create_ancestors(
    image: &RepositoryImage,
    path: &VirtualPath,
    created: &mut BTreeSet<VirtualPath>,
    actions: &mut Vec<RepositoryAction>,
) -> Result<(), ArchiveExecutionError> {
    let mut ancestors = Vec::new();
    let mut relative = path.relative().as_path().parent();
    while let Some(parent) = relative {
        if parent.as_os_str().is_empty() {
            break;
        }
        let relative_path = super::RootRelativePath::parse(parent).map_err(ProducerError::from)?;
        ancestors.push(
            VirtualPath::from_root(path.root_class(), relative_path)
                .map_err(ProducerError::from)?,
        );
        relative = parent.parent();
    }
    for ancestor in ancestors.into_iter().rev() {
        match captured_entry(image, &ancestor)? {
            RepositoryEntry::Absent if created.insert(ancestor.clone()) => {
                actions.push(RepositoryAction::CreateDirectory {
                    path: ancestor,
                    owner: RepositoryActionOwner::ArchiveExecution,
                    expected: ExpectedPreimage::Absent,
                })
            }
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => {}
            _ => {
                return Err(ArchiveExecutionError::UnsafeOccupant {
                    role: "archive ancestor",
                    path: ancestor.relative().as_str().to_string(),
                })
            }
        }
    }
    Ok(())
}

fn archive_path(image: &RepositoryImage, path: &str) -> Result<VirtualPath, ArchiveExecutionError> {
    Ok(image
        .layout()
        .classify_and_canonicalize(image.layout().worktree_root().join(path))
        .map_err(ProducerError::from)?)
}

/// Read one captured entry, typing an uncaptured path or malformed captured
/// bytes as the shared [`ProducerError`] leaf.
fn captured_entry<'a>(
    image: &'a RepositoryImage,
    path: &VirtualPath,
) -> Result<&'a RepositoryEntry, ArchiveExecutionError> {
    Ok(image.entry(path).map_err(ProducerError::from)?)
}

fn captured_issue<'a>(
    image: &RepositoryImage,
    id: &IssueId,
    issues: &'a mut BTreeMap<IssueId, Issue>,
) -> Result<&'a mut Issue, ArchiveExecutionError> {
    if !issues.contains_key(id) {
        let path = VirtualPath::issue_record(id);
        let bytes = image
            .file_bytes(&path)
            .map_err(ProducerError::from)?
            .ok_or_else(|| ArchiveExecutionError::MissingCapturedIssue(id.clone()))?;
        let issue = super::decode_issue_record(&path, bytes)?;
        if &issue.id != id {
            return Err(ArchiveExecutionError::IssueIdMismatch {
                expected: id.clone(),
                found: issue.id,
            });
        }
        issues.insert(id.clone(), issue);
    }
    issues
        .get_mut(id)
        .ok_or_else(|| ArchiveExecutionError::CapturedIssueCacheLost(id.clone()))
}

/// A typed failure raised while finalizing one archive execution.
///
/// Composed into [`RepositoryStateError::ArchiveExecution`]. Fields hold paths
/// or typed issue identifiers, never a pre-rendered message;
/// rendering lives in this type's `Display` impl. `role` distinguishes the
/// several captured-entry roles [`Self::NonFileSource`] and
/// [`Self::UnsafeOccupant`] classify across the finalizer (source, an
/// already-archived destination, a container marker, a captured issue
/// record, an ancestor directory, and a deletion source).
#[derive(Debug, thiserror::Error)]
pub enum ArchiveExecutionError {
    /// A publication artifact carried no planned mirror destination.
    #[error("archive publication has no destination: {0}")]
    MissingDestination(String),
    /// A publication or deletion artifact carried no captured content identity.
    #[error("archive publication has no content identity: {0}")]
    MissingContentIdentity(String),
    /// Bytes read at execution no longer match the identity captured at planning.
    #[error("artifact content identity changed after planning: {0}")]
    ContentIdentityChanged(String),
    /// A captured entry expected to be an ordinary file was some other kind.
    #[error("{role} is not an ordinary file: {path}")]
    NonFileSource {
        /// Which captured-entry role failed the ordinary-file check.
        role: &'static str,
        /// Raw path identifier.
        path: String,
    },
    /// A captured entry occupying a publication or ancestor target was not
    /// safely writable.
    #[error("{role} has an unsafe occupant: {path}")]
    UnsafeOccupant {
        /// Which captured-entry role found the unsafe occupant.
        role: &'static str,
        /// Raw path identifier.
        path: String,
    },
    /// A planned relink referenced a document index absent from its issue.
    #[error("planned document index {document_index} is absent on issue {issue}")]
    MissingPlannedDocument {
        /// Full durable issue id.
        issue: IssueId,
        /// Planned document index.
        document_index: usize,
    },
    /// A planned relink targeted a commit-pinned issue document.
    #[error("planned relink points at pinned issue document: {0}")]
    RelinkTargetsPinned(IssueId),
    /// A planned relink no longer matches its issue document's captured path.
    #[error("planned relink no longer matches issue {issue} document {document_index}")]
    RelinkStale {
        /// Full durable issue id.
        issue: IssueId,
        /// Planned document index.
        document_index: usize,
    },
    /// A referenced issue record was absent from the captured image.
    #[error("captured issue is absent: {0}")]
    MissingCapturedIssue(IssueId),
    /// A captured issue record did not decode.
    #[error(transparent)]
    MalformedIssueRecord(#[from] super::MalformedIssueRecordError),
    /// A captured issue's embedded id did not match its requested id.
    #[error("captured issue {expected} contains mismatched embedded id {found}")]
    IssueIdMismatch {
        /// Id the issue was looked up by.
        expected: IssueId,
        /// Id embedded in the captured issue record.
        found: IssueId,
    },
    /// A captured issue vanished from the in-memory cache immediately after
    /// insertion.
    #[error("captured issue cache lost {0}")]
    CapturedIssueCacheLost(IssueId),
    /// An issue serialized as changed was absent from the captured intent set.
    #[error("changed archive issue {0} was not captured")]
    MissingChangedIssue(IssueId),
    /// The archive plan's artifacts were not eligible for execution.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// A materialization producer failed reading captured evidence or
    /// canonicalizing a path, including the proposed-layout edge check.
    #[error(transparent)]
    Producer(#[from] ProducerError),
    /// The record finalizer failed.
    #[error(transparent)]
    Mutation(#[from] MutationError),
}

fn archive_coverage(
    executions: &[ArchiveExecution],
    target: &PlanTarget,
    root: &str,
) -> (Vec<(String, ContentIdentity)>, Vec<ReferenceChange>) {
    executions
        .iter()
        .filter(|execution| &execution.target == target && execution.destination_root == root)
        .fold(
            (Vec::new(), Vec::new()),
            |(mut publications, mut changes), execution| {
                publications.extend(
                    execution
                        .publications
                        .iter()
                        .map(|p| (p.destination.clone(), p.content_identity.clone())),
                );
                changes.extend(execution.reference_changes.iter().cloned());
                (publications, changes)
            },
        )
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_archive_coverage_collects_matching_target_and_root_only() {
        let target = PlanTarget::Document {
            path: "a.md".into(),
        };
        let executions = [
            crate::test_utils::archive_execution(target.clone(), "arc", &[("s/1", "arc/1")]),
            crate::test_utils::archive_execution(target.clone(), "other", &[("s/2", "other/2")]),
            crate::test_utils::archive_execution(
                PlanTarget::Document {
                    path: "b.md".into(),
                },
                "arc",
                &[("s/3", "arc/3")],
            ),
            crate::test_utils::archive_execution(target.clone(), "arc", &[("s/4", "arc/4")]),
        ];
        let (publications, changes) = archive_coverage(&executions, &target, "arc");
        let destinations = publications
            .iter()
            .map(|(destination, _)| destination.as_str())
            .collect::<Vec<_>>();
        assert_eq!(destinations, ["arc/1", "arc/4"]);
        assert!(changes.is_empty());
    }

    use super::*;
    use crate::domain::artifact_execution::archive_executions;
    use crate::domain::artifact_plan::{
        ArtifactEdge, ArtifactPlanEntry, ArtifactVersion, EdgeKind, EdgeResolutionMode,
        PendingDeletion, PolicyStatus,
    };
    use crate::repository_state::{CapturedBytes, EntryIdentity, RepositoryImage};
    use crate::test_harness::image::ImageBuilder;
    use crate::test_utils::fixture_issue;
    use crate::test_utils::{document_reference, document_reference_at};

    fn context() -> MutationContext {
        MutationContext::preview()
    }

    fn file_entry(bytes: &[u8]) -> RepositoryEntry {
        RepositoryEntry::File {
            content: CapturedBytes::new("obj", bytes.to_vec()).unwrap(),
            mode: FileMode::Regular,
        }
    }

    fn directory_entry() -> RepositoryEntry {
        RepositoryEntry::Directory {
            identity: EntryIdentity::for_bytes("dir", b"dir").unwrap(),
            mode: FileMode::Regular,
        }
    }

    /// A minimal, otherwise-eligible Move artifact publishing `source` bytes to
    /// `destination` under a `Document` target.
    fn move_artifact(source: &str, destination: &str, identity: ContentIdentity) -> ArtifactPlan {
        let artifact =
            ArtifactPlanEntry::new(source, ArtifactVersion::WorkingTree, ArtifactAction::Move)
                .with_content_identity(identity)
                .with_destination(destination);
        ArtifactPlan::new(
            PlanTarget::Document {
                path: source.to_string(),
            },
            "",
            PolicyStatus::Configured,
            vec![artifact],
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    /// A minimal, otherwise-eligible Retain artifact carrying one planned relink,
    /// so the reference-change pass runs without exercising the Move/Copy loop.
    fn relink_plan(change: ReferenceChange) -> ArtifactPlan {
        let artifact = ArtifactPlanEntry::new(
            "doc.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        )
        .with_reference_changes(vec![change]);
        ArtifactPlan::new(
            PlanTarget::Document {
                path: "doc.md".into(),
            },
            "",
            PolicyStatus::Configured,
            vec![artifact],
            Vec::new(),
            Vec::new(),
        )
        .unwrap()
    }

    fn captured_issue_entry(issue: &Issue) -> RepositoryEntry {
        file_entry(&crate::repository_state::serialize_issue(issue).unwrap())
    }

    fn archive_execution_error(
        image: &RepositoryImage,
        plan: &ArtifactPlan,
    ) -> ArchiveExecutionError {
        match finalize_archive_execution(
            image,
            &context(),
            plan,
            ArchiveEvidenceRecord {
                target: plan.target().clone(),
                executions: Vec::new(),
            },
        )
        .unwrap_err()
        {
            RepositoryStateError::ArchiveExecution(error) => error,
            other => panic!("expected a typed archive-execution error, got {other:?}"),
        }
    }

    fn evidence_write(
        prior: Option<serde_json::Value>,
    ) -> (serde_json::Value, Vec<ArchiveExecution>) {
        let path = VirtualPath::archive_evidence_record(&PlanTarget::Document {
            path: "doc.md".into(),
        });
        let directory = if prior.is_some() {
            directory_entry()
        } else {
            RepositoryEntry::Absent
        };
        let plan = move_artifact(
            "doc.md",
            "archived.md",
            ContentIdentity::from_bytes(b"content"),
        );
        let evidence = prior.clone().map_or_else(
            || ArchiveEvidenceRecord {
                target: plan.target().clone(),
                executions: Vec::new(),
            },
            |value| serde_json::from_value(value).unwrap(),
        );
        let entry = prior.map_or(RepositoryEntry::Absent, |value| {
            file_entry(&serde_json::to_vec(&value).unwrap())
        });
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(VirtualPath::ARCHIVE_EVIDENCE, directory.clone())
            .entry(
                path.ancestor_directories().unwrap().pop().unwrap(),
                directory,
            )
            .entry(path.clone(), entry.clone())
            .file("doc.md", b"content")
            .entry("archived.md", RepositoryEntry::Absent)
            .build();
        let (materialization, _) =
            finalize_archive_execution(&image, &context(), &plan, evidence).unwrap();
        let actions = materialization.delta().actions();
        let written = actions
            .iter()
            .find_map(|action| match action {
                RepositoryAction::WriteFile {
                    path: found,
                    bytes,
                    expected,
                    ..
                } if found == &path => {
                    assert_eq!(expected, &ExpectedPreimage::of(&entry));
                    Some(serde_json::from_slice(bytes).unwrap())
                }
                _ => None,
            })
            .expect("the event's delta must also write the target evidence");
        let events = actions
            .iter()
            .find_map(|action| match action {
                RepositoryAction::AppendEvents { bytes, .. } => {
                    Some(crate::domain::event_log::parse_known_events(bytes).unwrap())
                }
                _ => None,
            })
            .unwrap();
        (written, archive_executions(&events))
    }

    #[test]
    fn test_finalize_archive_execution_creates_evidence_with_its_event() {
        let (written, executions) = evidence_write(None);
        assert_eq!(
            written,
            serde_json::json!({
                "target": executions[0].target,
                "executions": executions,
            })
        );
    }

    #[test]
    fn test_finalize_archive_execution_appends_to_captured_evidence_in_order() {
        let prior = crate::test_utils::archive_execution(
            PlanTarget::Document {
                path: "doc.md".into(),
            },
            "earlier",
            &[],
        );
        let (written, executions) = evidence_write(Some(serde_json::json!({
            "target": prior.target,
            "executions": [prior.clone()],
        })));
        assert_eq!(
            written["executions"],
            serde_json::json!([prior, executions[0]])
        );
    }

    #[test]
    fn test_finalize_archive_execution_content_identity_changed_after_planning() {
        let planned_identity = ContentIdentity::from_bytes(b"planned content");
        let plan = move_artifact("doc.md", "archived.md", planned_identity);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::worktree("doc.md").unwrap(),
                file_entry(b"different content on disk"),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::ContentIdentityChanged(path) if path == "doc.md"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_non_file_source() {
        let plan = move_artifact(
            "doc.md",
            "archived.md",
            ContentIdentity::from_bytes(b"anything"),
        );
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(VirtualPath::worktree("doc.md").unwrap(), directory_entry())
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::NonFileSource { role: "archive source", path }
                if path == "doc.md"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_unsafe_destination_occupant() {
        let content = b"stable content";
        let plan = move_artifact(
            "doc.md",
            "archived.md",
            ContentIdentity::from_bytes(content),
        );
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::worktree("doc.md").unwrap(),
                file_entry(content),
            )
            .entry(
                VirtualPath::worktree("archived.md").unwrap(),
                directory_entry(),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::UnsafeOccupant { role: "archive destination", path }
                if path == "archived.md"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_unsafe_container_marker_occupant() {
        let artifact = ArtifactPlanEntry::new(
            "unused.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        );
        let plan = ArtifactPlan::new(
            PlanTarget::Container {
                id: crate::test_utils::issue_id(165),
            },
            "archive",
            PolicyStatus::Configured,
            vec![artifact],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::worktree("archive/.jit-container").unwrap(),
                directory_entry(),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::UnsafeOccupant { role: "container marker", path }
                if path == "archive/.jit-container"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_unsafe_deletion_source_occupant() {
        let artifact = ArtifactPlanEntry::new(
            "doc.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        )
        .with_content_identity(ContentIdentity::from_bytes(b"doc"))
        .with_pending_deletions(vec![PendingDeletion {
            source: "stale.md".into(),
            content_identity: ContentIdentity::from_bytes(b"stale content"),
        }]);
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "doc.md".into(),
            },
            "",
            PolicyStatus::Configured,
            vec![artifact],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::worktree("stale.md").unwrap(),
                directory_entry(),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::UnsafeOccupant { role: "archive deletion source", path }
                if path == "stale.md"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_missing_planned_document() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = crate::test_utils::issue_id(77);
        issue.documents = Vec::new();
        let change = ReferenceChange {
            issue: issue.id.clone(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&issue.id),
                captured_issue_entry(&issue),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::MissingPlannedDocument { issue: id, document_index: 0 }
                if id == crate::test_utils::issue_id(77)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_relink_targets_pinned_document() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = crate::test_utils::issue_id(77);
        issue.documents = vec![document_reference_at("doc.md", "a".repeat(40))];
        let change = ReferenceChange {
            issue: issue.id.clone(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&issue.id),
                captured_issue_entry(&issue),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::RelinkTargetsPinned(id) if id == crate::test_utils::issue_id(77)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_relink_stale() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = crate::test_utils::issue_id(77);
        issue.documents = vec![document_reference("other.md")];
        let change = ReferenceChange {
            issue: issue.id.clone(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&issue.id),
                captured_issue_entry(&issue),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::RelinkStale { issue: id, document_index: 0 }
                if id == crate::test_utils::issue_id(77)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_missing_captured_issue() {
        let change = ReferenceChange {
            issue: crate::test_utils::issue_id(166),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&crate::test_utils::issue_id(166)),
                RepositoryEntry::Absent,
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::MissingCapturedIssue(id) if id == crate::test_utils::issue_id(166)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_malformed_issue_json() {
        let change = ReferenceChange {
            issue: crate::test_utils::issue_id(167),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&crate::test_utils::issue_id(167)),
                file_entry(b"not json"),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::MalformedIssueRecord(_)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_issue_id_mismatch() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = crate::test_utils::issue_id(78);
        let change = ReferenceChange {
            issue: crate::test_utils::issue_id(168),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .entry(
                VirtualPath::issue_record(&crate::test_utils::issue_id(168)),
                file_entry(&crate::repository_state::serialize_issue(&issue).unwrap()),
            )
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::IssueIdMismatch { expected, found }
                if expected == crate::test_utils::issue_id(168) && found == crate::test_utils::issue_id(78)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_ineligible_plan_is_typed_plan_error() {
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "doc.md".into(),
            },
            "",
            PolicyStatus::Unconfigured,
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::Plan(PlanError::Ineligible)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_unresolved_supported_edge_is_typed_producer_error() {
        let artifact =
            ArtifactPlanEntry::new("doc.md", ArtifactVersion::WorkingTree, ArtifactAction::Move)
                .with_content_identity(ContentIdentity::from_bytes(b"content"))
                .with_destination("archived.md")
                .with_edges(vec![ArtifactEdge {
                    reference: "missing.png".into(),
                    target: Some("missing.png".into()),
                    kind: EdgeKind::Supported,
                    resolution_mode: EdgeResolutionMode::Relative,
                }]);
        let plan = ArtifactPlan::new(
            PlanTarget::Document {
                path: "doc.md".into(),
            },
            "",
            PolicyStatus::Configured,
            vec![artifact],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let image = ImageBuilder::new()
            .entry(VirtualPath::EVENTS, RepositoryEntry::Absent)
            .build();
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::Producer(ProducerError::ProposedLayout(
                crate::domain::artifact_classifier::ProposedLayoutViolation::TargetAbsent { target },
            ))
                if target == "missing.png"
        ));
    }

    // `MissingDestination` and `MissingContentIdentity` guard invariants that
    // `ArtifactPlanEntry::normalize` (re-run by every `executable_artifacts`
    // call, including the one at this finalizer's own entry) already enforces
    // for any value reachable through the public `ArtifactPlan` constructor;
    // they cannot be independently driven through `finalize_archive_execution`.
    // `CapturedIssueCacheLost` and `MissingChangedIssue` guard a `BTreeMap`
    // read immediately following either an insert or a same-map key iteration,
    // which cannot fail. All four are exercised here at the construction/
    // `Display` level instead, matching their defense-in-depth role.
    #[test]
    fn test_archive_execution_error_defense_in_depth_variants_render() {
        assert_eq!(
            ArchiveExecutionError::MissingDestination("doc.md".into()).to_string(),
            "archive publication has no destination: doc.md"
        );
        assert_eq!(
            ArchiveExecutionError::MissingContentIdentity("doc.md".into()).to_string(),
            "archive publication has no content identity: doc.md"
        );
        assert_eq!(
            ArchiveExecutionError::CapturedIssueCacheLost(crate::test_utils::issue_id(77))
                .to_string(),
            format!(
                "captured issue cache lost {}",
                crate::test_utils::issue_id(77)
            )
        );
        assert_eq!(
            ArchiveExecutionError::MissingChangedIssue(crate::test_utils::issue_id(77)).to_string(),
            format!(
                "changed archive issue {} was not captured",
                crate::test_utils::issue_id(77)
            )
        );
    }
}
