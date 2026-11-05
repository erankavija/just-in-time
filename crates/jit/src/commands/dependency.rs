//! Dependency graph operations

use super::*;
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::errors::{DependencyBatchRejectedError, RedundantDependencyError};
use crate::repository_state::{finalize, MutationIntent};
use crate::storage::IssueNotFoundError;
use std::collections::HashSet;

/// A dependency mutation over resolved issue identities.
#[derive(Clone)]
enum CapturedDependencyMutation {
    Batch {
        issue_id: IssueId,
        dependency_ids: Vec<IssueId>,
        policy: RedundancyPolicy,
    },
    RemoveSingle {
        issue_id: IssueId,
        dependency_id: IssueId,
    },
    RemoveBatch {
        issue_id: IssueId,
        dependency_ids: Vec<IssueId>,
    },
}

enum CapturedDependencyOutcome {
    Batch(DependenciesAddResult),
    RemovedSingle { warning: Option<String> },
    RemovedBatch(DependenciesRemoveResult),
}

struct DerivedDependencyMutation {
    outcome: CapturedDependencyOutcome,
    intents: Vec<MutationIntent>,
    error_after_apply: Option<crate::errors::TransitionBlockedError>,
}

/// Result of adding multiple dependencies.
///
/// All-or-nothing (jit:c8518f2a): [`add_dependencies_with_policy`](GraphOps::add_dependencies_with_policy)
/// returns this only when every requested edge validated. A batch with any
/// rejected edge instead returns `Err(`[`DependencyBatchRejectedError`]`)` and
/// leaves the dependency set — and the event log — completely unchanged, so
/// there is no per-batch `errors` field to populate here.
#[derive(Debug, Serialize)]
pub struct DependenciesAddResult {
    pub added: Vec<IssueId>,
    pub already_exist: Vec<IssueId>,
    pub skipped: Vec<(IssueId, String)>, // (id, reason)
    #[serde(skip)]
    pub warnings: Vec<String>,
}

