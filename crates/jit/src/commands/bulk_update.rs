//! Bulk update: apply one set of operations to every issue a query filter
//! matches, publishing the whole batch in one redo record.

use super::*;
use crate::domain::identity::GateKey;
use crate::domain::{Assignee, Issue, Priority, State};
use crate::query_engine::QueryFilter;
use crate::repository_state::MutationIntent;
use serde::Serialize;

/// Operations to apply to issues
#[derive(Debug, Clone, Default)]
pub struct UpdateOperations {
    /// New state to set
    pub state: Option<State>,
    /// Labels to add
    pub add_labels: Vec<String>,
    /// Labels to remove
    pub remove_labels: Vec<String>,
    /// New assignee to set
    pub assignee: Option<Assignee>,
    /// Clear assignee
    pub unassign: bool,
    /// New priority to set
    pub priority: Option<Priority>,
    /// Gates to add
    pub add_gates: Vec<GateKey>,
    /// Gates to remove
    pub remove_gates: Vec<GateKey>,
}

/// Result of bulk update operation
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct BulkUpdateResult {
    /// IDs that matched the filter
    pub matched: Vec<IssueId>,
    /// IDs whose change published, including a `Done` request diverted to `Gated`
    pub modified: Vec<IssueId>,
    /// IDs skipped with reasons (id, reason)
    pub skipped: Vec<(IssueId, String)>,
    /// IDs whose change failed or was diverted to `Gated`, with the error (id, error)
    pub errors: Vec<(IssueId, String)>,
    /// Per-issue non-enforcing graph-rule warnings (id, message).
    /// Warnings never block the transition; they are informational findings
    /// from rules with `enforce = false`.  Empty when no rules fired. Reported
    /// as outcome warnings, not in the payload.
    #[serde(skip)]
    pub warnings: Vec<(IssueId, String)>,
    /// Summary statistics
    pub summary: BulkUpdateSummary,
}

/// Summary statistics for bulk update
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct BulkUpdateSummary {
    pub total_matched: usize,
    pub total_modified: usize,
    pub total_skipped: usize,
    pub total_errors: usize,
}

impl BulkUpdateResult {
    /// Create a new empty result
    pub fn new() -> Self {
        BulkUpdateResult {
            matched: Vec::new(),
            modified: Vec::new(),
            skipped: Vec::new(),
            errors: Vec::new(),
            warnings: Vec::new(),
            summary: BulkUpdateSummary {
                total_matched: 0,
                total_modified: 0,
                total_skipped: 0,
                total_errors: 0,
            },
        }
    }

    /// Compute summary from current data
    pub fn compute_summary(&mut self) {
        self.summary = BulkUpdateSummary {
            total_matched: self.matched.len(),
            total_modified: self.modified.len(),
            total_skipped: self.skipped.len(),
            total_errors: self.errors.len(),
        };
    }
}

impl Default for BulkUpdateResult {
    fn default() -> Self {
        Self::new()
    }
}

/// One matched issue's precheck execution, retained across publication retries.
struct BulkPrecheck {
    captured: super::gate_check::CapturedGateEvidence,
    cached: CachedPrecheckExecution,
    /// The run records and prompt files the publishing base must read.
    capture_paths: Vec<crate::repository_state::VirtualPath>,
}

struct BulkUpdateAttemptContext {
    layout: crate::repository_state::RepositoryLayout,
    mutation: crate::repository_state::MutationContext,
    claims_guard: Option<crate::storage::claim_coordinator::ClaimsMutationGuard>,
}

enum BulkAttempt {
    Published(BulkUpdateResult),
    /// The session read the prechecks the batch runs and holds no execution of.
    Preflight(Vec<PrecheckPreflight>),
    Retry,
}

/// What one matched issue's change publishes and reports.
struct MatchedChange {
    intents: Vec<MutationIntent>,
    changed: bool,
    /// A `Done` request the change publishes as `Gated`, reporting `error`.
    diverted: bool,
    warnings: Vec<String>,
    error: Option<String>,
}

/// The batch derived so far over the captured issues, which `issues` holds as
/// the changes before the next one leave them.
struct BulkProjection {
    issues: Vec<Issue>,
    intents: Vec<MutationIntent>,
    result: BulkUpdateResult,
    /// Each matched issue whose change runs prechecks, as the change plans it,
    /// in match order.
    precheck_targets: Vec<Issue>,
}

impl BulkProjection {
    fn record(&mut self, id: &IssueId, change: Result<MatchedChange>) {
        let change = match change {
            Ok(change) => change,
            Err(error) => {
                self.result.errors.push((id.clone(), error.to_string()));
                return;
            }
        };
        change
            .intents
            .iter()
            .filter_map(|intent| match intent {
                MutationIntent::UpdateIssue { issue } => Some(issue),
                _ => None,
            })
            .for_each(|updated| {
                if let Some(slot) = self.issues.iter_mut().find(|issue| issue.id == updated.id) {
                    *slot = (**updated).clone();
                }
            });
        self.intents.extend(change.intents);
        match (change.error, change.changed) {
            (Some(error), _) => {
                if change.diverted {
                    self.result.modified.push(id.clone());
                }
                self.result.errors.push((id.clone(), error));
            }
            (None, changed) => {
                self.result.warnings.extend(
                    change
                        .warnings
                        .into_iter()
                        .map(|warning| (id.clone(), warning)),
                );
                match changed {
                    true => self.result.modified.push(id.clone()),
                    false => self
                        .result
                        .skipped
                        .push((id.clone(), "No changes needed".to_string())),
                }
            }
        }
    }
}

/// The request and captured evidence every matched issue's change derives from.
struct BulkDerivation<'a> {
    operations: &'a UpdateOperations,
    force: bool,
    prechecks: &'a [BulkPrecheck],
    image: &'a crate::repository_state::RepositoryImage,
    graph: &'a CapturedGraphEvidence<'a>,
    context: &'a crate::repository_state::MutationContext,
}

