//! Pure finalization of one dependency-aware archive execution.

use super::mutation::finalize_delta;
use super::{
    ExpectedPreimage, FileMode, MaterializationIntent, MaterializationPlan, MutationContext,
    MutationError, MutationIntent, ProducerError, RepositoryAction, RepositoryDelta,
    RepositoryEntry, RepositoryImage, RepositoryStateError, VirtualPath,
};
use crate::domain::artifact_execution::{ArchiveExecutionResult, ArchivePublication};
use crate::domain::artifact_plan::{
    ArtifactAction, ArtifactPlan, ContentIdentity, PlanError, PlanTarget, PlanWarning,
    ReferenceChange, WarningCode,
};
use crate::domain::{Event, EventLogError, Issue, State};
use std::collections::{BTreeMap, BTreeSet};

const OWNER: &str = "archive-execution";

/// Close a captured archive plan into one recoverable repository transaction.
pub fn finalize_archive_execution(
    image: &RepositoryImage,
    context: &MutationContext,
    plan: &ArtifactPlan,
) -> Result<(MaterializationPlan, ArchiveExecutionResult), RepositoryStateError> {
    let artifacts = plan
        .executable_artifacts()
        .map_err(ArchiveExecutionError::Plan)?;
    crate::domain::artifact_classifier::validate_proposed_layout(plan)
        .map_err(ArchiveExecutionError::Producer)?;
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
        let destination = artifact.destination().ok_or_else(|| {
            ArchiveExecutionError::MissingDestination(artifact.source().to_string())
        })?;
        let identity = artifact.content_identity().ok_or_else(|| {
            ArchiveExecutionError::MissingContentIdentity(artifact.source().to_string())
        })?;
        let destination_path = archive_path(image, destination)?;
        let (bytes, source_mode) = if artifact.already_archived() {
            match captured_entry(image, &destination_path)? {
                RepositoryEntry::File { bytes, mode, .. } => (bytes.clone(), *mode),
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
                RepositoryEntry::File { bytes, mode, .. } => (bytes.clone(), *mode),
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
                identity_matches(identity, bytes, destination)?;
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
                identity_matches(&identity, bytes, &destination)?;
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
            state_event = Some(Event::draft_issue_state_changed(
                id.clone(),
                from,
                State::Archived,
            ));
        }
    }
    let mut changed_issue_ids = Vec::new();
    for (id, issue) in &issues {
        let path = issue_path(id)?;
        let RepositoryEntry::File { bytes, .. } = captured_entry(image, &path)? else {
            return Err(ArchiveExecutionError::NonFileSource {
                role: "captured issue",
                path: id.clone(),
            }
            .into());
        };
        if crate::repository_state::serialize_issue(issue)
            .map_err(ArchiveExecutionError::Mutation)?
            .as_slice()
            != bytes
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
            RepositoryEntry::File { bytes, .. } => {
                if ContentIdentity::from_bytes(bytes) == deletion.content_identity {
                    actions.push(RepositoryAction::DeleteFile {
                        path,
                        owner: OWNER.into(),
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
    let record_actions = finalize_delta(image.layout(), image, context, &intents)
        .map_err(ArchiveExecutionError::Mutation)?
        .actions()
        .to_vec();
    let mut all_actions = actions;
    all_actions.extend(record_actions);
    let delta = RepositoryDelta::new(image.layout(), all_actions)?;
    let materialization = MaterializationPlan::new(
        image,
        &context
            .repository_seed(&intents)
            .map_err(ArchiveExecutionError::Mutation)?,
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
                    owner: OWNER.into(),
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

/// Canonical data-root-relative path for one captured issue record.
fn issue_path(id: &str) -> Result<VirtualPath, ArchiveExecutionError> {
    Ok(VirtualPath::data(format!("issues/{id}.json")).map_err(ProducerError::from)?)
}

fn captured_issue<'a>(
    image: &RepositoryImage,
    id: &str,
    issues: &'a mut BTreeMap<String, Issue>,
) -> Result<&'a mut Issue, ArchiveExecutionError> {
    if !issues.contains_key(id) {
        let path = issue_path(id)?;
        let bytes = image
            .file_bytes(&path)
            .map_err(ProducerError::from)?
            .ok_or_else(|| ArchiveExecutionError::MissingCapturedIssue(id.to_string()))?;
        let issue: Issue =
            serde_json::from_slice(bytes).map_err(ArchiveExecutionError::MalformedIssueJson)?;
        if issue.id != id {
            return Err(ArchiveExecutionError::IssueIdMismatch {
                expected: id.to_string(),
                found: issue.id,
            });
        }
        issues.insert(id.into(), issue);
    }
    issues
        .get_mut(id)
        .ok_or_else(|| ArchiveExecutionError::CapturedIssueCacheLost(id.to_string()))
}

pub(crate) fn captured_archive_events(
    image: &RepositoryImage,
) -> Result<Vec<Event>, ArchiveExecutionError> {
    let path = VirtualPath::EVENTS;
    let bytes = image
        .file_bytes(&path)
        .map_err(ProducerError::from)?
        .unwrap_or_default();
    let text = String::from_utf8(bytes.to_vec()).map_err(ProducerError::from)?;
    crate::domain::parse_known_events(&text).map_err(ArchiveExecutionError::EventLog)
}

/// A typed failure raised while finalizing one archive execution.
///
/// Composed into [`RepositoryStateError::ArchiveExecution`]. `String`-typed
/// fields hold raw path or issue identifiers, never a pre-rendered message;
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
        /// Raw path or issue identifier.
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
        issue: String,
        /// Planned document index.
        document_index: usize,
    },
    /// A planned relink targeted a commit-pinned issue document.
    #[error("planned relink points at pinned issue document: {0}")]
    RelinkTargetsPinned(String),
    /// A planned relink no longer matches its issue document's captured path.
    #[error("planned relink no longer matches issue {issue} document {document_index}")]
    RelinkStale {
        /// Full durable issue id.
        issue: String,
        /// Planned document index.
        document_index: usize,
    },
    /// A referenced issue record was absent from the captured image.
    #[error("captured issue is absent: {0}")]
    MissingCapturedIssue(String),
    /// A captured issue record was not valid JSON.
    #[error("invalid captured issue json: {0}")]
    MalformedIssueJson(#[source] serde_json::Error),
    /// A captured issue's embedded id did not match its requested id.
    #[error("captured issue {expected} contains mismatched embedded id {found}")]
    IssueIdMismatch {
        /// Id the issue was looked up by.
        expected: String,
        /// Id embedded in the captured issue record.
        found: String,
    },
    /// A captured issue vanished from the in-memory cache immediately after
    /// insertion.
    #[error("captured issue cache lost {0}")]
    CapturedIssueCacheLost(String),
    /// An issue serialized as changed was absent from the captured intent set.
    #[error("changed archive issue {0} was not captured")]
    MissingChangedIssue(String),
    /// The archive plan's artifacts were not eligible for execution.
    #[error(transparent)]
    Plan(#[from] PlanError),
    /// The captured event log could not be parsed.
    #[error(transparent)]
    EventLog(#[from] EventLogError),
    /// A materialization producer failed reading captured evidence or
    /// canonicalizing a path, including the proposed-layout edge check.
    #[error(transparent)]
    Producer(#[from] ProducerError),
    /// The record finalizer or seed derivation failed.
    #[error(transparent)]
    Mutation(#[from] MutationError),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::artifact_plan::{
        ArtifactEdge, ArtifactPlanEntry, ArtifactVersion, EdgeKind, EdgeResolutionMode,
        PendingDeletion, PolicyStatus,
    };
    use crate::domain::types::fixture_issue;
    use crate::domain::DocumentReference;
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, EntryIdentity, RepositoryImage, RepositoryLayout,
        RepositoryRootClass, RepositoryRootEvidence,
    };
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
            max_paths: 32,
            max_listings: 4,
            max_bytes: 1 << 20,
            max_depth: 8,
        }
    }

    fn context() -> MutationContext {
        MutationContext::preview()
    }

    /// Close an image over the given entries. Phase one accepts only fixed
    /// `Data(...)` paths (`@/inv/...` capture discipline); every `Worktree(...)`
    /// path — every document/artifact path this finalizer touches — is enqueued
    /// as a discovered phase-two path instead. Every finalization reads the
    /// archive-coverage event tail first, so an absent `events.jsonl` entry is
    /// captured by default unless the caller supplies its own.
    fn image_with(mut entries: Vec<(VirtualPath, RepositoryEntry)>) -> RepositoryImage {
        let events_path = VirtualPath::data("events.jsonl").unwrap();
        if !entries.iter().any(|(path, _)| path == &events_path) {
            entries.push((events_path, RepositoryEntry::Absent));
        }
        let (data_paths, worktree_paths): (Vec<_>, Vec<_>) = entries
            .iter()
            .map(|(path, _)| path.clone())
            .partition(|path| path.root_class() == RepositoryRootClass::Data);
        let mut spec = CaptureSpec::phase_one(data_paths, budget()).unwrap();
        spec.discover_paths(worktree_paths).unwrap();
        RepositoryImage::close(
            layout(),
            spec,
            entries.into_iter().collect::<BTreeMap<_, _>>(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn file_entry(bytes: &[u8]) -> RepositoryEntry {
        RepositoryEntry::File {
            identity: EntryIdentity::for_bytes("obj", bytes).unwrap(),
            bytes: bytes.to_vec(),
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

    fn captured_issue_entry(id: &str, issue: &Issue) -> (VirtualPath, RepositoryEntry) {
        (
            VirtualPath::data(format!("issues/{id}.json")).unwrap(),
            file_entry(&crate::repository_state::serialize_issue(issue).unwrap()),
        )
    }

    fn archive_execution_error(
        image: &RepositoryImage,
        plan: &ArtifactPlan,
    ) -> ArchiveExecutionError {
        match finalize_archive_execution(image, &context(), plan).unwrap_err() {
            RepositoryStateError::ArchiveExecution(error) => error,
            other => panic!("expected a typed archive-execution error, got {other:?}"),
        }
    }

    #[test]
    fn test_finalize_archive_execution_content_identity_changed_after_planning() {
        let planned_identity = ContentIdentity::from_bytes(b"planned content");
        let plan = move_artifact("doc.md", "archived.md", planned_identity);
        let image = image_with(vec![(
            VirtualPath::worktree("doc.md").unwrap(),
            file_entry(b"different content on disk"),
        )]);
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
        let image = image_with(vec![(
            VirtualPath::worktree("doc.md").unwrap(),
            directory_entry(),
        )]);
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
        let image = image_with(vec![
            (
                VirtualPath::worktree("doc.md").unwrap(),
                file_entry(content),
            ),
            (
                VirtualPath::worktree("archived.md").unwrap(),
                directory_entry(),
            ),
        ]);
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
                id: "container-1".into(),
            },
            "archive",
            PolicyStatus::Configured,
            vec![artifact],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let image = image_with(vec![(
            VirtualPath::worktree("archive/.jit-container").unwrap(),
            directory_entry(),
        )]);
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
        let image = image_with(vec![(
            VirtualPath::worktree("stale.md").unwrap(),
            directory_entry(),
        )]);
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
        issue.id = "owner-1".into();
        issue.documents = Vec::new();
        let change = ReferenceChange {
            issue: issue.id.clone(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = image_with(vec![captured_issue_entry(&issue.id, &issue)]);
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::MissingPlannedDocument { issue: id, document_index: 0 }
                if id == "owner-1"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_relink_targets_pinned_document() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = "owner-1".into();
        issue.documents = vec![DocumentReference::at_commit(
            "doc.md".into(),
            "a".repeat(40),
        )];
        let change = ReferenceChange {
            issue: issue.id.clone(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = image_with(vec![captured_issue_entry(&issue.id, &issue)]);
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::RelinkTargetsPinned(id) if id == "owner-1"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_relink_stale() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = "owner-1".into();
        issue.documents = vec![DocumentReference::new("other.md".into())];
        let change = ReferenceChange {
            issue: issue.id.clone(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = image_with(vec![captured_issue_entry(&issue.id, &issue)]);
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::RelinkStale { issue: id, document_index: 0 }
                if id == "owner-1"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_missing_captured_issue() {
        let change = ReferenceChange {
            issue: "absent-issue".into(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = image_with(vec![(
            VirtualPath::data("issues/absent-issue.json").unwrap(),
            RepositoryEntry::Absent,
        )]);
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::MissingCapturedIssue(id) if id == "absent-issue"
        ));
    }

    #[test]
    fn test_finalize_archive_execution_malformed_issue_json() {
        let change = ReferenceChange {
            issue: "broken-issue".into(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = image_with(vec![(
            VirtualPath::data("issues/broken-issue.json").unwrap(),
            file_entry(b"not json"),
        )]);
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::MalformedIssueJson(_)
        ));
    }

    #[test]
    fn test_finalize_archive_execution_issue_id_mismatch() {
        let mut issue = fixture_issue("Owner".into(), String::new());
        issue.id = "embedded-id".into();
        let change = ReferenceChange {
            issue: "requested-id".into(),
            document_index: 0,
            from_path: "doc.md".into(),
            to_path: "archived.md".into(),
        };
        let plan = relink_plan(change);
        let image = image_with(vec![(
            VirtualPath::data("issues/requested-id.json").unwrap(),
            file_entry(&crate::repository_state::serialize_issue(&issue).unwrap()),
        )]);
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::IssueIdMismatch { expected, found }
                if expected == "requested-id" && found == "embedded-id"
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
        let image = image_with(Vec::new());
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
        let image = image_with(Vec::new());
        let error = archive_execution_error(&image, &plan);
        assert!(matches!(
            error,
            ArchiveExecutionError::Producer(ProducerError::ProposedLayoutTargetAbsent { target })
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
            ArchiveExecutionError::CapturedIssueCacheLost("owner-1".into()).to_string(),
            "captured issue cache lost owner-1"
        );
        assert_eq!(
            ArchiveExecutionError::MissingChangedIssue("owner-1".into()).to_string(),
            "changed archive issue owner-1 was not captured"
        );
    }
}
