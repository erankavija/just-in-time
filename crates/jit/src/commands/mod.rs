//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `issue`: Issue CRUD operations and lifecycle management
//! - `dependency`: Dependency graph operations  
//! - `breakdown`: Bracket-aware breakdown operations
//! - `template`: Graph-template apply engine (`jit apply <template> <container>`):
//!   the plan-before-fan-out scaffold (planning node `P` + breakdown node `B`),
//!   validating, snapshotting anchors, and committing the expanded delta
//! - `template_expand`: pure expansion of a graph template into the delta the
//!   apply engine commits (issues to create, edges to add/remove, anchor gates)
//! - `config`: `jit config set` write (target-file resolution, typed value
//!   validation incl. `project.name`, atomic write) and `jit config get`'s
//!   generic dotted-path accessor over the whole configuration surface
//! - `gate`: Quality gate operations
//! - `graph`: Graph visualization and traversal
//! - `query`: Issue query operations
//! - `validate`: Validation and status operations
//! - `labels`: Label operations
//! - `document`: Document reference operations
//! - `events`: Event log operations
//! - `search`: Issue search operations
//! - `item`: Addressable-item queries (`jit item list/show/search`) over the pure
//!   item model

mod archive;
pub mod batch_create;
mod breakdown;
pub mod bulk_update;
pub mod claim;
mod config;
mod dependency;
mod document;
mod events;
mod gate;
mod gate_check;
mod gate_cli_tests;
pub mod graph;
pub mod hooks;
mod init;
pub mod invariant;
mod issue;
pub mod item;
mod labels;
pub mod migrate;
mod profile;
pub mod project;
mod query;
mod search;
pub mod serve;
pub mod snapshot;
mod template;
pub mod template_expand;
mod validate;
pub mod worktree;

// Every item inside is called only from #[cfg(test)] code within this crate.
// When `jit` is compiled as a plain `test-support` dependency without
// `cfg(test)` (e.g. crates/server's own test build, which links this crate's
// TransactionFailureInjector surface but never calls into this module), the
// module stays reachable but every item in it goes genuinely uncalled in that
// one configuration.
#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
pub mod test_helpers;

pub use batch_create::{
    BatchCreateOutcome, BatchDryRunOutcome, BatchIssueDef, BatchValidationError,
    BatchValidationProblem,
};
pub use breakdown::{BracketBreakdownResult, BracketChild};
pub use bulk_update::{BulkUpdatePreview, BulkUpdateResult, UpdateOperations};
pub use config::{resolve_dotted_key, ConfigGetOutcome, ConfigKeyError, ConfigSetOutcome};
pub use gate::{
    FieldEdit, GateNotRequiredError, GatePassAllEntry, GatePassFailed, GatePassOutcome, GateUpdate,
    ManualGateAttestationRequiredError, PassAllOutcome,
};
pub use gate_check::VerdictSource;
pub use graph::{BatchExport, BoundaryEdge, GraphExportFormat};
pub use init::{FreshInitResult, ProfileSelection};
pub use invariant::InvariantCheckResult;
pub use issue::DescriptionUpdate;
pub use item::{ItemListResult, ItemShowResult};
pub use migrate::LifecycleBackfillResult;
pub use profile::{ProfileApplyError, ProfileDependencyError, ProfileResolutionError};
pub use project::{ProjectRenderResult, ProjectionRenderReport};
pub use template::TemplateApplyResult;
pub use template_expand::{
    expand_template, validate_delta_acyclic, AnchorGates, DeltaEdge, DeltaEndpoint, PlannedNode,
    TemplateDelta,
};
pub(crate) use validate::{find_planning_node, planning_node_plan_path, validate_claims_index_at};
pub use validate::{DANGLING_LINK_RULE, ENFORCEMENT_DRIFT_RULE, REVIEW_PLACEHOLDER_RULE};

// Re-export WorktreeIdentity for init return type
pub use crate::storage::worktree_identity::WorktreeIdentity;

// Common imports used across modules
use crate::config::JitConfig;
use crate::config_manager::ConfigManager;
use crate::declarations::rules::{RuleConfigError, RuleSet};
use crate::declarations::{GateDefinition, GateMode};
use crate::domain::{
    is_dependency_met, Event, GateState, GateStatus, Issue, LabelNamespaces, Priority,
    ReadinessCorrection, State,
};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Context, Result};
use serde::Serialize;
use std::sync::OnceLock;

// ── Mutation-session retry contract (mutation-session-contract) ──────────────
//
// One owner of the command-layer capture/plan/apply/retry protocol: the retry
// bound, the apply-conflict classification, and the terminal "did not converge"
// error each live here exactly once. Every command site drives its retry
// through these combinators rather than a hand-copied
// `for _ in 0..MUTATION_SESSION_RETRY_LIMIT` loop or a per-site
// `RetryableConflict => continue` arm.
//
// Two entry points share the same bound and terminal error:
//
// * [`with_mutation_attempts`] — the retry driver. It holds NO session, so a
//   caller may open and release a preflight session, take a claims guard, and
//   run an external subprocess between attempts without inverting lock ordering.
// * [`with_mutation_session`] — the session-passed convenience for pure
//   single-session sites: it opens one fresh recovered session per attempt and
//   applies the plan the closure returns through [`classify_apply`].

/// Maximum capture/apply attempts before a mutation session is reported
/// non-convergent.
///
/// The single home of the retry bound that command retry loops previously
/// copied as the literal `8`.
const MUTATION_SESSION_RETRY_LIMIT: usize = 8;

/// Terminal error for a mutation session that keeps losing its capture/apply
/// race.
///
/// Replaces the bespoke "did not converge after repeated conflicts" bails: it
/// names the operation and the number of attempts made, and maps to the generic
/// exit code through a single arm in `main.rs`.
#[derive(Debug, thiserror::Error)]
#[error("{operation} did not converge after {attempts} capture conflicts")]
pub struct MutationSessionExhausted {
    operation: &'static str,
    attempts: usize,
}

/// Outcome of one retry attempt: a converged value, or a signal to retry.
pub enum AttemptOutcome<T> {
    /// The attempt converged with this value; stop retrying.
    Done(T),
    /// The attempt lost a capture/apply race; retry against a fresh capture.
    Retry,
}

/// Fold one `apply` result into an [`AttemptOutcome`] — the single place a
/// [`RepositoryStateStoreError::RetryableConflict`] raised on apply is
/// interpreted.
///
/// A retryable conflict becomes [`AttemptOutcome::Retry`]; any other storage
/// error propagates; a successful apply carries `value` as
/// [`AttemptOutcome::Done`].
///
/// [`RepositoryStateStoreError::RetryableConflict`]: crate::storage::RepositoryStateStoreError::RetryableConflict
pub fn classify_apply<T>(
    outcome: std::result::Result<impl Sized, crate::storage::RepositoryStateStoreError>,
    value: T,
) -> Result<AttemptOutcome<T>> {
    match outcome {
        Ok(_) => Ok(AttemptOutcome::Done(value)),
        Err(crate::storage::RepositoryStateStoreError::RetryableConflict { .. }) => {
            Ok(AttemptOutcome::Retry)
        }
        Err(error) => Err(error.into()),
    }
}