/// Result of removing multiple dependencies
#[derive(Debug, Default, Serialize)]
pub struct DependenciesRemoveResult {
    pub removed: Vec<IssueId>,
    pub not_found: Vec<IssueId>,
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl<S: IssueStore> GraphOps<'_, S> {
    /// Resolve a requested batch, aggregating resolution failures before graph validation.
    pub fn add_requested_dependencies(
        &self,
        from_id: &IssueId,
        requested: &[String],
        policy: RedundancyPolicy,
    ) -> Result<DependenciesAddResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let resolve = |id: &str| self.exec.issues().resolve_issue_id(&id.parse()?);
        let (resolved, rejected): (Vec<_>, Vec<_>) = requested
            .iter()
            .map(|id| (id, resolve(id)))
            .partition(|(_, result)| result.is_ok());
        if rejected.is_empty() {
            let resolved = resolved
                .into_iter()
                .map(|(_, result)| result)
                .collect::<Result<Vec<_>>>()?;
            self.add_dependencies_with_policy(from_id, &resolved, policy)
        } else {
            Err(crate::errors::DependencyBatchRejectedError::new(
                from_id.clone(),
                rejected
                    .into_iter()
                    .filter_map(|(id, result)| result.err().map(|error| (id.clone(), error)))
                    .collect(),
            )
            .into())
        }
    }

    /// Resolve targets against stored edges, including deleted issues. Repeated
    /// targets match only edges no earlier target named; unmatched text is retained.
    pub fn remove_requested_dependencies(
        &self,
        from_id: &IssueId,
        requested: &[String],
    ) -> Result<(DependenciesRemoveResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let prefixes = requested
            .iter()
            .map(|id| id.parse::<IssueIdPrefix>())
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let stored = self.exec.storage.load_issue(from_id)?.dependencies;
        let (matched, unmatched) = prefixes.into_iter().try_fold(
            (Vec::new(), Vec::new()),
            |(mut matched, mut unmatched): (Vec<IssueId>, Vec<String>), prefix| {
                let remaining = stored
                    .iter()
                    .filter(|id| !matched.contains(id))
                    .cloned()
                    .collect::<Vec<_>>();
                match crate::storage::resolve_dependency_id(&prefix, &remaining) {
                    Ok(id) => matched.push(id),
                    Err(error) if error.is::<crate::storage::IssueNotFoundError>() => {
                        unmatched.push(prefix.as_str().to_string())
                    }
                    Err(error) => return Err(error),
                }
                Ok((matched, unmatched))
            },
        )?;
        Ok((self.remove_dependencies(from_id, &matched)?, unmatched))
    }

    /// Remove a dependency from an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn remove_dependency(&self, issue_id: &IssueId, dep_id: &IssueId) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        match self.exec.publish_captured_dependency_mutation(
            CapturedDependencyMutation::RemoveSingle {
                issue_id: issue_id.clone(),
                dependency_id: dep_id.clone(),
            },
        )? {
            CapturedDependencyOutcome::RemovedSingle { warning } => {
                Ok(warning.into_iter().collect())
            }
            _ => unreachable!("single removal returns a single-removal outcome"),
        }
    }

    /// Add multiple dependencies under an explicit [`RedundancyPolicy`], atomically.
    ///
    /// Takes full ids the caller already resolved. All-or-nothing
    /// (jit:c8518f2a): every requested edge is validated — id resolution
    /// against the capture, cycle detection and (under
    /// [`RedundancyPolicy::Reject`]) the transitive-reduction check, all against
    /// the WOULD-BE final graph with every edge of this call applied at once —
    /// before anything is written. If any edge fails, `Err(`[`DependencyBatchRejectedError`]`)`
    /// names every rejected edge (REQ-02) and the dependency set, and the event
    /// log, are left completely unchanged (REQ-01, REQ-03). Only when every
    /// edge validates are the surviving edges applied and event-logged in one
    /// pass.
    ///
    /// Validating the full candidate graph up front (rather than one edge at a
    /// time against the graph as it stood before this call) also catches a
    /// violation that only emerges from the COMBINATION of two edges in the
    /// same call — e.g. two sibling edges that are each fine alone but jointly
    /// make one of them transitively redundant.
    pub fn add_dependencies_with_policy(
        &self,
        issue_id: &IssueId,
        dep_ids: &[IssueId],
        policy: RedundancyPolicy,
    ) -> Result<DependenciesAddResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }
        let warnings = self.exec.refuse_foreign_lease(issue_id)?;
        match self
            .exec
            .publish_captured_dependency_mutation(CapturedDependencyMutation::Batch {
                issue_id: issue_id.clone(),
                dependency_ids: dep_ids.to_vec(),
                policy,
            })? {
            CapturedDependencyOutcome::Batch(result) => {
                Ok(DependenciesAddResult { warnings, ..result })
            }
            _ => unreachable!("batch dependency request returns a batch outcome"),
        }
    }
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Capture the complete active issue graph and publish one dependency mutation.
    ///
    /// The issues listing is the sole discovery authority. The second phase captures every
    /// issue record, the complete issues-directory listing, and the event log;
    /// semantic derivation reads only that closed image. A conflict restarts both
    /// phases in a fresh mutation session while reusing one mutation context.
    fn publish_captured_dependency_mutation(
        &self,
        request: CapturedDependencyMutation,
    ) -> Result<CapturedDependencyOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let request = match request {
            removal @ (CapturedDependencyMutation::RemoveSingle { .. }
            | CapturedDependencyMutation::RemoveBatch { .. }) => {
                return self.publish_captured_dependency_removal(removal)
            }
            request => request,
        };
        let layout = self.require_layout()?;
        let context = super::production_mutation_context();

        with_mutation_attempts("dependency mutation", || {
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(crate::commands::validate::MutationBase { image, issues }) =
                self.capture_mutation_base(session.as_mut(), &[], None)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let CapturedDependencyMutation::Batch {
                issue_id,
                dependency_ids,
                policy,
            } = &request
            else {
                unreachable!("removals use the captured removal coordinator");
            };
            let source_id = super::require_issue_in_capture(&issues, issue_id)?;
            let graph = super::CapturedGraphEvidence::capture(&image, &issues, &context)?;
            let derived =
                derive_batch_dependency_add(&issues, &source_id, dependency_ids, *policy, &graph)?;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.into()),
                    None => Ok(AttemptOutcome::Done(derived.outcome)),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            match derived.error_after_apply {
                Some(error) => Err(error.into()),
                None => Ok(AttemptOutcome::Done(derived.outcome)),
            }
        })
    }

    fn publish_captured_dependency_removal(
        &self,
        request: CapturedDependencyMutation,
    ) -> Result<CapturedDependencyOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let layout = self.require_layout()?;
        let context = super::production_mutation_context();
        let enforce_lease = matches!(request, CapturedDependencyMutation::RemoveSingle { .. });
        with_mutation_attempts("dependency removal", || {
            // The claims guard depends only on the request, so it is taken before
            // the one session whose image resolves the removal and lease policy.
            let claims_guard = enforce_lease
                .then(|| claims_mutation_guard(self.require_worktree_paths()?))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(crate::commands::validate::MutationBase { image, issues }) =
                self.capture_mutation_base(session.as_mut(), &[], None)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let graph = super::CapturedGraphEvidence::capture(&image, &issues, &context)?;
            let resolved_request: CapturedDependencyMutation = match &request {
                CapturedDependencyMutation::RemoveSingle {
                    issue_id,
                    dependency_id,
                } => CapturedDependencyMutation::RemoveSingle {
                    issue_id: super::require_issue_in_capture(&issues, issue_id)?,
                    dependency_id: dependency_id.clone(),
                },
                CapturedDependencyMutation::RemoveBatch {
                    issue_id,
                    dependency_ids,
                } => CapturedDependencyMutation::RemoveBatch {
                    issue_id: issue_id.clone(),
                    dependency_ids: dependency_ids.clone(),
                },
                _ => unreachable!("removal coordinator receives only removals"),
            };
            let warning = if enforce_lease {
                let (CapturedDependencyMutation::RemoveSingle {
                    issue_id: resolved_id,
                    ..
                }
                | CapturedDependencyMutation::RemoveBatch {
                    issue_id: resolved_id,
                    ..
                }) = &resolved_request
                else {
                    unreachable!("removal coordinator receives only removals");
                };
                refuse_foreign_leases_held(
                    graph.declarations.config().enforcement_mode(),
                    std::slice::from_ref(resolved_id),
                    claims_guard.as_ref(),
                )?
                .into_iter()
                .next()
            } else {
                None
            };
            let derived = derive_dependency_removal(&issues, &resolved_request, warning, &graph)?;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.into()),
                    None => Ok(AttemptOutcome::Done(derived.outcome)),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            match derived.error_after_apply {
                Some(error) => Err(error.into()),
                None => Ok(AttemptOutcome::Done(derived.outcome)),
            }
        })
    }
}

