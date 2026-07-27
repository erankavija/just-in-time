//! Dependency graph operations

use super::*;
use crate::errors::{DependencyBatchRejectedError, RedundantDependencyError};
use crate::repository_state::{finalize, MutationContext, MutationIntent};
use crate::storage::{
    AmbiguousIdError, InvalidIdPrefixError, IssueNotFoundError, MIN_ID_PREFIX_LENGTH,
};
use std::collections::HashSet;

#[derive(Clone)]
enum CapturedDependencyMutation {
    Single {
        issue_id: String,
        dependency_id: String,
        policy: RedundancyPolicy,
    },
    Batch {
        issue_id: String,
        dependency_ids: Vec<String>,
        policy: RedundancyPolicy,
    },
    ReduceAll {
        dry_run: bool,
    },
    RemoveSingle {
        issue_id: String,
        dependency_id: String,
    },
    RemoveBatch {
        issue_id: String,
        dependency_ids: Vec<String>,
    },
}

impl CapturedDependencyMutation {
    fn issue_id(&self) -> &str {
        match self {
            Self::Single { issue_id, .. }
            | Self::Batch { issue_id, .. }
            | Self::RemoveSingle { issue_id, .. }
            | Self::RemoveBatch { issue_id, .. } => issue_id,
            Self::ReduceAll { .. } => "",
        }
    }
}

enum CapturedDependencyOutcome {
    Single {
        result: DependencyAddResult,
        warning: Option<String>,
    },
    Batch(DependenciesAddResult),
    Reduced {
        count: usize,
        messages: Vec<String>,
    },
    RemovedSingle {
        warning: Option<String>,
    },
    RemovedBatch(DependenciesRemoveResult),
}

struct DerivedDependencyMutation {
    outcome: CapturedDependencyOutcome,
    intents: Vec<MutationIntent>,
    error_after_apply: Option<crate::errors::TransitionBlockedError>,
}

/// Result of adding multiple dependencies.
///
/// All-or-nothing (jit:c8518f2a): [`add_dependencies_with_policy`](CommandExecutor::add_dependencies_with_policy)
/// returns this only when every requested edge validated. A batch with any
/// rejected edge instead returns `Err(`[`DependencyBatchRejectedError`]`)` and
/// leaves the dependency set — and the event log — completely unchanged, so
/// there is no per-batch `errors` field to populate here.
#[derive(Debug, Serialize)]
pub struct DependenciesAddResult {
    pub added: Vec<String>,
    pub already_exist: Vec<String>,
    pub skipped: Vec<(String, String)>, // (id, reason)
}

/// Result of removing multiple dependencies
#[derive(Debug, Serialize)]
pub struct DependenciesRemoveResult {
    pub removed: Vec<String>,
    pub not_found: Vec<String>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Add a dependency to an issue, dropping any now-redundant edge(s).
    ///
    /// Convenience wrapper over
    /// [`add_dependency_with_policy`](Self::add_dependency_with_policy) with
    /// [`RedundancyPolicy::Reduce`]: the resulting graph is left acyclic and
    /// transitively reduced, which internal callers (templates, breakdown,
    /// batch-create) rely on. Returns `(result, warnings)` where `warnings`
    /// carries any lease warning.
    pub fn add_dependency(
        &self,
        issue_id: &str,
        dep_id: &str,
    ) -> Result<(DependencyAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.add_dependency_with_policy(issue_id, dep_id, RedundancyPolicy::Reduce)
    }

    /// Add a dependency under an explicit transitive-reduction [`RedundancyPolicy`].
    ///
    /// Cycle detection runs first (the existing write-time guard,
    /// @/inv/dag-acyclic). The candidate graph (the current issues plus the new
    /// edge) is then checked for transitive-reduction violations with the same
    /// [`DependencyGraph::find_redundant_edges`] property `jit validate` enforces:
    ///
    /// * [`RedundancyPolicy::Reject`] — if the edge shadows an existing direct
    ///   edge or is itself already reachable, the add fails with a
    ///   [`RedundantDependencyError`](crate::errors::RedundantDependencyError)
    ///   naming the offending edge pair; nothing is written.
    /// * [`RedundancyPolicy::Reduce`] — the edge is added and every now-redundant
    ///   edge is dropped in the same operation, leaving the graph reduced.
    ///
    /// A non-redundant edge is added normally under either policy.
    pub fn add_dependency_with_policy(
        &self,
        issue_id: &str,
        dep_id: &str,
        policy: RedundancyPolicy,
    ) -> Result<(DependencyAddResult, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        match self.publish_captured_dependency_mutation(CapturedDependencyMutation::Single {
            issue_id: issue_id.to_string(),
            dependency_id: dep_id.to_string(),
            policy,
        })? {
            CapturedDependencyOutcome::Single { result, warning } => {
                Ok((result, warning.into_iter().collect()))
            }
            _ => unreachable!("single dependency request returns a single outcome"),
        }
    }