/// Fold one `capture` result into a retry signal for sites that classify
/// capture conflicts inline.
///
/// A retryable conflict on capture becomes `Ok(None)` (retry); any other
/// storage error propagates; a successful capture yields `Ok(Some(image))`.
/// This mirrors the fold the `capture_*` helpers already apply, so an inline
/// capture site can share the same interpretation instead of writing its own
/// `RetryableConflict => continue` arm.
pub fn capture_or_retry(
    outcome: std::result::Result<
        crate::repository_state::RepositoryImage,
        crate::storage::RepositoryStateStoreError,
    >,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    match outcome {
        Ok(image) => Ok(Some(image)),
        Err(crate::storage::RepositoryStateStoreError::RetryableConflict { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Drive `attempt` up to [`MUTATION_SESSION_RETRY_LIMIT`] times, owning the
/// retry bound and the terminal [`MutationSessionExhausted`] error.
///
/// Each call returns [`AttemptOutcome::Done`] to stop with a value,
/// [`AttemptOutcome::Retry`] to try again, or an error to abort immediately.
/// The driver holds no session, so the closure may open and release sessions,
/// take a claims guard, and run subprocesses between attempts. When the bound is
/// exhausted `operation` is reported as non-convergent via
/// [`MutationSessionExhausted`].
pub fn with_mutation_attempts<T>(
    operation: &'static str,
    mut attempt: impl FnMut() -> Result<AttemptOutcome<T>>,
) -> Result<T> {
    for _ in 0..MUTATION_SESSION_RETRY_LIMIT {
        match attempt()? {
            AttemptOutcome::Done(value) => return Ok(value),
            AttemptOutcome::Retry => continue,
        }
    }
    Err(MutationSessionExhausted {
        operation,
        attempts: MUTATION_SESSION_RETRY_LIMIT,
    }
    .into())
}

/// One step of a session-passed mutation attempt driven by
/// [`with_mutation_session`].
///
/// The plan carried by `Apply` is the dominant field, but this enum is a
/// transient control value returned once per attempt and immediately consumed by
/// the driver — never stored in a collection — so the inline
/// [`MaterializationPlan`](crate::repository_state::MaterializationPlan) is
/// carried by value rather than boxed onto the retry hot path.
#[allow(clippy::large_enum_variant)]
pub enum SessionStep<T> {
    /// Apply this plan through [`classify_apply`]; on success the attempt
    /// converges with the carried value.
    Apply(crate::repository_state::MaterializationPlan, T),
    /// The attempt converged without applying a plan (e.g. a read-only path).
    Done(T),
    /// The captured base is stale; open a fresh session and retry.
    Retry,
}

/// Retry driver for the pure single-session sites: open one fresh recovered
/// session per attempt, hand it to `attempt`, and apply the returned plan
/// through [`classify_apply`].
///
/// Built on [`with_mutation_attempts`], so the retry bound and terminal error
/// stay shared with the self-managed sites. The store generic keeps this a free
/// function usable from both methods and the free-function command entry points.
pub fn with_mutation_session<S: crate::storage::RepositoryStateStore, T>(
    store: &S,
    layout: &crate::repository_state::RepositoryLayout,
    operation: &'static str,
    mut attempt: impl FnMut(
        &mut dyn crate::storage::RepositoryMutationSession,
    ) -> Result<SessionStep<T>>,
) -> Result<T> {
    with_mutation_attempts(operation, || {
        let mut session = store.open_mutation_session(layout.clone())?;
        match attempt(session.as_mut())? {
            SessionStep::Done(value) => Ok(AttemptOutcome::Done(value)),
            SessionStep::Retry => Ok(AttemptOutcome::Retry),
            SessionStep::Apply(plan, value) => classify_apply(session.apply(&plan), value),
        }
    })
}

/// Closed issue-local operations whose final record is derived from the issue
/// captured by the mutation session. Ordinary commands use this instead of
/// constructing a full-record `UpdateIssue` from an earlier storage read.
#[derive(Clone)]
enum CapturedIssueMutation {
    Assign {
        issue_id: String,
        assignee: crate::domain::Assignee,
    },
    Unassign {
        issue_id: String,
    },
    AddGates {
        issue_id: String,
        gate_keys: Vec<String>,
    },
    RemoveGates {
        issue_id: String,
        gate_keys: Vec<String>,
    },
    SetManualGateStatus {
        issue_id: String,
        gate_key: String,
        status: GateStatus,
        by: Option<crate::domain::Assignee>,
    },
}

impl CapturedIssueMutation {
    fn issue_id(&self) -> &str {
        match self {
            Self::Assign { issue_id, .. }
            | Self::Unassign { issue_id }
            | Self::AddGates { issue_id, .. }
            | Self::RemoveGates { issue_id, .. }
            | Self::SetManualGateStatus { issue_id, .. } => issue_id,
        }
    }

    fn captures_gate_registry(&self) -> bool {
        matches!(
            self,
            Self::AddGates { .. } | Self::SetManualGateStatus { .. }
        )
    }
}

enum CapturedIssueMutationOutcome {
    Changed,
    Unchanged,
    GatesAdded {
        added: Vec<String>,
        already_exist: Vec<String>,
    },
    GatesRemoved {
        removed: Vec<String>,
        not_found: Vec<String>,
    },
}

struct DerivedCapturedIssueMutation {
    outcome: CapturedIssueMutationOutcome,
    intents: Vec<crate::repository_state::MutationIntent>,
}

/// Pure result of deriving one lifecycle transition from a closed repository
/// image. Graph-rule refusals carry their audit append so the coordinator can
/// commit the attempted transition before returning the typed error.
enum DerivedStateTransition {
    Applied {
        issue: Box<Issue>,
        warnings: Vec<String>,
        events: PhasedEvents,
        changed: bool,
    },
    GraphBlocked {
        error: crate::errors::TransitionBlockedError,
        events: PhasedEvents,
    },
}

struct CapturedTransitionEvidence<'a> {
    issues: &'a [Issue],
    declarations: &'a crate::repository_state::CapturedRepositoryDeclarations,
    config: &'a JitConfig,
    plan_content: &'a std::collections::HashMap<String, String>,
    context: &'a crate::repository_state::MutationContext,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecheckEvidence {
    inputs: Vec<(
        crate::repository_state::VirtualPath,
        Option<crate::repository_state::EntryIdentity>,
    )>,
    validation_view: Option<PrecheckValidationView>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PrecheckValidationView {
    listings: std::collections::BTreeMap<
        crate::repository_state::VirtualPath,
        crate::repository_state::ListingFingerprint,
    >,
    pinned: std::collections::BTreeMap<
        (String, String),
        crate::repository_state::PinnedDocumentEvidence,
    >,
    linked: std::collections::BTreeMap<
        crate::repository_state::VirtualPath,
        crate::repository_state::LinkedWorktreeEvidence,
    >,
}

impl PrecheckEvidence {
    fn matches(&self, image: &crate::repository_state::RepositoryImage) -> Result<bool> {
        let inputs = self
            .inputs
            .iter()
            .map(|(path, _)| {
                image
                    .entry(path)
                    .map(|entry| (path.clone(), entry.identity().cloned()))
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let validation_view = self
            .validation_view
            .as_ref()
            .map(|_| PrecheckValidationView {
                listings: image.listing_fingerprints().clone(),
                pinned: image.pinned_evidence().clone(),
                linked: image.linked_worktree_evidence().clone(),
            });
        Ok(inputs == self.inputs && validation_view == self.validation_view)
    }
}

struct CapturedPrecheckPlan {
    evidence: PrecheckEvidence,
    prompts: std::collections::HashMap<String, String>,
}

struct CachedPrecheckExecution {
    target_id: String,
    evidence: PrecheckEvidence,
    execution: gate_check::PrecheckExecution,
}

fn checker_consumes_run_history(checker: &crate::declarations::GateChecker) -> bool {
    matches!(
        checker,
        crate::declarations::GateChecker::Exec {
            pass_context: true,
            ..
        }
    )
}

fn captured_precheck_plan(
    image: &crate::repository_state::RepositoryImage,
    issue: &Issue,
    registry: &crate::declarations::GateRegistry,
) -> Result<CapturedPrecheckPlan> {
    use crate::declarations::{GateChecker, GateStage};
    use crate::repository_state::VirtualPath;
    use std::collections::{BTreeSet, HashMap};

    let gates = issue
        .gates_required
        .iter()
        .filter_map(|key| registry.gates.get(key).map(|gate| (key, gate)))
        .filter(|(_, gate)| gate.stage == GateStage::Precheck)
        .collect::<Vec<_>>();
    let broad_builtin = gates.iter().any(|(_, gate)| {
        gate.mode == GateMode::Auto
            && !matches!(gate.checker, Some(GateChecker::Exec { .. }))
            && !matches!(gate.checker, Some(GateChecker::ReviewPlaceholder))
    });
    let consumes_history = gates.iter().any(|(_, gate)| {
        gate.mode == GateMode::Auto
            && gate
                .checker
                .as_ref()
                .is_some_and(checker_consumes_run_history)
    });
    let mut paths = if broad_builtin {
        image.entries().keys().cloned().collect::<BTreeSet<_>>()
    } else {
        BTreeSet::from([
            VirtualPath::data(format!("issues/{}.json", issue.id))?,
            VirtualPath::GATES,
        ])
    };
    if !broad_builtin {
        paths.extend(
            issue
                .dependencies
                .iter()
                .map(|id| VirtualPath::data(format!("issues/{id}.json")))
                .collect::<std::result::Result<Vec<_>, _>>()?,
        );
        if consumes_history {
            let run_paths = crate::repository_state::captured_gate_run_result_paths(image)?
                .ok_or_else(|| anyhow!("captured gate-run root has no complete listing"))?;
            for path in run_paths {
                match image.entry(&path)? {
                    crate::repository_state::RepositoryEntry::File { bytes, .. } => {
                        let run: crate::domain::GateRunResult = serde_json::from_slice(bytes)
                            .with_context(|| {
                                format!("failed to parse captured gate run at {path:?}")
                            })?;
                        if run.issue_id == issue.id {
                            paths.insert(path);
                        }
                    }
                    crate::repository_state::RepositoryEntry::Absent => {}
                    _ => anyhow::bail!("captured gate run at {path:?} is not a regular file"),
                }
            }
        }
    }
    let mut prompts = HashMap::new();
    for (key, gate) in gates {
        if gate.mode != GateMode::Auto {
            continue;
        }
        if let Some(GateChecker::Exec {
            pass_context: true,
            prompt_file: Some(path),
            ..
        }) = &gate.checker
        {
            let path = image.layout().classify_repository_relative(path)?;
            let bytes = image
                .file_bytes(&path)?
                .ok_or_else(|| anyhow!("captured precheck prompt file '{:?}' is missing", path))?;
            prompts.insert(key.clone(), String::from_utf8(bytes.to_vec())?);
            paths.insert(path);
        }
    }
    let inputs = paths
        .into_iter()
        .map(|path| {
            image
                .entry(&path)
                .map(|entry| (path, entry.identity().cloned()))
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let validation_view = broad_builtin.then(|| PrecheckValidationView {
        listings: image.listing_fingerprints().clone(),
        pinned: image.pinned_evidence().clone(),
        linked: image.linked_worktree_evidence().clone(),
    });
    Ok(CapturedPrecheckPlan {
        evidence: PrecheckEvidence {
            inputs,
            validation_view,
        },
        prompts,
    })
}

fn claims_mutation_guard(
    layout: &crate::repository_state::RepositoryLayout,
) -> Result<Option<crate::storage::claim_coordinator::ClaimsMutationGuard>> {
    use crate::storage::worktree_paths::WorktreePaths;
    use crate::storage::{ClaimCoordinator, FileLocker};
    use std::time::Duration;

    let in_git = std::process::Command::new("git")
        .arg("-C")
        .arg(layout.worktree_root())
        .args(["rev-parse", "--is-inside-work-tree"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !in_git {
        return Ok(None);
    }
    let paths = WorktreePaths::detect_from(layout.worktree_root())?;
    let agent = crate::agent_config::resolve_agent_id(None)
        .unwrap_or_else(|_| "system:lease-check".to_string());
    let worktree_id = crate::storage::worktree_identity::read_worktree_id(layout.worktree_root())?
        .unwrap_or_else(|| layout.worktree_root().display().to_string());
    let coordinator = ClaimCoordinator::new(
        paths,
        FileLocker::new(Duration::from_secs(
            crate::runtime_defaults::LOCK_TIMEOUT_SECS,
        )),
        worktree_id,
        agent,
    );
    coordinator.lock_claims_for_repository_mutation().map(Some)
}

fn captured_lease_warnings(
    mode: crate::config::EnforcementMode,
    targets: &[String],
    issues: &[Issue],
    guard: Option<&crate::storage::claim_coordinator::ClaimsMutationGuard>,
) -> Result<Vec<String>> {
    use crate::agent_config::resolve_agent_id;
    use crate::config::EnforcementMode;

    if mode == EnforcementMode::Off {
        return Ok(Vec::new());
    }
    let agent = resolve_agent_id(None).ok();
    let mut warnings = Vec::new();
    for id in targets {
        let active = guard
            .map(|guard| {
                guard.has_active_lease(id, agent.as_deref(), |raw| {
                    resolve_issue_from_capture(issues, raw)
                })
            })
            .transpose()?
            .unwrap_or(false);
        if active {
            continue;
        }
        let message =
            format!("No active lease for issue {id}.\nAcquire lease with: jit claim acquire {id}");
        match mode {
            EnforcementMode::Warn => warnings.push(message),
            EnforcementMode::Strict => return Err(anyhow!(message)),
            EnforcementMode::Off => unreachable!(),
        }
    }
    Ok(warnings)
}

fn ensure_claim_available(
    guard: Option<&crate::storage::claim_coordinator::ClaimsMutationGuard>,
    target: &str,
    assignee: &crate::domain::Assignee,
    issues: &[Issue],
) -> Result<()> {
    let Some(lease) = guard
        .map(|guard| {
            guard.conflicting_lease(target, Some(&assignee.to_string()), |raw| {
                resolve_issue_from_capture(issues, raw)
            })
        })
        .transpose()?
        .flatten()
    else {
        return Ok(());
    };
    let expires = lease
        .expires_at
        .map(|time| time.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|| "indefinitely".to_string());
    Err(anyhow!(
        "Issue {} is currently leased by {} {}.\nUse 'jit claim acquire' to properly coordinate work.",
        target,
        lease.agent_id,
        expires
    ))
}

#[derive(Clone)]
enum CapturedLifecycleMutation {
    State {
        issue_id: String,
        target: State,
        force: bool,
        divert_unpassed_gates: bool,
        enforce_lease: bool,
    },
    Claim {
        issue_id: String,
        assignee: crate::domain::Assignee,
    },
    Release {
        issue_id: String,
        reason: String,
    },
    AutoReady {
        issue_id: String,
    },
    /// Demote a `Ready` issue an unmet dependency blocks back to `Backlog`,
    /// restoring agreement between stored readiness and the dependency graph
    /// (`@/invariant/derived-state-coherence`). The inverse of [`Self::AutoReady`].
    AutoBacklog {
        issue_id: String,
    },
    AutoDone {
        issue_id: String,
    },
}

impl CapturedLifecycleMutation {
    fn issue_id(&self) -> &str {
        match self {
            Self::State { issue_id, .. }
            | Self::Claim { issue_id, .. }
            | Self::Release { issue_id, .. }
            | Self::AutoReady { issue_id }
            | Self::AutoBacklog { issue_id }
            | Self::AutoDone { issue_id } => issue_id,
        }
    }
}

fn lifecycle_requires_prechecks(request: &CapturedLifecycleMutation, issue: &Issue) -> bool {
    issue.state == State::Ready
        && matches!(
            request,
            CapturedLifecycleMutation::State {
                target: State::InProgress,
                ..
            } | CapturedLifecycleMutation::Claim { .. }
        )
}

struct CapturedLifecycleOutcome {
    target_id: String,
    changed: bool,
    warnings: Vec<String>,
    storage_warnings: Vec<crate::storage::StorageWarning>,
}

#[derive(Clone)]
struct CapturedFieldUpdate {
    issue_id: String,
    title: Option<String>,
    description: Option<DescriptionUpdate>,
    priority: Option<Priority>,
    state: Option<State>,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
    label_edit: Option<CapturedLabelEdit>,
    content_format: Option<Option<crate::domain::ContentFormat>>,
    issue_type: Option<String>,
    add_gates: Vec<String>,
    remove_gates: Vec<String>,
    assignee: Option<crate::domain::Assignee>,
    unassign: bool,
    bulk: bool,
    force: bool,
    enforce_lease: bool,
}

#[derive(Clone)]
enum CapturedLabelEdit {
    AppendUnchecked(String),
    ReplaceExact { old: String, new: String },
}

impl CapturedFieldUpdate {
    /// Build the bulk-update request shape for one issue. Shared by the real
    /// per-issue publication path ([`CommandExecutor::publish_captured_bulk_update`])
    /// and the no-op verification derivation
    /// ([`CommandExecutor::confirm_bulk_noop_candidates`]) so both run the
    /// identical authoritative request through [`derive_field_update`] --
    /// there is exactly one place that turns `UpdateOperations` into a
    /// captured request, so the two call sites cannot silently drift apart.
    fn bulk(issue_id: String, operations: &UpdateOperations, force: bool) -> Result<Self> {
        Ok(Self {
            issue_id,
            title: None,
            description: None,
            priority: operations.priority,
            state: operations.state,
            add_labels: operations.add_labels.clone(),
            remove_labels: operations.remove_labels.clone(),
            label_edit: None,
            content_format: None,
            issue_type: None,
            add_gates: operations.add_gates.clone(),
            remove_gates: operations.remove_gates.clone(),
            assignee: operations.assignee.as_deref().map(str::parse).transpose()?,
            unassign: operations.unassign,
            bulk: true,
            force,
            enforce_lease: false,
        })
    }

    fn label_edit(issue_id: String, label_edit: CapturedLabelEdit) -> Self {
        Self {
            issue_id,
            title: None,
            description: None,
            priority: None,
            state: None,
            add_labels: Vec::new(),
            remove_labels: Vec::new(),
            label_edit: Some(label_edit),
            content_format: None,
            issue_type: None,
            add_gates: Vec::new(),
            remove_gates: Vec::new(),
            assignee: None,
            unassign: false,
            bulk: false,
            force: false,
            enforce_lease: false,
        }
    }
}

struct DerivedFieldUpdate {
    changed: bool,
    warnings: Vec<String>,
    intents: Vec<crate::repository_state::MutationIntent>,
    error_after_apply: Option<crate::errors::TransitionBlockedError>,
}

struct CapturedFieldUpdateOutcome {
    changed: bool,
    warnings: Vec<String>,
}

fn derive_write_validation(
    issue: &Issue,
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
    config: &JitConfig,
    force: bool,
) -> Result<WriteValidation> {
    let repo_format = config
        .validation
        .as_ref()
        .map(crate::config::ValidationConfig::content_format)
        .transpose()?
        .unwrap_or(crate::domain::ContentFormat::Markdown);
    let strictness = config
        .validation
        .as_ref()
        .map(crate::config::ValidationConfig::strictness)
        .transpose()?
        .unwrap_or_default();
    let evaluation = crate::validation::evaluate_local(issue, &declarations.rules, repo_format)
        .map_err(|error| anyhow!("rule evaluation failed: {error}"))?
        .with_strictness(strictness);
    let blocking = evaluation.blocking_rules();
    if !blocking.is_empty() && !force {
        return Err(crate::errors::ValidationFailedError::new(
            evaluation
                .rejection_message()
                .unwrap_or_else(|| "blocked by validation rule(s)".to_string()),
        )
        .into());
    }
    Ok(WriteValidation {
        warnings: evaluation.warnings(),
        bypassed_rules: blocking,
    })
}

/// Derive one state transition without storage access.
fn derive_state_transition(
    mut issue: Issue,
    target: State,
    force: bool,
    evidence: CapturedTransitionEvidence<'_>,
) -> Result<DerivedStateTransition> {
    use crate::declarations::rules::{RuleScope, Severity};
    use crate::validation::graph::evaluate_graph;
    use std::collections::HashSet;

    let old_state = issue.state;
    if old_state == target {
        return Ok(DerivedStateTransition::Applied {
            issue: Box::new(issue),
            warnings: Vec::new(),
            events: Vec::new(),
            changed: false,
        });
    }

    let mut warnings = Vec::new();
    if old_state == State::Archived {
        match issue.archived_from {
            Some(origin) if target != origin => {
                return Err(crate::errors::TransitionBlockedError::archived_revive(
                    issue.id.clone(),
                    target,
                    origin,
                )
                .into());
            }
            None => warnings.push(format!(
                "issue {} was archived before its pre-archive state was recorded; reviving to \
                 '{}' without a verified origin",
                issue.short_id(),
                target.as_str()
            )),
            Some(_) => {}
        }
    }

    if matches!(target, State::Ready | State::Done) {
        let blockers = issue::blocking_dependencies(
            &issue,
            &crate::domain::queries::build_issue_map(evidence.issues),
        );
        if !blockers.is_empty() {
            return Err(crate::errors::TransitionBlockedError::dependencies(
                issue.id.clone(),
                target,
                old_state,
                blockers,
            )
            .into());
        }
        if target == State::Done && issue.has_unpassed_gates() {
            return Err(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                target,
                old_state,
                unpassed_gate_blockers(&issue, &evidence.declarations.gates),
            )
            .into());
        }
    }

    let mut bypass_events = Vec::new();
    if !matches!(target, State::Rejected | State::Archived) {
        let mut projected = issue.clone();
        projected.state = target;
        let rules = evidence
            .declarations
            .rules
            .rules
            .iter()
            .filter(|rule| rule.scope == RuleScope::Graph && rule.severity != Severity::Off)
            .filter(|rule| !rule.assert.is_repo_wide_at_transition())
            .filter(|rule| rule.when.matches(&projected))
            .collect::<Vec<_>>();
        if !rules.is_empty() {
            let refs = evidence.issues.iter().collect::<Vec<_>>();
            let graph = DependencyGraph::new(&refs);
            let mut ids = HashSet::from([projected.id.clone()]);
            ids.extend(
                graph
                    .get_transitive_dependents(&projected.id)
                    .into_iter()
                    .map(|candidate| candidate.id.clone()),
            );
            ids.extend(
                graph
                    .get_transitive_dependencies(&projected.id)
                    .into_iter()
                    .map(|candidate| candidate.id.clone()),
            );
            let slice = evidence
                .issues
                .iter()
                .filter(|candidate| ids.contains(&candidate.id))
                .map(|candidate| {
                    if candidate.id == projected.id {
                        projected.clone()
                    } else {
                        candidate.clone()
                    }
                })
                .collect::<Vec<_>>();
            let namespaces = crate::config_manager::namespaces_from_config(evidence.config);
            let hierarchy = crate::repository_state::hierarchy_config(&namespaces);
            let repo_format = evidence
                .config
                .validation
                .as_ref()
                .map(crate::config::ValidationConfig::content_format)
                .transpose()?
                .unwrap_or(crate::domain::ContentFormat::Markdown);
            let strictness = evidence
                .config
                .validation
                .as_ref()
                .map(crate::config::ValidationConfig::strictness)
                .transpose()?
                .unwrap_or_default();
            let enforcing = rules
                .iter()
                .filter(|rule| rule.enforce)
                .map(|rule| rule.name.as_str())
                .collect::<HashSet<_>>();
            let findings = evaluate_graph(
                &rules,
                &slice,
                &hierarchy,
                repo_format,
                evidence.context.timestamp(),
                evidence.plan_content,
            );
            let mut blocking = Vec::new();
            for finding in findings {
                let config_error = finding.is_config_error();
                let pertains =
                    config_error || finding.issue_id.as_deref() == Some(projected.id.as_str());
                if pertains
                    && strictness.blocks(
                        enforcing.contains(finding.finding.rule.as_str()),
                        finding.finding.severity,
                    )
                {
                    let message = if config_error {
                        format!(
                            "rule '{}' is misconfigured: {}; fix the rule or use --force",
                            finding.finding.rule, finding.finding.message
                        )
                    } else {
                        finding.finding.message.clone()
                    };
                    blocking.push((finding.finding.rule, message));
                } else {
                    warnings.push(format!(
                        "[{}] {}",
                        finding.finding.rule, finding.finding.message
                    ));
                }
            }
            if !blocking.is_empty() {
                let events = blocking
                    .iter()
                    .map(|(rule, _)| {
                        (
                            1,
                            if force {
                                Event::draft_graph_rule_bypassed(
                                    issue.id.clone(),
                                    target,
                                    rule.clone(),
                                )
                            } else {
                                Event::draft_transition_blocked(
                                    issue.id.clone(),
                                    target,
                                    rule.clone(),
                                )
                            },
                        )
                    })
                    .collect::<Vec<_>>();
                if force {
                    bypass_events = events;
                } else {
                    return Ok(DerivedStateTransition::GraphBlocked {
                        error: crate::errors::TransitionBlockedError::graph_rules(
                            issue.id.clone(),
                            target,
                            old_state,
                            blocking,
                        ),
                        events,
                    });
                }
            }
        }
    }

    issue.state = target;
    if target == State::Archived {
        issue.archived_from = Some(old_state);
    } else if old_state == State::Archived {
        issue.archived_from = None;
    }
    bypass_events.push((
        2,
        Event::draft_issue_state_changed(issue.id.clone(), old_state, target),
    ));
    if target == State::Done {
        bypass_events.push((3, Event::draft_issue_completed(issue.id.clone())));
    }
    Ok(DerivedStateTransition::Applied {
        issue: Box::new(issue),
        warnings,
        events: bypass_events,
        changed: true,
    })
}

fn derive_field_update(
    original: Issue,
    request: &CapturedFieldUpdate,
    evidence: CapturedTransitionEvidence<'_>,
) -> Result<DerivedFieldUpdate> {
    use crate::repository_state::MutationIntent;

    let mut issue = original.clone();
    if let Some(title) = &request.title {
        issue.title = title.clone();
    }
    if let Some(description) = &request.description {
        issue.description = description.clone().apply(&issue.description);
    }
    if let Some(priority) = request.priority {
        issue.priority = priority;
    }
    if let Some(content_format) = request.content_format {
        issue.content_format = content_format;
    }
    for label in &request.add_labels {
        if !issue.labels.contains(label) {
            issue.labels.push(label.clone());
        }
    }
    for label in &request.remove_labels {
        issue.labels.retain(|candidate| candidate != label);
    }
    match &request.label_edit {
        Some(CapturedLabelEdit::AppendUnchecked(label)) => issue.labels.push(label.clone()),
        Some(CapturedLabelEdit::ReplaceExact { old, new })
            if issue.labels.iter().any(|label| label == old) =>
        {
            issue.labels.retain(|label| label != old);
            issue.labels.push(new.clone());
        }
        Some(CapturedLabelEdit::ReplaceExact { .. }) => {}
        None => {}
    }
    let missing_gates = request
        .add_gates
        .iter()
        .filter(|key| !evidence.declarations.gates.gates.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    if !missing_gates.is_empty() {
        return Err(crate::storage::GateNotFoundError::new(missing_gates).into());
    }
    for gate in &request.add_gates {
        if !issue.gates_required.contains(gate) {
            issue.gates_required.push(gate.clone());
        }
    }
    for gate in &request.remove_gates {
        issue.gates_required.retain(|candidate| candidate != gate);
        issue.gates_status.remove(gate);
    }
    if let Some(assignee) = &request.assignee {
        issue.assignee = Some(assignee.clone());
    } else if request.unassign {
        issue.assignee = None;
    }
    if let Some(kind) = &request.issue_type {
        issue
            .labels
            .retain(|label| !label_utils::is_type_label(label));
        issue.labels.push(label_utils::type_label(kind));
    }

    let mut changed_fields = Vec::new();
    if issue.title != original.title {
        changed_fields.push("title".to_string());
    }
    if issue.description != original.description {
        changed_fields.push("description".to_string());
    }
    if issue.priority != original.priority {
        changed_fields.push("priority".to_string());
    }
    if issue.labels != original.labels {
        changed_fields.push("labels".to_string());
    }
    if issue.content_format != original.content_format {
        changed_fields.push("content_format".to_string());
    }
    if issue.gates_required != original.gates_required {
        changed_fields.push("gates".to_string());
    }
    if issue.assignee != original.assignee {
        changed_fields.push("assignee".to_string());
    }

    let mut gate_error = None;
    let target = match request.state {
        Some(State::Done) if issue.has_unpassed_gates() => {
            let blockers = issue::blocking_dependencies(
                &issue,
                &crate::domain::queries::build_issue_map(evidence.issues),
            );
            if !blockers.is_empty() {
                return Err(crate::errors::TransitionBlockedError::dependencies(
                    issue.id.clone(),
                    State::Done,
                    original.state,
                    blockers,
                )
                .into());
            }
            gate_error = Some(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                State::Done,
                State::Gated,
                unpassed_gate_blockers(&issue, &evidence.declarations.gates),
            ));
            Some(State::Gated)
        }
        target => target,
    };

    let mut projected = issue.clone();
    if let Some(target) = target {
        projected.state = target;
    }
    let validation = match &request.label_edit {
        Some(CapturedLabelEdit::AppendUnchecked(_)) => {
            let repo_format = evidence
                .config
                .validation
                .as_ref()
                .map(crate::config::ValidationConfig::content_format)
                .transpose()?
                .unwrap_or(crate::domain::ContentFormat::Markdown);
            let evaluation = crate::validation::evaluate_local(
                &projected,
                &evidence.declarations.rules,
                repo_format,
            )
            .map_err(|error| anyhow!("rule evaluation failed: {error}"))?;
            WriteValidation {
                warnings: evaluation
                    .findings()
                    .into_iter()
                    .filter(|finding| finding.severity != crate::declarations::rules::Severity::Off)
                    .map(|finding| format!("[{}] {}", finding.rule, finding.message))
                    .collect(),
                bypassed_rules: Vec::new(),
            }
        }
        Some(CapturedLabelEdit::ReplaceExact { .. }) => WriteValidation {
            warnings: Vec::new(),
            bypassed_rules: Vec::new(),
        },
        None => derive_write_validation(
            &projected,
            evidence.declarations,
            evidence.config,
            request.force,
        )?,
    };
    if request.issue_type.is_some() {
        let repo_format = evidence
            .config
            .validation
            .as_ref()
            .map(crate::config::ValidationConfig::content_format)
            .transpose()?
            .unwrap_or(crate::domain::ContentFormat::Markdown);
        let explicit_type = crate::validation::evaluate_local(
            &projected,
            &evidence.declarations.rules,
            repo_format,
        )
        .map_err(|error| anyhow!("rule evaluation failed: {error}"))?;
        if let Some(finding) = explicit_type
            .findings()
            .into_iter()
            .find(|finding| finding.rule == "type-hierarchy-known")
        {
            return Err(crate::errors::ValidationFailedError::new(finding.message.clone()).into());
        }
    }

    let mut warnings = validation.warnings;
    let mut events = Vec::new();
    if let Some(target) = target {
        match derive_state_transition(
            issue,
            target,
            request.force,
            CapturedTransitionEvidence { ..evidence },
        )? {
            DerivedStateTransition::Applied {
                issue: transitioned,
                warnings: transition_warnings,
                events: transition_events,
                ..
            } => {
                issue = *transitioned;
                warnings.extend(transition_warnings);
                events.extend(transition_events);
            }
            DerivedStateTransition::GraphBlocked {
                error,
                events: blocked_events,
            } => {
                return Ok(DerivedFieldUpdate {
                    changed: false,
                    warnings,
                    intents: blocked_events
                        .into_iter()
                        .map(|(phase, event)| MutationIntent::RecordEvent {
                            phase,
                            event: Box::new(event),
                        })
                        .collect(),
                    error_after_apply: Some(error),
                });
            }
        }
    }

    let changed = !changed_fields.is_empty() || issue.state != original.state;
    if issue.assignee != original.assignee {
        if let Some(assignee) = issue.assignee.clone() {
            events.push((4, Event::draft_issue_claimed(issue.id.clone(), assignee)));
        }
    }
    if !changed_fields.is_empty() && request.label_edit.is_none() {
        events.push((
            5,
            Event::draft_issue_updated(
                issue.id.clone(),
                if request.bulk {
                    "bulk-update".to_string()
                } else {
                    "issue-update".to_string()
                },
                changed_fields,
            ),
        ));
    }
    events.extend(
        validation
            .bypassed_rules
            .into_iter()
            .map(|rule| (9, Event::draft_local_rule_bypassed(issue.id.clone(), rule))),
    );
    let intents = changed
        .then(|| MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        })
        .into_iter()
        .chain(
            events
                .into_iter()
                .map(|(phase, event)| MutationIntent::RecordEvent {
                    phase,
                    event: Box::new(event),
                }),
        )
        .collect();
    Ok(DerivedFieldUpdate {
        changed,
        warnings,
        intents,
        error_after_apply: gate_error,
    })
}

/// Derive the full-record low-level intent and its audit events from one captured
/// issue. Keeping this pure makes the retry contract directly testable: every
/// attempt receives the new image's record and cannot retain an ambient preimage.
fn derive_captured_issue_mutation(
    mut issue: Issue,
    registry: &crate::declarations::GateRegistry,
    request: &CapturedIssueMutation,
) -> Result<DerivedCapturedIssueMutation> {
    use crate::repository_state::MutationIntent;

    let (outcome, events, changed) = match request {
        CapturedIssueMutation::Assign { assignee, .. } => {
            if issue.assignee.as_ref() == Some(assignee) {
                (CapturedIssueMutationOutcome::Unchanged, Vec::new(), false)
            } else {
                issue.assignee = Some(assignee.clone());
                let event = Event::draft_issue_claimed(issue.id.clone(), assignee.clone());
                (
                    CapturedIssueMutationOutcome::Changed,
                    vec![(1, event)],
                    true,
                )
            }
        }
        CapturedIssueMutation::Unassign { .. } => {
            if issue.assignee.is_none() {
                (CapturedIssueMutationOutcome::Unchanged, Vec::new(), false)
            } else {
                issue.assignee = None;
                let event = Event::draft_issue_updated(
                    issue.id.clone(),
                    "issue-unassign".to_string(),
                    vec!["assignee".to_string()],
                );
                (
                    CapturedIssueMutationOutcome::Changed,
                    vec![(1, event)],
                    true,
                )
            }
        }
        CapturedIssueMutation::AddGates { gate_keys, .. } => {
            let missing = gate_keys
                .iter()
                .filter(|key| !registry.gates.contains_key(*key))
                .cloned()
                .collect::<Vec<_>>();
            if !missing.is_empty() {
                return Err(crate::storage::GateNotFoundError::new(missing).into());
            }
            let mut added = Vec::new();
            let mut already_exist = Vec::new();
            for key in gate_keys {
                if issue.gates_required.contains(key) {
                    already_exist.push(key.clone());
                    continue;
                }
                issue.gates_required.push(key.clone());
                issue.gates_status.entry(key.clone()).or_insert(GateState {
                    status: GateStatus::Pending,
                    updated_by: None,
                    updated_at: chrono::DateTime::default(),
                });
                added.push(key.clone());
            }
            let events = added
                .iter()
                .map(|key| (1, Event::draft_gate_added(issue.id.clone(), key.clone())))
                .collect();
            let changed = !added.is_empty();
            (
                CapturedIssueMutationOutcome::GatesAdded {
                    added,
                    already_exist,
                },
                events,
                changed,
            )
        }
        CapturedIssueMutation::RemoveGates { gate_keys, .. } => {
            let mut removed = Vec::new();
            let mut not_found = Vec::new();
            for key in gate_keys {
                if !issue.gates_required.contains(key) {
                    not_found.push(key.clone());
                    continue;
                }
                issue.gates_required.retain(|required| required != key);
                issue.gates_status.remove(key);
                removed.push(key.clone());
            }
            let events = removed
                .iter()
                .map(|key| (1, Event::draft_gate_removed(issue.id.clone(), key.clone())))
                .collect();
            let changed = !removed.is_empty();
            (
                CapturedIssueMutationOutcome::GatesRemoved { removed, not_found },
                events,
                changed,
            )
        }
        CapturedIssueMutation::SetManualGateStatus {
            gate_key,
            status,
            by,
            ..
        } => {
            if !matches!(status, GateStatus::Passed | GateStatus::Failed) {
                return Err(anyhow!("manual gate status must be passed or failed"));
            }
            if !issue.gates_required.contains(gate_key) {
                return Err(gate::GateNotRequiredError {
                    issue_id: issue.id.clone(),
                    gate_key: gate_key.clone(),
                }
                .into());
            }
            let gate = registry
                .gates
                .get(gate_key)
                .ok_or_else(|| crate::storage::GateNotFoundError::single(gate_key))?;
            if gate.mode == GateMode::Auto {
                return Err(anyhow!(
                    "Gate '{}' is automated and cannot be manually changed. Use 'jit gate evaluate {} {}' to run the checker.",
                    gate_key,
                    issue.id,
                    gate_key
                ));
            }
            let replacement = GateState {
                status: *status,
                updated_by: by.clone(),
                updated_at: chrono::DateTime::default(),
            };
            issue.gates_status.insert(gate_key.clone(), replacement);
            let event = match status {
                GateStatus::Passed => {
                    Event::draft_gate_passed(issue.id.clone(), gate_key.clone(), by.clone())
                }
                GateStatus::Failed => {
                    Event::draft_gate_failed(issue.id.clone(), gate_key.clone(), by.clone())
                }
                GateStatus::Pending => unreachable!("validated manual gate evidence status"),
            };
            (
                CapturedIssueMutationOutcome::Changed,
                vec![(1, event)],
                true,
            )
        }
    };
    let intents = if changed {
        std::iter::once(MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        })
        .chain(
            events
                .into_iter()
                .map(|(phase, event)| MutationIntent::RecordEvent {
                    phase,
                    event: Box::new(event),
                }),
        )
        .collect()
    } else {
        Vec::new()
    };
    Ok(DerivedCapturedIssueMutation { outcome, intents })
}

/// The unpassed gates of `issue` paired with their current status and registry
/// mode, in the order [`Issue::get_unpassed_gates`] reports them.
///
/// A required gate with no recorded run counts as [`GateStatus::Pending`]. A
/// gate key missing from `registry` (should not normally happen) defaults to
/// [`GateMode::Manual`], matching [`CommandExecutor::pass_gate`]'s own
/// fallback: an unregistered gate never matches the `Auto` branch there
/// either, so it is treated as requiring `--by` just the same. Shared by the
/// transition guard and the gate-diversion path so both describe a
/// gate-blocked completion with the same blockers, and so the remediation
/// hint can name the `--by <attestor>` form for a manual gate (jit:1d59070d
/// REQ-03).
fn unpassed_gate_blockers(
    issue: &Issue,
    registry: &crate::declarations::GateRegistry,
) -> Vec<(String, GateStatus, GateMode)> {
    issue
        .get_unpassed_gates()
        .into_iter()
        .map(|gate_key| {
            let status = issue
                .gates_status
                .get(&gate_key)
                .map(|gate| gate.status)
                .unwrap_or(GateStatus::Pending);
            let mode = registry
                .gates
                .get(&gate_key)
                .map(|gate| gate.mode)
                .unwrap_or(GateMode::Manual);
            (gate_key, status, mode)
        })
        .collect()
}

/// Information about a git commit
#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug, Serialize)]
pub struct StatusSummary {
    pub open: usize, // Backlog count (kept as 'open' for compatibility)
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize, // New: count of rejected issues
    pub blocked: usize,
    pub total: usize,
}

/// Result of listing document references for an issue
#[derive(Debug, Serialize)]
pub struct DocumentListResult {
    pub issue_id: String,
    pub documents: Vec<crate::domain::DocumentReference>,
    pub count: usize,
}

/// Git history for a document
#[derive(Debug, Serialize)]
pub struct DocumentHistory {
    pub path: String,
    pub commits: Vec<CommitInfo>,
}

/// Result of listing assets for a document
#[derive(Debug, Serialize)]
pub struct AssetListResult {
    pub issue_id: String,
    pub document_path: String,
    /// Number of assets in `assets` (list envelope `count`; equals `assets.len()`).
    pub count: usize,
    pub assets: Vec<crate::document::Asset>,
    pub summary: AssetSummary,
    pub warnings: Vec<String>,
}

/// Document content display result
#[derive(Debug, Serialize)]
pub struct DocumentContentResult {
    pub path: String,
    pub label: Option<String>,
    pub commit: String,
    pub doc_type: Option<String>,
    pub content: String,
}

/// Document diff result
#[derive(Debug, Serialize)]
pub struct DocumentDiffResult {
    pub path: String,
    pub from_commit: String,
    pub to_commit: String,
    pub diff: String,
}

/// Result of adding a document reference
#[derive(Debug, Serialize)]
pub struct DocumentAddResult {
    pub issue_id: String,
    pub document: crate::domain::DocumentReference,
    /// `true` when `path` was already linked to the issue and this call
    /// refreshed that entry in place; `false` when it appended a new one.
    pub updated: bool,
}

/// Result of removing a document reference
#[derive(Debug, Serialize)]
pub struct DocumentRemoveResult {
    pub issue_id: String,
    pub path: String,
}

/// Summary of asset counts by category
#[derive(Debug, Serialize)]
pub struct AssetSummary {
    pub total: usize,
    pub per_doc: usize,
    pub shared: usize,
    pub external: usize,
    pub missing: usize,
}

/// Result of exporting a snapshot
#[derive(Debug, Serialize)]
pub struct SnapshotExportResult {
    pub path: String,
    pub issue_count: usize,
    pub document_count: usize,
    pub format: String,
    pub size_bytes: Option<u64>,
}

/// Result of checking document links
#[derive(Debug, Serialize)]
pub struct LinkCheckResult {
    pub valid: bool,
    pub errors: Vec<serde_json::Value>,
    pub warnings: Vec<serde_json::Value>,
    #[serde(skip)]
    pub exit_code: i32,
    #[serde(skip)]
    pub scope: String,
    pub summary: LinkCheckSummary,
}

/// Summary of link check results
#[derive(Debug, Serialize)]
pub struct LinkCheckSummary {
    pub total_documents: usize,
    pub valid: usize,
    pub errors: usize,
    pub warnings: usize,
}

/// Result of adding a dependency
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencyAddResult {
    /// Dependency was added
    Added,
    /// Dependency was skipped because it's transitive (redundant)
    Skipped { reason: String },
    /// Dependency already existed
    AlreadyExists,
}

/// How `jit dep add` treats an edge that would break transitive reduction.
///
/// Cycle detection is a write-time guard (@/inv/dag-acyclic); this policy makes the
/// transitive-reduction property a write-time guard too, closing the asymmetry
/// where a redundant edge was written silently and only rejected at a later
/// `jit validate`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RedundancyPolicy {
    /// Reject the add, naming the offending edge pair (nonzero exit). Default.
    #[default]
    Reject,
    /// Add the edge and drop the now-redundant edge(s) in the same operation,
    /// leaving the graph transitively reduced.
    Reduce,
}

/// Outcome of the unified write-time validation pass.
///
/// Derived from the captured declaration bundle before an issue is persisted. It
/// carries the non-blocking warnings and the `enforce` rules bypassed by a forced
/// write. Bypass events are emitted only by the same captured mutation, so a
/// failed save never leaves a false audit entry.
#[derive(Debug, Clone, Default)]
pub struct WriteValidation {
    /// Non-blocking warnings (legacy validator + local `warn`/non-enforce
    /// findings) to surface to the caller.
    pub warnings: Vec<String>,
    /// Names of `enforce` rules whose blocking findings were overridden by
    /// `--force`. One [`Event::LocalRuleBypassed`] must be logged per entry,
    /// AFTER the write commits.
    pub bypassed_rules: Vec<String>,
}

type PhasedEvents = Vec<(u8, Event)>;

/// Convert repo-relative planned changes into a canonical overlay-override map.
///
/// Each `(repo-relative path, Some(bytes) | None)` becomes a
/// `(VirtualPath, Some|None)` entry (`.jit/...` is `Data`, everything else
/// `Worktree`) suitable for
/// [`apply_overlay`](crate::repository_state::apply_overlay) and the overlaid
/// validation capture. Production init/profile now compose typed deltas directly;
/// this remains as a proposed-overlay test helper for validation/gate-check suites
/// that need a compact `.jit`-prefix path adapter.
#[cfg(test)]
pub(crate) fn overrides_from_repo_changes(
    layout: &crate::repository_state::RepositoryLayout,
    changes: impl IntoIterator<Item = (std::path::PathBuf, Option<Vec<u8>>)>,
) -> Result<std::collections::BTreeMap<crate::repository_state::VirtualPath, Option<Vec<u8>>>> {
    changes
        .into_iter()
        .map(|(path, value)| {
            let repo_rel = path.to_string_lossy();
            let vpath = layout.classify_repository_relative(repo_rel.as_ref())?;
            Ok((vpath, value))
        })
        .collect()
}

/// Read a repo-relative path's bytes from the captured image.
///
/// A `.jit/`-prefixed path is a `Data(...)` entry and every other repo-relative
/// path a `Worktree(...)` entry, matching the materialization producers' mapping.
/// `Ok(None)` when the captured entry is absent or not a file.
fn image_repo_bytes(
    image: &crate::repository_state::RepositoryImage,
    repo_rel: &str,
) -> Result<Option<Vec<u8>>> {
    let vpath = image.layout().classify_repository_relative(repo_rel)?;
    Ok(image.file_bytes(&vpath)?.map(<[u8]>::to_vec))
}

/// Project a finalized delta to the file-overlay validation reads: each written
/// file's bytes and each deleted file's absence, ignoring directory and mode
/// actions. This is the exact proposed repository state — only what the delta
/// writes — used by init/profile to validate their proposed state under one
/// coherent captured image.
pub(crate) fn validation_overlay(
    delta: &crate::repository_state::RepositoryDelta,
) -> std::collections::BTreeMap<crate::repository_state::VirtualPath, Option<Vec<u8>>> {
    use crate::repository_state::RepositoryAction;
    delta
        .actions()
        .iter()
        .filter_map(|action| match action {
            RepositoryAction::WriteFile { path, bytes, .. } => {
                Some((path.clone(), Some(bytes.clone())))
            }
            RepositoryAction::DeleteFile { path, .. } => Some((path.clone(), None)),
            RepositoryAction::CreateDirectory { .. } | RepositoryAction::SetMode { .. } => None,
        })
        .collect()
}

/// Parse the active issue set from the captured index and issue records.
fn captured_active_issues(image: &crate::repository_state::RepositoryImage) -> Result<Vec<Issue>> {
    use crate::repository_state::{RepositoryEntry, VirtualPath};

    let index_path = VirtualPath::INDEX;
    let index_bytes = image
        .file_bytes(&index_path)?
        .ok_or_else(|| anyhow!("captured image has no .jit/index.json"))?;
    let index = crate::storage::json::parse_repository_index(index_bytes)?;
    let mut issues = index
        .all_ids
        .iter()
        .map(|id| {
            let path = VirtualPath::data(format!("issues/{id}.json"))?;
            let bytes = match image.entry(&path)? {
                RepositoryEntry::File { bytes, .. } => bytes,
                RepositoryEntry::Absent => {
                    return Err(crate::storage::IssueNotFoundError::new(id).into())
                }
                _ => return Err(anyhow!("indexed issue {id} is not an ordinary file")),
            };
            let issue: Issue = serde_json::from_slice(bytes)
                .with_context(|| format!("failed to parse captured issue {id}"))?;
            if issue.id != *id {
                return Err(anyhow!(
                    "indexed issue {id} contains mismatched embedded id {}",
                    issue.id
                ));
            }
            Ok(issue)
        })
        .collect::<Result<Vec<_>>>()?;
    issues.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(issues)
}

/// Close gate-run history over one complete root listing and exact canonical
/// result leaves. Direct non-directory occupants are captured as evidence but
/// ignored as storage clutter, matching the typed readers.
fn capture_gate_run_results(
    session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    image: crate::repository_state::RepositoryImage,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    use crate::repository_state::VirtualPath;

    let root = VirtualPath::GATE_RUNS;
    let mut spec = image.capture_spec().clone();
    let image = if image.listing_fingerprints().contains_key(&root) {
        image
    } else {
        spec.discover_paths([root.clone()])?;
        spec.discover_listing(root.clone())?;
        let Some(image) = capture_or_retry(session.capture(spec.clone()))? else {
            return Ok(None);
        };
        image
    };
    let children = image
        .listing_fingerprints()
        .get(&root)
        .ok_or_else(|| anyhow!("captured gate-run root has no complete listing"))?
        .children()
        .keys()
        .map(|run_id| VirtualPath::data(format!("gate-runs/{run_id}")))
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if children.is_empty() {
        return Ok(Some(image));
    }
    spec.discover_paths(children)?;
    let Some(image) = capture_or_retry(session.capture(spec.clone()))? else {
        return Ok(None);
    };
    let results = crate::repository_state::captured_gate_run_result_paths(&image)?
        .ok_or_else(|| anyhow!("captured gate-run root has no complete listing"))?;
    if results.is_empty() {
        return Ok(Some(image));
    }
    spec.discover_paths(results)?;
    capture_or_retry(session.capture(spec))
}

fn resolve_issue_from_capture(issues: &[Issue], requested: &str) -> Result<String> {
    if let Some(issue) = issues.iter().find(|issue| issue.id == requested) {
        return Ok(issue.id.clone());
    }
    let normalized = requested.to_lowercase().replace('-', "");
    if normalized.len() < crate::storage::MIN_ID_PREFIX_LENGTH {
        return Err(crate::storage::InvalidIdPrefixError::new(requested).into());
    }
    let matches = issues
        .iter()
        .filter(|issue| {
            issue
                .id
                .to_lowercase()
                .replace('-', "")
                .starts_with(&normalized)
        })
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(crate::storage::IssueNotFoundError::new(requested).into()),
        [issue] => Ok(issue.id.clone()),
        _ => Err(crate::storage::AmbiguousIdError::issue(
            requested,
            matches
                .iter()
                .map(|issue| format!("{} | {}", issue.short_id(), issue.title)),
        )
        .into()),
    }
}

/// Executes CLI commands with business logic and validation.
///
/// Generic over storage backend to support different implementations
/// (JSON files, SQLite, in-memory, etc.).
pub struct CommandExecutor<S: IssueStore> {
    storage: S,
    pub config_manager: ConfigManager,
    /// Lazily-parsed `.jit/rules.toml`, cached for the lifetime of the
    /// executor so the validation ruleset is read at most once per process
    /// rather than re-parsed on every write. The cached value retains any
    /// load/parse error so callers can surface a misconfigured rules file
    /// instead of silently treating it as "no rules".
    rules: OnceLock<Result<RuleSet, RuleConfigError>>,
    /// Lazily-built EFFECTIVE rule set. When `.jit/rules.toml` is present it
    /// supplies the rules, with the `origin = "default"` family reconciled against
    /// this repo's `config.toml` registry at load
    /// ([`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config));
    /// when absent, the built-in
    /// [`default_ruleset`](crate::repository_state::default_ruleset) is built
    /// IN MEMORY. The former hard-coded checks (a0f0f342 migration) now live as
    /// default rules here. A load/parse error from either source is retained as an
    /// `Err` so a misconfigured repo surfaces the problem rather than silently
    /// disabling enforcement.
    effective_rules: OnceLock<Result<RuleSet, String>>,
    /// Lazily-loaded `.jit/config.toml`, cached so the unified write-time
    /// validation entry point does not re-read and re-parse `config.toml` on
    /// every write. The parsed config seeds the built-in default rules. A
    /// malformed config is retained as an `Err` so it is surfaced rather than
    /// swallowed.
    config: OnceLock<Result<JitConfig, String>>,
    /// Lazily-built label namespace registry, derived from the cached config and
    /// cached alongside it for the same reason.
    namespaces: OnceLock<Result<LabelNamespaces, String>>,
    /// The canonical repository layout every session this executor opens mutates
    /// through.
    ///
    /// This is the PERMANENT construction seam of the executor boundary, not an
    /// interim shim: the executor is constructed with its canonical layout at
    /// startup (from the CLI's Git-optional worktree root and selected data root,
    /// or a fixture layout in tests), and every session-opening command opens
    /// `open_mutation_session` for exactly this layout. The layout always arrives
    /// from the construction boundary and is never inferred from the storage
    /// parent (the layout authority forbids that). Deepening this seam to a fully
    /// retained recovered session — reentrant only for this same canonical layout
    /// (`@/inv` layout coherence) — changes what the executor holds internally, not
    /// these call sites. `None` only when no layout was supplied; a session-opening
    /// command then reports a wiring error rather than proceeding.
    layout: Option<crate::repository_state::RepositoryLayout>,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        let config_manager = ConfigManager::new(storage.root());
        Self {
            storage,
            config_manager,
            rules: OnceLock::new(),
            effective_rules: OnceLock::new(),
            config: OnceLock::new(),
            namespaces: OnceLock::new(),
            layout: None,
        }
    }

    /// Construct this executor over its canonical [`RepositoryLayout`].
    ///
    /// The permanent construction surface for the layout every session-opening
    /// command mutates through (see the [`layout`](Self::layout) field). The CLI
    /// supplies it at startup from the Git-optional worktree root plus the selected
    /// data root; tests supply a fixture layout over a temporary worktree.
    pub fn with_layout(mut self, layout: crate::repository_state::RepositoryLayout) -> Self {
        self.storage.configure_repository_layout(&layout);
        self.layout = Some(layout);
        self
    }

    /// The canonical mutation layout, or a typed error when none was supplied.
    ///
    /// A session-opening command needs an explicit worktree/data-root layout; a
    /// missing one is a construction-wiring error, reported rather than inferred
    /// from the storage parent.
    pub(crate) fn require_layout(&self) -> Result<crate::repository_state::RepositoryLayout> {
        self.repository_layout().cloned()
    }

    /// Explicit repository layout selected when this executor was constructed.
    pub fn repository_layout(&self) -> Result<&crate::repository_state::RepositoryLayout> {
        self.layout
            .as_ref()
            .ok_or_else(|| anyhow!("no repository layout configured for this command"))
    }

    /// Capture, finalize, and publish one typed repository export intent.
    pub(crate) fn publish_repository_export(
        &self,
        layout: &crate::repository_state::RepositoryLayout,
        intent: &crate::repository_state::RepositoryExportIntent,
        budget: crate::repository_state::CaptureBudget,
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::finalize_repository_export;

        with_mutation_session(&self.storage, layout, "repository export", |session| {
            let Some(image) = capture_or_retry(session.capture(intent.capture_spec(budget)?))?
            else {
                return Ok(SessionStep::Retry);
            };
            let plan = finalize_repository_export(&image, intent)?;
            Ok(SessionStep::Apply(plan, ()))
        })
    }

    /// Rebase one closed issue-local operation on a freshly captured record.
    ///
    /// The request determines its complete capture set. The caller cannot pass a
    /// record, capture closure, or arbitrary patch. A conflict reopens a session
    /// and derives again while retaining the operation's identity/time context.
    fn publish_captured_issue_mutation(
        &self,
        request: CapturedIssueMutation,
    ) -> Result<CapturedIssueMutationOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{
            finalize, CaptureBudget, CaptureSpec, MutationContext, RepositoryEntry, VirtualPath,
        };
        use std::collections::BTreeSet;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        with_mutation_session(
            &self.storage,
            &layout,
            "captured issue mutation",
            |session| {
                let mut paths = BTreeSet::from([
                    VirtualPath::data(format!("issues/{}.json", request.issue_id()))?,
                    VirtualPath::EVENTS,
                ]);
                if request.captures_gate_registry() {
                    paths.insert(VirtualPath::GATES);
                }
                let spec = CaptureSpec::phase_one(
                    paths.clone(),
                    CaptureBudget {
                        max_paths: paths.len(),
                        max_listings: 0,
                        max_bytes: 64 * 1024 * 1024,
                        max_depth: 4,
                    },
                )?;
                let Some(image) = capture_or_retry(session.capture(spec))? else {
                    return Ok(SessionStep::Retry);
                };
                let issue_path = VirtualPath::data(format!("issues/{}.json", request.issue_id()))?;
                let issue: Issue = match image.entry(&issue_path)? {
                    RepositoryEntry::File { bytes, .. } => serde_json::from_slice(bytes)
                        .with_context(|| {
                            format!("failed to parse captured issue {}", request.issue_id())
                        })?,
                    RepositoryEntry::Absent => {
                        return Err(
                            crate::storage::IssueNotFoundError::new(request.issue_id()).into()
                        )
                    }
                    _ => return Err(anyhow!("captured issue path is not an ordinary file")),
                };
                if issue.id != request.issue_id() {
                    return Err(anyhow!(
                        "captured issue identity mismatch: requested {}, found {}",
                        request.issue_id(),
                        issue.id
                    ));
                }
                let registry = if request.captures_gate_registry() {
                    let path = VirtualPath::GATES;
                    match image.entry(&path)? {
                        RepositoryEntry::File { bytes, .. } => {
                            crate::declarations::parse_gate_registry(bytes)
                                .context("failed to parse captured gate registry")?
                        }
                        RepositoryEntry::Absent => crate::declarations::GateRegistry::default(),
                        _ => return Err(anyhow!("captured gate registry is not an ordinary file")),
                    }
                } else {
                    crate::declarations::GateRegistry::default()
                };

                let derived = derive_captured_issue_mutation(issue, &registry, &request)?;
                if derived.intents.is_empty() {
                    return Ok(SessionStep::Done(derived.outcome));
                }
                let plan = finalize(&layout, &image, &context, &derived.intents)?;
                Ok(SessionStep::Apply(plan, derived.outcome))
            },
        )
    }

    fn publish_issue_creation(
        &self,
        draft: Issue,
        explicit_type: bool,
        force: bool,
    ) -> Result<(String, WriteValidation)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, MutationIntent, VirtualPath};
        use std::collections::BTreeMap;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        with_mutation_session(&self.storage, &layout, "issue creation", |session| {
            let issue_id = context.identifier_at(0);
            let Some(image) = self.capture_proposed_base(
                session,
                &BTreeMap::new(),
                &[
                    VirtualPath::ISSUES,
                    VirtualPath::data(format!("issues/{issue_id}.json"))?,
                ],
                None,
            )?
            else {
                return Ok(SessionStep::Retry);
            };
            let declarations = crate::repository_state::declarations_from_image(&image)?;
            let mut final_draft = draft.clone();
            if label_utils::type_label_value(&final_draft.labels).is_none() {
                if let Some(default_type) = declarations
                    .config()
                    .validation
                    .as_ref()
                    .and_then(|validation| validation.default_type.as_deref())
                {
                    final_draft
                        .labels
                        .push(label_utils::type_label(default_type));
                }
            }
            if explicit_type {
                let repo_format = declarations
                    .config()
                    .validation
                    .as_ref()
                    .map(crate::config::ValidationConfig::content_format)
                    .transpose()?
                    .unwrap_or(crate::domain::ContentFormat::Markdown);
                let evaluation = crate::validation::evaluate_local(
                    &final_draft,
                    declarations.rules(),
                    repo_format,
                )?;
                if let Some(finding) = evaluation
                    .findings()
                    .into_iter()
                    .find(|finding| finding.rule == "type-hierarchy-known")
                {
                    return Err(
                        crate::errors::ValidationFailedError::new(finding.message.clone()).into(),
                    );
                }
            }
            let validation =
                derive_write_validation(&final_draft, &declarations, declarations.config(), force)?;
            let intents = std::iter::once(MutationIntent::CreateIssue {
                draft: Box::new(final_draft),
            })
            .chain(
                validation
                    .bypassed_rules
                    .iter()
                    .map(|rule| MutationIntent::RecordEvent {
                        phase: 9,
                        event: Box::new(Event::draft_local_rule_bypassed(
                            issue_id.clone(),
                            rule.clone(),
                        )),
                    }),
            )
            .collect::<Vec<_>>();
            let plan = finalize(&layout, &image, &context, &intents)?;
            Ok(SessionStep::Apply(plan, (issue_id, validation)))
        })
    }

    /// Get reference to the storage backend
    pub fn storage(&self) -> &S {
        &self.storage
    }

    /// Return the parsed validation ruleset, loading `.jit/rules.toml` on first
    /// access and caching the result for subsequent calls.
    ///
    /// A MISSING `.jit/rules.toml` is not an error: it yields `Ok(`an empty
    /// [`RuleSet`]`)`. A genuine parse or load failure (malformed TOML, an
    /// invalid `assert` table, an unsafe schema reference, etc.) is returned as
    /// `Err` rather than being swallowed, so a misconfigured repository cannot
    /// silently disable all rule enforcement. The load is performed at most once
    /// and the outcome (success or failure) is cached.
    pub fn rules(&self) -> Result<&RuleSet, &RuleConfigError> {
        self.rules
            .get_or_init(|| {
                let config =
                    self.config_manager
                        .load()
                        .map_err(|error| RuleConfigError::Configuration {
                            message: error.to_string(),
                        })?;
                crate::storage::ruleset_store::load_ruleset(self.storage.root(), &config)
            })
            .as_ref()
    }

    /// Return the EFFECTIVE rule set, with `.jit/rules.toml` as the operative
    /// ruleset and its `origin = "default"` rules reconciled against `config.toml`
    /// at load (DR §8.2/§8.4).
    ///
    /// Semantics depend on whether the file EXISTS, not whether it is empty:
    ///
    /// - **File present (even with zero rules):** the parsed file supplies the
    ///   operative rule set — which rules exist and, for the built-in rules `jit
    ///   init` scaffolds (marked `origin = "default"`), their editable policy
    ///   fields (severity, enforce, selector). The default family is reconciled
    ///   against the declared `[namespaces]` / `[type_hierarchy]` registry IN
    ///   MEMORY at load
    ///   ([`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config)):
    ///   assertions are re-derived, a rule is added for a newly-declared namespace
    ///   and dropped for a removed one, so a hand edit of the registry cannot
    ///   desync validation against a stale `schemas/default-*.json` projection.
    ///   Custom rules (any other `origin`)
    ///   are used verbatim, reading their own declared schema files. An
    ///   intentionally-emptied file yields an empty set.
    /// - **File ABSENT (pre-init repo or deleted file):** build the FIXED
    ///   [`default_ruleset`](crate::repository_state::default_ruleset) from the
    ///   repo's namespace registry IN MEMORY (MF4). This is read-only — NO disk
    ///   write, NO warning, NO error — so gates, the server, read-only checkouts,
    ///   and multiple worktrees stay safe. Only `jit init` materializes the file
    ///   to disk (under the write lock).
    ///
    /// The result is built at most once and cached. A malformed `config.toml` or
    /// `.jit/rules.toml` is surfaced as an `Err` rather than silently dropping
    /// enforcement.
    pub fn effective_rules(&self) -> Result<&RuleSet> {
        self.effective_rules
            .get_or_init(|| {
                let rules_path = self.storage.root().join("rules.toml");
                if rules_path.exists() {
                    // File present (even empty) supplies the operative rule set.
                    // Its default-origin rules are a config PROJECTION: reconcile
                    // the default family against the declared registry IN MEMORY at
                    // load — re-derive assertions, add a rule for a newly-declared
                    // namespace, drop one whose namespace is gone — so the baked
                    // `schemas/default-*.json` files are never the authority. Custom
                    // rules and the default rules' editable policy fields
                    // (severity/enforce/selector) are taken from the file unchanged.
                    let user = self.rules().map_err(|e| e.to_string())?.clone();
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    Ok(
                        crate::repository_state::reconcile_default_rules_with_config(
                            user, namespaces,
                        ),
                    )
                } else {
                    // File absent: build the fixed defaults IN MEMORY (no write,
                    // no warning) from the repo's namespace registry. Materialized
                    // to disk only by `jit init`.
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    Ok(crate::repository_state::default_ruleset(namespaces))
                }
            })
            .as_ref()
            .map_err(|err| anyhow!("{err}"))
    }

    /// Return the cached parsed `.jit/config.toml`, loading it on first access.
    ///
    /// The config is read at most once per executor (cached in a `OnceLock`) so
    /// the unified write-validation path does not re-parse `config.toml` on every
    /// write. A malformed config is surfaced as an error rather than swallowed.
    fn cached_config(&self) -> Result<&JitConfig> {
        self.config
            .get_or_init(|| self.config_manager.load().map_err(|err| err.to_string()))
            .as_ref()
            .map_err(|err| {
                crate::errors::InvalidArgumentError::new(format!("invalid .jit/config.toml: {err}"))
                    .into()
            })
    }

    /// Return the cached label namespace registry, building it on first access.
    ///
    /// Cached alongside [`cached_config`](Self::cached_config) so the registry is
    /// derived at most once per executor.
    fn cached_namespaces(&self) -> Result<&LabelNamespaces> {
        self.namespaces
            .get_or_init(|| {
                // Derive namespaces from the already-cached config so a single
                // write command reads `config.toml` at most once (the prior
                // `get_namespaces()` call re-loaded it from disk).
                match self.cached_config() {
                    Ok(config) => Ok(self.config_manager.namespaces_from_config(config)),
                    Err(err) => Err(format!("{err}")),
                }
            })
            .as_ref()
            .map_err(|err| {
                crate::errors::InvalidArgumentError::new(format!(
                    "invalid namespace configuration: {err}"
                ))
                .into()
            })
    }

    /// Resolve the repository's configured default content format.
    fn repo_content_format(&self) -> Result<crate::domain::ContentFormat> {
        self.cached_config()?
            .validation
            .as_ref()
            .map(crate::config::ValidationConfig::content_format)
            .transpose()
            .map(|format| format.unwrap_or(crate::domain::ContentFormat::Markdown))
    }

    fn publish_captured_field_update(
        &self,
        request: CapturedFieldUpdate,
    ) -> Result<CapturedFieldUpdateOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext};
        use std::collections::BTreeMap;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        with_mutation_attempts("issue update", || {
            let (expected_target, expected_lease_mode) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) =
                    self.capture_proposed_base(preflight.as_mut(), &BTreeMap::new(), &[], None)?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let issues = captured_active_issues(&image)?;
                let target = resolve_issue_from_capture(&issues, &request.issue_id)?;
                let declarations = crate::repository_state::declarations_from_image(&image)?;
                (
                    target,
                    self.config_manager
                        .enforcement_mode_from_config(declarations.config())?,
                )
            };
            let claims_guard = request
                .enforce_lease
                .then(|| claims_mutation_guard(&layout))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) =
                self.capture_proposed_base(session.as_mut(), &BTreeMap::new(), &[], None)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let issues = captured_active_issues(&image)?;
            if resolve_issue_from_capture(&issues, &request.issue_id)? != expected_target {
                return Ok(AttemptOutcome::Retry);
            }
            let issue = issues
                .iter()
                .find(|issue| issue.id == expected_target)
                .cloned()
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(&expected_target))?;
            let declarations = crate::repository_state::declarations_from_image(&image)?;
            let config = declarations.config();
            if request.enforce_lease
                && self
                    .config_manager
                    .enforcement_mode_from_config(declarations.config())?
                    != expected_lease_mode
            {
                return Ok(AttemptOutcome::Retry);
            }
            let lease_warnings = if request.enforce_lease {
                captured_lease_warnings(
                    expected_lease_mode,
                    std::slice::from_ref(&expected_target),
                    &issues,
                    claims_guard.as_ref(),
                )?
            } else {
                Vec::new()
            };
            let plan_content = validate::plan_content_from_image(&image, &issues)?;
            let mut effective_request = request.clone();
            effective_request.issue_id = expected_target;
            let mut derived = derive_field_update(
                issue,
                &effective_request,
                CapturedTransitionEvidence {
                    issues: &issues,
                    declarations: &declarations,
                    config,
                    plan_content: &plan_content,
                    context: &context,
                },
            )?;
            let mut lease_warnings = lease_warnings;
            lease_warnings.append(&mut derived.warnings);
            derived.warnings = lease_warnings;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.with_warnings(derived.warnings).into()),
                    None => Ok(AttemptOutcome::Done(CapturedFieldUpdateOutcome {
                        changed: derived.changed,
                        warnings: derived.warnings,
                    })),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            match derived.error_after_apply {
                Some(error) => Err(error.with_warnings(derived.warnings).into()),
                None => Ok(AttemptOutcome::Done(CapturedFieldUpdateOutcome {
                    changed: derived.changed,
                    warnings: derived.warnings,
                })),
            }
        })
    }

    fn publish_captured_bulk_update(
        &self,
        issue_id: String,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<(bool, Vec<String>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let request = CapturedFieldUpdate::bulk(issue_id, operations, force)?;
        let outcome = self.publish_captured_field_update(request)?;
        Ok((outcome.changed, outcome.warnings))
    }

    fn publish_captured_state_transition(
        &self,
        issue_id: &str,
        target: State,
        force: bool,
        divert_unpassed_gates: bool,
        enforce_lease: bool,
    ) -> Result<CapturedLifecycleOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_lifecycle_mutation(CapturedLifecycleMutation::State {
            issue_id: issue_id.to_string(),
            target,
            force,
            divert_unpassed_gates,
            enforce_lease,
        })
    }

    fn publish_captured_claim(
        &self,
        issue_id: String,
        assignee: crate::domain::Assignee,
    ) -> Result<Vec<crate::storage::StorageWarning>>
    where
        S: crate::storage::RepositoryStateStore,
    {
        Ok(self
            .publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Claim {
                issue_id,
                assignee,
            })?
            .storage_warnings)
    }

    fn publish_captured_release(&self, issue_id: String, reason: String) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Release {
            issue_id,
            reason,
        })?;
        Ok(())
    }

    fn publish_captured_auto_transition(&self, issue_id: String, target: State) -> Result<bool>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let request = match target {
            State::Ready => CapturedLifecycleMutation::AutoReady { issue_id },
            State::Backlog => CapturedLifecycleMutation::AutoBacklog { issue_id },
            State::Done => CapturedLifecycleMutation::AutoDone { issue_id },
            _ => {
                return Err(anyhow!(
                    "unsupported automatic transition target '{}'",
                    target.as_str()
                ))
            }
        };
        Ok(self.publish_captured_lifecycle_mutation(request)?.changed)
    }

    fn publish_captured_lifecycle_mutation(
        &self,
        request: CapturedLifecycleMutation,
    ) -> Result<CapturedLifecycleOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, MutationContext, MutationIntent};
        use std::collections::BTreeMap;

        let layout = self.require_layout()?;
        let context = MutationContext::production();
        let mut cached_precheck = None::<CachedPrecheckExecution>;
        with_mutation_attempts("lifecycle mutation", || {
            let (
                expected_target,
                expected_precheck_required,
                expected_precheck,
                precheck_image,
                precheck_issue,
                precheck_registry,
                expected_lease_mode,
                enforce_lease,
            ) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                let Some(image) = self.capture_proposed_base(
                    preflight.as_mut(),
                    &BTreeMap::new(),
                    &[],
                    Some(request.issue_id()),
                )?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let issues = captured_active_issues(&image)?;
                let target_id = resolve_issue_from_capture(&issues, request.issue_id())?;
                let issue = issues
                    .iter()
                    .find(|issue| issue.id == target_id)
                    .ok_or_else(|| crate::storage::IssueNotFoundError::new(&target_id))?;
                let should_precheck = lifecycle_requires_prechecks(&request, issue);
                let enforce_lease = matches!(
                    &request,
                    CapturedLifecycleMutation::State {
                        enforce_lease: true,
                        ..
                    }
                );
                let declarations = crate::repository_state::declarations_from_image(&image)?;
                let lease_mode = self
                    .config_manager
                    .enforcement_mode_from_config(declarations.config())?;
                let registry = declarations.gates;
                let plan = should_precheck
                    .then(|| captured_precheck_plan(&image, issue, &registry))
                    .transpose()?;
                (
                    target_id,
                    should_precheck,
                    plan,
                    image,
                    issue.clone(),
                    registry,
                    lease_mode,
                    enforce_lease,
                )
            };
            let coordinate_claims =
                enforce_lease || matches!(request, CapturedLifecycleMutation::Claim { .. });
            if coordinate_claims {
                let preflight_issues = captured_active_issues(&precheck_image)?;
                let preliminary_guard = claims_mutation_guard(&layout)?;
                if enforce_lease {
                    captured_lease_warnings(
                        expected_lease_mode,
                        std::slice::from_ref(&expected_target),
                        &preflight_issues,
                        preliminary_guard.as_ref(),
                    )?;
                }
                if let CapturedLifecycleMutation::Claim { assignee, .. } = &request {
                    ensure_claim_available(
                        preliminary_guard.as_ref(),
                        &expected_target,
                        assignee,
                        &preflight_issues,
                    )?;
                }
            }
            if let Some(plan) = &expected_precheck {
                let reuse = cached_precheck.as_ref().is_some_and(|cached| {
                    cached.target_id == expected_target && cached.evidence == plan.evidence
                });
                if !reuse {
                    cached_precheck = Some(CachedPrecheckExecution {
                        target_id: expected_target.clone(),
                        evidence: plan.evidence.clone(),
                        execution: self.execute_captured_prechecks(
                            &precheck_image,
                            &precheck_issue,
                            &precheck_registry,
                            &plan.prompts,
                        )?,
                    });
                }
            }
            let precheck_runs = cached_precheck
                .as_ref()
                .filter(|_| expected_precheck.is_some())
                .map(|cached| cached.execution.runs.clone())
                .unwrap_or_default();
            let precheck_has_error = cached_precheck
                .as_ref()
                .filter(|_| expected_precheck.is_some())
                .is_some_and(|cached| cached.execution.error.is_some());
            let mut precheck_run_paths = vec![crate::repository_state::VirtualPath::GATE_RUNS];
            precheck_run_paths.extend(
                (0..precheck_runs.len())
                    .map(|index| {
                        let id = context.identifier_at(index as u64);
                        let result = crate::repository_state::gate_run_result_relative_path(&id)?;
                        let directory = result.as_path().parent().ok_or_else(|| {
                            anyhow!("canonical gate-run result path has no parent")
                        })?;
                        Ok([
                            crate::repository_state::VirtualPath::data(directory)?,
                            crate::repository_state::VirtualPath::data(result.as_path())?,
                        ])
                    })
                    .collect::<Result<Vec<_>>>()?
                    .into_iter()
                    .flatten(),
            );

            let claims_guard = coordinate_claims
                .then(|| claims_mutation_guard(&layout))
                .transpose()?
                .flatten();

            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(image) = self.capture_proposed_base(
                session.as_mut(),
                &BTreeMap::new(),
                &precheck_run_paths,
                Some(request.issue_id()),
            )?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let issues = captured_active_issues(&image)?;
            if resolve_issue_from_capture(&issues, request.issue_id())? != expected_target {
                return Ok(AttemptOutcome::Retry);
            }
            let mut issue = issues
                .iter()
                .find(|issue| issue.id == expected_target)
                .cloned()
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(&expected_target))?;
            let declarations = crate::repository_state::declarations_from_image(&image)?;
            if enforce_lease
                && self
                    .config_manager
                    .enforcement_mode_from_config(declarations.config())?
                    != expected_lease_mode
            {
                return Ok(AttemptOutcome::Retry);
            }
            let current_precheck_required = lifecycle_requires_prechecks(&request, &issue);
            if current_precheck_required != expected_precheck_required {
                return Ok(AttemptOutcome::Retry);
            }
            if let Some(expected) = &expected_precheck {
                let evidence_matches = if expected.evidence.validation_view.is_some() {
                    expected.evidence.matches(&image)?
                } else {
                    captured_precheck_plan(&image, &issue, &declarations.gates)?.evidence
                        == expected.evidence
                };
                if !evidence_matches {
                    return Ok(AttemptOutcome::Retry);
                }
            }
            let mut external_warnings = if enforce_lease {
                captured_lease_warnings(
                    expected_lease_mode,
                    std::slice::from_ref(&expected_target),
                    &issues,
                    claims_guard.as_ref(),
                )?
            } else {
                Vec::new()
            };
            let storage_warnings = claims_guard
                .as_ref()
                .map_or_else(Vec::new, |guard| guard.warnings());
            if let CapturedLifecycleMutation::Claim { assignee, .. } = &request {
                ensure_claim_available(claims_guard.as_ref(), &expected_target, assignee, &issues)?;
            }
            let mut precheck_events = Vec::new();
            if expected_precheck.is_some() {
                for result in &precheck_runs {
                    let (state, by) = gate_check::gate_state_from_run(result)?;
                    let status = state.status;
                    issue.gates_status.insert(result.gate_key.clone(), state);
                    precheck_events.push((
                        1,
                        if status == GateStatus::Passed {
                            Event::draft_gate_passed(issue.id.clone(), result.gate_key.clone(), by)
                        } else {
                            Event::draft_gate_failed(issue.id.clone(), result.gate_key.clone(), by)
                        },
                    ));
                }
            }
            if precheck_has_error {
                if precheck_runs.is_empty() {
                    let error = cached_precheck
                        .take()
                        .and_then(|cached| cached.execution.error)
                        .ok_or_else(|| anyhow!("precheck execution lost its recorded error"))?;
                    return Err(error);
                }
                let intents = std::iter::once(MutationIntent::UpdateIssue {
                    issue: Box::new(issue),
                })
                .chain(
                    precheck_runs
                        .into_iter()
                        .map(|draft| MutationIntent::RecordGateRun {
                            draft: Box::new(draft),
                        }),
                )
                .chain(precheck_events.into_iter().map(|(phase, event)| {
                    MutationIntent::RecordEvent {
                        phase,
                        event: Box::new(event),
                    }
                }))
                .collect::<Vec<_>>();
                let plan = finalize(&layout, &image, &context, &intents)?;
                if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                    return Ok(AttemptOutcome::Retry);
                }
                let error = cached_precheck
                    .take()
                    .and_then(|cached| cached.execution.error)
                    .ok_or_else(|| anyhow!("precheck execution lost its recorded error"))?;
                return Err(error);
            }
            let prechecks_changed = !precheck_runs.is_empty();
            let (target, force, divert) = match &request {
                CapturedLifecycleMutation::State {
                    target,
                    force,
                    divert_unpassed_gates,
                    ..
                } => (*target, *force, *divert_unpassed_gates),
                CapturedLifecycleMutation::Claim { assignee, .. } => {
                    if let Some(existing) = &issue.assignee {
                        if existing != assignee {
                            return Err(anyhow!(
                                "Issue {} is already assigned to {existing}; refusing to claim as \
                                 {assignee} (re-claiming as {existing} succeeds and promotes it to in_progress)",
                                issue.id,
                            ));
                        }
                        if issue.state == State::InProgress {
                            return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                                target_id: expected_target.clone(),
                                changed: false,
                                warnings: Vec::new(),
                                storage_warnings,
                            }));
                        }
                    }
                    if issue.state == State::Backlog {
                        let blockers = issue::blocking_dependencies(
                            &issue,
                            &crate::domain::queries::build_issue_map(&issues),
                        );
                        if !blockers.is_empty() {
                            return Err(crate::errors::TransitionBlockedError::dependencies(
                                issue.id.clone(),
                                State::InProgress,
                                issue.state,
                                blockers,
                            )
                            .into());
                        }
                    }
                    (
                        if issue.state == State::Ready {
                            State::InProgress
                        } else {
                            issue.state
                        },
                        false,
                        false,
                    )
                }
                CapturedLifecycleMutation::Release { .. } => (
                    if issue.state == State::InProgress {
                        State::Ready
                    } else {
                        issue.state
                    },
                    false,
                    false,
                ),
                CapturedLifecycleMutation::AutoReady { .. } => {
                    let resolved = crate::domain::queries::build_issue_map(&issues);
                    if !issue.should_auto_transition_to_ready(&resolved) {
                        return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                            target_id: expected_target.clone(),
                            changed: false,
                            warnings: Vec::new(),
                            storage_warnings,
                        }));
                    }
                    (State::Ready, false, false)
                }
                CapturedLifecycleMutation::AutoBacklog { .. } => {
                    let resolved = crate::domain::queries::build_issue_map(&issues);
                    if issue.derive_readiness_correction(&resolved)
                        != Some(ReadinessCorrection::Demote)
                    {
                        return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                            target_id: expected_target.clone(),
                            changed: false,
                            warnings: Vec::new(),
                            storage_warnings,
                        }));
                    }
                    (State::Backlog, false, false)
                }
                CapturedLifecycleMutation::AutoDone { .. } => {
                    if !issue.should_auto_transition_to_done() {
                        return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                            target_id: expected_target.clone(),
                            changed: false,
                            warnings: Vec::new(),
                            storage_warnings,
                        }));
                    }
                    (State::Done, false, false)
                }
            };
            let declarations = crate::repository_state::declarations_from_image(&image)?;
            let config = declarations.config();
            let mut after_apply_error = None;
            let effective_target = if divert && target == State::Done && issue.has_unpassed_gates()
            {
                let blockers = issue::blocking_dependencies(
                    &issue,
                    &crate::domain::queries::build_issue_map(&issues),
                );
                if !blockers.is_empty() {
                    return Err(crate::errors::TransitionBlockedError::dependencies(
                        issue.id.clone(),
                        target,
                        issue.state,
                        blockers,
                    )
                    .into());
                }
                after_apply_error = Some(crate::errors::TransitionBlockedError::gates(
                    issue.id.clone(),
                    State::Done,
                    State::Gated,
                    unpassed_gate_blockers(&issue, &declarations.gates),
                ));
                State::Gated
            } else {
                target
            };
            let plan_content = validate::plan_content_from_image(&image, &issues)?;
            let (mut warnings, mut events, mut update, blocked) = if issue.state == effective_target
            {
                (Vec::new(), Vec::new(), None, None)
            } else {
                match derive_state_transition(
                    issue.clone(),
                    effective_target,
                    force,
                    CapturedTransitionEvidence {
                        issues: &issues,
                        declarations: &declarations,
                        config,
                        plan_content: &plan_content,
                        context: &context,
                    },
                )? {
                    DerivedStateTransition::Applied {
                        issue,
                        warnings,
                        events,
                        changed,
                    } => (warnings, events, changed.then_some(*issue), None),
                    DerivedStateTransition::GraphBlocked { error, events } => {
                        (Vec::new(), events, None, Some(error))
                    }
                }
            };
            events.extend(precheck_events);
            if prechecks_changed && update.is_none() {
                update = Some(issue.clone());
            }
            warnings.append(&mut external_warnings);
            if blocked.is_none() {
                let record = update.as_mut().unwrap_or(&mut issue);
                match &request {
                    CapturedLifecycleMutation::Claim { assignee, .. } => {
                        record.assignee = Some(assignee.clone());
                        events.push((
                            4,
                            Event::draft_issue_claimed(record.id.clone(), assignee.clone()),
                        ));
                        if update.is_none() {
                            update = Some(issue);
                        }
                    }
                    CapturedLifecycleMutation::Release { reason, .. } => {
                        if let Some(assignee) = record.assignee.take() {
                            events.push((
                                4,
                                Event::draft_issue_released(
                                    record.id.clone(),
                                    assignee,
                                    reason.clone(),
                                ),
                            ));
                            if update.is_none() {
                                update = Some(issue);
                            }
                        }
                    }
                    _ => {}
                }
            }
            let changed = update.is_some();
            let intents = update
                .into_iter()
                .map(|issue| MutationIntent::UpdateIssue {
                    issue: Box::new(issue),
                })
                .chain(
                    precheck_runs
                        .into_iter()
                        .map(|draft| MutationIntent::RecordGateRun {
                            draft: Box::new(draft),
                        }),
                )
                .chain(
                    events
                        .into_iter()
                        .map(|(phase, event)| MutationIntent::RecordEvent {
                            phase,
                            event: Box::new(event),
                        }),
                )
                .collect::<Vec<_>>();
            if intents.is_empty() {
                return match after_apply_error {
                    Some(error) => Err(error.into()),
                    None => Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                        target_id: expected_target.clone(),
                        changed,
                        warnings,
                        storage_warnings,
                    })),
                };
            }
            let plan = finalize(&layout, &image, &context, &intents)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            if let Some(error) = blocked.or(after_apply_error) {
                return Err(error.with_warnings(std::mem::take(&mut warnings)).into());
            }
            Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                target_id: expected_target.clone(),
                changed,
                warnings,
                storage_warnings,
            }))
        })
    }

    /// The dependency and gate guards a transition must clear, evaluated against
    /// `issue` in its CURRENT state.
    ///
    /// Used by the read-only bulk preview path.
    /// entering [`State::Ready`] or [`State::Done`] requires every dependency to
    /// have reached a terminal state, and entering [`State::Done`] additionally
    /// requires every required gate to have passed (`@/inv/gate-semantics`).
    /// Dependencies are checked before gates, so an issue that is both
    /// dependency-blocked and gate-blocked reports its dependencies.
    ///
    /// Every other target (including [`State::Rejected`], which abandons an issue)
    /// is unguarded. `force` has no bearing here: it overrides validation rules,
    /// never the graph's own semantics.
    fn transition_blockers(&self, issue: &Issue, target: State) -> Result<()> {
        if !matches!(target, State::Ready | State::Done) {
            return Ok(());
        }

        let issues = self.storage.list_issues()?;
        let resolved = crate::domain::queries::build_issue_map(&issues);
        let blockers = issue::blocking_dependencies(issue, &resolved);
        if !blockers.is_empty() {
            return Err(crate::errors::TransitionBlockedError::dependencies(
                issue.id.clone(),
                target,
                issue.state,
                blockers,
            )
            .into());
        }

        if target == State::Done && issue.has_unpassed_gates() {
            let registry = self.storage.load_gate_registry()?;
            return Err(crate::errors::TransitionBlockedError::gates(
                issue.id.clone(),
                State::Done,
                issue.state,
                unpassed_gate_blockers(issue, &registry),
            )
            .into());
        }

        Ok(())
    }

    /// Create or refresh machine-local worktree identity after repository
    /// scaffold publication.
    ///
    /// Called after the fresh repository transaction so optional Git host state
    /// remains outside repository publication.
    pub fn initialize_worktree_identity(
        &self,
    ) -> Result<(
        Option<WorktreeIdentity>,
        Vec<crate::storage::StorageWarning>,
    )> {
        use crate::storage::worktree_identity::load_or_create_worktree_identity_with_warnings;
        use crate::storage::worktree_paths::WorktreePaths;

        // Check if we're actually in a git repository
        let in_git_repo = std::process::Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !in_git_repo {
            return Ok((None, Vec::new()));
        }

        // Create worktree identity
        let paths = WorktreePaths::detect()?;

        // Get git branch name
        let branch = std::process::Command::new("git")
            .args(["branch", "--show-current"])
            .current_dir(&paths.worktree_root)
            .output()
            .ok()
            .and_then(|output| {
                if output.status.success() {
                    String::from_utf8(output.stdout).ok()
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "main".to_string())
            .trim()
            .to_string();

        // Create or update worktree identity
        // This handles copied files in git worktrees automatically
        let (identity, warnings) = load_or_create_worktree_identity_with_warnings(
            &paths.local_jit,
            &paths.worktree_root,
            &branch,
        )?;

        Ok((Some(identity), warnings))
    }

    /// Check if an active lease exists for the given issue by the current agent.
    ///
    /// Returns true if the current agent has an active lease (not expired or stale).
    /// Returns false if no lease exists, lease is stale, or belongs to another agent.
    ///
    /// In tests: If JIT_AGENT_ID is not set, any valid lease counts (single-user mode).
    fn check_active_lease(&self, issue_id: &str) -> Result<bool> {
        use crate::agent_config::resolve_agent_id;
        use crate::storage::claim_coordinator::ClaimsIndex;
        use crate::storage::worktree_paths::WorktreePaths;

        // Get worktree paths to access shared control plane
        let paths = match WorktreePaths::detect() {
            Ok(p) => p,
            Err(_) => {
                // Not in a git repository - no claims possible
                return Ok(false);
            }
        };

        // Load the active-lease index through storage (an absent index yields an
        // empty one, i.e. no active leases).
        let claims_index = ClaimsIndex::load(&paths)?;

        // Resolve current agent identity (or None for single-user mode)
        let current_agent = resolve_agent_id(None).ok();

        // Check if active lease exists for this issue
        let full_id = self.storage.resolve_issue_id(issue_id)?;
        let now = chrono::Utc::now();

        let has_active_lease = claims_index.leases.iter().any(|lease| {
            // Must match issue ID
            if lease.issue_id != full_id {
                return false;
            }

            // Must not be expired
            if let Some(expires) = lease.expires_at {
                if expires <= now {
                    return false;
                }
            }

            // Must not be stale
            if claims_index.is_stale(lease) {
                return false;
            }

            // Agent verification:
            // - If current_agent is Some, lease must belong to this agent
            // - If current_agent is None (single-user mode), any valid lease counts
            match &current_agent {
                Some(agent_id) => lease.agent_id == *agent_id,
                None => true, // Single-user mode: any valid lease
            }
        });

        Ok(has_active_lease)
    }

    /// Require an active lease for the given issue, respecting enforcement mode.
    ///
    /// Checks the configured enforcement mode and either blocks, warns, or bypasses
    /// the lease requirement. Used before structural operations that modify issues.
    ///
    /// # Errors
    ///
    /// Returns an error in `strict` mode when no active lease exists.
    /// In `warn` mode, returns Ok with a warning message.
    /// In `off` mode, always returns Ok with None.
    ///
    /// Returns: Result<Option<String>> where Some(warning) should be printed by the CLI layer.
    pub fn require_active_lease(&self, issue_id: &str) -> Result<Option<String>> {
        use crate::config::EnforcementMode;

        // This legacy preflight is rechecked from captured config by mutation
        // coordinators that enforce leases during publication.
        let mode = self
            .config_manager
            .enforcement_mode_from_config(self.cached_config()?)?;

        match mode {
            EnforcementMode::Off => Ok(None),
            EnforcementMode::Warn | EnforcementMode::Strict => {
                let has_lease = self.check_active_lease(issue_id)?;

                if !has_lease {
                    let msg = format!(
                        "No active lease for issue {}.\nAcquire lease with: jit claim acquire {}",
                        issue_id, issue_id
                    );

                    match mode {
                        EnforcementMode::Warn => Ok(Some(msg)),
                        EnforcementMode::Strict => {
                            anyhow::bail!("{}", msg)
                        }
                        _ => unreachable!(),
                    }
                } else {
                    Ok(None)
                }
            }
        }
    }
}

// Helper functions for parsing command-line arguments
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_repository_layout_normalizes_only_leading_current_directory_components() {
        use crate::repository_state::{RepositoryLayout, RepositoryRootEvidence, VirtualPath};
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/external/jit-data", "data", true),
        )
        .unwrap();

        assert_eq!(
            layout
                .classify_repository_relative("./contrib/gates/code-review-prompt.md")
                .unwrap(),
            VirtualPath::worktree("contrib/gates/code-review-prompt.md").unwrap()
        );
        assert_eq!(
            layout
                .classify_repository_relative("././.jit/gates.toml")
                .unwrap(),
            VirtualPath::data("gates.toml").unwrap()
        );

        let control = format!("./scripts/{}prompt", '\u{0007}');
        for path in [
            "",
            "./",
            "././",
            ".jit/",
            "./.jit/",
            "../outside",
            "./../outside",
            "/absolute",
            "./scripts/./prompt",
            "./scripts//prompt",
            "./scripts\\prompt",
            "./C:/prompt",
            control.as_str(),
        ] {
            assert!(
                layout.classify_repository_relative(path).is_err(),
                "{path:?}"
            );
        }
        assert!(VirtualPath::worktree("./contrib/gates/code-review-prompt.md").is_err());
    }

    #[test]
    fn test_publish_repository_export_retries_apply_conflict() {
        use crate::repository_state::{CaptureBudget, RepositoryExportIntent, VirtualPath};
        use crate::storage::{InMemoryStorage, IssueStore};

        let storage = InMemoryStorage::new();
        storage.add_data_file("issues/existing.json", "{}");
        let layout = storage.repository_layout();
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout.clone());
        storage.inject_repository_state_apply_conflicts(1);
        let intent = RepositoryExportIntent::new_absent_file(
            VirtualPath::data("issues/snapshot.tar").unwrap(),
            b"snapshot".to_vec(),
        );

        executor
            .publish_repository_export(
                &layout,
                &intent,
                CaptureBudget {
                    max_paths: 4,
                    max_listings: 1,
                    max_bytes: 1024,
                    max_depth: 8,
                },
            )
            .unwrap();
        assert_eq!(
            storage
                .read_repo_file(".jit/issues/snapshot.tar")
                .unwrap()
                .as_deref(),
            Some("snapshot")
        );
    }

    fn precheck_evidence_fixture() -> (
        crate::storage::InMemoryStorage,
        CommandExecutor<crate::storage::InMemoryStorage>,
        String,
    ) {
        use crate::declarations::{GateChecker, GateDefinition, GateRegistry, GateStage};
        use std::collections::HashMap;

        let storage = crate::storage::InMemoryStorage::new();
        storage.add_data_file("config.toml", "");
        storage.add_worktree_file("start-prompt.md", "captured prompt");
        let mut registry = GateRegistry::default();
        registry.gates.insert(
            "auto-start".to_string(),
            GateDefinition {
                version: 1,
                key: "auto-start".to_string(),
                title: "Auto start".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Auto,
                checker: Some(GateChecker::Exec {
                    command: "true".to_string(),
                    timeout_seconds: 10,
                    working_dir: None,
                    env: HashMap::new(),
                    pass_context: true,
                    prompt: None,
                    prompt_file: Some("start-prompt.md".to_string()),
                }),
                priority: 1,
                reserved: HashMap::new(),
                auto: true,
                example_integration: None,
                inputs: None,
            },
        );
        registry.gates.insert(
            "manual-start".to_string(),
            GateDefinition {
                version: 1,
                key: "manual-start".to_string(),
                title: "Manual start".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 2,
                reserved: HashMap::new(),
                auto: false,
                example_integration: None,
                inputs: None,
            },
        );
        crate::commands::test_helpers::seed_gate_registry(&storage, &registry);
        let mut issue = crate::domain::types::fixture_issue("checked".to_string(), String::new());
        issue.state = State::Ready;
        issue.gates_required = vec!["auto-start".to_string(), "manual-start".to_string()];
        issue.gates_status.insert(
            "manual-start".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: chrono::Utc::now(),
            },
        );
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let executor =
            CommandExecutor::new(storage.clone()).with_layout(storage.repository_layout());
        (storage, executor, issue_id)
    }

    fn capture_precheck_plan(
        executor: &CommandExecutor<crate::storage::InMemoryStorage>,
        issue_id: &str,
    ) -> CapturedPrecheckPlan {
        use crate::storage::RepositoryStateStore;
        let mut session = executor
            .storage
            .open_mutation_session(executor.require_layout().unwrap())
            .unwrap();
        let image = executor
            .capture_proposed_base(
                session.as_mut(),
                &std::collections::BTreeMap::new(),
                &[],
                Some(issue_id),
            )
            .unwrap()
            .unwrap();
        let issue = captured_active_issues(&image)
            .unwrap()
            .into_iter()
            .find(|issue| issue.id == issue_id)
            .unwrap();
        let registry = crate::repository_state::declarations_from_image(&image)
            .unwrap()
            .gates;
        captured_precheck_plan(&image, &issue, &registry).unwrap()
    }

    fn prior_precheck_run(issue_id: &str) -> crate::domain::GateRunResult {
        crate::domain::GateRunResult {
            schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
            run_id: "prior-run".to_string(),
            gate_key: "auto-start".to_string(),
            stage: crate::declarations::GateStage::Precheck,
            issue_id: issue_id.to_string(),
            commit: None,
            branch: None,
            tree_dirty: None,
            status: crate::domain::GateRunStatus::Passed,
            started_at: chrono::Utc::now(),
            completed_at: Some(chrono::Utc::now()),
            duration_ms: Some(1),
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            command: "true".to_string(),
            by: None,
            message: None,
            findings: None,
            inputs_digest: None,
            origin: crate::domain::GateVerdictOrigin::Executed,
        }
    }

    fn captured_issue_spec(id: &str) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        let paths = std::collections::BTreeSet::from([
            VirtualPath::data(format!("issues/{id}.json")).unwrap(),
            VirtualPath::data("events.jsonl").unwrap(),
        ]);
        CaptureSpec::phase_one(
            paths,
            CaptureBudget {
                max_paths: 2,
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 4,
            },
        )
        .unwrap()
    }

    fn captured_gate_issue_spec(id: &str) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        let paths = std::collections::BTreeSet::from([
            VirtualPath::data(format!("issues/{id}.json")).unwrap(),
            VirtualPath::data("events.jsonl").unwrap(),
            VirtualPath::data("gates.toml").unwrap(),
        ]);
        CaptureSpec::phase_one(
            paths,
            CaptureBudget {
                max_paths: 3,
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 4,
            },
        )
        .unwrap()
    }

    fn issue_from_image(image: &crate::repository_state::RepositoryImage, id: &str) -> Issue {
        let path = crate::repository_state::VirtualPath::data(format!("issues/{id}.json")).unwrap();
        serde_json::from_slice(image.file_bytes(&path).unwrap().unwrap()).unwrap()
    }

    fn event_bytes(plan: &crate::repository_state::MaterializationPlan) -> Vec<u8> {
        plan.delta()
            .actions()
            .iter()
            .find_map(|action| match action {
                crate::repository_state::RepositoryAction::WriteFile { path, bytes, .. }
                    if path
                        == &crate::repository_state::VirtualPath::data("events.jsonl").unwrap() =>
                {
                    Some(bytes.clone())
                }
                _ => None,
            })
            .unwrap()
    }

    fn manual_gate_registry(key: &str) -> crate::declarations::GateRegistry {
        use crate::declarations::{GateDefinition, GateMode, GateRegistry, GateStage};
        let definition = GateDefinition {
            version: 1,
            key: key.to_string(),
            title: key.to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            auto: false,
            example_integration: None,
            inputs: None,
        };
        GateRegistry {
            gates: std::collections::HashMap::from([(key.to_string(), definition)]),
        }
    }

    fn gate_registry_from_image(
        image: &crate::repository_state::RepositoryImage,
    ) -> crate::declarations::GateRegistry {
        let path = crate::repository_state::VirtualPath::data("gates.toml").unwrap();
        crate::declarations::parse_gate_registry(image.file_bytes(&path).unwrap().unwrap()).unwrap()
    }

    #[test]
    fn test_captured_issue_retry_preserves_unrelated_change_and_context_identity() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError};

        let storage = InMemoryStorage::new();
        let issue = crate::domain::types::fixture_issue(
            "captured-retry-issue".to_string(),
            "Captured retry".to_string(),
        );
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let layout = storage.repository_layout();
        let request = CapturedIssueMutation::Assign {
            issue_id: id.clone(),
            assignee: "agent:retry".parse().unwrap(),
        };
        let context = MutationContext::deterministic(
            [7; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-20T10:11:12Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session.capture(captured_issue_spec(&id)).unwrap();
        let first = derive_captured_issue_mutation(
            issue_from_image(&first_image, &id),
            &crate::declarations::GateRegistry::default(),
            &request,
        )
        .unwrap();
        let first_plan = finalize(&layout, &first_image, &context, &first.intents).unwrap();

        // Model an unrelated writer between capture and apply. The stale plan
        // conflicts instead of replacing that writer's label.
        let mut concurrent = storage.load_issue(&id).unwrap();
        concurrent.labels.push("owner:concurrent".to_string());
        crate::commands::test_helpers::seed_issue(&storage, concurrent);
        assert!(matches!(
            first_session.apply(&first_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
        let retry_image = retry_session.capture(captured_issue_spec(&id)).unwrap();
        let retry = derive_captured_issue_mutation(
            issue_from_image(&retry_image, &id),
            &crate::declarations::GateRegistry::default(),
            &request,
        )
        .unwrap();
        let retry_plan = finalize(&layout, &retry_image, &context, &retry.intents).unwrap();
        assert_eq!(
            event_bytes(&first_plan),
            event_bytes(&retry_plan),
            "one operation context must keep event identity and time stable"
        );
        retry_session.apply(&retry_plan).unwrap();

        let updated = storage.load_issue(&id).unwrap();
        assert!(updated
            .labels
            .iter()
            .any(|label| label == "owner:concurrent"));
        assert_eq!(updated.assignee.unwrap().to_string(), "agent:retry");
    }

    #[test]
    fn test_captured_issue_idempotent_retry_derives_true_noop() {
        let mut issue = crate::domain::types::fixture_issue(
            "captured-noop-issue".to_string(),
            "Captured no-op".to_string(),
        );
        issue.assignee = Some("agent:same".parse().unwrap());
        let request = CapturedIssueMutation::Assign {
            issue_id: issue.id.clone(),
            assignee: "agent:same".parse().unwrap(),
        };
        let derived = derive_captured_issue_mutation(
            issue,
            &crate::declarations::GateRegistry::default(),
            &request,
        )
        .unwrap();
        assert!(
            derived.intents.is_empty(),
            "rebased no-op must write nothing"
        );
        assert!(matches!(
            derived.outcome,
            CapturedIssueMutationOutcome::Unchanged
        ));
    }

    #[test]
    fn test_manual_gate_evidence_repeats_but_operation_retry_is_stable() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError};

        for status in [GateStatus::Passed, GateStatus::Failed] {
            let storage = InMemoryStorage::new();
            crate::commands::test_helpers::seed_gate_registry(
                &storage,
                &manual_gate_registry("review"),
            );
            let mut issue = crate::domain::types::fixture_issue(
                format!("repeat-{status:?}"),
                "Repeated evidence".to_string(),
            );
            let id = issue.id.clone();
            let actor: crate::domain::Assignee = "agent:reviewer".parse().unwrap();
            issue.gates_required.push("review".to_string());
            issue.gates_status.insert(
                "review".to_string(),
                GateState {
                    status,
                    updated_by: Some(actor.clone()),
                    updated_at: chrono::DateTime::UNIX_EPOCH,
                },
            );
            crate::commands::test_helpers::seed_issue(&storage, issue);
            let layout = storage.repository_layout();
            let request = CapturedIssueMutation::SetManualGateStatus {
                issue_id: id.clone(),
                gate_key: "review".to_string(),
                status,
                by: Some(actor.clone()),
            };
            let first_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc);
            let first_context = MutationContext::deterministic([31; 32], first_time);
            let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
            let first_image = first_session
                .capture(captured_gate_issue_spec(&id))
                .unwrap();
            let first = derive_captured_issue_mutation(
                issue_from_image(&first_image, &id),
                &gate_registry_from_image(&first_image),
                &request,
            )
            .unwrap();
            let first_plan =
                finalize(&layout, &first_image, &first_context, &first.intents).unwrap();

            // Force a genuine captured-image retry. The operation context must
            // retain its event identity and timestamp when the issue preimage
            // changes between attempts.
            let mut concurrent = storage.load_issue(&id).unwrap();
            concurrent.labels.push("owner:concurrent".to_string());
            crate::commands::test_helpers::seed_issue(&storage, concurrent);
            assert!(matches!(
                first_session.apply(&first_plan),
                Err(RepositoryStateStoreError::RetryableConflict { .. })
            ));
            drop(first_session);

            let mut retry_session = storage.open_mutation_session(layout.clone()).unwrap();
            let retry_image = retry_session
                .capture(captured_gate_issue_spec(&id))
                .unwrap();
            let retry = derive_captured_issue_mutation(
                issue_from_image(&retry_image, &id),
                &gate_registry_from_image(&retry_image),
                &request,
            )
            .unwrap();
            let retry_plan =
                finalize(&layout, &retry_image, &first_context, &retry.intents).unwrap();
            assert_eq!(
                event_bytes(&first_plan),
                event_bytes(&retry_plan),
                "one operation context must keep manual evidence identity and time stable"
            );
            retry_session.apply(&retry_plan).unwrap();
            assert_eq!(
                storage.load_issue(&id).unwrap().gates_status["review"].updated_at,
                first_time
            );

            let second_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T11:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc);
            let second_context = MutationContext::deterministic([32; 32], second_time);
            let mut second_session = storage.open_mutation_session(layout.clone()).unwrap();
            let second_image = second_session
                .capture(captured_gate_issue_spec(&id))
                .unwrap();
            let second = derive_captured_issue_mutation(
                issue_from_image(&second_image, &id),
                &gate_registry_from_image(&second_image),
                &request,
            )
            .unwrap();
            let second_plan =
                finalize(&layout, &second_image, &second_context, &second.intents).unwrap();
            assert_ne!(event_bytes(&first_plan), event_bytes(&second_plan));
            second_session.apply(&second_plan).unwrap();

            assert_eq!(
                storage.load_issue(&id).unwrap().gates_status["review"].updated_at,
                second_time
            );
            let events = storage.read_events().unwrap();
            assert_eq!(events.len(), 2);
            let evidence = events
                .iter()
                .map(|event| match (status, event) {
                    (
                        GateStatus::Passed,
                        Event::GatePassed {
                            id,
                            timestamp,
                            updated_by,
                            ..
                        },
                    )
                    | (
                        GateStatus::Failed,
                        Event::GateFailed {
                            id,
                            timestamp,
                            updated_by,
                            ..
                        },
                    ) if updated_by.as_ref() == Some(&actor) => (id, timestamp),
                    _ => panic!("unexpected manual gate evidence: {event:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(*evidence[0].1, first_time);
            assert_eq!(*evidence[1].1, second_time);
            assert_ne!(evidence[0].0, evidence[1].0);
        }
    }

    #[test]
    fn test_manual_gate_pending_status_is_rejected_before_other_validation() {
        let issue = crate::domain::types::fixture_issue(
            "pending-precedence".to_string(),
            "Pending precedence".to_string(),
        );
        let request = CapturedIssueMutation::SetManualGateStatus {
            issue_id: issue.id.clone(),
            gate_key: "missing-and-not-required".to_string(),
            status: GateStatus::Pending,
            by: None,
        };

        let error = match derive_captured_issue_mutation(
            issue,
            &crate::declarations::GateRegistry::default(),
            &request,
        ) {
            Ok(_) => panic!("pending status unexpectedly accepted"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "manual gate status must be passed or failed"
        );
    }

    #[test]
    fn test_gate_add_retry_rejects_removed_registry_declaration() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{
            GateNotFoundError, InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError,
        };

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_gate_registry(
            &storage,
            &manual_gate_registry("review"),
        );
        let issue = crate::domain::types::fixture_issue(
            "registry-race".to_string(),
            "Registry race".to_string(),
        );
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let layout = storage.repository_layout();
        let request = CapturedIssueMutation::AddGates {
            issue_id: id.clone(),
            gate_keys: vec!["review".to_string()],
        };
        let context = MutationContext::deterministic(
            [41; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-20T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session
            .capture(captured_gate_issue_spec(&id))
            .unwrap();
        let first = derive_captured_issue_mutation(
            issue_from_image(&first_image, &id),
            &gate_registry_from_image(&first_image),
            &request,
        )
        .unwrap();
        let stale_plan = finalize(&layout, &first_image, &context, &first.intents).unwrap();

        crate::commands::test_helpers::seed_gate_registry(
            &storage,
            &crate::declarations::GateRegistry::default(),
        );
        assert!(matches!(
            first_session.apply(&stale_plan),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
        drop(first_session);

        let mut retry_session = storage.open_mutation_session(layout).unwrap();
        let retry_image = retry_session
            .capture(captured_gate_issue_spec(&id))
            .unwrap();
        let error = match derive_captured_issue_mutation(
            issue_from_image(&retry_image, &id),
            &gate_registry_from_image(&retry_image),
            &request,
        ) {
            Ok(_) => panic!("retry must revalidate against the changed registry"),
            Err(error) => error,
        };
        assert_eq!(
            error.downcast_ref::<GateNotFoundError>(),
            Some(&GateNotFoundError::Batch(vec!["review".to_string()]))
        );
        assert!(storage.load_issue(&id).unwrap().gates_required.is_empty());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_rules_are_parsed_once_and_cached() {
        use crate::declarations::rules::Assertion;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();

        let rules_path = storage.root().join("rules.toml");
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "first"
assert = { require-section = { heading = "Goals" } }
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);

        // First access parses and caches the file.
        let first = executor.rules().expect("valid rules");
        assert_eq!(first.rules.len(), 1);
        assert_eq!(first.rules[0].name, "first");
        let ptr_first = std::ptr::from_ref(first);

        // Mutate the file on disk AFTER the first parse.
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "second"
assert = { require-section = { heading = "Other" } }

[[rules]]
name = "third"
assert = { require-doc-type = { doc-type = "design" } }
"#,
        )
        .unwrap();

        // Second access must return the cached (original) parse, NOT re-read.
        let second = executor.rules().expect("valid rules");
        assert_eq!(second.rules.len(), 1, "ruleset was re-read from disk");
        assert_eq!(second.rules[0].name, "first");
        assert!(matches!(
            second.rules[0].assert,
            Assertion::RequireSection { .. }
        ));
        // Same cached instance is returned each time.
        assert_eq!(ptr_first, std::ptr::from_ref(second));
    }

    #[test]
    fn test_namespaces_are_parsed_once_and_cached() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();

        let config_path = storage.root().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);

        // First access parses config.toml and caches the namespace registry.
        let first = executor.cached_namespaces().expect("valid namespaces");
        let ptr_first = std::ptr::from_ref(first);

        // Mutate config.toml on disk AFTER the first parse. A re-read would pick
        // up the new namespace; a cached registry must not.
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false

[namespaces.owner]
description = "Ownership tags"
unique = false
"#,
        )
        .unwrap();

        // Second access must return the SAME cached instance, proving the
        // namespace config is parsed once per executor and not re-read per write.
        let second = executor.cached_namespaces().expect("valid namespaces");
        assert_eq!(
            ptr_first,
            std::ptr::from_ref(second),
            "namespace registry was re-read from disk instead of cached"
        );
    }

    #[test]
    fn test_rules_missing_file_yields_empty_ok() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();
        // No rules.toml written.

        let executor = CommandExecutor::new(storage);
        let rules = executor.rules().expect("missing file is not an error");
        assert!(rules.rules.is_empty());
    }

    #[test]
    fn test_rules_malformed_file_yields_err() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        std::fs::create_dir_all(storage.root()).unwrap();

        // A rule whose assert table has no kind is a genuine config error and
        // must NOT be downgraded to an empty (no-rules) set.
        std::fs::write(
            storage.root().join("rules.toml"),
            r#"
[[rules]]
name = "broken"
assert = {}
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage);
        assert!(
            executor.rules().is_err(),
            "malformed rules.toml must surface an error, not an empty set"
        );
    }

    // Enforcement tests
    #[test]
    fn test_require_active_lease_off_mode() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);

        // Create the root directory and config with enforcement off
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should always succeed in off mode, even without lease, and return None
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), None);
    }

    #[test]
    fn test_check_active_lease_no_claims_index() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);

        let executor = CommandExecutor::new(storage);

        // No claims index - should return false
        let result = executor.check_active_lease(&issue_id);
        assert!(result.is_ok());
        assert!(!result.unwrap());
    }

    #[test]
    fn test_require_active_lease_strict_mode_no_lease() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);

        // Create the root directory and config with enforcement strict
        std::fs::create_dir_all(storage.root()).unwrap();
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        std::fs::write(storage.root().join("config.toml"), config_toml).unwrap();

        let executor = CommandExecutor::new(storage);

        // Should fail in strict mode without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("No active lease"));
        assert!(err_msg.contains("jit claim acquire"));
    }

    #[test]
    fn test_require_active_lease_off_mode_default() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();

        // Create a test issue
        let issue =
            crate::domain::types::fixture_issue("test-issue".to_string(), "Test".to_string());
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);

        // No config file - should default to off mode (single-agent development)
        let executor = CommandExecutor::new(storage);

        // Should succeed in off mode (default) without lease
        let result = executor.require_active_lease(&issue_id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_precheck_evidence_detects_manual_status_mutation() {
        use crate::storage::IssueStore;
        let (storage, executor, issue_id) = precheck_evidence_fixture();
        let before = capture_precheck_plan(&executor, &issue_id).evidence;

        let mut issue = storage.load_issue(&issue_id).unwrap();
        issue.gates_status.get_mut("manual-start").unwrap().status = GateStatus::Failed;
        crate::commands::test_helpers::seed_issue(&storage, issue);

        assert_ne!(capture_precheck_plan(&executor, &issue_id).evidence, before);
    }

    #[test]
    fn test_precheck_evidence_detects_run_history_mutation() {
        let (storage, executor, issue_id) = precheck_evidence_fixture();
        let before = capture_precheck_plan(&executor, &issue_id).evidence;
        let run = prior_precheck_run(&issue_id);
        storage.add_data_file(
            "gate-runs/prior-run/result.json",
            &serde_json::to_string(&run).unwrap(),
        );

        assert_ne!(capture_precheck_plan(&executor, &issue_id).evidence, before);
    }

    #[test]
    fn test_precheck_evidence_detects_prompt_mutation_and_preserves_captured_bytes() {
        let (storage, executor, issue_id) = precheck_evidence_fixture();
        let before = capture_precheck_plan(&executor, &issue_id);
        assert_eq!(before.prompts["auto-start"], "captured prompt");

        storage.add_worktree_file("start-prompt.md", "mutated prompt");
        let after = capture_precheck_plan(&executor, &issue_id);

        assert_eq!(after.prompts["auto-start"], "mutated prompt");
        assert_ne!(after.evidence, before.evidence);
    }

    #[test]
    fn test_claims_guard_uses_explicit_layout_not_process_cwd() {
        use crate::storage::worktree_paths::WorktreePaths;
        use crate::storage::{ClaimCoordinator, FileLocker};
        use std::time::Duration;

        let repo = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(repo.path().join(".jit")).unwrap();
        assert!(std::process::Command::new("git")
            .arg("init")
            .arg("-q")
            .arg(repo.path())
            .status()
            .unwrap()
            .success());
        let paths = WorktreePaths::detect_from(repo.path()).unwrap();
        let coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(Duration::from_secs(1)),
            "wt:test".to_string(),
            "agent:test".to_string(),
        );
        coordinator.init().unwrap();
        let issue_id = "abcd1111111111111111111111111111";
        coordinator.acquire_claim(issue_id, 600).unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), repo.path().join(".jit"))
                .unwrap();

        let guard = claims_mutation_guard(&layout).unwrap().unwrap();

        assert!(guard
            .has_active_lease(issue_id, None, |raw| Ok(raw.to_string()))
            .unwrap());
    }
}