impl BulkDerivation<'_> {
    fn derive(&self, mut projection: BulkProjection, id: &IssueId) -> BulkProjection {
        let change = projection
            .issues
            .iter()
            .find(|issue| &issue.id == id)
            .cloned()
            .ok_or_else(|| crate::storage::IssueNotFoundError::new(id.clone()).into())
            .and_then(|issue| {
                self.derive_change(issue, &projection.issues, &mut projection.precheck_targets)
            });
        projection.record(id, change);
        projection
    }

    /// Derive the change to `issue` against `issues`, noting in
    /// `precheck_targets` the issue as planned when the change runs prechecks.
    fn derive_change(
        &self,
        issue: Issue,
        issues: &[Issue],
        precheck_targets: &mut Vec<Issue>,
    ) -> Result<MatchedChange> {
        let registry = &self.graph.declarations.gates;
        let request = CapturedFieldUpdate::bulk(issue.id.clone(), self.operations, self.force)?;
        let projected = project_field_update(issue.clone(), &request, registry, self.context)?;
        let planned = projected.issue.clone();
        let runs_prechecks = request
            .state
            .is_some_and(|target| state_change_requires_prechecks(&issue, target))
            && has_precheck_gates(&planned, registry);
        let precheck = match runs_prechecks {
            false => None,
            true => {
                precheck_targets.push(planned.clone());
                self.prechecks
                    .iter()
                    .find(|precheck| precheck.cached.target_id == issue.id)
            }
        };
        let record = || {
            record_captured_prechecks(
                precheck.map(|precheck| &precheck.cached),
                precheck.map(|precheck| &precheck.captured),
                self.image,
                &planned,
                registry,
            )
        };
        let recorded = record()?;
        let refusal = precheck
            .and_then(|precheck| precheck.cached.execution.error.as_ref())
            .filter(|_| recorded.refused);
        if let Some(refusal) = refusal {
            return Ok(MatchedChange {
                error: Some(refusal.to_string()),
                intents: recorded.recording_on(issue),
                changed: false,
                diverted: false,
                warnings: Vec::new(),
            });
        }
        let evidence = CapturedTransitionEvidence {
            issues,
            graph: self.graph,
        };
        match derive_prechecked_field_update(issue.clone(), projected, recorded, &request, evidence)
        {
            Ok(derived) => {
                let diverted = derived.changed
                    && derived
                        .error_after_apply
                        .as_ref()
                        .is_some_and(|error| error.actual_state() == State::Gated);
                let error = derived
                    .error_after_apply
                    .map(|error| error.with_warnings(derived.warnings.clone()).to_string());
                Ok(MatchedChange {
                    intents: derived.intents,
                    changed: derived.changed,
                    diverted,
                    warnings: derived.warnings,
                    error,
                })
            }
            // The batch reserved the identifiers of the runs its prechecks
            // executed, so a rejected change still records them.
            Err(error) => Ok(MatchedChange {
                intents: record()?.recording_on(issue),
                changed: false,
                diverted: false,
                warnings: Vec::new(),
                error: Some(error.to_string()),
            }),
        }
    }
}

/// `intents` with only the last update of each issue, which carries every
/// earlier change the batch made to it.
fn latest_issue_updates(intents: Vec<MutationIntent>) -> Vec<MutationIntent> {
    let mut updated = std::collections::HashSet::new();
    let mut latest = intents
        .into_iter()
        .rev()
        .filter(|intent| match intent {
            MutationIntent::UpdateIssue { issue } => updated.insert(issue.id.clone()),
            _ => true,
        })
        .collect::<Vec<_>>();
    latest.reverse();
    latest
}