    /// Remove a dependency from an issue.
    ///
    /// Returns warnings (e.g., lease warnings) if any.
    pub fn remove_dependency(&self, issue_id: &str, dep_id: &str) -> Result<Vec<String>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        match self.publish_captured_dependency_mutation(
            CapturedDependencyMutation::RemoveSingle {
                issue_id: issue_id.to_string(),
                dependency_id: dep_id.to_string(),
            },
        )? {
            CapturedDependencyOutcome::RemovedSingle { warning } => {
                Ok(warning.into_iter().collect())
            }
            _ => unreachable!("single removal returns a single-removal outcome"),
        }
    }

    /// Add multiple dependencies to an issue, dropping now-redundant edge(s).
    ///
    /// Convenience wrapper over
    /// [`add_dependencies_with_policy`](Self::add_dependencies_with_policy) with
    /// [`RedundancyPolicy::Reduce`], preserving the eager-reduction behavior
    /// internal callers rely on.
    pub fn add_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesAddResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.add_dependencies_with_policy(issue_id, dep_ids, RedundancyPolicy::Reduce)
    }

    /// Add multiple dependencies under an explicit [`RedundancyPolicy`], atomically.
    ///
    /// All-or-nothing (jit:c8518f2a): every requested edge is validated —
    /// id resolution, then cycle detection and (under
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
        issue_id: &str,
        dep_ids: &[String],
        policy: RedundancyPolicy,
    ) -> Result<DependenciesAddResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }
        match self.publish_captured_dependency_mutation(CapturedDependencyMutation::Batch {
            issue_id: issue_id.to_string(),
            dependency_ids: dep_ids.to_vec(),
            policy,
        })? {
            CapturedDependencyOutcome::Batch(result) => Ok(result),
            _ => unreachable!("batch dependency request returns a batch outcome"),
        }
    }

    /// Repair every transitive-reduction violation from one captured graph.
    pub(super) fn reduce_all_dependencies(&self, dry_run: bool) -> Result<(usize, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        match self.publish_captured_dependency_mutation(CapturedDependencyMutation::ReduceAll {
            dry_run,
        })? {
            CapturedDependencyOutcome::Reduced { count, messages } => Ok((count, messages)),
            _ => unreachable!("reduction repair returns a reduction outcome"),
        }
    }

    /// Capture the complete active issue graph and publish one dependency mutation.
    ///
    /// The index is the sole discovery authority. The second phase captures every
    /// indexed record, the complete issues-directory listing, and the event log;
    /// semantic derivation reads only that closed image. A conflict restarts both
    /// phases in a fresh recovered session while reusing one mutation context.
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
        let context = MutationContext::production();

        with_mutation_attempts("dependency mutation", || {
            let (expected_source, expected_lease_mode, enforce_lease) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) = self.capture_proposed_base(
                    preflight.as_mut(),
                    &std::collections::BTreeMap::new(),
                    &[],
                    None,
                )?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let issues = super::captured_active_issues(&image)?;
                let source = match &request {
                    CapturedDependencyMutation::Single { issue_id, .. }
                    | CapturedDependencyMutation::Batch { issue_id, .. } => {
                        Some(resolve_captured_issue_id(&issues, issue_id)?)
                    }
                    CapturedDependencyMutation::ReduceAll { .. } => None,
                    CapturedDependencyMutation::RemoveSingle { .. }
                    | CapturedDependencyMutation::RemoveBatch { .. } => {
                        unreachable!("removals use the captured removal coordinator")
                    }
                };
                let config = crate::repository_state::assemble_config(&image)?;
                (
                    source,
                    self.config_manager.enforcement_mode_from_config(&config)?,
                    matches!(request, CapturedDependencyMutation::Single { .. }),
                )
            };
            let claims_guard = enforce_lease
                .then(|| claims_mutation_guard(&layout))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) = self.capture_proposed_base(
                session.as_mut(),
                &std::collections::BTreeMap::new(),
                &[],
                None,
            )?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let issues = super::captured_active_issues(&image)?;
            let current_source = match &request {
                CapturedDependencyMutation::Single { issue_id, .. }
                | CapturedDependencyMutation::Batch { issue_id, .. } => {
                    Some(resolve_captured_issue_id(&issues, issue_id)?)
                }
                CapturedDependencyMutation::ReduceAll { .. } => None,
                _ => unreachable!("removals use the captured removal coordinator"),
            };
            let config = crate::repository_state::assemble_config(&image)?;
            if current_source != expected_source
                || (enforce_lease
                    && self.config_manager.enforcement_mode_from_config(&config)?
                        != expected_lease_mode)
            {
                return Ok(AttemptOutcome::Retry);
            }
            let lease_warnings = if enforce_lease {
                captured_lease_warnings(
                    expected_lease_mode,
                    std::slice::from_ref(
                        expected_source
                            .as_ref()
                            .ok_or_else(|| anyhow!("missing source"))?,
                    ),
                    &issues,
                    claims_guard.as_ref(),
                )?
            } else {
                Vec::new()
            };
            let pinned_request = match &request {
                CapturedDependencyMutation::Single {
                    dependency_id,
                    policy,
                    ..
                } => CapturedDependencyMutation::Single {
                    issue_id: expected_source.clone().expect("single add has a source"),
                    dependency_id: dependency_id.clone(),
                    policy: *policy,
                },
                CapturedDependencyMutation::Batch {
                    dependency_ids,
                    policy,
                    ..
                } => CapturedDependencyMutation::Batch {
                    issue_id: expected_source.clone().expect("batch add has a source"),
                    dependency_ids: dependency_ids.clone(),
                    policy: *policy,
                },
                CapturedDependencyMutation::ReduceAll { dry_run } => {
                    CapturedDependencyMutation::ReduceAll { dry_run: *dry_run }
                }
                _ => unreachable!("removals use the captured removal coordinator"),
            };
            let warning = lease_warnings.into_iter().next();
            let derived = derive_dependency_mutation(&issues, &pinned_request, warning)?;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.into()),
                    None => Ok(AttemptOutcome::Done(derived.outcome)),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
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
        use std::collections::BTreeMap;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        with_mutation_attempts("dependency removal", || {
            let (resolved_request, expected_lease_mode, enforce_lease) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) =
                    self.capture_proposed_base(preflight.as_mut(), &BTreeMap::new(), &[], None)?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let issues = super::captured_active_issues(&image)?;
                let resolved = match &request {
                    CapturedDependencyMutation::RemoveSingle {
                        issue_id,
                        dependency_id,
                    } => CapturedDependencyMutation::RemoveSingle {
                        issue_id: resolve_captured_issue_id(&issues, issue_id)?,
                        dependency_id: resolve_captured_issue_id(&issues, dependency_id)?,
                    },
                    CapturedDependencyMutation::RemoveBatch {
                        issue_id,
                        dependency_ids,
                    } => CapturedDependencyMutation::RemoveBatch {
                        issue_id: resolve_captured_issue_id(&issues, issue_id)?,
                        dependency_ids: dependency_ids.clone(),
                    },
                    _ => unreachable!("removal coordinator receives only removals"),
                };
                let config = crate::repository_state::assemble_config(&image)?;
                (
                    resolved,
                    self.config_manager.enforcement_mode_from_config(&config)?,
                    matches!(request, CapturedDependencyMutation::RemoveSingle { .. }),
                )
            };
            let claims_guard = enforce_lease
                .then(|| claims_mutation_guard(&layout))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) =
                self.capture_proposed_base(session.as_mut(), &BTreeMap::new(), &[], None)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let issues = super::captured_active_issues(&image)?;
            let declarations = crate::repository_state::declarations_from_image(&image)?;
            let config = crate::repository_state::assemble_config(&image)?;
            if enforce_lease
                && self.config_manager.enforcement_mode_from_config(&config)? != expected_lease_mode
            {
                return Ok(AttemptOutcome::Retry);
            }
            let recaptured_request = match &request {
                CapturedDependencyMutation::RemoveSingle {
                    issue_id,
                    dependency_id,
                } => CapturedDependencyMutation::RemoveSingle {
                    issue_id: resolve_captured_issue_id(&issues, issue_id)?,
                    dependency_id: resolve_captured_issue_id(&issues, dependency_id)?,
                },
                CapturedDependencyMutation::RemoveBatch {
                    issue_id,
                    dependency_ids,
                } => CapturedDependencyMutation::RemoveBatch {
                    issue_id: resolve_captured_issue_id(&issues, issue_id)?,
                    dependency_ids: dependency_ids.clone(),
                },
                _ => unreachable!("removal coordinator receives only removals"),
            };
            if recaptured_request.issue_id() != resolved_request.issue_id()
                || matches!(
                    (&recaptured_request, &resolved_request),
                    (
                        CapturedDependencyMutation::RemoveSingle {
                            dependency_id: current,
                            ..
                        },
                        CapturedDependencyMutation::RemoveSingle {
                            dependency_id: expected,
                            ..
                        }
                    ) if current != expected
                )
            {
                return Err(anyhow!(
                    "dependency removal target resolution changed during coordinated publication"
                ));
            }
            let plan_content = super::validate::plan_content_from_image(&image, &issues)?;
            let warning = if enforce_lease {
                captured_lease_warnings(
                    expected_lease_mode,
                    &[resolved_request.issue_id().to_string()],
                    &issues,
                    claims_guard.as_ref(),
                )?
                .into_iter()
                .next()
            } else {
                None
            };
            let derived = derive_dependency_removal(
                &issues,
                &resolved_request,
                warning,
                super::CapturedTransitionEvidence {
                    issues: &issues,
                    declarations: &declarations,
                    config: &config,
                    plan_content: &plan_content,
                    context: &context,
                },
            )?;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.into()),
                    None => Ok(AttemptOutcome::Done(derived.outcome)),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            match derived.error_after_apply {
                Some(error) => Err(error.into()),
                None => Ok(AttemptOutcome::Done(derived.outcome)),
            }
        })
    }

    /// Remove multiple dependencies from an issue.
    ///
    /// This is what `jit dep rm <from> <id>...` calls. Each `dep_id` is
    /// matched directly against `issue`'s OWN stored `dependencies` (by exact
    /// id, or by normalized prefix once at least 4 characters are given) —
    /// NOT resolved through the repo-wide index first. A dependency whose
    /// target issue was deleted no longer resolves there, which previously
    /// made a dangling edge permanently unremovable via the CLI (jit:f847df3f);
    /// matching the raw stored id sidesteps that resolution step entirely and
    /// works whether the target is alive or gone. An exact stored id always
    /// wins; a prefix that matches more than one stored dependency is rejected
    /// as ambiguous rather than removing an arbitrary one.
    pub fn remove_dependencies(
        &self,
        issue_id: &str,
        dep_ids: &[String],
    ) -> Result<DependenciesRemoveResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        if dep_ids.is_empty() {
            return Err(anyhow!("Must provide at least one dependency"));
        }
        match self.publish_captured_dependency_mutation(
            CapturedDependencyMutation::RemoveBatch {
                issue_id: issue_id.to_string(),
                dependency_ids: dep_ids.to_vec(),
            },
        )? {
            CapturedDependencyOutcome::RemovedBatch(result) => Ok(result),
            _ => unreachable!("batch removal returns a batch-removal outcome"),
        }
    }
}