#[cfg(test)]
mod mutation_session_contract_tests {
    use super::*;
    use crate::storage::RepositoryStateStoreError;

    fn retryable() -> RepositoryStateStoreError {
        RepositoryStateStoreError::RetryableConflict {
            path: "issues/x.json".to_string(),
        }
    }

    fn non_retryable() -> RepositoryStateStoreError {
        RepositoryStateStoreError::UnsafeTarget("escaping/target".to_string())
    }

    #[test]
    fn test_classify_apply_converges_on_apply_success() {
        let outcome: std::result::Result<(), RepositoryStateStoreError> = Ok(());
        match classify_apply(outcome, 42).unwrap() {
            AttemptOutcome::Done(value) => assert_eq!(value, 42),
            AttemptOutcome::Retry => panic!("a successful apply must converge, not retry"),
        }
    }

    #[test]
    fn test_classify_apply_retries_on_retryable_conflict() {
        let outcome: std::result::Result<(), RepositoryStateStoreError> = Err(retryable());
        assert!(matches!(
            classify_apply(outcome, 42).unwrap(),
            AttemptOutcome::Retry
        ));
    }

    #[test]
    fn test_classify_apply_propagates_non_retryable_error() {
        let outcome: std::result::Result<(), RepositoryStateStoreError> = Err(non_retryable());
        assert!(
            classify_apply(outcome, 42).is_err(),
            "a non-retryable storage error must abort, not fold into a retry"
        );
    }

