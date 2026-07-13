//! Unified dependency-aware archive planning and coordinated execution.

use super::CommandExecutor;
use crate::domain::artifact_classifier::{
    artifact_destination_root, artifact_mirror_destination, classify_artifacts,
    ArtifactClassificationInventory, ArtifactClassificationPolicy, ArtifactLocation,
    ArtifactLocationFacts,
};
use crate::domain::artifact_execution::{ArchiveExecutionResult, ArchivePublication};
use crate::domain::artifact_inventory::{inventory_explicit_roots, ExplicitRootTarget};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArchiveCandidates, ArtifactAction, ArtifactPlan, BlockerCode,
    ContentIdentity, EdgeKind, EdgeResolutionMode, PendingDeletion, PlanBlocker, PlanTarget,
    PlanWarning, ReferenceChange, WarningCode,
};
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{Event, Issue};
use crate::storage::{
    collect_artifact_classification_facts, discover_artifact_dependencies,
    discover_repository_embedded_owners, GitRevisionResolver, IssueStore, JsonFileStorage,
    VerifiedArtifact,
};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

enum ArchiveTarget<'a> {
    Document(&'a str),
    Container(&'a str),
}

fn terminal_container_ids(issues: &[Issue], hierarchy: &HierarchyConfig) -> Vec<String> {
    let leaf_level = hierarchy.types().map(|(_, level)| *level).max();
    let mut ids = issues
        .iter()
        .filter(|issue| issue.state.is_terminal())
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
    /// Fully evaluate every terminal configured non-leaf container without mutation.
    pub fn archive_candidates(&self) -> Result<ArchiveCandidates> {
        let hierarchy = crate::config_manager::get_hierarchy_config(&self.storage)?;
        let issues = self.storage.list_issues()?;
        let plans = terminal_container_ids(&issues, &hierarchy)
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
        let repo_root = self
            .storage
            .root()
            .parent()
            .context("archive planning requires .jit beneath a repository root")?;
        let resolver = GitRevisionResolver::new(repo_root);

        let root_container_id = match target {
            ArchiveTarget::Document(_) => None,
            ArchiveTarget::Container(id) => Some(self.storage.resolve_issue_id(id)?),
        };
        let policy =
            ArtifactClassificationPolicy::from_documentation(config.documentation.as_ref());
        let target_for_layout = match (&target, root_container_id.as_deref()) {
            (ArchiveTarget::Document(path), _) => PlanTarget::Document {
                path: normalize_artifact_path(path),
            },
            (ArchiveTarget::Container(_), Some(id)) => PlanTarget::Container { id: id.into() },
            (ArchiveTarget::Container(_), None) => unreachable!("container id was resolved"),
        };
        let destination_root = artifact_destination_root(&target_for_layout, &policy.archive_root);
        // Feed inverse-mirror sources to inventory while retaining the real
        // durable issue records for exact apply/observe decisions below.
        let mut inventory_issues = issues.clone();
        remap_archived_references(&mut inventory_issues, &destination_root);
        let explicit_target = match (&target, root_container_id.as_deref()) {
            (ArchiveTarget::Document(path), _) => ExplicitRootTarget::Document(path),
            (ArchiveTarget::Container(_), Some(id)) => ExplicitRootTarget::Container(id),
            (ArchiveTarget::Container(_), None) => unreachable!("container id was resolved"),
        };
        let inventory =
            inventory_explicit_roots(&inventory_issues, &hierarchy, explicit_target, &resolver)?;
        let discovered = discover_artifact_dependencies(&self.storage, inventory)?;
        let member_ids = discovered
            .member_ids()
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>();
        let embedded_owners =
            discover_repository_embedded_owners(&self.storage, &issues, &member_ids)?;
        let (plan_target, artifacts, mut blockers) = discovered.into_plan_parts();

        if let Some(container_id) = root_container_id.as_deref() {
            if issues
                .iter()
                .find(|issue| issue.id == container_id)
                .is_some_and(|issue| !issue.state.is_terminal())
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
            ArtifactClassificationInventory::new(plan_target, artifacts, blockers),
            policy,
            facts,
        )
        .map_err(Into::into)
    }
}

/// Deterministic archive failure-injection boundary.
///
/// Production uses the default no-op implementation. Tests can fail or edit a
/// named phase without replacing storage, changing permissions, or using global
/// mutable state.
pub trait ArchiveExecutionHooks {
    fn before_stage(&mut self, _index: usize, _source: &str) -> Result<()> {
        Ok(())
    }
    fn before_relink(&mut self, _index: usize, _issue: &str) -> Result<()> {
        Ok(())
    }
    fn before_publish(&mut self, _index: usize, _destination: &str) -> Result<()> {
        Ok(())
    }
    fn before_marker_inspect(&mut self, _destination: &str) -> Result<()> {
        Ok(())
    }
    fn before_event_append(&mut self) -> Result<()> {
        Ok(())
    }
    fn before_revert(&mut self, _index: usize, _issue: &str) -> Result<()> {
        Ok(())
    }
    fn before_delete(&mut self, _index: usize, _source: &str) -> Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct NoArchiveExecutionHooks;
impl ArchiveExecutionHooks for NoArchiveExecutionHooks {}

struct StagedPublication {
    source: String,
    destination: String,
    identity: ContentIdentity,
    staged: VerifiedArtifact,
}

#[derive(Default)]
struct ArchiveCoverage {
    publications: BTreeSet<(String, String, u64)>,
    reference_changes: BTreeSet<(String, usize, String, String)>,
}

struct FailedExecutionState<'a> {
    plan: &'a ArtifactPlan,
    artifacts: &'a [crate::domain::artifact_plan::ArtifactPlanEntry],
    coverage: &'a ArchiveCoverage,
    publications: Vec<ArchivePublication>,
    reconciling: bool,
}

impl ArchiveCoverage {
    fn from_events(events: &[Event], target: &PlanTarget, destination_root: &str) -> Self {
        events
            .iter()
            .filter_map(|event| match event {
                Event::ArtifactArchiveExecuted {
                    target: event_target,
                    destination_root: event_root,
                    publications,
                    reference_changes,
                    ..
                } if event_target == target && event_root == destination_root => {
                    Some((publications, reference_changes))
                }
                _ => None,
            })
            .fold(Self::default(), |mut coverage, (publications, changes)| {
                coverage
                    .publications
                    .extend(publications.iter().map(|publication| {
                        (
                            publication.destination.clone(),
                            publication.content_identity.sha256().to_string(),
                            publication.content_identity.byte_size(),
                        )
                    }));
                coverage
                    .reference_changes
                    .extend(changes.iter().map(|change| {
                        (
                            change.issue.clone(),
                            change.document_index,
                            change.from_path.clone(),
                            change.to_path.clone(),
                        )
                    }));
                coverage
            })
    }

    fn covers_change(&self, change: &ReferenceChange) -> bool {
        self.reference_changes.contains(&(
            change.issue.clone(),
            change.document_index,
            change.from_path.clone(),
            change.to_path.clone(),
        ))
    }

    fn covers_publication(&self, destination: &str, identity: &ContentIdentity) -> bool {
        self.publications.contains(&(
            destination.to_string(),
            identity.sha256().to_string(),
            identity.byte_size(),
        ))
    }
}

impl CommandExecutor<JsonFileStorage> {
    /// Execute a freshly recomputed document archive plan under one write guard.
    pub fn execute_archive_document(&self, path: &str) -> Result<ArchiveExecutionResult> {
        self.execute_archive_document_with_hooks(path, &mut NoArchiveExecutionHooks)
    }

    /// Execute a freshly recomputed container archive plan under one write guard.
    pub fn execute_archive_container(&self, id: &str) -> Result<ArchiveExecutionResult> {
        self.execute_archive_container_with_hooks(id, &mut NoArchiveExecutionHooks)
    }

    #[doc(hidden)]
    pub fn execute_archive_document_with_hooks<H: ArchiveExecutionHooks>(
        &self,
        path: &str,
        hooks: &mut H,
    ) -> Result<ArchiveExecutionResult> {
        self.execute_archive_target(ArchiveTarget::Document(path), hooks)
    }

    #[doc(hidden)]
    pub fn execute_archive_container_with_hooks<H: ArchiveExecutionHooks>(
        &self,
        id: &str,
        hooks: &mut H,
    ) -> Result<ArchiveExecutionResult> {
        self.execute_archive_target(ArchiveTarget::Container(id), hooks)
    }

    fn execute_archive_target<H: ArchiveExecutionHooks>(
        &self,
        target: ArchiveTarget<'_>,
        hooks: &mut H,
    ) -> Result<ArchiveExecutionResult> {
        // This guard belongs to the same storage instance and thread used for
        // every nested read/write below. It intentionally spans recomputation
        // through the final deletion attempt.
        let _repo_write_guard = self.storage.acquire_repo_write_lock()?;
        let plan = self.plan_archive_target(target)?;
        let artifacts = plan.executable_artifacts()?;

        let prior_events = self.storage.read_artifact_archive_events()?;
        let coverage =
            ArchiveCoverage::from_events(&prior_events, plan.target(), plan.destination_root());
        let mut stage_index = 0;
        let mut staged = Vec::new();
        for artifact in artifacts.iter().filter(|artifact| {
            matches!(
                artifact.action(),
                ArtifactAction::Move | ArtifactAction::Copy
            ) && !artifact.already_archived()
        }) {
            hooks.before_stage(stage_index, artifact.source())?;
            stage_index += 1;
            let identity = artifact
                .content_identity()
                .context("executable publication lacks content identity")?
                .clone();
            let destination = artifact
                .destination()
                .context("executable publication lacks destination")?
                .to_string();
            let verified = self.storage.verify_staged_artifact(
                self.storage.stage_artifact(artifact.source())?,
                &identity,
            )?;
            staged.push(StagedPublication {
                source: artifact.source().to_string(),
                destination,
                identity,
                staged: verified,
            });
        }
        validate_proposed_layout(&plan)?;

        let marker = self.prepare_container_marker(&plan, hooks, &mut stage_index)?;
        if let Some((publication, staged_marker)) = marker.as_ref() {
            if !publication.adopted {
                self.storage
                    .preflight_staged_artifact(staged_marker, &publication.destination)?;
            }
        }
        for publication in &staged {
            self.storage
                .preflight_staged_artifact(&publication.staged, &publication.destination)?;
        }
        let mut publications = artifacts
            .iter()
            .filter(|artifact| artifact.already_archived())
            .filter_map(|artifact| {
                let destination = artifact.destination()?;
                let identity = artifact.content_identity()?;
                (!coverage.covers_publication(destination, identity)).then(|| ArchivePublication {
                    source: Some(artifact.source().to_string()),
                    destination: destination.to_string(),
                    content_identity: identity.clone(),
                    adopted: true,
                })
            })
            .collect::<Vec<_>>();
        let mut reconciling = !publications.is_empty();
        let mut publish_index = 0;

        if let Some((publication, staged_marker)) = marker {
            if publication.adopted {
                if !coverage
                    .covers_publication(&publication.destination, &publication.content_identity)
                {
                    reconciling = true;
                    publications.push(publication);
                }
            } else {
                let publish = hooks
                    .before_publish(publish_index, &publication.destination)
                    .and_then(|()| {
                        self.storage
                            .publish_staged_artifact(staged_marker, &publication.destination)
                    });
                publish_index += 1;
                if let Err(cause) = publish {
                    return Err(self.record_failed_execution_mutations(
                        FailedExecutionState {
                            plan: &plan,
                            artifacts,
                            coverage: &coverage,
                            publications,
                            reconciling,
                        },
                        hooks,
                        cause,
                    ));
                }
                publications.push(publication);
            }
        }
        for publication in staged {
            let durable = ArchivePublication {
                source: Some(publication.source),
                destination: publication.destination,
                content_identity: publication.identity,
                adopted: false,
            };
            let publish = hooks
                .before_publish(publish_index, &durable.destination)
                .and_then(|()| {
                    self.storage
                        .publish_staged_artifact(publication.staged, &durable.destination)
                });
            publish_index += 1;
            if let Err(cause) = publish {
                return Err(self.record_failed_execution_mutations(
                    FailedExecutionState {
                        plan: &plan,
                        artifacts,
                        coverage: &coverage,
                        publications,
                        reconciling,
                    },
                    hooks,
                    cause,
                ));
            }
            publications.push(durable);
        }

        let (saved_snapshots, mut event_changes, observed_uncovered_change) =
            match self.apply_reference_changes(artifacts, &coverage, hooks) {
                Ok(applied) => applied,
                Err(cause) => {
                    return Err(self.record_failed_execution_mutations(
                        FailedExecutionState {
                            plan: &plan,
                            artifacts,
                            coverage: &coverage,
                            publications,
                            reconciling,
                        },
                        hooks,
                        cause,
                    ));
                }
            };
        if observed_uncovered_change {
            reconciling = true;
        }
        event_changes.sort_by(|left, right| {
            (
                &left.issue,
                left.document_index,
                &left.from_path,
                &left.to_path,
            )
                .cmp(&(
                    &right.issue,
                    right.document_index,
                    &right.from_path,
                    &right.to_path,
                ))
        });
        event_changes.dedup();

        let mut warnings = plan.warnings().to_vec();
        warnings.extend(
            artifacts
                .iter()
                .flat_map(|artifact| artifact.warnings().iter().cloned()),
        );
        let deletion_candidates = self.preflight_deletions(artifacts, &mut warnings)?;
        let event_needed = !publications.is_empty()
            || !event_changes.is_empty()
            || !deletion_candidates.is_empty();
        let planned_deletions = artifacts
            .iter()
            .flat_map(|artifact| artifact.pending_deletions().iter().cloned())
            .collect::<Vec<_>>();

        if event_needed {
            let event = Event::new_artifact_archive_executed(
                plan.target().clone(),
                plan.destination_root().to_string(),
                publications.clone(),
                event_changes.clone(),
                planned_deletions.clone(),
                reconciling,
            );
            let append = hooks
                .before_event_append()
                .and_then(|()| self.storage.append_event(&event));
            if let Err(cause) = append {
                return Err(rollback_reference_changes(
                    &self.storage,
                    saved_snapshots,
                    hooks,
                    cause,
                ));
            }
        }

        let mut deleted_sources = Vec::new();
        for (index, deletion) in deletion_candidates.iter().enumerate() {
            let attempt = hooks.before_delete(index, &deletion.source).and_then(|()| {
                self.storage
                    .delete_artifact_if_identity(&deletion.source, &deletion.content_identity)
            });
            match attempt {
                Ok(()) => deleted_sources.push(deletion.source.clone()),
                Err(_) => warnings.push(PlanWarning::new(
                    WarningCode::DeletionFailed,
                    Some(&deletion.source),
                )),
            }
        }
        canonicalize_warnings(&mut warnings);

        Ok(ArchiveExecutionResult {
            schema_version: 1,
            target: plan.target().clone(),
            destination_root: plan.destination_root().to_string(),
            publications,
            reference_changes: event_changes,
            planned_deletions,
            deleted_sources,
            warnings,
            event_appended: event_needed,
            reconciling,
        })
    }

    fn prepare_container_marker<H: ArchiveExecutionHooks>(
        &self,
        plan: &ArtifactPlan,
        hooks: &mut H,
        stage_index: &mut usize,
    ) -> Result<Option<(ArchivePublication, VerifiedArtifact)>> {
        let PlanTarget::Container { id } = plan.target() else {
            return Ok(None);
        };
        let destination = format!("{}/.jit-container", plan.destination_root());
        let bytes = format!("{id}\n").into_bytes();
        let identity = ContentIdentity::from_bytes(&bytes);
        hooks.before_marker_inspect(&destination)?;
        match self.storage.stage_artifact_if_exists(&destination)? {
            Some(existing) => {
                let verified = self
                    .storage
                    .verify_staged_artifact(existing, &identity)
                    .with_context(|| {
                        format!("container ownership marker changed after planning: {destination}")
                    })?;
                Ok(Some((
                    ArchivePublication {
                        source: None,
                        destination,
                        content_identity: identity,
                        adopted: true,
                    },
                    verified,
                )))
            }
            None => {
                hooks.before_stage(*stage_index, &destination)?;
                *stage_index += 1;
                let verified = self.storage.verify_staged_artifact(
                    self.storage.stage_artifact_bytes(&bytes)?,
                    &identity,
                )?;
                Ok(Some((
                    ArchivePublication {
                        source: None,
                        destination,
                        content_identity: identity,
                        adopted: false,
                    },
                    verified,
                )))
            }
        }
    }

    fn apply_reference_changes<H: ArchiveExecutionHooks>(
        &self,
        artifacts: &[crate::domain::artifact_plan::ArtifactPlanEntry],
        coverage: &ArchiveCoverage,
        hooks: &mut H,
    ) -> Result<(Vec<Issue>, Vec<ReferenceChange>, bool)> {
        let changes = artifacts
            .iter()
            .flat_map(|artifact| artifact.reference_changes().iter().cloned())
            .collect::<Vec<_>>();
        let grouped = changes.into_iter().fold(
            BTreeMap::<String, Vec<ReferenceChange>>::new(),
            |mut grouped, change| {
                grouped
                    .entry(change.issue.clone())
                    .or_default()
                    .push(change);
                grouped
            },
        );
        let mut prepared = Vec::new();
        let mut observed = Vec::new();
        for (issue_id, changes) in grouped {
            let original = self.storage.load_issue(&issue_id)?;
            let mut updated = original.clone();
            let mut applied = Vec::new();
            for change in changes {
                let document = updated
                    .documents
                    .get_mut(change.document_index)
                    .ok_or_else(|| {
                        anyhow!(
                            "planned document index {} is absent on issue {}",
                            change.document_index,
                            issue_id
                        )
                    })?;
                if document.commit.is_some() {
                    bail!("planned relink points at pinned issue document: {issue_id}");
                }
                let actual = normalize_artifact_path(&document.path);
                if actual == change.from_path {
                    document.path = change.to_path.clone();
                    document.assets.clear();
                    applied.push(change);
                } else if actual == change.to_path {
                    if !coverage.covers_change(&change) {
                        observed.push(change);
                    }
                } else {
                    bail!(
                        "planned relink no longer matches issue {} document {}: expected {} or {}, found {}",
                        issue_id,
                        change.document_index,
                        change.from_path,
                        change.to_path,
                        actual
                    );
                }
            }
            if !applied.is_empty() {
                prepared.push((original, updated, applied));
            }
        }

        let observed_uncovered = !observed.is_empty();
        let mut snapshots = Vec::new();
        let mut event_changes = observed;
        for (index, (original, updated, applied)) in prepared.into_iter().enumerate() {
            if let Err(cause) = hooks
                .before_relink(index, &original.id)
                .and_then(|()| self.storage.save_issue(updated))
            {
                return Err(rollback_reference_changes(
                    &self.storage,
                    snapshots,
                    hooks,
                    cause,
                ));
            }
            event_changes.extend(applied);
            snapshots.push(original);
        }
        Ok((snapshots, event_changes, observed_uncovered))
    }

    fn record_failed_execution_mutations<H: ArchiveExecutionHooks>(
        &self,
        state: FailedExecutionState<'_>,
        hooks: &mut H,
        cause: anyhow::Error,
    ) -> anyhow::Error {
        let reference_changes = match self
            .durable_uncovered_reference_changes(state.artifacts, state.coverage)
        {
            Ok(changes) => changes,
            Err(inspect) => {
                return cause.context(format!(
                    "archive failed after a durable mutation and residual reference inspection failed: {inspect:#}"
                ));
            }
        };
        if state.publications.is_empty() && reference_changes.is_empty() {
            return cause;
        }
        let event = Event::new_artifact_archive_executed(
            state.plan.target().clone(),
            state.plan.destination_root().to_string(),
            state.publications,
            reference_changes,
            Vec::new(),
            state.reconciling,
        );
        match hooks
            .before_event_append()
            .and_then(|()| self.storage.append_event(&event))
        {
            Ok(()) => cause.context(
                "archive aborted after durable mutations; their exact successful subset was recorded",
            ),
            Err(record) => cause.context(format!(
                "archive aborted after durable mutations and recording that subset failed: {record:#}; rerun will reconcile"
            )),
        }
    }

    fn durable_uncovered_reference_changes(
        &self,
        artifacts: &[crate::domain::artifact_plan::ArtifactPlanEntry],
        coverage: &ArchiveCoverage,
    ) -> Result<Vec<ReferenceChange>> {
        let mut issues = BTreeMap::new();
        let candidates = artifacts
            .iter()
            .flat_map(|artifact| artifact.reference_changes())
            .filter(|change| !coverage.covers_change(change))
            .collect::<Vec<_>>();
        let mut changes = Vec::new();
        for change in candidates {
            if !issues.contains_key(&change.issue) {
                issues.insert(
                    change.issue.clone(),
                    self.storage.load_issue(&change.issue)?,
                );
            }
            if issues[&change.issue]
                .documents
                .get(change.document_index)
                .is_some_and(|document| {
                    document.commit.is_none()
                        && normalize_artifact_path(&document.path) == change.to_path
                })
            {
                changes.push(change.clone());
            }
        }
        changes.sort_by(|left, right| {
            (
                &left.issue,
                left.document_index,
                &left.from_path,
                &left.to_path,
            )
                .cmp(&(
                    &right.issue,
                    right.document_index,
                    &right.from_path,
                    &right.to_path,
                ))
        });
        changes.dedup();
        Ok(changes)
    }

    fn preflight_deletions(
        &self,
        artifacts: &[crate::domain::artifact_plan::ArtifactPlanEntry],
        warnings: &mut Vec<PlanWarning>,
    ) -> Result<Vec<PendingDeletion>> {
        let mut candidates = Vec::new();
        for artifact in artifacts {
            for deletion in artifact.pending_deletions() {
                let durable = artifact.reference_changes().iter().all(|change| {
                    self.storage
                        .load_issue(&change.issue)
                        .ok()
                        .and_then(|issue| issue.documents.get(change.document_index).cloned())
                        .is_some_and(|document| {
                            document.commit.is_none()
                                && normalize_artifact_path(&document.path) == change.to_path
                        })
                });
                if !durable {
                    warnings.push(PlanWarning::new(
                        WarningCode::DeletionFailed,
                        Some(&deletion.source),
                    ));
                    continue;
                }
                let verified = self
                    .storage
                    .stage_artifact(&deletion.source)
                    .and_then(|stage| {
                        self.storage
                            .verify_staged_artifact(stage, &deletion.content_identity)
                    });
                match verified {
                    Ok(stage) => {
                        drop(stage);
                        candidates.push(deletion.clone());
                    }
                    Err(_) => warnings.push(PlanWarning::new(
                        WarningCode::DeletionFailed,
                        Some(&deletion.source),
                    )),
                }
            }
        }
        Ok(candidates)
    }
}

fn rollback_reference_changes<H: ArchiveExecutionHooks>(
    storage: &JsonFileStorage,
    snapshots: Vec<Issue>,
    hooks: &mut H,
    cause: anyhow::Error,
) -> anyhow::Error {
    for (index, snapshot) in snapshots.into_iter().rev().enumerate() {
        if let Err(revert) = hooks
            .before_revert(index, &snapshot.id)
            .and_then(|()| storage.restore_issue_verbatim(snapshot))
        {
            return cause.context(format!(
                "archive reference rollback failed after durable mutation: {revert:#}; rerun will reconcile the adopted state"
            ));
        }
    }
    cause.context("archive aborted; every applied reference change was reverted")
}

fn validate_proposed_layout(plan: &ArtifactPlan) -> Result<()> {
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
                anyhow!("supported archive edge target is absent from plan: {target_source}")
            })?;
            let available = proposed_available_paths(target);
            for parent_path in proposed_available_paths(parent) {
                let resolved = match edge.resolution_mode {
                    EdgeResolutionMode::Relative => {
                        let parent_dir = Path::new(&parent_path).parent().unwrap_or(Path::new(""));
                        normalize_artifact_path(&parent_dir.join(&edge.reference).to_string_lossy())
                    }
                    EdgeResolutionMode::RootRelative => {
                        normalize_artifact_path(edge.reference.trim_start_matches('/'))
                    }
                    EdgeResolutionMode::External => continue,
                };
                if !available.contains(&resolved) {
                    bail!(
                        "supported edge {} from {} resolves to {} in the proposed layout, not an available target location ({})",
                        edge.reference,
                        parent.source(),
                        resolved,
                        available.iter().cloned().collect::<Vec<_>>().join(", ")
                    );
                }
            }
        }
    }
    Ok(())
}