fn resolve_captured_issue_id(issues: &[Issue], partial_id: &str) -> Result<String> {
    let normalized = partial_id.to_lowercase().replace('-', "");
    if normalized.len() < MIN_ID_PREFIX_LENGTH {
        return Err(InvalidIdPrefixError::new(partial_id).into());
    }
    let matches = issues
        .iter()
        .filter(|issue| {
            issue
                .id
                .replace('-', "")
                .to_lowercase()
                .starts_with(&normalized)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(IssueNotFoundError::new(partial_id).into()),
        [issue] => Ok(issue.id.clone()),
        _ => Err(AmbiguousIdError::issue(
            partial_id,
            matches
                .iter()
                .map(|issue| format!("{} | {}", issue.short_id(), issue.title)),
        )
        .into()),
    }
}

fn captured_issue<'a>(issues: &'a [Issue], id: &str) -> Result<&'a Issue> {
    issues
        .iter()
        .find(|issue| issue.id == id)
        .ok_or_else(|| IssueNotFoundError::new(id).into())
}

fn derive_dependency_mutation(
    issues: &[Issue],
    request: &CapturedDependencyMutation,
    warning: Option<String>,
) -> Result<DerivedDependencyMutation> {
    match request {
        CapturedDependencyMutation::Single {
            issue_id,
            dependency_id,
            policy,
        } => derive_single_dependency_add(issues, issue_id, dependency_id, *policy, warning),
        CapturedDependencyMutation::Batch {
            issue_id,
            dependency_ids,
            policy,
        } => derive_batch_dependency_add(issues, issue_id, dependency_ids, *policy),
        CapturedDependencyMutation::ReduceAll { dry_run } => {
            derive_dependency_repair(issues, *dry_run)
        }
        CapturedDependencyMutation::RemoveSingle { .. }
        | CapturedDependencyMutation::RemoveBatch { .. } => {
            unreachable!("removals use their captured derivation")
        }
    }
}