    #[test]
    fn test_capture_or_retry_returns_image_on_success() {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        use crate::storage::{InMemoryStorage, RepositoryStateStore};

        let storage = InMemoryStorage::new();
        storage.add_data_file("config.toml", "");
        let layout = storage.repository_layout();
        let mut session = storage.open_mutation_session(layout).unwrap();
        let budget = CaptureBudget {
            max_paths: 4,
            max_listings: 0,
            max_bytes: 1024,
            max_depth: 6,
        };
        let image = session
            .capture(
                CaptureSpec::phase_one([VirtualPath::data("config.toml").unwrap()], budget)
                    .unwrap(),
            )
            .unwrap();
        assert!(capture_or_retry(Ok(image)).unwrap().is_some());
    }

    #[test]
    fn test_capture_or_retry_folds_retryable_conflict_into_none() {
        let outcome: std::result::Result<crate::repository_state::RepositoryImage, _> =
            Err(retryable());
        assert!(
            capture_or_retry(outcome).unwrap().is_none(),
            "a retryable capture conflict must signal retry via None"
        );
    }

    #[test]
    fn test_capture_or_retry_propagates_non_retryable_error() {
        let outcome: std::result::Result<crate::repository_state::RepositoryImage, _> =
            Err(non_retryable());
        assert!(capture_or_retry(outcome).is_err());
    }