impl<S: IssueStore> GraphOps<'_, S> {
    /// Remove the stored dependencies `dep_ids` from `issue_id`.
    ///
    /// Each id is compared exactly with `issue_id`'s own stored dependencies,
    /// so an edge whose target was deleted is removable (jit:f847df3f). An id
    /// not stored, or repeated after its removal, is reported as not found.
    pub fn remove_dependencies(
        &self,
        issue_id: &IssueId,
        dep_ids: &[IssueId],
    ) -> Result<DependenciesRemoveResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        if dep_ids.is_empty() {
            return Ok(DependenciesRemoveResult::default());
        }
        let warnings = self.exec.refuse_foreign_lease(issue_id)?;
        match self.exec.publish_captured_dependency_mutation(
            CapturedDependencyMutation::RemoveBatch {
                issue_id: issue_id.clone(),
                dependency_ids: dep_ids.to_vec(),
            },
        )? {
            CapturedDependencyOutcome::RemovedBatch(result) => {
                Ok(DependenciesRemoveResult { warnings, ..result })
            }
            _ => unreachable!("batch removal returns a batch-removal outcome"),
        }
    }
}

fn captured_issue<'a>(issues: &'a [Issue], id: &IssueId) -> Result<&'a Issue> {
    issues
        .iter()
        .find(|issue| &issue.id == id)
        .ok_or_else(|| IssueNotFoundError::new(id.clone()).into())
}