fn derive_dependency_removal(
    issues: &[Issue],
    request: &CapturedDependencyMutation,
    warning: Option<String>,
    evidence: super::CapturedTransitionEvidence<'_>,
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
            let mut removed = Vec::new();
            let mut reported_removed = Vec::new();
            let mut not_found = Vec::new();
            for requested in dependency_ids {
                let normalized = requested.to_lowercase().replace('-', "");
                let exact = issue
                    .dependencies
                    .iter()
                    .filter(|stored| !removed.contains(*stored))
                    .find(|stored| stored.as_str() == requested.as_str())
                    .cloned();
                let matched = match exact {
                    Some(exact) => Some(exact),
                    None => {
                        if normalized.len() < MIN_ID_PREFIX_LENGTH {
                            return Err(InvalidIdPrefixError::new(requested).into());
                        }
                        let matches = issue
                            .dependencies
                            .iter()
                            .filter(|stored| !removed.contains(*stored))
                            .filter(|stored| {
                                stored
                                    .to_lowercase()
                                    .replace('-', "")
                                    .starts_with(&normalized)
                            })
                            .cloned()
                            .collect::<Vec<_>>();
                        match matches.as_slice() {
                            [] => None,
                            [only] => Some(only.clone()),
                            _ => {
                                return Err(AmbiguousIdError::dependency(requested, matches).into())
                            }
                        }
                    }
                };
                match matched {
                    Some(id) => {
                        removed.push(id);
                        reported_removed.push(requested.clone());
                    }
                    None => not_found.push(requested.clone()),
                }
            }
            (
                issue_id.clone(),
                removed,
                CapturedDependencyOutcome::RemovedBatch(DependenciesRemoveResult {
                    removed: reported_removed,
                    not_found,
                }),
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
        .ok_or_else(|| IssueNotFoundError::new(&source_id))? = issue.clone();
    let resolved = crate::domain::queries::build_issue_map(&candidate);
    let mut intents = vec![MutationIntent::RecordEvent {
        phase: 1,
        event: Box::new(Event::draft_issue_updated(
            source_id.clone(),
            "dependency-remove".to_string(),
            vec!["dependencies".to_string()],
        )),
    }];
    if issue.should_auto_transition_to_ready(&resolved) {
        match super::derive_state_transition(issue.clone(), State::Ready, false, evidence)? {
            super::DerivedStateTransition::Applied {
                issue: transitioned,
                events,
                ..
            } => {
                issue = *transitioned;
                intents.extend(events.into_iter().map(|(phase, event)| {
                    MutationIntent::RecordEvent {
                        phase: phase.saturating_add(1),
                        event: Box::new(event),
                    }
                }));
            }
            super::DerivedStateTransition::GraphBlocked { error, events } => {
                intents.extend(events.into_iter().map(|(phase, event)| {
                    MutationIntent::RecordEvent {
                        phase: phase.saturating_add(1),
                        event: Box::new(event),
                    }
                }));
                intents.insert(
                    0,
                    MutationIntent::UpdateIssue {
                        issue: Box::new(issue),
                    },
                );
                return Ok(DerivedDependencyMutation {
                    outcome,
                    intents,
                    error_after_apply: Some(error),
                });
            }
        }
    }
    intents.insert(
        0,
        MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        },
    );
    Ok(DerivedDependencyMutation {
        outcome,
        intents,
        error_after_apply: None,
    })
}