impl<S: IssueStore> IssueOps<'_, S> {
    /// Apply `operations` to every issue `filter` matches.
    ///
    /// Each attempt opens one publishing session, matches over its captured
    /// issues, and derives each matched issue's change against the issues as
    /// the changes before it in match order leave them. Every change that is
    /// not rejected publishes, with its readiness corrections, in one redo
    /// record; a rejected change is reported in `errors`, and a `Done` request
    /// diverted to `Gated`, which publishes, in `modified` and `errors`. When a
    /// change moves a `Ready` issue with precheck gates to `InProgress`, a
    /// preflight session reads the prechecks first, because they run outside
    /// the repository lock.
    /// One claims guard spans those prechecks and every session attempt.
    pub fn apply_bulk_update(
        &mut self,
        filter: &QueryFilter,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<BulkUpdateResult>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let attempt_context = BulkUpdateAttemptContext {
            layout: self.exec.require_layout()?,
            mutation: production_mutation_context(),
            claims_guard: super::claims_mutation_guard(self.exec.require_worktree_paths()?)?,
        };
        let mut prechecks = Vec::new();
        with_mutation_attempts("bulk update", || {
            let attempt =
                self.attempt_bulk_update(&attempt_context, filter, operations, force, &prechecks)?;
            let preflights = match attempt {
                BulkAttempt::Published(result) => return Ok(AttemptOutcome::Done(result)),
                BulkAttempt::Retry => return Ok(AttemptOutcome::Retry),
                BulkAttempt::Preflight(preflights) => preflights,
            };
            prechecks = self.execute_bulk_prechecks(
                &attempt_context.layout,
                preflights,
                &attempt_context.mutation,
            )?;
            Ok(
                match self.attempt_bulk_update(
                    &attempt_context,
                    filter,
                    operations,
                    force,
                    &prechecks,
                )? {
                    BulkAttempt::Published(result) => AttemptOutcome::Done(result),
                    BulkAttempt::Preflight(_) | BulkAttempt::Retry => AttemptOutcome::Retry,
                },
            )
        })
    }

    /// Run each preflight's prechecks, the batch's runs taking consecutive
    /// gate-run ids.
    fn execute_bulk_prechecks(
        &self,
        layout: &crate::repository_state::RepositoryLayout,
        preflights: Vec<PrecheckPreflight>,
        context: &crate::repository_state::MutationContext,
    ) -> Result<Vec<BulkPrecheck>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let mut first_run = 0;
        preflights
            .into_iter()
            .map(|preflight| {
                let target = preflight.issue.id.clone();
                let mut cached = None;
                let count = self.exec.execute_preflight_prechecks(
                    &mut cached,
                    &target,
                    Some(&preflight),
                    first_run,
                    context,
                )? as u64;
                let runs = first_run..first_run + count;
                first_run += count;
                let capture_paths = gate_run_record_paths(context, &target, runs)
                    .into_iter()
                    .chain(
                        precheck_prompt_paths(layout, &preflight.issue, &preflight.registry)?
                            .into_iter()
                            .map(|(_, path)| path),
                    )
                    .collect();
                Ok(BulkPrecheck {
                    captured: preflight.captured,
                    cached: cached
                        .ok_or_else(|| anyhow!("precheck execution for {target} was not kept"))?,
                    capture_paths,
                })
            })
            .collect()
    }

    /// Derive the batch in one session over `prechecks`, the executions it
    /// holds, and publish it unless the batch runs prechecks it holds none of.
    fn attempt_bulk_update(
        &self,
        attempt_context: &BulkUpdateAttemptContext,
        filter: &QueryFilter,
        operations: &UpdateOperations,
        force: bool,
        prechecks: &[BulkPrecheck],
    ) -> Result<BulkAttempt>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let layout = &attempt_context.layout;
        let context = &attempt_context.mutation;
        let claims_guard = attempt_context.claims_guard.as_ref();
        let targets = prechecks
            .iter()
            .map(|precheck| precheck.cached.target_id.clone())
            .collect::<Vec<_>>();
        let capture_paths = prechecks
            .iter()
            .flat_map(|precheck| precheck.capture_paths.iter().cloned())
            .collect::<Vec<_>>();
        let mut session = self.exec.storage.open_mutation_session(layout.clone())?;
        let Some(mut base) =
            self.exec
                .capture_mutation_base(session.as_mut(), &capture_paths, &targets)?
        else {
            return Ok(BulkAttempt::Retry);
        };
        let issues = std::mem::take(&mut base.issues);
        let image = &base.image;
        let matched = filter
            .filter_issues(&issues)?
            .into_iter()
            .map(|issue| issue.id.clone())
            .collect::<Vec<_>>();
        let mut result = BulkUpdateResult::new();
        result.matched = matched.clone();
        let graph = CapturedGraphEvidence::capture(base.declarations()?, image, &issues, context)?;
        let enforcement_mode = graph.declarations.config().enforcement_mode();
        let derivation = BulkDerivation {
            operations,
            force,
            prechecks,
            image,
            graph: &graph,
            context,
        };
        let BulkProjection {
            intents,
            mut result,
            precheck_targets,
            ..
        } = matched.iter().fold(
            BulkProjection {
                issues,
                intents: Vec::new(),
                result,
                precheck_targets: Vec::new(),
            },
            |mut projection, id| match super::refuse_foreign_leases_held(
                enforcement_mode,
                std::slice::from_ref(id),
                claims_guard,
            ) {
                Ok(warnings) => {
                    projection
                        .result
                        .warnings
                        .extend(warnings.into_iter().map(|warning| (id.clone(), warning)));
                    derivation.derive(projection, id)
                }
                Err(error) => {
                    projection
                        .result
                        .errors
                        .push((id.clone(), error.to_string()));
                    projection
                }
            },
        );
        if !precheck_targets
            .iter()
            .map(|issue| &issue.id)
            .eq(targets.iter())
        {
            let ids = precheck_targets
                .iter()
                .map(|issue| issue.id.clone())
                .collect::<Vec<_>>();
            let Some(base) =
                self.exec
                    .extend_mutation_base(session.as_mut(), image.clone(), &ids)?
            else {
                return Ok(BulkAttempt::Retry);
            };
            return Ok(
                match self.exec.capture_precheck_preflights(
                    session.as_mut(),
                    base.image,
                    precheck_targets,
                    &graph.declarations.gates,
                )? {
                    Some(preflights) => BulkAttempt::Preflight(preflights),
                    None => BulkAttempt::Retry,
                },
            );
        }
        result.compute_summary();
        if intents.is_empty() {
            return Ok(BulkAttempt::Published(result));
        }
        let plan = crate::repository_state::finalize(
            layout,
            image,
            context,
            &latest_issue_updates(intents),
            None,
        )?;
        Ok(match classify_apply(session.apply(&plan), result)? {
            AttemptOutcome::Done(result) => BulkAttempt::Published(result),
            AttemptOutcome::Retry => BulkAttempt::Retry,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::EventBody;
    use crate::domain::{Issue, Priority, State};
    use crate::query_engine::QueryFilter;

    fn create_test_issue(id: &IssueId, state: State, labels: Vec<&str>) -> Issue {
        Issue {
            id: id.clone(),
            title: format!("Test {}", id),
            description: String::new(),
            state,
            priority: Priority::Normal,
            assignee: None,
            dependencies: vec![],
            gates_required: vec![],
            gates_status: Default::default(),
            context: Default::default(),
            documents: vec![],
            labels: labels.iter().map(|s| s.to_string()).collect(),
            content_format: None,
            created_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            updated_at: "2024-01-01T00:00:00Z".parse().unwrap(),
            first_ready_at: None,
            claimed_at: None,
            done_at: None,
            archived_from: None,
        }
    }

    #[test]
    fn test_update_operations_default() {
        let ops = UpdateOperations::default();
        assert!(ops.state.is_none());
        assert!(ops.add_labels.is_empty());
        assert!(ops.remove_labels.is_empty());
        assert!(ops.assignee.is_none());
        assert!(!ops.unassign);
        assert!(ops.priority.is_none());
    }

    #[test]
    fn test_bulk_update_result_new() {
        let result = BulkUpdateResult::new();
        assert!(result.matched.is_empty());
        assert!(result.modified.is_empty());
        assert!(result.skipped.is_empty());
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        assert_eq!(result.summary.total_matched, 0);
    }

    #[test]
    fn test_bulk_update_result_compute_summary() {
        let mut result = BulkUpdateResult::new();
        result.matched = vec![
            crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
            crate::test_utils::issue_id(0xf80e_59d7_a3b8_cbf8_57ae_21a2_6463_191f),
        ];
        result.modified = vec![crate::test_utils::issue_id(
            0x4d6d537d80cc5cce1e28447cf692926d,
        )];
        result.skipped = vec![(
            crate::test_utils::issue_id(0xf80e_59d7_a3b8_cbf8_57ae_21a2_6463_191f),
            "no changes".to_string(),
        )];

        result.compute_summary();

        assert_eq!(result.summary.total_matched, 2);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_skipped, 1);
        assert_eq!(result.summary.total_errors, 0);
    }

    #[test]
    fn test_apply_bulk_update_assignee_stamps_claimed_at_and_logs_event() {
        use crate::query_engine::QueryFilter;
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0x5a6a_549e_b9fb_2027_b699_f39d_12c9_39a1),
                State::Ready,
                vec!["type:task"],
            ),
        );
        // A clone shares the in-memory state, so we can read events after the
        // executor takes ownership of `storage`.
        let reader = storage.clone();

        let executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("agent:worker-1".parse().unwrap()),
            ..Default::default()
        };
        executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        // claimed_at stamped by the bulk assignment.
        let updated = executor
            .issues()
            .show_issue(&crate::test_utils::issue_id(
                0x5a6a549eb9fb2027b699f39d12c939a1,
            ))
            .unwrap();
        assert!(updated.claimed_at.is_some());

        // @/inv/event-log: the assignee mutation appended an issue_claimed event.
        let claimed = reader
            .read_events()
            .unwrap()
            .into_iter()
            .filter(|e| matches!(e.body(), EventBody::IssueClaimed { .. }))
            .count();
        assert_eq!(claimed, 1);

        // First-occurrence: a second assignee change does not move claimed_at.
        let first = updated.claimed_at;
        let ops2 = UpdateOperations {
            assignee: Some("agent:worker-2".parse().unwrap()),
            ..Default::default()
        };
        executor
            .issues()
            .apply_bulk_update(&filter, &ops2, false)
            .unwrap();
        assert_eq!(
            executor
                .issues()
                .show_issue(&crate::test_utils::issue_id(
                    0x5a6a549eb9fb2027b699f39d12c939a1
                ))
                .unwrap()
                .claimed_at,
            first
        );
    }

    #[test]
    fn test_apply_bulk_update_single_issue() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create test issue
        let issue = create_test_issue(
            &crate::test_utils::issue_id(0x5a6a_549e_b9fb_2027_b699_f39d_12c9_39a1),
            State::Ready,
            vec!["type:task"],
        );
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Apply bulk update
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify issue was updated
        let updated = executor
            .issues()
            .show_issue(&crate::test_utils::issue_id(
                0x5a6a549eb9fb2027b699f39d12c939a1,
            ))
            .unwrap();
        assert_eq!(updated.state, State::Done);
    }

    #[test]
    fn test_apply_bulk_update_multiple_issues() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create test issues
        for issue in [
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec![],
            ),
            create_test_issue(
                &crate::test_utils::issue_id(0xc56d_5478_ab94_4a28_fb3c_d51d_1598_c99b),
                State::Ready,
                vec![],
            ),
            create_test_issue(
                &crate::test_utils::issue_id(0xa0cb_57d0_ac34_69f2_7978_37c6_6185_9fa8),
                State::InProgress,
                vec![],
            ),
        ] {
            crate::commands::test_helpers::seed_issue(&storage, issue);
        }

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Update all ready issues
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        assert_eq!(result.summary.total_matched, 2);
        assert_eq!(result.summary.total_modified, 2);

        // Verify labels added
        let issue1 = executor
            .issues()
            .show_issue(&crate::test_utils::issue_id(
                0xedeb5b73a03a451ec76b0d6a0370f495,
            ))
            .unwrap();
        assert!(issue1.labels.contains(&"milestone:v1.0".to_string()));

        let issue3 = executor
            .issues()
            .show_issue(&crate::test_utils::issue_id(
                0xa0cb57d0ac3469f2797837c661859fa8,
            ))
            .unwrap();
        assert!(!issue3.labels.contains(&"milestone:v1.0".to_string()));
    }

    #[test]
    fn test_apply_bulk_update_strict_refuses_leased_match_and_updates_unleased_match() {
        use crate::commands::test_helpers::{plant_foreign_lease, seed_issue};

        let executor = crate::commands::test_helpers::setup_coordinated_with_enforcement("strict");
        let leased_id = crate::test_utils::issue_id(0x1111_0001);
        let unleased_id = crate::test_utils::issue_id(0x1111_0002);
        [leased_id.clone(), unleased_id.clone()]
            .into_iter()
            .map(|id| create_test_issue(&id, State::Ready, vec!["type:task"]))
            .for_each(|issue| seed_issue(&executor.storage, issue));
        let holder = plant_foreign_lease(&executor, &leased_id);

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    priority: Some(Priority::High),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert!(result
            .errors
            .iter()
            .any(|(id, error)| { id == &leased_id && error.contains(&holder.to_string()) }));
        assert!(result.modified.contains(&unleased_id));
        assert_eq!(result.summary.total_errors, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(
            executor.storage.load_issue(&leased_id).unwrap().priority,
            Priority::Normal
        );
        assert_eq!(
            executor.storage.load_issue(&unleased_id).unwrap().priority,
            Priority::High
        );
    }

    #[test]
    fn test_apply_bulk_update_warn_reports_the_leased_match_and_applies_it() {
        use crate::commands::test_helpers::{plant_foreign_lease, seed_issue};

        let executor = crate::commands::test_helpers::setup_coordinated_with_enforcement("warn");
        let issue_id = crate::test_utils::issue_id(0x1111_0003);
        seed_issue(
            &executor.storage,
            create_test_issue(&issue_id, State::Ready, vec!["type:task"]),
        );
        let holder = plant_foreign_lease(&executor, &issue_id);

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    priority: Some(Priority::High),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert!(result
            .warnings
            .iter()
            .any(|(id, warning)| { id == &issue_id && warning.contains(&holder.to_string()) }));
        assert!(result.modified.contains(&issue_id));
        assert_eq!(
            executor.storage.load_issue(&issue_id).unwrap().priority,
            Priority::High
        );
    }

    #[test]
    fn test_apply_bulk_update_does_not_check_leased_issue_promoted_by_readiness_correction() {
        use crate::commands::test_helpers::{plant_foreign_lease, seed_issue};

        let executor = crate::commands::test_helpers::setup_coordinated_with_enforcement("strict");
        let blocker_id = crate::test_utils::issue_id(0x1111_0004);
        let dependent_id = crate::test_utils::issue_id(0x1111_0005);
        seed_issue(
            &executor.storage,
            create_test_issue(&blocker_id, State::Ready, vec!["type:task"]),
        );
        let mut dependent = create_test_issue(&dependent_id, State::Backlog, vec!["type:task"]);
        dependent.dependencies.push(blocker_id.clone());
        seed_issue(&executor.storage, dependent);
        plant_foreign_lease(&executor, &dependent_id);

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    state: Some(State::Done),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert!(result.errors.is_empty());
        assert_eq!(
            executor.storage.load_issue(&dependent_id).unwrap().state,
            State::Ready
        );
    }

    #[test]
    fn test_apply_bulk_update_no_changes() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0x4d6d_537d_80cc_5cce_1e28_447c_f692_926d),
                State::Done,
                vec![],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to set same state
        let filter = QueryFilter::parse("state:done").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_skipped, 1);
    }

    #[test]
    fn test_apply_bulk_update_with_errors() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create issue with unpassed gates
        let mut issue = create_test_issue(
            &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
            State::Gated,
            vec![],
        );
        issue.gates_required = vec!["tests".parse().unwrap()];
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to transition to Done without passing gates
        let filter = QueryFilter::parse("state:gated").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        // The typed gate blocker from the transition chokepoint, rendered.
        assert!(result.errors[0].1.contains("1 gate(s) not passed"));
        assert!(result.errors[0].1.contains("tests [pending]"));
    }

    #[test]
    fn test_apply_bulk_update_best_effort() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create mix of valid and invalid issues
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec![],
            ),
        );

        let mut blocked = create_test_issue(
            &crate::test_utils::issue_id(0xc56d_5478_ab94_4a28_fb3c_d51d_1598_c99b),
            State::Ready,
            vec![],
        );
        blocked.gates_required = vec!["tests".parse().unwrap()];
        crate::commands::test_helpers::seed_issue(&storage, blocked);

        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xa0cb_57d0_ac34_69f2_7978_37c6_6185_9fa8),
                State::Ready,
                vec![],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to transition all to Done
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        // Should succeed for 2, fail for 1
        assert_eq!(result.summary.total_matched, 3);
        assert_eq!(result.summary.total_modified, 2);
        assert_eq!(result.summary.total_errors, 1);

        // Verify partial success
        assert!(result.modified.contains(&crate::test_utils::issue_id(
            0xedeb5b73a03a451ec76b0d6a0370f495
        )));
        assert!(result.modified.contains(&crate::test_utils::issue_id(
            0xa0cb57d0ac3469f2797837c661859fa8
        )));
        assert_eq!(
            result.errors[0].0,
            crate::test_utils::issue_id(0xc56d_5478_ab94_4a28_fb3c_d51d_1598_c99b)
        );
    }

    #[test]
    fn test_bulk_update_rejects_invalid_label_format() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec!["type:task"],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to add label without colon (invalid format)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["bad_label_no_colon".to_string()],
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        // Should reject with error. After the a0f0f342 consolidation the rejection
        // comes from the always-enforced `label-format` rule (canonical format,
        // origin = "default"), not the removed inline `validate_label_operations`
        // check.
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("label-format"));
    }

    #[test]
    fn test_bulk_update_rejects_duplicate_unique_namespace() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        // `type` is unique only because this repository declares it so.
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        storage.add_data_file(
            "config.toml",
            &format!(
                "[worktree]\nenforce_leases = \"off\"\n\n{}",
                taxonomy.config_fragment()
            ),
        );

        // Issue already carries the leaf kind that configuration declares.
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec![&format!("type:{}", taxonomy.type_at_level(4))],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Try to add another type:* label (violates uniqueness)
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec![format!("type:{}", taxonomy.type_at_level(2))],
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        // Should reject with error. After the a0f0f342 consolidation the rejection
        // comes from the always-enforced `namespace-unique-type` rule (origin =
        // "default"), not the removed inline uniqueness check.
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 0);
        assert_eq!(result.summary.total_errors, 1);
        assert!(result.errors[0].1.contains("namespace-unique-type"));
    }

    #[test]
    fn test_bulk_update_sets_typed_assignee() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec![],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Valid assignee format should work
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            assignee: Some("agent:copilot".parse().unwrap()),
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        // Should succeed
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify assignee set correctly
        let updated = executor
            .issues()
            .show_issue(&crate::test_utils::issue_id(
                0xedeb5b73a03a451ec76b0d6a0370f495,
            ))
            .unwrap();
        assert_eq!(updated.assignee, Some("agent:copilot".parse().unwrap()));
    }

    /// Strip the volatile fields (event id, timestamp) so two event logs
    /// recorded by different executors compare on their semantic payload.
    fn event_payload(event: &Event) -> serde_json::Value {
        let mut value = serde_json::to_value(event).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("id");
        object.remove("timestamp");
        value
    }

    /// Every event the log recorded for `issue_id`, in order, payload-normalized.
    fn event_payloads_for(
        storage: &crate::storage::InMemoryStorage,
        issue_id: &IssueId,
    ) -> Vec<String> {
        use crate::storage::IssueStore;
        storage
            .read_events()
            .unwrap()
            .iter()
            .map(event_payload)
            .filter(|value| value["issue_id"].as_str() == Some(issue_id.as_str()))
            .map(|value| value.to_string())
            .collect()
    }

    /// REQ-3: a bulk state transition emits the same event sequence per issue as
    /// the equivalent single-issue transition. Both runs start from an identical
    /// repository, so any divergence is the transition path's own doing.
    #[test]
    fn test_bulk_transition_emits_same_events_as_single_transitions() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let seed = |storage: &InMemoryStorage| {
            for id in [0xaaaa1111, 0xbbbb2222]
                .map(crate::test_utils::issue_id)
                .iter()
            {
                crate::commands::test_helpers::seed_issue(
                    storage,
                    create_test_issue(id, State::Ready, vec!["type:task"]),
                );
            }
        };

        let bulk_storage = InMemoryStorage::new();
        seed(&bulk_storage);
        let bulk_reader = bulk_storage.clone();
        let bulk = crate::commands::test_helpers::memory_executor(bulk_storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            ..Default::default()
        };
        let result = bulk
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();
        assert_eq!(result.summary.total_modified, 2);

        let single_storage = InMemoryStorage::new();
        seed(&single_storage);
        let single_reader = single_storage.clone();
        let single = crate::commands::test_helpers::memory_executor(single_storage);
        for id in [0xaaaa1111, 0xbbbb2222]
            .map(crate::test_utils::issue_id)
            .iter()
        {
            single
                .issues()
                .update_issue(
                    id,
                    None,
                    None,
                    None,
                    Some(State::Done),
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap();
        }

        for id in [0xaaaa1111, 0xbbbb2222]
            .map(crate::test_utils::issue_id)
            .iter()
        {
            assert_eq!(
                event_payloads_for(&bulk_reader, id),
                event_payloads_for(&single_reader, id),
                "event sequence for issue {id} diverges between bulk and single transition"
            );
        }
    }

    #[test]
    fn test_apply_bulk_update_gate_add_records_pending_status_and_one_gate_added_per_key() {
        use crate::query_engine::QueryFilter;
        use crate::storage::{InMemoryStorage, IssueStore};

        let keys: Vec<GateKey> = vec!["tests".parse().unwrap(), "review".parse().unwrap()];
        let storage = InMemoryStorage::new();
        crate::test_harness::seed_memory_gate_keys(&storage, &keys);
        let issue_id = crate::test_utils::issue_id(0x6a7e_0001);
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(&issue_id, State::Ready, vec!["type:task"]),
        );
        let reader = storage.clone();
        let executor = crate::commands::test_helpers::memory_executor(storage);

        executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    add_gates: keys.clone(),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        let stored = reader.load_issue(&issue_id).unwrap();
        assert!(keys.iter().all(|key| stored
            .gates_status
            .get(key)
            .is_some_and(|state| state.status == GateStatus::Pending)));
        let mut added = reader
            .read_events()
            .unwrap()
            .into_iter()
            .filter_map(|event| match event.body() {
                EventBody::GateAdded {
                    issue_id: id,
                    gate_key,
                } if id == &issue_id => Some(gate_key.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        added.sort();
        let mut expected = keys;
        expected.sort();
        assert_eq!(added, expected);
    }

    /// A gate or assignee change requested through any command.
    #[derive(Debug, Clone)]
    enum FieldChange {
        Assign(Assignee),
        Unassign,
        AttachGates(Vec<GateKey>),
        DetachGates(Vec<GateKey>),
    }

    impl FieldChange {
        fn operations(&self) -> UpdateOperations {
            match self.clone() {
                Self::Assign(assignee) => UpdateOperations {
                    assignee: Some(assignee),
                    ..Default::default()
                },
                Self::Unassign => UpdateOperations {
                    unassign: true,
                    ..Default::default()
                },
                Self::AttachGates(add_gates) => UpdateOperations {
                    add_gates,
                    ..Default::default()
                },
                Self::DetachGates(remove_gates) => UpdateOperations {
                    remove_gates,
                    ..Default::default()
                },
            }
        }
    }

    const FIELD_GATES: [&str; 3] = ["tests", "review", "docs"];

    fn field_gate_keys(mask: &[bool]) -> Vec<GateKey> {
        FIELD_GATES
            .iter()
            .zip(mask)
            .filter(|(_, selected)| **selected)
            .map(|(key, _)| key.parse().unwrap())
            .collect()
    }

    fn field_change_strategy() -> impl proptest::strategy::Strategy<Value = FieldChange> {
        use proptest::prelude::*;
        let assignee = prop_oneof![Just("agent:a"), Just("agent:b")]
            .prop_map(|token| token.parse::<Assignee>().unwrap());
        let keys = proptest::collection::vec(any::<bool>(), FIELD_GATES.len())
            .prop_filter("a gate command names a key", |mask| mask.contains(&true))
            .prop_map(|mask| field_gate_keys(&mask));
        prop_oneof![
            assignee.prop_map(FieldChange::Assign),
            Just(FieldChange::Unassign),
            keys.clone().prop_map(FieldChange::AttachGates),
            keys.prop_map(FieldChange::DetachGates),
        ]
    }

    /// The target issue and the stored outcome of `change` applied by `apply`,
    /// timestamps cleared, and the target's event payloads.
    fn field_change_outcome(
        initial: &Issue,
        apply: impl FnOnce(&CommandExecutor<crate::storage::InMemoryStorage>),
    ) -> (Issue, Vec<String>) {
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        crate::test_harness::seed_memory_gate_keys(
            &storage,
            &FIELD_GATES.map(|key| key.parse().unwrap()),
        );
        crate::commands::test_helpers::seed_issue(&storage, initial.clone());
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(&crate::test_utils::issue_id(0xdec0), State::Ready, vec![]),
        );
        let reader = storage.clone();
        apply(&crate::commands::test_helpers::memory_executor(storage));
        let mut stored = reader.load_issue(&initial.id).unwrap();
        stored.updated_at = initial.updated_at;
        stored.claimed_at = None;
        stored
            .gates_status
            .values_mut()
            .for_each(|state| state.updated_at = chrono::DateTime::default());
        (stored, event_payloads_for(&reader, &initial.id))
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig::with_cases(12))]

        /// A gate or assignee change derives one stored issue and one event
        /// sequence whether the dedicated command, a single-issue update, or a
        /// bulk update filtered to the issue requests it.
        #[test]
        fn test_field_change_derives_same_issue_and_events_through_every_command(
            assignee in proptest::option::of(proptest::prelude::prop_oneof![
                proptest::prelude::Just("agent:a"),
                proptest::prelude::Just("agent:b"),
            ]),
            required in proptest::collection::vec(proptest::prelude::any::<bool>(), FIELD_GATES.len()),
            change in field_change_strategy(),
        ) {
            use crate::query_engine::QueryFilter;

            let mut initial = create_test_issue(
                &crate::test_utils::issue_id(0xf1e1d),
                State::Ready,
                vec!["type:task"],
            );
            initial.assignee = assignee.map(|token| token.parse().unwrap());
            initial.gates_required = field_gate_keys(&required);
            initial.gates_status = initial
                .gates_required
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        crate::domain::GateState {
                            status: GateStatus::Pending,
                            updated_by: None,
                            updated_at: chrono::DateTime::default(),
                            run_id: None,
                        },
                    )
                })
                .collect();
            let id = initial.id.clone();
            let prefix = crate::domain::identity::IssueIdPrefix::from(&id);

            let dedicated = field_change_outcome(&initial, |executor| match change.clone() {
                FieldChange::Assign(assignee) => {
                    executor.issues().assign_issue(&prefix, assignee).map(drop).unwrap()
                }
                FieldChange::Unassign => executor.issues().unassign_issue(&prefix).map(drop).unwrap(),
                FieldChange::AttachGates(keys) => {
                    executor.gates().add_gates(&id, &keys).map(drop).unwrap()
                }
                FieldChange::DetachGates(keys) => {
                    executor.gates().remove_gates(&id, &keys).map(drop).unwrap()
                }
            });
            let single = field_change_outcome(&initial, |executor| {
                let operations = change.operations();
                executor
                    .issues()
                    .update_issue_resolved(
                        &prefix,
                        None,
                        None,
                        None,
                        None,
                        vec![],
                        vec![],
                        None,
                        None,
                        crate::commands::IssueLocalUpdate {
                            add_gates: operations.add_gates,
                            remove_gates: operations.remove_gates,
                            assignee: operations.assignee,
                            unassign: operations.unassign,
                        },
                        false,
                    )
                    .map(drop)
                    .unwrap()
            });
            let bulk = field_change_outcome(&initial, |executor| {
                let result = executor
                    .issues()
                    .apply_bulk_update(
                        &QueryFilter::parse("label:type:task").unwrap(),
                        &change.operations(),
                        false,
                    )
                    .unwrap();
                assert_eq!(result.matched, vec![id.clone()]);
                assert!(result.errors.is_empty(), "{:?}", result.errors);
            });

            proptest::prop_assert_eq!(&dedicated, &single);
            proptest::prop_assert_eq!(&single, &bulk);
        }
    }

    #[test]
    fn test_bulk_update_accepts_valid_labels() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec!["type:task"],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);

        // Valid labels should work
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            add_labels: vec!["milestone:v1.0".to_string(), "epic:auth".to_string()],
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        // Should succeed
        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(result.summary.total_modified, 1);
        assert_eq!(result.summary.total_errors, 0);

        // Verify labels added
        let updated = executor
            .issues()
            .show_issue(&crate::test_utils::issue_id(
                0xedeb5b73a03a451ec76b0d6a0370f495,
            ))
            .unwrap();
        assert!(updated.labels.contains(&"milestone:v1.0".to_string()));
        assert!(updated.labels.contains(&"epic:auth".to_string()));
    }

    /// Bulk-update a `Ready` blocker to `Done`, optionally with a `Backlog`
    /// dependent; returns the session opens the update took and the
    /// dependent's state.
    fn bulk_complete_blocker(with_dependent: bool) -> (usize, Option<State>) {
        use crate::commands::test_helpers::{memory_executor, seed_issue, SessionOpenCounter};
        use crate::storage::InMemoryStorage;

        let counter = SessionOpenCounter::new();
        let storage = InMemoryStorage::new().with_mutation_interruption_view(counter.clone());
        let mut blocker = crate::test_utils::fixture_issue("Blocker".into(), String::new());
        blocker.state = State::Ready;
        let blocker_id = blocker.id.clone();
        seed_issue(&storage, blocker);
        let dependent_id = with_dependent.then(|| {
            let mut dependent = crate::test_utils::fixture_issue("Dependent".into(), String::new());
            dependent.dependencies.push(blocker_id.clone());
            let id = dependent.id.clone();
            seed_issue(&storage, dependent);
            id
        });
        let executor = memory_executor(storage);
        let opened = counter.mutation_opens();

        let result = executor
            .issues()
            .apply_bulk_update(
                &crate::query_engine::QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    state: Some(State::Done),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(result.modified, [blocker_id]);
        (
            counter.mutation_opens() - opened,
            dependent_id.map(|id| executor.storage.load_issue(&id).unwrap().state),
        )
    }

    #[test]
    fn test_apply_bulk_update_completing_a_blocker_promotes_its_dependent_in_the_same_publication()
    {
        let (baseline, _) = bulk_complete_blocker(false);
        let (sessions, dependent) = bulk_complete_blocker(true);

        assert_eq!(dependent, Some(State::Ready));
        assert_eq!(
            sessions, baseline,
            "the dependent's promotion must add no session to the blocker's update"
        );
    }

    /// A single-issue `Done` request on `id`, the one `Ready` issue `seed`
    /// builds, and the same request through a bulk update, each over its own
    /// seeded repository: the single update's error, the bulk result, and the
    /// state each leaves the issue in.
    fn complete_single_and_bulk(
        seed: impl Fn(&crate::storage::InMemoryStorage),
        id: &IssueId,
    ) -> (anyhow::Error, BulkUpdateResult, State, State) {
        use crate::storage::{InMemoryStorage, IssueStore};

        let single_storage = InMemoryStorage::new();
        seed(&single_storage);
        let single = crate::commands::test_helpers::memory_executor(single_storage.clone());
        let single_error = single
            .issues()
            .update_issue(
                id,
                None,
                None,
                None,
                Some(State::Done),
                vec![],
                vec![],
                None,
                None,
                false,
            )
            .unwrap_err();

        let bulk_storage = InMemoryStorage::new();
        seed(&bulk_storage);
        let bulk = crate::commands::test_helpers::memory_executor(bulk_storage.clone());
        let result = bulk
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    state: Some(State::Done),
                    ..Default::default()
                },
                false,
            )
            .unwrap();
        (
            single_error,
            result,
            single_storage.load_issue(id).unwrap().state,
            bulk_storage.load_issue(id).unwrap().state,
        )
    }

    /// A dependency-blocked bulk item fails with the single-issue update's
    /// error and leaves the issue as that update does.
    #[test]
    fn test_apply_bulk_update_dependency_blocked_reports_single_update_error() {
        let blocked_id = crate::test_utils::issue_id(0xbbbb_2222);
        let (single_error, result, single_state, bulk_state) = complete_single_and_bulk(
            |storage| {
                crate::commands::test_helpers::seed_issue(
                    storage,
                    create_test_issue(
                        &crate::test_utils::issue_id(0xaaaa_1111),
                        State::Backlog,
                        vec!["type:task"],
                    ),
                );
                let mut blocked = create_test_issue(&blocked_id, State::Ready, vec!["type:task"]);
                blocked.dependencies = vec![crate::test_utils::issue_id(0xaaaa_1111)];
                crate::commands::test_helpers::seed_issue(storage, blocked);
            },
            &blocked_id,
        );

        assert!(single_error
            .downcast_ref::<crate::errors::TransitionBlockedError>()
            .is_some());
        assert_eq!(
            result.errors,
            vec![(blocked_id.clone(), single_error.to_string())]
        );
        assert_eq!(bulk_state, single_state);
    }

    /// `@/inv/gate-semantics`: a bulk `Done` request with an unpassed gate
    /// diverts the issue to `Gated` and reports the single-issue update's error.
    #[test]
    fn test_apply_bulk_update_done_with_unpassed_gate_diverts_like_single_update() {
        let gated_id = crate::test_utils::issue_id(0xaaaa_1111);
        let (single_error, result, single_state, bulk_state) = complete_single_and_bulk(
            |storage| {
                crate::test_harness::seed_memory_gate_keys(storage, &["tests".parse().unwrap()]);
                let mut gated = create_test_issue(&gated_id, State::Ready, vec!["type:task"]);
                gated.gates_required = vec!["tests".parse().unwrap()];
                crate::commands::test_helpers::seed_issue(storage, gated);
            },
            &gated_id,
        );

        assert!(single_error
            .downcast_ref::<crate::errors::TransitionBlockedError>()
            .is_some());
        assert_eq!(
            result.errors,
            vec![(gated_id.clone(), single_error.to_string())]
        );
        assert_eq!(
            result.modified,
            std::slice::from_ref(&gated_id),
            "a diversion to Gated is reported as modified and failed"
        );
        assert_eq!(bulk_state, State::Gated);
        assert_eq!(bulk_state, single_state);
    }

    /// Seed `count` `Ready` issues into a store counting redo-record
    /// publications, at the priority `priority` sets.
    fn publication_counted_ready_issues(
        count: usize,
        priority: Priority,
    ) -> (
        CommandExecutor<crate::storage::InMemoryStorage>,
        std::sync::Arc<crate::commands::test_helpers::PublicationCounter>,
    ) {
        use crate::commands::test_helpers::PublicationCounter;

        let publications = PublicationCounter::new();
        let storage = crate::storage::InMemoryStorage::new()
            .with_mutation_interruption_view(publications.clone());
        (0..count).for_each(|index| {
            let mut issue =
                crate::test_utils::fixture_issue(format!("Issue {index}"), String::new());
            issue.state = State::Ready;
            issue.priority = priority;
            crate::commands::test_helpers::seed_issue(&storage, issue);
        });
        (
            crate::commands::test_helpers::memory_executor(storage),
            publications,
        )
    }

    #[test]
    fn test_apply_bulk_update_publishes_every_matched_change_in_one_redo_record() {
        let issues = 3;
        let (executor, publications) = publication_counted_ready_issues(issues, Priority::Normal);
        let before = publications.count();

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    priority: Some(Priority::High),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(result.modified.len(), issues);
        assert_eq!(publications.count() - before, 1);
    }

    #[test]
    fn test_apply_bulk_update_matches_needing_no_change_publish_no_record() {
        let (executor, publications) = publication_counted_ready_issues(3, Priority::High);
        let before = publications.count();

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    priority: Some(Priority::High),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(result.skipped.len(), result.matched.len());
        assert_eq!(publications.count(), before);
    }

    /// A concurrent write conflicting with the batch's publication retries the
    /// whole batch, which then publishes once.
    #[test]
    fn test_apply_bulk_update_apply_conflict_republishes_the_whole_batch() {
        let issues = 2;
        let (executor, publications) = publication_counted_ready_issues(issues, Priority::Normal);
        executor.storage.inject_repository_state_apply_conflicts(1);
        let before = publications.count();

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    priority: Some(Priority::High),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(result.modified.len(), issues);
        assert_eq!(publications.count() - before, 1);
    }

    /// Each matched change derives against the issues as the changes before it
    /// leave them: completing a dependency promotes its matched dependent,
    /// whose own change then completes it.
    #[test]
    fn test_apply_bulk_update_dependent_matched_after_its_dependency_completes_with_it() {
        use crate::storage::IssueStore;

        let storage = crate::storage::InMemoryStorage::new();
        let dependency = crate::test_utils::issue_id(0xaaaa_1111);
        let dependent = crate::test_utils::issue_id(0xbbbb_2222);
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(&dependency, State::Ready, vec!["type:task"]),
        );
        let mut blocked = create_test_issue(&dependent, State::Backlog, vec!["type:task"]);
        blocked.dependencies = vec![dependency.clone()];
        crate::commands::test_helpers::seed_issue(&storage, blocked);
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("label:type:task").unwrap(),
                &UpdateOperations {
                    state: Some(State::Done),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(result.matched, vec![dependency.clone(), dependent.clone()]);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(storage.load_issue(&dependency).unwrap().state, State::Done);
        assert_eq!(storage.load_issue(&dependent).unwrap().state, State::Done);
    }

    /// Declare `gate` an automatic precheck whose checker runs `command`.
    fn seed_exec_precheck(
        storage: &crate::storage::InMemoryStorage,
        gate: &GateKey,
        command: &str,
    ) {
        use crate::declarations::{GateChecker, GateDefinition, GateMode, GateRegistry, GateStage};

        let mut registry = GateRegistry::default();
        registry.gates.insert(
            gate.clone(),
            GateDefinition {
                version: 1,
                key: gate.clone(),
                title: "Precheck".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: command.to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: std::collections::HashMap::new(),
                    pass_context: false,
                    prompt: None,
                    prompt_file: None,
                }),
                priority: 1,
                reserved: std::collections::HashMap::new(),
                inputs: None,
            },
        );
        crate::commands::test_helpers::seed_gate_registry(storage, &registry);
    }

    /// A bulk move into `InProgress` runs a matched `Ready` issue's prechecks in
    /// one preflight before one publishing session: a failing precheck refuses
    /// that issue alone.
    #[test]
    fn test_apply_bulk_update_failing_precheck_refuses_its_issue_after_one_preflight() {
        use crate::commands::test_helpers::SessionOpenCounter;
        use crate::storage::IssueStore;

        let counter = SessionOpenCounter::new();
        let storage =
            crate::storage::InMemoryStorage::new().with_mutation_interruption_view(counter.clone());
        let gate: GateKey = "precheck-fail".parse().unwrap();
        seed_exec_precheck(&storage, &gate, "exit 1");
        let checked = crate::test_utils::issue_id(0xaaaa_1111);
        let mut issue = create_test_issue(&checked, State::Ready, vec!["type:task"]);
        issue.gates_required = vec![gate];
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let unchecked = crate::test_utils::issue_id(0xbbbb_2222);
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(&unchecked, State::Ready, vec!["type:task"]),
        );
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());
        let before = counter.mutation_opens();

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    state: Some(State::InProgress),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(
            result.errors.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            [&checked]
        );
        assert_eq!(result.modified, std::slice::from_ref(&unchecked));
        assert_eq!(storage.load_issue(&checked).unwrap().state, State::Ready);
        assert_eq!(
            storage.load_issue(&unchecked).unwrap().state,
            State::InProgress
        );
        assert_eq!(
            counter.mutation_opens() - before,
            2,
            "one preflight session and one publishing session"
        );
    }

    /// Prechecks of several matched issues run in one preflight and record
    /// one run each, under distinct run ids.
    #[test]
    fn test_apply_bulk_update_prechecks_of_several_issues_record_distinct_runs() {
        use crate::storage::IssueStore;

        let storage = crate::storage::InMemoryStorage::new();
        let gate: GateKey = "precheck-pass".parse().unwrap();
        seed_exec_precheck(&storage, &gate, "true");
        let ids = [0xaaaa_1111, 0xbbbb_2222].map(crate::test_utils::issue_id);
        ids.iter().for_each(|id| {
            let mut issue = create_test_issue(id, State::Ready, vec!["type:task"]);
            issue.gates_required = vec![gate.clone()];
            crate::commands::test_helpers::seed_issue(&storage, issue);
        });
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());

        let result = executor
            .issues()
            .apply_bulk_update(
                &QueryFilter::parse("state:ready").unwrap(),
                &UpdateOperations {
                    state: Some(State::InProgress),
                    ..Default::default()
                },
                false,
            )
            .unwrap();

        assert_eq!(result.modified, ids);
        let runs = ids
            .iter()
            .map(|id| {
                let issue = storage.load_issue(id).unwrap();
                assert_eq!(issue.state, State::InProgress);
                issue.gates_status[&gate].run_id.clone().unwrap()
            })
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(runs.len(), ids.len());
    }

    /// The local rules run over the projected issue even when no field
    /// changes, so a field-level no-op over a rule-violating issue is an error,
    /// not a skip.
    #[test]
    fn test_apply_bulk_update_field_level_noop_with_preexisting_violation_still_errors() {
        use crate::query_engine::QueryFilter;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        // Seeded directly, bypassing create-time validation, so the issue
        // already violates the always-enforced default label-format rule.
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xedeb_5b73_a03a_451e_c76b_0d6a_0370_f495),
                State::Ready,
                vec!["bad_label_no_colon"],
            ),
        );

        let executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        // Field-level no-op: the requested priority already holds.
        let ops = UpdateOperations {
            priority: Some(Priority::Normal),
            ..Default::default()
        };

        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        assert_eq!(result.summary.total_matched, 1);
        assert_eq!(
            result.summary.total_modified, 0,
            "no field actually changed"
        );
        assert_eq!(
            result.summary.total_errors, 1,
            "a pre-existing rule violation must still surface as an error, not a skip"
        );
        assert_eq!(result.summary.total_skipped, 0);
        assert!(result.errors[0].1.contains("label-format"));
    }

    fn seed_blocked_dependent(storage: &crate::storage::InMemoryStorage) {
        let mut dependent = create_test_issue(
            &crate::test_utils::issue_id(0xaaaa_1111),
            State::Backlog,
            vec!["type:task"],
        );
        dependent.dependencies = vec![crate::test_utils::issue_id(0xbbbb_2222)];
        crate::commands::test_helpers::seed_issue(storage, dependent);
        crate::commands::test_helpers::seed_issue(
            storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xbbbb_2222),
                State::Ready,
                vec!["type:task"],
            ),
        );
    }

    /// REQ-01, REQ-02, REQ-04: a bulk move to either terminal state promotes the
    /// unblocked dependent exactly as the per-issue update by id does.
    #[test]
    fn test_apply_bulk_update_terminal_state_promotes_unblocked_dependent_like_per_issue() {
        use crate::storage::IssueStore;

        for target in [State::Done, State::Rejected] {
            let bulk_storage = crate::storage::InMemoryStorage::new();
            seed_blocked_dependent(&bulk_storage);
            let bulk_reader = bulk_storage.clone();
            let bulk = crate::commands::test_helpers::memory_executor(bulk_storage);
            let filter = QueryFilter::parse("state:ready").unwrap();
            let ops = UpdateOperations {
                state: Some(target),
                ..Default::default()
            };
            let result = bulk
                .issues()
                .apply_bulk_update(&filter, &ops, false)
                .unwrap();
            assert_eq!(
                result.modified,
                vec![crate::test_utils::issue_id(0xbbbb_2222)]
            );

            let single_storage = crate::storage::InMemoryStorage::new();
            seed_blocked_dependent(&single_storage);
            let single_reader = single_storage.clone();
            let single = crate::commands::test_helpers::memory_executor(single_storage);
            single
                .issues()
                .update_issue(
                    &crate::test_utils::issue_id(0xbbbb_2222),
                    None,
                    None,
                    None,
                    Some(target),
                    vec![],
                    vec![],
                    None,
                    None,
                    false,
                )
                .unwrap();

            let bulk_state = bulk_reader
                .load_issue(&crate::test_utils::issue_id(0xaaaa_1111))
                .unwrap()
                .state;
            assert_eq!(bulk_state, State::Ready, "dependent after bulk {target:?}");
            assert_eq!(
                bulk_state,
                single_reader
                    .load_issue(&crate::test_utils::issue_id(0xaaaa_1111))
                    .unwrap()
                    .state,
                "bulk and per-issue {target:?} diverge"
            );
        }
    }

    /// REQ-03: a bulk update that moves nothing to a terminal state leaves
    /// dependents untouched.
    #[test]
    fn test_apply_bulk_update_non_terminal_state_leaves_dependent_unchanged() {
        use crate::storage::IssueStore;

        let storage = crate::storage::InMemoryStorage::new();
        seed_blocked_dependent(&storage);
        let reader = storage.clone();
        let executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:ready").unwrap();
        let ops = UpdateOperations {
            state: Some(State::InProgress),
            ..Default::default()
        };
        executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();

        assert_eq!(
            reader
                .load_issue(&crate::test_utils::issue_id(0xaaaa_1111))
                .unwrap()
                .state,
            State::Backlog
        );
    }

    /// REQ-03: a terminal target that changes only issues already in that state
    /// leaves a stale backlog dependent untouched.
    #[test]
    fn test_apply_bulk_update_terminal_target_over_already_terminal_issue_leaves_dependent_unchanged(
    ) {
        use crate::storage::IssueStore;

        let storage = crate::storage::InMemoryStorage::new();
        let mut dependent = create_test_issue(
            &crate::test_utils::issue_id(0xaaaa_1111),
            State::Backlog,
            vec!["type:task"],
        );
        dependent.dependencies = vec![crate::test_utils::issue_id(0xbbbb_2222)];
        crate::commands::test_helpers::seed_issue(&storage, dependent);
        crate::commands::test_helpers::seed_issue(
            &storage,
            create_test_issue(
                &crate::test_utils::issue_id(0xbbbb_2222),
                State::Done,
                vec!["type:task"],
            ),
        );
        let reader = storage.clone();
        let executor = crate::commands::test_helpers::memory_executor(storage);
        let filter = QueryFilter::parse("state:done").unwrap();
        let ops = UpdateOperations {
            state: Some(State::Done),
            add_labels: vec!["milestone:v1.0".to_string()],
            ..Default::default()
        };
        let result = executor
            .issues()
            .apply_bulk_update(&filter, &ops, false)
            .unwrap();
        assert_eq!(
            result.modified,
            vec![crate::test_utils::issue_id(0xbbbb_2222)]
        );

        assert_eq!(
            reader
                .load_issue(&crate::test_utils::issue_id(0xaaaa_1111))
                .unwrap()
                .state,
            State::Backlog
        );
    }
}