fn derive_dependency_removal(
    issues: &[Issue],
    request: &CapturedDependencyMutation,
    warning: Option<String>,
    graph: &super::CapturedGraphEvidence<'_>,
) -> Result<DerivedDependencyMutation> {
    let (source_id, removed, outcome) = match request {
        CapturedDependencyMutation::RemoveSingle {
            issue_id,
            dependency_id,
        } => {
            let issue = captured_issue(issues, issue_id)?;
            let removed = if issue.dependencies.contains(dependency_id) {
                vec![dependency_id.clone()]
            } else {
                Vec::new()
            };
            (
                issue_id.clone(),
                removed,
                CapturedDependencyOutcome::RemovedSingle { warning },
            )
        }
        CapturedDependencyMutation::RemoveBatch {
            issue_id,
            dependency_ids,
        } => {
            let issue = captured_issue(issues, issue_id)?;
            let mut result = DependenciesRemoveResult::default();
            for id in dependency_ids {
                if issue.dependencies.contains(id) && !result.removed.contains(id) {
                    result.removed.push(id.clone());
                } else {
                    result.not_found.push(id.clone());
                }
            }
            (
                issue_id.clone(),
                result.removed.clone(),
                CapturedDependencyOutcome::RemovedBatch(result),
            )
        }
        _ => unreachable!("removal derivation receives only removals"),
    };
    if removed.is_empty() {
        return Ok(DerivedDependencyMutation {
            outcome,
            intents: Vec::new(),
            error_after_apply: None,
        });
    }

    let mut issue = captured_issue(issues, &source_id)?.clone();
    issue
        .dependencies
        .retain(|dependency| !removed.contains(dependency));
    let mut candidate = issues.to_vec();
    *candidate
        .iter_mut()
        .find(|candidate| candidate.id == source_id)
        .ok_or_else(|| IssueNotFoundError::new(source_id.clone()))? = issue.clone();
    let readiness =
        super::derive_readiness_intents(issues, &candidate, graph, ReadinessScope::Affected)?;
    let intents = (!readiness.corrects(&source_id))
        .then(|| MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        })
        .into_iter()
        .chain(std::iter::once(MutationIntent::RecordEvent {
            phase: 1,
            event: Box::new(EventBody::draft_issue_updated(
                source_id,
                "dependency-remove".to_string(),
                vec!["dependencies".to_string()],
            )),
        }))
        .chain(readiness.intents)
        .collect();
    Ok(DerivedDependencyMutation {
        outcome,
        intents,
        error_after_apply: readiness.error_after_apply,
    })
}