fn derive_single_dependency_add(
    issues: &[Issue],
    issue_id: &str,
    dependency_id: &str,
    policy: RedundancyPolicy,
    warning: Option<String>,
) -> Result<DerivedDependencyMutation> {
    let full_issue_id = resolve_captured_issue_id(issues, issue_id)?;
    let full_dependency_id = resolve_captured_issue_id(issues, dependency_id)?;
    let graph = dependency_graph(issues);
    graph.validate_add_dependency(&full_issue_id, &full_dependency_id)?;
    let from = captured_issue(issues, &full_issue_id)?;
    if from.dependencies.contains(&full_dependency_id) {
        return Ok(DerivedDependencyMutation {
            outcome: CapturedDependencyOutcome::Single {
                result: DependencyAddResult::AlreadyExists,
                warning,
            },
            intents: Vec::new(),
            error_after_apply: None,
        });
    }

    let candidate = candidate_issues(issues, &full_issue_id, [&full_dependency_id])?;
    let candidate_graph = dependency_graph(&candidate);
    let mut redundant = candidate_graph.find_redundant_edges();
    redundant.sort();
    if !redundant.is_empty() && policy == RedundancyPolicy::Reject {
        return Err(
            RedundantDependencyError::new((full_issue_id, full_dependency_id), redundant).into(),
        );
    }
    let reduced = candidate_graph.compute_transitive_reduction(&full_issue_id);
    let original = dependency_set(from);
    let result = if reduced == original {
        DependencyAddResult::Skipped {
            reason: "transitive (already reachable via other dependencies)".to_string(),
        }
    } else {
        DependencyAddResult::Added
    };
    let intents = if reduced == original {
        Vec::new()
    } else {
        derive_reduced_graph_intents(issues, &candidate, Some((&full_issue_id, &reduced)))?
    };
    Ok(DerivedDependencyMutation {
        outcome: CapturedDependencyOutcome::Single { result, warning },
        intents,
        error_after_apply: None,
    })
}