    #[test]
    fn test_with_mutation_attempts_returns_value_on_first_success() {
        let mut calls = 0usize;
        let value = with_mutation_attempts("first-success", || {
            calls += 1;
            Ok(AttemptOutcome::Done(7))
        })
        .unwrap();
        assert_eq!(value, 7);
        assert_eq!(calls, 1, "a first-attempt success must not retry");
    }

    #[test]
    fn test_with_mutation_attempts_retries_until_convergence() {
        let mut calls = 0usize;
        let value = with_mutation_attempts("eventual-success", || {
            calls += 1;
            if calls < 3 {
                Ok(AttemptOutcome::Retry)
            } else {
                Ok(AttemptOutcome::Done(calls))
            }
        })
        .unwrap();
        assert_eq!(value, 3);
        assert_eq!(calls, 3);
    }

    #[test]
    fn test_with_mutation_attempts_reports_exhaustion_after_retry_limit() {
        let mut calls = 0usize;
        let result: Result<()> = with_mutation_attempts("never-converges", || {
            calls += 1;
            Ok(AttemptOutcome::Retry)
        });
        assert_eq!(
            calls, MUTATION_SESSION_RETRY_LIMIT,
            "the driver must attempt exactly the shared retry bound before giving up"
        );
        let error = result.unwrap_err();
        assert!(
            error.downcast_ref::<MutationSessionExhausted>().is_some(),
            "exhaustion must surface the typed terminal error"
        );
    }