fn derive_batch_dependency_add(
    issues: &[Issue],
    full_issue_id: &IssueId,
    dependency_ids: &[IssueId],
    policy: RedundancyPolicy,
    graph: &super::CapturedGraphEvidence<'_>,
) -> Result<DerivedDependencyMutation> {
    let full_issue_id = full_issue_id.clone();
    let from_deps = dependency_set(captured_issue(issues, &full_issue_id)?);
    let mut already_exist = Vec::new();
    let mut new_edges = Vec::new();
    let mut rejected = Vec::new();
    let mut seen = HashSet::new();

    for (ordinal, prefix) in dependency_ids.iter().enumerate() {
        let dependency_id = prefix.clone();
        match super::require_issue_in_capture(issues, prefix) {
            Ok(full_id) if from_deps.contains(&full_id) || !seen.insert(full_id.clone()) => {
                already_exist.push(dependency_id);
            }
            Ok(full_id) => new_edges.push((ordinal, dependency_id, full_id)),
            Err(error) => rejected.push((ordinal, dependency_id, error)),
        }
    }

    let candidate = candidate_issues(
        issues,
        &full_issue_id,
        new_edges.iter().map(|(_, _, full_id)| full_id),
    )?;
    let candidate_graph = dependency_graph(&candidate);
    let mut cycle_failed = HashSet::new();
    for (ordinal, text, full_id) in &new_edges {
        if let Err(error) = candidate_graph.validate_add_dependency(&full_issue_id, full_id) {
            cycle_failed.insert(*ordinal);
            rejected.push((*ordinal, text.clone(), error.into()));
        }
    }
    let valid_edges = new_edges
        .iter()
        .filter(|(ordinal, _, _)| !cycle_failed.contains(ordinal))
        .cloned()
        .collect::<Vec<_>>();
    let mut skipped = Vec::new();
    let mut reduced_from = from_deps.clone();
    let mut acyclic_candidate = issues.to_vec();

    if !valid_edges.is_empty() {
        acyclic_candidate = candidate_issues(
            issues,
            &full_issue_id,
            valid_edges.iter().map(|(_, _, full_id)| full_id),
        )?;
        let graph = dependency_graph(&acyclic_candidate);
        let redundant = graph
            .find_redundant_edges()
            .into_iter()
            .map(|(from, to)| (from.id.clone(), to.id.clone()))
            .collect::<Vec<_>>();
        let self_redundant = redundant
            .iter()
            .filter(|(from, to)| {
                from == &full_issue_id && valid_edges.iter().any(|(_, _, full_id)| full_id == to)
            })
            .map(|(_, to)| to.clone())
            .collect::<HashSet<_>>();

        for (ordinal, text, full_id) in &valid_edges {
            if self_redundant.contains(full_id) {
                match policy {
                    RedundancyPolicy::Reject => rejected.push((
                        *ordinal,
                        text.clone(),
                        RedundantDependencyError::new(
                            (full_issue_id.clone(), full_id.clone()),
                            redundant.clone(),
                        )
                        .into(),
                    )),
                    RedundancyPolicy::Reduce => skipped.push((
                        text.clone(),
                        "transitive (already reachable via other dependencies)".to_string(),
                    )),
                }
            }
        }

        if policy == RedundancyPolicy::Reject {
            for (ordinal, text, full_id) in &valid_edges {
                if self_redundant.contains(full_id) {
                    continue;
                }
                let singleton = candidate_issues(issues, &full_issue_id, [full_id])?;
                let singleton_redundant = dependency_graph(&singleton)
                    .find_redundant_edges()
                    .into_iter()
                    .map(|(from, to)| (from.id.clone(), to.id.clone()))
                    .collect::<Vec<_>>();
                if !singleton_redundant.is_empty() {
                    rejected.push((
                        *ordinal,
                        text.clone(),
                        RedundantDependencyError::new(
                            (full_issue_id.clone(), full_id.clone()),
                            singleton_redundant,
                        )
                        .into(),
                    ));
                }
            }
        }
        reduced_from = graph
            .compute_transitive_reduction(&full_issue_id)
            .into_iter()
            .map(|node| node.id.clone())
            .collect();
    }

    if !rejected.is_empty() {
        rejected.sort_by_key(|(ordinal, _, _)| *ordinal);
        return Err(DependencyBatchRejectedError::new(
            full_issue_id.clone(),
            rejected
                .into_iter()
                .map(|(_, dependency_id, error)| (dependency_id.to_string(), error))
                .collect(),
        )
        .into());
    }
    let added_ids = reduced_from
        .difference(&from_deps)
        .cloned()
        .collect::<HashSet<_>>();
    let added = new_edges
        .iter()
        .filter(|(_, _, full_id)| added_ids.contains(full_id))
        .map(|(_, text, _)| text.clone())
        .collect();
    let outcome = CapturedDependencyOutcome::Batch(DependenciesAddResult {
        added,
        already_exist,
        skipped,
        warnings: Vec::new(),
    });
    if reduced_from == from_deps {
        return Ok(DerivedDependencyMutation {
            outcome,
            intents: Vec::new(),
            error_after_apply: None,
        });
    }
    let reduced = derive_reduced_graph_intents(
        issues,
        &acyclic_candidate,
        Some((&full_issue_id, &reduced_from)),
    )?;
    let projected = issues
        .iter()
        .map(|issue| {
            reduced
                .iter()
                .find_map(|intent| match intent {
                    MutationIntent::UpdateIssue { issue: updated } if updated.id == issue.id => {
                        Some((**updated).clone())
                    }
                    _ => None,
                })
                .unwrap_or_else(|| issue.clone())
        })
        .collect::<Vec<_>>();
    let readiness =
        super::derive_readiness_intents(issues, &projected, graph, ReadinessScope::Affected)?;
    let mut intents = reduced
        .into_iter()
        .filter(|intent| {
            !matches!(intent, MutationIntent::UpdateIssue { issue } if readiness.corrects(&issue.id))
        })
        .collect::<Vec<_>>();
    intents.extend(readiness.intents);
    Ok(DerivedDependencyMutation {
        outcome,
        intents,
        error_after_apply: readiness.error_after_apply,
    })
}