fn derive_batch_dependency_add(
    issues: &[Issue],
    issue_id: &str,
    dependency_ids: &[String],
    policy: RedundancyPolicy,
) -> Result<DerivedDependencyMutation> {
    let full_issue_id = resolve_captured_issue_id(issues, issue_id)?;
    let from_deps = dependency_set(captured_issue(issues, &full_issue_id)?);
    let mut already_exist = Vec::new();
    let mut new_edges = Vec::new();
    let mut rejected = Vec::new();
    let mut seen = HashSet::new();

    for (ordinal, dependency_id) in dependency_ids.iter().enumerate() {
        match resolve_captured_issue_id(issues, dependency_id) {
            Ok(full_id) if from_deps.contains(&full_id) || !seen.insert(full_id.clone()) => {
                already_exist.push(dependency_id.clone());
            }
            Ok(full_id) => new_edges.push((ordinal, dependency_id.clone(), full_id)),
            Err(error) => rejected.push((ordinal, dependency_id.clone(), error)),
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
        let mut redundant = graph.find_redundant_edges();
        redundant.sort();
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
                let mut singleton_redundant = dependency_graph(&singleton).find_redundant_edges();
                singleton_redundant.sort();
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
        reduced_from = graph.compute_transitive_reduction(&full_issue_id);
    }

    if !rejected.is_empty() {
        rejected.sort_by_key(|(ordinal, _, _)| *ordinal);
        return Err(DependencyBatchRejectedError::new(
            full_issue_id,
            rejected
                .into_iter()
                .map(|(_, dependency_id, error)| (dependency_id, error))
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
    let intents = if reduced_from == from_deps {
        Vec::new()
    } else {
        derive_reduced_graph_intents(
            issues,
            &acyclic_candidate,
            Some((&full_issue_id, &reduced_from)),
        )?
    };
    Ok(DerivedDependencyMutation {
        outcome: CapturedDependencyOutcome::Batch(DependenciesAddResult {
            added,
            already_exist,
            skipped,
        }),
        intents,
        error_after_apply: None,
    })
}

fn derive_dependency_repair(issues: &[Issue], dry_run: bool) -> Result<DerivedDependencyMutation> {
    let graph = dependency_graph(issues);
    let reductions = issues
        .iter()
        .filter_map(|issue| {
            let reduced = graph.compute_transitive_reduction(&issue.id);
            let removed = issue.dependencies.len().saturating_sub(reduced.len());
            (removed > 0).then_some((issue, reduced, removed))
        })
        .collect::<Vec<_>>();
    let count = reductions.iter().map(|(_, _, count)| count).sum();
    let messages = if dry_run {
        Vec::new()
    } else {
        reductions
            .iter()
            .map(|(issue, _, fixed)| {
                format!(
                    "Fixed {} redundant {} in issue {}",
                    fixed,
                    if *fixed == 1 {
                        "dependency"
                    } else {
                        "dependencies"
                    },
                    &issue.id[..8.min(issue.id.len())]
                )
            })
            .collect()
    };
    let intents = if dry_run || reductions.is_empty() {
        Vec::new()
    } else {
        derive_reduced_graph_intents(issues, issues, None)?
    };
    Ok(DerivedDependencyMutation {
        outcome: CapturedDependencyOutcome::Reduced { count, messages },
        intents,
        error_after_apply: None,
    })
}

fn candidate_issues<'a>(
    issues: &[Issue],
    source_id: &str,
    dependencies: impl IntoIterator<Item = &'a String>,
) -> Result<Vec<Issue>> {
    let mut candidate = issues.to_vec();
    let source = candidate
        .iter_mut()
        .find(|issue| issue.id == source_id)
        .ok_or_else(|| IssueNotFoundError::new(source_id))?;
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

fn dependency_set(issue: &Issue) -> HashSet<String> {
    issue.dependencies.iter().cloned().collect()
}

fn derive_reduced_graph_intents(
    original: &[Issue],
    candidate: &[Issue],
    source: Option<(&str, &HashSet<String>)>,
) -> Result<Vec<MutationIntent>> {
    let graph = dependency_graph(candidate);
    // Dependency states are what readiness reads, and this derivation changes no
    // issue's state, so the original view resolves every dependency the reduced
    // sets can name.
    let resolved = crate::domain::queries::build_issue_map(original);
    let mut updates = Vec::new();
    let mut events = Vec::new();

    for candidate_issue in candidate {
        let original_issue = captured_issue(original, &candidate_issue.id)?;
        let original_deps = dependency_set(original_issue);
        let reduced = match source {
            Some((source_id, source_reduced)) if source_id == candidate_issue.id => {
                source_reduced.clone()
            }
            _ => graph.compute_transitive_reduction(&candidate_issue.id),
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

        if source.is_some_and(|(source_id, _)| source_id == issue_id) {
            // Readiness derives from the final dependency set through the shared
            // domain helper, the same one the graph-template apply path consults,
            // so the two cannot drift (`@/invariant/derived-state-coherence`). An
            // edge addition owns only the demotion direction; promotion belongs to
            // the removal path below and to dependency completion.
            let demoted =
                issue.derive_readiness_correction(&resolved) == Some(ReadinessCorrection::Demote);
            if demoted {
                issue.state = State::Backlog;
                events.push(MutationIntent::RecordEvent {
                    phase: 1,
                    event: Box::new(Event::draft_issue_state_changed(
                        issue_id.clone(),
                        State::Ready,
                        State::Backlog,
                    )),
                });
            }
            events.push(MutationIntent::RecordEvent {
                phase: 2,
                event: Box::new(Event::draft_issue_updated(
                    issue_id,
                    "dependency-add".to_string(),
                    vec!["dependencies".to_string()],
                )),
            });
        } else {
            events.push(MutationIntent::RecordEvent {
                phase: 3,
                event: Box::new(Event::draft_dependency_reduced(
                    issue_id,
                    original_deps.len(),
                    reduced.len(),
                    removed,
                )),
            });
        }
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
    use crate::storage::{InMemoryStorage, IssueStore, RepositoryStateStore};

    #[derive(Default)]
    struct OneFailure(std::sync::Mutex<Option<crate::storage::TransactionFailurePoint>>);

    impl crate::storage::TransactionFailureInjector for OneFailure {
        fn check(&self, point: &crate::storage::TransactionFailurePoint) -> std::io::Result<()> {
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
        let dependency = crate::domain::types::fixture_issue("dependency".into(), String::new());
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.state = State::Ready;
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source.clone());
        source.title = "concurrent title".to_string();
        let storage = crate::commands::test_helpers::with_open_race(
            storage,
            2,
            crate::commands::test_helpers::OpenRaceAction::Save(Box::new(source)),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);

        executor.add_dependency(&source_id, &dependency_id).unwrap();

        let updated = executor.storage.load_issue(&source_id).unwrap();
        assert_eq!(updated.title, "concurrent title");
        assert_eq!(updated.dependencies, vec![dependency_id]);
        assert_eq!(updated.state, State::Backlog);
        let events = executor.storage.read_events().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].get_type(), "issue_state_changed");
        assert_eq!(events[1].get_type(), "issue_updated");
        let first = serde_json::to_value(&events[0]).unwrap();
        let second = serde_json::to_value(&events[1]).unwrap();
        assert_eq!(first["timestamp"], second["timestamp"]);
    }

    #[test]
    fn test_dependency_removal_and_auto_ready_recover_together() {
        let failures = std::sync::Arc::new(OneFailure(std::sync::Mutex::new(Some(
            crate::storage::TransactionFailurePoint::RepositoryAfterAction { action: 0 },
        ))));
        let storage = InMemoryStorage::with_repository_state_failures(failures);
        storage.add_data_file("config.toml", "");
        let recovered = storage.without_repository_state_failures();
        let layout = storage.repository_layout();

        let mut dependency =
            crate::domain::types::fixture_issue("dependency".into(), String::new());
        dependency.state = State::Done;
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.state = State::Backlog;
        source.dependencies = vec![dependency_id.clone()];
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source);

        let executor = CommandExecutor::new(storage).with_layout(layout.clone());
        assert!(executor
            .remove_dependencies(&source_id, std::slice::from_ref(&dependency_id))
            .is_err());

        drop(recovered.open_mutation_session(layout.clone()).unwrap());
        let unchanged = recovered.load_issue(&source_id).unwrap();
        assert_eq!(unchanged.state, State::Backlog);
        assert_eq!(unchanged.dependencies, vec![dependency_id.clone()]);
        assert!(recovered.read_events().unwrap().is_empty());

        let executor = CommandExecutor::new(recovered.clone()).with_layout(layout);
        executor
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

        let mut dependency =
            crate::domain::types::fixture_issue("dependency".into(), String::new());
        dependency.state = State::Done;
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.state = State::Backlog;
        source.labels = vec!["type:epic".into()];
        source.dependencies = vec![dependency_id.clone()];
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source);

        let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
        let error = executor
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
        assert_eq!(event_types, vec!["issue_updated", "transition_blocked"]);
    }

    #[test]
    fn test_dependency_removal_duplicate_alias_is_not_found_after_first_match() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "");
        let layout = storage.repository_layout();
        let dependency = crate::domain::types::fixture_issue("dependency".into(), String::new());
        let dependency_id = dependency.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.dependencies = vec![dependency_id.clone()];
        let source_id = source.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, source);
        let prefix = dependency_id[..8].to_string();

        let executor = CommandExecutor::new(storage).with_layout(layout);
        let result = executor
            .remove_dependencies(&source_id, &[dependency_id.clone(), prefix.clone()])
            .unwrap();
        assert_eq!(result.removed, vec![dependency_id]);
        assert_eq!(result.not_found, vec![prefix]);
    }

    #[test]
    fn test_single_removal_rejects_dependency_prefix_that_becomes_ambiguous() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let mut dependency = crate::domain::types::fixture_issue("dep".into(), String::new());
        dependency.id = "22221111111111111111111111111111".to_string();
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.id = "11111111111111111111111111111111".to_string();
        source.dependencies = vec![dependency.id.clone()];
        crate::commands::test_helpers::seed_issue(&storage, source.clone());
        crate::commands::test_helpers::seed_issue(&storage, dependency);
        let mut collision = crate::domain::types::fixture_issue("collision".into(), String::new());
        collision.id = "22222222222222222222222222222222".to_string();
        let storage = crate::commands::test_helpers::with_open_race(
            storage,
            2,
            crate::commands::test_helpers::OpenRaceAction::Save(Box::new(collision)),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);
        let events_before = executor.storage.read_events().unwrap();

        let error = executor
            .remove_dependency(&source.id, "2222")
            .expect_err("recaptured dependency prefix must become ambiguous");

        assert!(error
            .downcast_ref::<crate::storage::AmbiguousIdError>()
            .is_some());
        assert_eq!(
            executor
                .storage
                .load_issue(&source.id)
                .unwrap()
                .dependencies,
            source.dependencies
        );
        assert_eq!(executor.storage.read_events().unwrap(), events_before);
    }

    #[test]
    fn test_single_removal_rejects_dependency_deleted_between_phases() {
        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "[worktree]\nenforce_leases = \"off\"\n");
        let mut dependency = crate::domain::types::fixture_issue("dep".into(), String::new());
        dependency.id = "22221111111111111111111111111111".to_string();
        let mut source = crate::domain::types::fixture_issue("source".into(), String::new());
        source.id = "11111111111111111111111111111111".to_string();
        source.dependencies = vec![dependency.id.clone()];
        crate::commands::test_helpers::seed_issue(&storage, source.clone());
        crate::commands::test_helpers::seed_issue(&storage, dependency.clone());
        let storage = crate::commands::test_helpers::with_open_race(
            storage,
            2,
            crate::commands::test_helpers::OpenRaceAction::Delete(dependency.id),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage);
        let events_before = executor.storage.read_events().unwrap();

        let error = executor
            .remove_dependency(&source.id, "2222")
            .expect_err("deleted recaptured dependency must be rejected");

        assert!(error
            .downcast_ref::<crate::storage::IssueNotFoundError>()
            .is_some());
        assert_eq!(
            executor
                .storage
                .load_issue(&source.id)
                .unwrap()
                .dependencies,
            source.dependencies
        );
        assert_eq!(executor.storage.read_events().unwrap(), events_before);
    }
}