    #[test]
    fn test_with_mutation_attempts_propagates_closure_error_without_retry() {
        let mut calls = 0usize;
        let result: Result<()> = with_mutation_attempts("aborting", || {
            calls += 1;
            Err(anyhow!("fatal"))
        });
        assert!(result.is_err());
        assert_eq!(calls, 1, "a fatal closure error must abort immediately");
    }

    #[test]
    fn test_with_mutation_session_opens_fresh_session_per_attempt_until_done() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        let layout = storage.repository_layout();
        let mut attempts = 0usize;
        let value = with_mutation_session(&storage, &layout, "session-retry", |_session| {
            attempts += 1;
            if attempts < 3 {
                Ok(SessionStep::Retry)
            } else {
                Ok(SessionStep::Done(attempts))
            }
        })
        .unwrap();
        assert_eq!(value, 3);
        assert_eq!(
            attempts, 3,
            "each attempt opens one fresh recovered session"
        );
    }

    #[test]
    fn test_with_mutation_session_reports_exhaustion_through_shared_bound() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        let layout = storage.repository_layout();
        let mut attempts = 0usize;
        let result: Result<()> =
            with_mutation_session(&storage, &layout, "session-stuck", |_session| {
                attempts += 1;
                Ok(SessionStep::Retry)
            });
        assert_eq!(
            attempts, MUTATION_SESSION_RETRY_LIMIT,
            "the session path shares the single retry bound"
        );
        assert!(result
            .unwrap_err()
            .downcast_ref::<MutationSessionExhausted>()
            .is_some());
    }
}