fn candidate_issues<'a>(
    issues: &[Issue],
    source_id: &IssueId,
    dependencies: impl IntoIterator<Item = &'a IssueId>,
) -> Result<Vec<Issue>> {
    let mut candidate = issues.to_vec();
    let source = candidate
        .iter_mut()
        .find(|issue| &issue.id == source_id)
        .ok_or_else(|| IssueNotFoundError::new(source_id.clone()))?;
    source
        .dependencies
        .extend(dependencies.into_iter().cloned());
    source.dependencies.sort();
    source.dependencies.dedup();
    Ok(candidate)
}

fn dependency_graph(issues: &[Issue]) -> DependencyGraph<'_, Issue> {
    DependencyGraph::new(&issues.iter().collect::<Vec<_>>())
}

fn dependency_set(issue: &Issue) -> HashSet<IssueId> {
    issue.dependencies.iter().cloned().collect()
}

pub(super) fn derive_reduced_graph_intents(
    original: &[Issue],
    candidate: &[Issue],
    source: Option<(&IssueId, &HashSet<IssueId>)>,
) -> Result<Vec<MutationIntent>> {
    let graph = dependency_graph(candidate);
    let mut updates = Vec::new();
    let mut events = Vec::new();

    for candidate_issue in candidate {
        let original_issue = captured_issue(original, &candidate_issue.id)?;
        let original_deps = dependency_set(original_issue);
        let reduced = match source {
            Some((source_id, source_reduced)) if source_id == &candidate_issue.id => {
                source_reduced.clone()
            }
            _ => graph
                .compute_transitive_reduction(&candidate_issue.id)
                .into_iter()
                .map(|node| node.id.clone())
                .collect(),
        };
        if reduced == original_deps {
            continue;
        }
        let mut issue = original_issue.clone();
        let mut removed = original_deps
            .difference(&reduced)
            .cloned()
            .collect::<Vec<_>>();
        removed.sort();
        issue.dependencies = reduced.iter().cloned().collect();
        issue.dependencies.sort();
        let issue_id = issue.id.clone();

        let (phase, event) = if source.is_some_and(|(source_id, _)| source_id == &issue_id) {
            (
                2,
                EventBody::draft_issue_updated(
                    issue_id,
                    "dependency-add".to_string(),
                    vec!["dependencies".to_string()],
                ),
            )
        } else {
            (
                3,
                EventBody::draft_dependency_reduced(
                    issue_id,
                    original_deps.len(),
                    reduced.len(),
                    removed,
                ),
            )
        };
        events.push(MutationIntent::RecordEvent {
            phase,
            event: Box::new(event),
        });
        updates.push(MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        });
    }
    updates.extend(events);
    Ok(updates)
}