fn proposed_available_paths(
    artifact: &crate::domain::artifact_plan::ArtifactPlanEntry,
) -> BTreeSet<String> {
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

fn canonicalize_warnings(warnings: &mut Vec<PlanWarning>) {
    warnings.sort_by(|left, right| {
        (left.code.as_str(), left.path.as_deref())
            .cmp(&(right.code.as_str(), right.path.as_deref()))
    });
    warnings.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, Issue, State};
    use crate::storage::JsonFileStorage;
    use std::collections::HashMap;
    use std::fs;
    use tempfile::TempDir;

    #[derive(Default)]
    struct FaultHooks {
        fail_stage: Option<usize>,
        fail_publish: Option<usize>,
        fail_relink: Option<usize>,
        fail_event: bool,
        fail_revert: Option<usize>,
        fail_delete: Option<usize>,
        edit_before_delete: Option<(std::path::PathBuf, Vec<u8>)>,
        torn_event_path: Option<std::path::PathBuf>,
        marker_inspect: Option<Box<dyn FnOnce() -> Result<()>>>,
    }

    #[test]
    fn test_terminal_container_ids_use_live_non_leaf_levels_and_terminal_semantics() {
        let hierarchy = HierarchyConfig::new(
            HashMap::from([("portfolio".to_string(), 2), ("unit".to_string(), 7)]),
            HashMap::new(),
        )
        .unwrap();
        let issue = |id: &str, state: State, issue_type: Option<&str>| {
            let mut issue = Issue::new(id.to_string(), String::new());
            issue.id = id.to_string();
            issue.state = state;
            issue.labels = issue_type
                .map(|kind| vec![format!("type:{kind}")])
                .unwrap_or_default();
            issue
        };
        let issues = vec![
            issue("done-container", State::Done, Some("portfolio")),
            issue("rejected-container", State::Rejected, Some("portfolio")),
            issue("active-container", State::InProgress, Some("portfolio")),
            issue("archived-container", State::Archived, Some("portfolio")),
            issue("done-leaf", State::Done, Some("unit")),
            issue("done-unknown", State::Done, Some("epic")),
            issue("done-untyped", State::Done, None),
        ];

        assert_eq!(
            terminal_container_ids(&issues, &hierarchy),
            vec!["done-container", "rejected-container"]
        );
    }

    impl ArchiveExecutionHooks for FaultHooks {
        fn before_stage(&mut self, index: usize, _source: &str) -> Result<()> {
            if self.fail_stage == Some(index) {
                bail!("injected stage failure {index}");
            }
            Ok(())
        }

        fn before_relink(&mut self, index: usize, _issue: &str) -> Result<()> {
            if self.fail_relink == Some(index) {
                bail!("injected relink failure {index}");
            }
            Ok(())
        }

        fn before_publish(&mut self, index: usize, _destination: &str) -> Result<()> {
            if self.fail_publish == Some(index) {
                bail!("injected publication failure {index}");
            }
            Ok(())
        }

        fn before_marker_inspect(&mut self, _destination: &str) -> Result<()> {
            if let Some(inspect) = self.marker_inspect.take() {
                inspect()?;
            }
            Ok(())
        }

        fn before_event_append(&mut self) -> Result<()> {
            if let Some(path) = self.torn_event_path.take() {
                use std::io::Write;
                std::fs::OpenOptions::new()
                    .append(true)
                    .open(path)?
                    .write_all(b"{\"type\":\"artifact_archive")?;
            }
            if self.fail_event {
                bail!("injected event append failure");
            }
            Ok(())
        }

        fn before_revert(&mut self, index: usize, _issue: &str) -> Result<()> {
            if self.fail_revert == Some(index) {
                bail!("injected revert failure {index}");
            }
            Ok(())
        }

        fn before_delete(&mut self, index: usize, _source: &str) -> Result<()> {
            if let Some((path, bytes)) = self.edit_before_delete.take() {
                fs::write(path, bytes)?;
            }
            if self.fail_delete == Some(index) {
                bail!("injected deletion failure {index}");
            }
            Ok(())
        }
    }

    fn executable_document_repo(
        owner_count: usize,
        content: &str,
    ) -> (TempDir, CommandExecutor<JsonFileStorage>, Vec<String>) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), content).unwrap();
        let ids = (0..owner_count)
            .map(|index| {
                let mut issue = Issue::new(format!("Owner {index}"), String::new());
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
                storage.save_issue(issue).unwrap();
                id
            })
            .collect();
        (repo, CommandExecutor::new(storage), ids)
    }

    fn assert_guard_released(storage: &JsonFileStorage) {
        let storage = storage.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let acquired = storage.acquire_repo_write_lock().is_ok();
            sender.send(acquired).unwrap();
        });
        assert_eq!(
            receiver.recv_timeout(std::time::Duration::from_secs(2)),
            Ok(true),
            "failed execution retained the repository write guard"
        );
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
        active_executor.storage.save_issue(issue).unwrap();
        assert!(active_executor
            .execute_archive_document("fixtures/root.md")
            .is_err());
        assert!(active_repo.path().join("fixtures/root.md").exists());
        assert!(!active_repo.path().join("archive/fixtures/root.md").exists());
    }

    #[test]
    fn test_partial_staging_and_partial_relink_failures_cleanup_and_restore_exactly() {
        let (repo, executor, _) = executable_document_repo(1, "![asset](asset.png)\n");
        fs::write(repo.path().join("fixtures/asset.png"), b"asset").unwrap();
        let mut stage_fault = FaultHooks {
            fail_stage: Some(1),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut stage_fault)
            .is_err());
        assert!(repo.path().join("fixtures/root.md").exists());
        assert!(!repo.path().join("archive/fixtures/root.md").exists());
        assert_eq!(
            fs::read_dir(repo.path().join(".jit/tmp")).unwrap().count(),
            0
        );
        assert_guard_released(&executor.storage);

        let (repo, executor, ids) = executable_document_repo(2, "two owners");
        let before = ids
            .iter()
            .map(|id| executor.storage.load_issue(id).unwrap())
            .collect::<Vec<_>>();
        let mut relink_fault = FaultHooks {
            fail_relink: Some(1),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut relink_fault)
            .is_err());
        for snapshot in before {
            assert_eq!(executor.storage.load_issue(&snapshot.id).unwrap(), snapshot);
        }
        assert!(repo.path().join("fixtures/root.md").exists());
        assert_guard_released(&executor.storage);
    }

    #[test]
    fn test_later_publication_failure_records_only_successful_publications_before_return() {
        let (repo, executor, _) = executable_document_repo(1, "![asset](asset.png)\n");
        fs::write(repo.path().join("fixtures/asset.png"), b"asset").unwrap();
        let mut fault = FaultHooks {
            fail_publish: Some(1),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut fault)
            .is_err());
        let first_events = executor.storage.read_artifact_archive_events().unwrap();
        assert_eq!(first_events.len(), 1);
        let first_publications = match &first_events[0] {
            Event::ArtifactArchiveExecuted {
                publications,
                reference_changes,
                planned_deletions,
                ..
            } => {
                assert!(reference_changes.is_empty());
                assert!(planned_deletions.is_empty());
                publications.clone()
            }
            _ => unreachable!(),
        };
        assert_eq!(first_publications.len(), 1);

        executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        let events = executor.storage.read_artifact_archive_events().unwrap();
        assert_eq!(events.len(), 2);
        let destinations = events
            .iter()
            .flat_map(|event| match event {
                Event::ArtifactArchiveExecuted { publications, .. } => publications.as_slice(),
                _ => &[],
            })
            .map(|publication| publication.destination.clone())
            .collect::<Vec<_>>();
        assert_eq!(destinations.len(), 2);
        assert_eq!(destinations.iter().collect::<BTreeSet<_>>().len(), 2);
    }

    #[test]
    fn test_post_publication_relink_failure_records_publication_and_rerun_does_not_duplicate_it() {
        let (_repo, executor, ids) = executable_document_repo(2, "two owners");
        let before = ids
            .iter()
            .map(|id| executor.storage.load_issue(id).unwrap())
            .collect::<Vec<_>>();
        let mut fault = FaultHooks {
            fail_relink: Some(1),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut fault)
            .is_err());
        for snapshot in &before {
            assert_eq!(
                executor.storage.load_issue(&snapshot.id).unwrap(),
                *snapshot
            );
        }
        let first_events = executor.storage.read_artifact_archive_events().unwrap();
        assert!(matches!(
            first_events.as_slice(),
            [Event::ArtifactArchiveExecuted {
                publications,
                reference_changes,
                planned_deletions,
                ..
            }] if publications.len() == 1
                && reference_changes.is_empty()
                && planned_deletions.is_empty()
        ));

        executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        let events = executor.storage.read_artifact_archive_events().unwrap();
        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[1],
            Event::ArtifactArchiveExecuted {
                publications,
                reference_changes,
                ..
            } if publications.is_empty() && reference_changes.len() == 2
        ));
    }

    #[test]
    fn test_failed_relink_compensation_records_exact_residual_reference() {
        let (_repo, executor, ids) = executable_document_repo(2, "two owners");
        let mut fault = FaultHooks {
            fail_relink: Some(1),
            fail_revert: Some(0),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut fault)
            .is_err());
        let destination_count = ids
            .iter()
            .filter(|id| {
                executor.storage.load_issue(id).unwrap().documents[0].path
                    == "archive/fixtures/root.md"
            })
            .count();
        assert_eq!(destination_count, 1);
        let events = executor.storage.read_artifact_archive_events().unwrap();
        assert!(matches!(
            events.as_slice(),
            [Event::ArtifactArchiveExecuted {
                publications,
                reference_changes,
                planned_deletions,
                ..
            }] if publications.len() == 1
                && reference_changes.len() == 1
                && planned_deletions.is_empty()
        ));
    }

    #[test]
    fn test_event_failure_compensates_and_torn_tail_rerun_reconciles() {
        let (repo, executor, ids) = executable_document_repo(1, "event failure");
        let before = executor.storage.load_issue(&ids[0]).unwrap();
        let mut fault = FaultHooks {
            fail_event: true,
            torn_event_path: Some(repo.path().join(".jit/events.jsonl")),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut fault)
            .is_err());
        assert_eq!(executor.storage.load_issue(&ids[0]).unwrap(), before);
        assert!(repo.path().join("fixtures/root.md").exists());
        assert!(repo.path().join("archive/fixtures/root.md").exists());
        assert_guard_released(&executor.storage);

        let rerun = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(rerun.reconciling);
        assert!(rerun.event_appended);
        assert!(!repo.path().join("fixtures/root.md").exists());
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

        let valid = Event::new_artifact_archive_executed(
            PlanTarget::Document {
                path: "fixtures/root.md".into(),
            },
            "archive".into(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            true,
        );
        executor.storage.append_event(&valid).unwrap();
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            1
        );
        let valid_json = serde_json::to_string(&valid).unwrap();
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
    fn test_failed_event_and_revert_reconcile_and_edited_source_is_retained_stably() {
        let (repo, executor, ids) = executable_document_repo(1, "original");
        let mut double_fault = FaultHooks {
            fail_event: true,
            fail_revert: Some(0),
            ..Default::default()
        };
        assert!(executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut double_fault)
            .is_err());
        assert_eq!(
            executor.storage.load_issue(&ids[0]).unwrap().documents[0].path,
            "archive/fixtures/root.md"
        );
        assert_guard_released(&executor.storage);
        let reconciled = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(reconciled.reconciling);
        assert_eq!(reconciled.reference_changes.len(), 1);
        assert!(matches!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .last(),
            Some(Event::ArtifactArchiveExecuted {
                reference_changes,
                reconciling: true,
                ..
            }) if reference_changes.len() == 1
        ));
        assert!(!repo.path().join("fixtures/root.md").exists());

        let (edited_repo, edited_executor, _) = executable_document_repo(1, "original");
        let mut edit_fault = FaultHooks {
            edit_before_delete: Some((
                edited_repo.path().join("fixtures/root.md"),
                b"edited after planning".to_vec(),
            )),
            ..Default::default()
        };
        let first = edited_executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut edit_fault)
            .unwrap();
        assert!(first
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::DeletionFailed));
        assert_eq!(
            fs::read(edited_repo.path().join("fixtures/root.md")).unwrap(),
            b"edited after planning"
        );
        let event_count = edited_executor
            .storage
            .read_artifact_archive_events()
            .unwrap()
            .len();
        let stable = edited_executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(!stable.event_appended);
        assert!(stable
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::DeletionFailed));
        assert_eq!(
            edited_executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            event_count
        );
    }

    #[test]
    fn test_deletion_failure_rerun_converges_without_duplicate_destination_or_reference() {
        let (repo, executor, ids) = executable_document_repo(1, "delete retry");
        let mut fault = FaultHooks {
            fail_delete: Some(0),
            ..Default::default()
        };
        let first = executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut fault)
            .unwrap();
        assert!(first
            .warnings
            .iter()
            .any(|warning| warning.code == WarningCode::DeletionFailed));
        assert!(repo.path().join("fixtures/root.md").exists());
        assert!(repo.path().join("archive/fixtures/root.md").exists());

        let second = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert_eq!(second.deleted_sources, vec!["fixtures/root.md"]);
        assert!(!repo.path().join("fixtures/root.md").exists());
        assert_eq!(
            executor.storage.load_issue(&ids[0]).unwrap().documents[0].path,
            "archive/fixtures/root.md"
        );
    }

    #[test]
    fn test_proposed_layout_rejects_relative_edge_that_will_not_resolve_before_metadata() {
        use crate::domain::artifact_plan::{
            ArtifactEdge, ArtifactPlanEntry, ArtifactVersion, PolicyStatus,
        };
        let identity = ContentIdentity::from_bytes(b"root");
        let parent = ArtifactPlanEntry::new(
            "fixtures/root.md",
            ArtifactVersion::WorkingTree,
            ArtifactAction::Move,
        )
        .with_content_identity(identity.clone())
        .with_destination("archive/fixtures/root.md")
        .with_edges(vec![ArtifactEdge {
            reference: "target.png".into(),
            target: Some("fixtures/target.png".into()),
            kind: EdgeKind::Supported,
            resolution_mode: EdgeResolutionMode::Relative,
        }])
        .with_pending_deletions(vec![PendingDeletion {
            source: "fixtures/root.md".into(),
            content_identity: identity,
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
        assert!(validate_proposed_layout(&plan).is_err());
    }

    #[test]
    fn test_execution_preserves_positive_relative_and_root_relative_multi_edge_layout() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"shared\"]\narchive_root = \"archive\"\n",
        )
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
        let executor = CommandExecutor::new(storage);

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
    fn test_preflight_refuses_deletion_until_every_selected_reference_is_durable() {
        let (repo, executor, _) = executable_document_repo(1, "still linked");
        let plan = executor
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        let mut warnings = Vec::new();
        let candidates = executor
            .preflight_deletions(plan.executable_artifacts().unwrap(), &mut warnings)
            .unwrap();
        assert!(candidates.is_empty());
        assert!(warnings
            .iter()
            .any(|warning| warning.code == WarningCode::DeletionFailed));
        assert!(repo.path().join("fixtures/root.md").exists());
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
    fn test_replaced_identical_destination_uses_identity_bound_coverage_once() {
        let (repo, executor, _) = executable_document_repo(1, "original");
        let mut retain_source = FaultHooks {
            fail_delete: Some(0),
            ..Default::default()
        };
        executor
            .execute_archive_document_with_hooks("fixtures/root.md", &mut retain_source)
            .unwrap();
        let source = repo.path().join("fixtures/root.md");
        let destination = repo.path().join("archive/fixtures/root.md");
        fs::remove_file(&destination).unwrap();
        fs::write(&source, b"replacement").unwrap();
        fs::write(&destination, b"replacement").unwrap();

        let reconciled = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(reconciled.reconciling);
        assert!(reconciled.event_appended);
        assert!(reconciled.publications.iter().any(|publication| {
            publication.destination == "archive/fixtures/root.md"
                && publication.adopted
                && publication.content_identity == ContentIdentity::from_bytes(b"replacement")
        }));
        assert_eq!(
            executor
                .storage
                .read_artifact_archive_events()
                .unwrap()
                .len(),
            2
        );
        let stable = executor
            .execute_archive_document("fixtures/root.md")
            .unwrap();
        assert!(!stable.event_appended);
        assert!(stable.publications.is_empty());
    }

    #[test]
    fn test_publication_only_execution_records_one_event() {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir(repo.path().join("docs")).unwrap();
        fs::write(repo.path().join("docs/permanent.md"), b"permanent").unwrap();
        let executor = CommandExecutor::new(storage);
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
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = [\"docs\"]\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir_all(repo.path().join("archive/docs")).unwrap();
        fs::create_dir(repo.path().join("docs")).unwrap();
        fs::write(repo.path().join("docs/permanent.md"), b"permanent").unwrap();
        fs::write(repo.path().join("archive/docs/permanent.md"), b"permanent").unwrap();
        let executor = CommandExecutor::new(storage);

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
        let destination_root = format!("archive/{}", &id[..8]);
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
    fn test_container_marker_reinspection_rejects_leaf_and_parent_symlink_races() {
        use std::os::unix::fs::symlink;

        let (leaf_repo, leaf_executor, leaf_id) = configured_repo();
        let leaf_root = leaf_repo.path().join("archive").join(&leaf_id[..8]);
        fs::create_dir_all(&leaf_root).unwrap();
        let leaf_marker = leaf_root.join(".jit-container");
        let leaf_bytes = format!("{leaf_id}\n");
        fs::write(&leaf_marker, &leaf_bytes).unwrap();
        let leaf_target = leaf_repo.path().join("matching-marker");
        fs::write(&leaf_target, &leaf_bytes).unwrap();
        let mut leaf_race = FaultHooks {
            marker_inspect: Some(Box::new(move || {
                fs::remove_file(&leaf_marker)?;
                symlink(&leaf_target, &leaf_marker)?;
                Ok(())
            })),
            ..Default::default()
        };
        assert!(leaf_executor
            .execute_archive_container_with_hooks(&leaf_id, &mut leaf_race)
            .is_err());
        assert!(leaf_executor
            .storage
            .read_artifact_archive_events()
            .unwrap()
            .is_empty());

        let (parent_repo, parent_executor, parent_id) = configured_repo();
        let parent_root = parent_repo.path().join("archive").join(&parent_id[..8]);
        fs::create_dir_all(&parent_root).unwrap();
        fs::write(parent_root.join(".jit-container"), format!("{parent_id}\n")).unwrap();
        let external = parent_repo.path().join("matching-container");
        fs::create_dir(&external).unwrap();
        fs::write(external.join(".jit-container"), format!("{parent_id}\n")).unwrap();
        let mut parent_race = FaultHooks {
            marker_inspect: Some(Box::new(move || {
                fs::remove_file(parent_root.join(".jit-container"))?;
                fs::remove_dir(&parent_root)?;
                symlink(&external, &parent_root)?;
                Ok(())
            })),
            ..Default::default()
        };
        assert!(parent_executor
            .execute_archive_container_with_hooks(&parent_id, &mut parent_race)
            .is_err());
        assert!(parent_executor
            .storage
            .read_artifact_archive_events()
            .unwrap()
            .is_empty());
    }

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
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n\n[type_hierarchy]\ntypes = { epic = 1, task = 2 }\n[type_hierarchy.label_associations]\nepic = \"epic\"\n",
        )
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("fixtures/root.md"), b"shared").unwrap();
        git(repo.path(), &["init", "-q"]);
        git(repo.path(), &["config", "user.email", "test@example.com"]);
        git(repo.path(), &["config", "user.name", "Test"]);
        git(repo.path(), &["add", "fixtures/root.md"]);
        git(repo.path(), &["commit", "-qm", "fixture"]);
        let commit = git(repo.path(), &["rev-parse", "HEAD"]);

        let mut container = Issue::new("Container".into(), String::new());
        container.state = State::Done;
        container.labels = vec!["type:epic".into()];
        container.documents = vec![
            DocumentReference::new("fixtures/root.md".into()),
            DocumentReference::at_commit("fixtures/root.md".into(), commit),
        ];
        let container_id = container.id.clone();
        storage.save_issue(container).unwrap();
        let mut outside = Issue::new("Outside active".into(), String::new());
        outside.state = State::InProgress;
        outside.labels = vec!["type:task".into()];
        outside.documents = vec![DocumentReference::new("fixtures/root.md".into())];
        let outside_id = outside.id.clone();
        storage.save_issue(outside).unwrap();

        let executor = CommandExecutor::new(storage);
        let result = executor.execute_archive_container(&container_id).unwrap();
        let destination = format!("archive/{}/fixtures/root.md", &container_id[..8]);
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
        storage.init().unwrap();
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

        let mut epic = Issue::new("Archive fixture".into(), String::new());
        epic.state = State::Done;
        epic.labels = vec!["type:epic".into()];
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
        storage.save_issue(epic).unwrap();
        (repo, CommandExecutor::new(storage), id)
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
        storage.init().unwrap();
        fs::write(repo.path().join("root.csv"), "a,b").unwrap();
        let executor = CommandExecutor::new(storage.clone());
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
        storage.init().unwrap();
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
                storage.root().join("config.toml"),
                format!("[documentation]\n{authored}\n"),
            )
            .unwrap();
            let plan = CommandExecutor::new(storage.clone())
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
        executor.storage.save_issue(issue).unwrap();
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
        storage.init().unwrap();
        fs::write(
            storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir(repo.path().join("fixtures")).unwrap();
        fs::write(repo.path().join("real.md"), "[referent](secret.html)").unwrap();
        fs::write(repo.path().join("fixtures/secret.html"), "referent-only").unwrap();
        symlink(
            repo.path().join("real.md"),
            repo.path().join("fixtures/link.md"),
        )
        .unwrap();

        let plan = CommandExecutor::new(storage)
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
        regular_storage.init().unwrap();
        fs::write(
            regular_storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
        .unwrap();
        fs::create_dir(regular_repo.path().join("fixtures")).unwrap();
        fs::create_dir(regular_repo.path().join("real-archive")).unwrap();
        fs::write(regular_repo.path().join("fixtures/root.md"), "root").unwrap();
        symlink(
            regular_repo.path().join("real-archive"),
            regular_repo.path().join("archive"),
        )
        .unwrap();
        let destination_symlink = CommandExecutor::new(regular_storage)
            .preview_archive_document("fixtures/root.md")
            .unwrap();
        assert!(destination_symlink.artifacts()[0]
            .blockers()
            .iter()
            .any(|blocker| blocker.code == BlockerCode::SymlinkArtifact));

        let owner_repo = TempDir::new().unwrap();
        let owner_storage = JsonFileStorage::new(owner_repo.path().join(".jit"));
        owner_storage.init().unwrap();
        fs::write(
            owner_storage.root().join("config.toml"),
            "[documentation]\nmanaged_paths = [\"fixtures\"]\npermanent_paths = []\narchive_root = \"archive\"\n",
        )
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
        let mut outside = Issue::new("Active outside owner".into(), String::new());
        outside.state = State::InProgress;
        outside.documents = vec![DocumentReference::new("docs/outside.md".into())];
        owner_storage.save_issue(outside).unwrap();

        let owner_plan = CommandExecutor::new(owner_storage)
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
}