#[cfg(test)]
mod captured_tests {
    use super::*;
    use crate::domain::event_catalog::EventTag;
    use crate::storage::{InMemoryStorage, IssueStore, MutationPoint, RepositoryStateStore};
    use crate::test_utils::add_reducing_edge;

    #[derive(Default)]
    struct OneFailure(std::sync::Mutex<Option<MutationPoint>>);

    impl crate::storage::MutationInterruptionInjector for OneFailure {
        fn check(&self, point: &MutationPoint) -> std::io::Result<()> {
            let mut selected = self.0.lock().unwrap();
            if selected.as_ref() == Some(point) {
                selected.take();
                return Err(std::io::Error::other("injected dependency removal failure"));
            }
            Ok(())
        }
    }

    #[test]
    fn test_dependency_add_rederives_and_preserves_concurrent_issue_change() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let dependency = crate::test_utils::fixture_issue("dependency".into(), String::new());
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.state = State::Ready;
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source.clone());
        source.title = "concurrent title".to_string();
        let storage = crate::commands::test_helpers::with_apply_race(
            storage,
            crate::commands::test_helpers::SessionRaceAction::Save(Box::new(source)),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);

        add_reducing_edge(&executor, &source_id, &dependency_id).unwrap();

        let updated = executor.storage.load_issue(&source_id).unwrap();
        assert_eq!(updated.title, "concurrent title");
        assert_eq!(updated.dependencies, vec![dependency_id]);
        assert_eq!(updated.state, State::Backlog);
        let events = executor.storage.read_events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(
            events
                .iter()
                .map(|event| event.get_type())
                .collect::<Vec<_>>(),
            [EventTag::IssueStateChanged, EventTag::IssueUpdated].map(EventTag::as_str)
        );
        let first = serde_json::to_value(&events[0]).unwrap();
        let second = serde_json::to_value(&events[1]).unwrap();
        assert_eq!(first["timestamp"], second["timestamp"]);
    }

    #[test]
    fn test_dependency_removal_and_auto_ready_recover_together() {
        let failures = std::sync::Arc::new(OneFailure(std::sync::Mutex::new(Some(
            MutationPoint::BeforeCommitCheck,
        ))));
        let storage = InMemoryStorage::with_mutation_interruptions(failures);
        storage.add_data_file("config.toml", "");
        let recovered = storage.without_mutation_interruptions();
        let layout = storage.repository_layout();

        let mut dependency = crate::test_utils::fixture_issue("dependency".into(), String::new());
        dependency.state = State::Done;
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.state = State::Backlog;
        source.dependencies = vec![dependency_id.clone()];
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source);

        let executor = CommandExecutor::new(storage)
            .with_layout(layout.clone())
            .unwrap();
        assert!(executor
            .graph()
            .remove_dependencies(&source_id, std::slice::from_ref(&dependency_id))
            .is_err());

        drop(recovered.open_mutation_session(layout.clone()).unwrap());
        let unchanged = recovered.load_issue(&source_id).unwrap();
        assert_eq!(unchanged.state, State::Backlog);
        assert_eq!(unchanged.dependencies, vec![dependency_id.clone()]);
        assert!(recovered.read_events().unwrap().is_empty());

        let executor = CommandExecutor::new(recovered.clone())
            .with_layout(layout)
            .unwrap();
        executor
            .graph()
            .remove_dependencies(&source_id, std::slice::from_ref(&dependency_id))
            .unwrap();
        let updated = recovered.load_issue(&source_id).unwrap();
        assert_eq!(updated.state, State::Ready);
        assert!(updated.dependencies.is_empty());
        let events = recovered.read_events().unwrap();
        assert!(events
            .iter()
            .any(|event| event.get_type() == "issue_updated"));
        assert!(events
            .iter()
            .any(|event| event.get_type() == "issue_state_changed"));
    }

    #[test]
    fn test_dependency_removal_graph_block_commits_edge_and_attempt_event() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "");
        storage.add_data_file(
            "rules.toml",
            r#"
[[rules]]
name = "ready-needs-design"
when = { type = "epic", state = "ready" }
severity = "error"
enforce = true
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
"#,
        );
        let layout = storage.repository_layout();

        let mut dependency = crate::test_utils::fixture_issue("dependency".into(), String::new());
        dependency.state = State::Done;
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.state = State::Backlog;
        source.labels = vec!["type:epic".into()];
        source.dependencies = vec![dependency_id.clone()];
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source);

        let executor = CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .unwrap();
        let error = executor
            .graph()
            .remove_dependencies(&source_id, std::slice::from_ref(&dependency_id))
            .expect_err("the ready graph rule blocks only the automatic transition");
        assert!(error
            .downcast_ref::<crate::errors::TransitionBlockedError>()
            .is_some());

        let updated = storage.load_issue(&source_id).unwrap();
        assert_eq!(updated.state, State::Backlog);
        assert!(updated.dependencies.is_empty());
        let event_types = storage
            .read_events()
            .unwrap()
            .into_iter()
            .map(|event| event.get_type().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            event_types,
            [EventTag::IssueUpdated, EventTag::TransitionBlocked].map(EventTag::as_str)
        );
    }

    #[test]
    fn test_dependency_removal_repeated_id_is_not_found_after_its_removal() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "");
        let layout = storage.repository_layout();
        let dependency = crate::test_utils::fixture_issue("dependency".into(), String::new());
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.dependencies = vec![dependency_id.clone()];
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source);

        let executor = CommandExecutor::new(storage).with_layout(layout).unwrap();
        let result = executor
            .graph()
            .remove_dependencies(&source_id, &[dependency_id.clone(), dependency_id.clone()])
            .unwrap();
        assert_eq!(result.removed, vec![dependency_id.clone()]);
        assert_eq!(result.not_found, vec![dependency_id]);
    }

    #[test]
    fn test_remove_dependency_uses_the_exact_target_when_issue_prefixes_collide() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let mut dependency = crate::test_utils::fixture_issue("dep".into(), String::new());
        dependency.id = crate::test_utils::issue_id(0x1111_1111_1111_1111_1111_1111_2222_1111);
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.id = crate::test_utils::issue_id(0x1111_1111_1111_1111_1111_1111_1111_1111);
        source.dependencies = vec![dependency.id.clone()];
        crate::commands::test_helpers::seed_issue(&storage, source.clone());
        crate::commands::test_helpers::seed_issue(&storage, dependency.clone());
        let mut collision = crate::test_utils::fixture_issue("collision".into(), String::new());
        collision.id = crate::test_utils::issue_id(0x2222_2222_2222_2222_2222_2222_2222_2222);
        crate::commands::test_helpers::seed_issue(&storage, collision);
        let executor = crate::commands::test_helpers::memory_executor(storage);

        executor
            .graph()
            .remove_dependency(&source.id, &dependency.id)
            .unwrap();
        assert!(executor
            .storage
            .load_issue(&source.id)
            .unwrap()
            .dependencies
            .is_empty());
    }

    #[test]
    fn test_remove_dependency_removes_a_target_whose_record_is_gone() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let mut dependency = crate::test_utils::fixture_issue("dep".into(), String::new());
        dependency.id = crate::test_utils::issue_id(0x1111_1111_1111_1111_1111_1111_2222_1111);
        let mut source = crate::test_utils::fixture_issue("source".into(), String::new());
        source.id = crate::test_utils::issue_id(0x1111_1111_1111_1111_1111_1111_1111_1111);
        source.dependencies = vec![dependency.id.clone()];
        crate::commands::test_helpers::seed_issue(&storage, source.clone());
        let executor = crate::commands::test_helpers::memory_executor(storage);

        executor
            .graph()
            .remove_dependency(&source.id, &dependency.id)
            .unwrap();
        assert!(executor
            .storage
            .load_issue(&source.id)
            .unwrap()
            .dependencies
            .is_empty());
    }
}
