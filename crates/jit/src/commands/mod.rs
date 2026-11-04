//! Command execution logic for all CLI operations.
//!
//! The `CommandExecutor` handles all business logic for issue management,
//! dependency manipulation, gate operations, and event logging.
//!
//! This module is organized into submodules by functional area:
//! - `bootstrap`: Repository opening for one CLI invocation
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
mod bootstrap;
pub mod bulk_update;
pub mod claim;
mod config;
mod dependency;
mod document;
#[cfg(test)]
mod event_tail_tests;
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
// MutationInterruptionInjector surface but never calls into this module), the
// module stays reachable but every item in it goes genuinely uncalled in that
// one configuration.
#[cfg(any(test, feature = "test-support"))]
#[allow(dead_code)]
pub mod test_helpers;

pub use archive::ArchiveOps;
pub use batch_create::{
    BatchCreateOutcome, BatchDryRunOutcome, BatchIssueDef, BatchValidationError,
    BatchValidationProblem,
};
pub use bootstrap::{open_repository, OpenedRepository, RecoveryFailedError, RepositoryInvocation};
pub use bulk_update::{BulkUpdatePreview, BulkUpdateResult, UpdateOperations};
pub use claim::ClaimOps;
pub use config::{
    resolve_dotted_key, ConfigGetOutcome, ConfigKeyError, ConfigOps, ConfigSetOutcome,
    ConfigValidationReport,
};
pub use document::DocumentOps;
pub use gate::{
    FieldEdit, GateNotRequiredError, GateOps, GatePassAllEntry, GatePassFailed, GatePassOutcome,
    GateStatusArgumentsTransposedError, GateUpdate, ManualGateAttestationRequiredError,
    PassAllOutcome,
};
pub use gate_check::VerdictSource;
pub use graph::{BatchExport, BoundaryEdge, GraphExportFormat, GraphOps};
pub use init::FreshInitResult;
pub use invariant::InvariantCheckResult;
pub use issue::{
    DescriptionUpdate, IssueCreation, IssueDeletion, IssueLocalUpdate, IssueMutation, IssueOps,
};
pub use item::{ItemListResult, ItemOps, ItemShowResult};
pub use profile::{
    parse_profile_selector, parse_profile_selectors, parse_profile_variable_assignments,
    ProfileApplyError, ProfileNotFoundError, ProfileOps, ProfileResolutionError, ProfileSelection,
    ProfileSelectorError, ProfileVariableOptions, SelectedProfile, UNCOMMITTED_TARGETS_RESOLUTION,
};
pub use project::{ProjectRenderResult, ProjectionRenderReport};
pub use query::QueryOps;
pub use search::ContentMatch;
pub use template::{TemplateApplyResult, TemplateOps};
pub use template_expand::{
    expand_template, validate_delta_acyclic, AnchorGates, DeltaEdge, DeltaEndpoint, PlannedNode,
    TemplateDelta,
};
pub use validate::ValidationOps;

// Re-export WorktreeIdentity for init return type
pub use crate::storage::worktree_identity::WorktreeIdentity;

// Common imports used across modules
use crate::config::JitConfig;
use crate::config_manager::ConfigManager;
use crate::declarations::rules::{RuleConfigError, RuleSet};
use crate::declarations::{GateDefinition, GateMode};
use crate::domain::identity::{GateKey, IssueId};
use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::domain::{
    is_dependency_met, Event, EventBody, GateEvaluationStability, GateState, GateStatus, Issue,
    LabelNamespaces, Priority, ReadinessCorrection, State,
};
use crate::graph::DependencyGraph;
use crate::labels as label_utils;
use crate::storage::IssueStore;
// Type hierarchy validation (currently only validates type labels)
use anyhow::{anyhow, Context, Result};
use schemars::JsonSchema;
use serde::Serialize;
use std::cell::RefCell;
use std::sync::OnceLock;

thread_local! {
    /// The one production-context factory for the current dispatch thread.
    ///
    /// CLI dispatch is synchronous, while the thread-local boundary prevents parallel
    /// in-process invocations from sharing annotations. Scoped replacement below restores
    /// the previous value for nested calls and during unwinding.
    static PRODUCTION_MUTATION_CONTEXT_FACTORY: RefCell<crate::repository_state::MutationContextFactory> =
        RefCell::new(crate::repository_state::MutationContextFactory::default());
}

/// Construct the operation-scoped production mutation context.
///
/// Every command and integration helper uses this factory boundary. The context receives
/// whichever annotation dispatch installed, and an invocation whose dispatch found no fact
/// worth annotating builds an unannotated context.
pub fn production_mutation_context() -> crate::repository_state::MutationContext {
    PRODUCTION_MUTATION_CONTEXT_FACTORY.with(|factory| factory.borrow().create())
}

/// Bounds for the captures that mutate one named issue: a claim, a captured
/// issue mutation, and a document reference edit.
///
/// Each declares every path it reads — one issue record, the event log's end, and for
/// a gate transition the gate registry — and requests no listing, so no path can
/// enter the capture that the caller did not name. Shape is what bounds it, and
/// the depth is the deepest of those constructions: an issue record one name
/// below the `issues` directory. A caller reaching deeper overrides `max_depth`
/// alone, since that is the only dimension its shape decides.
///
/// `max_bytes` is unbounded: issue records and the log's final line have no
/// declared size limit. The capture never reads earlier log lines.
fn issue_mutation_capture_budget() -> crate::repository_state::CaptureBudget {
    use crate::repository_state::{CaptureBudget, VirtualPath};

    CaptureBudget {
        max_listings: 0,
        max_bytes: u64::MAX,
        max_depth: VirtualPath::ISSUES.relative().depth().saturating_add(1),
    }
}

/// The `gate-runs` directory, `issue`'s directory in it, and the paths the first
/// `count` gate-run records `context` derives ids for are published at: what a
/// mutation publishing those records of `issue` must capture.
///
/// Gate-run records take the first identifiers a mutation derives, so a
/// mutation that also creates issues cannot use this.
fn gate_run_record_paths(
    context: &crate::repository_state::MutationContext,
    issue: &IssueId,
    count: usize,
) -> Vec<crate::repository_state::VirtualPath> {
    use crate::repository_state::VirtualPath;

    [
        VirtualPath::GATE_RUNS,
        VirtualPath::gate_run_issue_directory(issue),
    ]
    .into_iter()
    .chain(
        (0..count as u64)
            .map(|index| VirtualPath::gate_run_record(issue, &context.gate_run_id_at(index))),
    )
    .collect()
}

/// The verdict record a mutation recording `run` publishes, with the
/// directories above it that decide its creates: what that mutation must
/// capture. Empty when `run` holds no reusable verdict.
fn gate_verdict_record_paths(
    run: &crate::domain::GateRunResult,
) -> Result<Vec<crate::repository_state::VirtualPath>> {
    run.reusable_digest()
        .map(|digest| {
            let record =
                crate::repository_state::VirtualPath::gate_verdict_record(&run.gate_key, digest);
            let mut paths = record.ancestor_directories()?;
            paths.push(record);
            Ok(paths)
        })
        .transpose()
        .map(Option::unwrap_or_default)
}

/// An annotation installed for as long as this guard lives.
///
/// This is the mechanism behind the dispatch-scoped annotation: dropping the guard
/// restores the previously installed annotation, whether the scope ended by returning
/// or by unwinding, so nested and repeated in-process invocations cannot leak facts
/// into one another. Top-level CLI dispatch holds the guard, because its command match
/// spans the remainder of the dispatch scope rather than a closure;
/// [`with_dispatch_mutation_annotation`] is the closure form over this same guard.
#[must_use = "the annotation is uninstalled as soon as the guard is dropped"]
pub struct DispatchMutationAnnotation(Option<crate::repository_state::MutationContextAnnotation>);

impl DispatchMutationAnnotation {
    /// Install `annotation` for every production mutation context built until the
    /// returned guard is dropped.
    pub fn install(annotation: crate::repository_state::MutationContextAnnotation) -> Self {
        Self(
            PRODUCTION_MUTATION_CONTEXT_FACTORY
                .with(|factory| factory.borrow_mut().replace_annotation(Some(annotation))),
        )
    }
}

impl Drop for DispatchMutationAnnotation {
    fn drop(&mut self) {
        PRODUCTION_MUTATION_CONTEXT_FACTORY.with(|factory| {
            factory.borrow_mut().replace_annotation(self.0.take());
        });
    }
}

/// Run one dispatch with an annotation stamped into every production mutation context it
/// constructs.
///
/// The prior annotation is restored when `dispatch` returns or unwinds, so nested and
/// repeated in-process invocations cannot leak facts into one another.
pub fn with_dispatch_mutation_annotation<T>(
    annotation: crate::repository_state::MutationContextAnnotation,
    dispatch: impl FnOnce() -> T,
) -> T {
    let _scope = DispatchMutationAnnotation::install(annotation);
    dispatch()
}

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
//   single-session sites: it opens one fresh mutation session per attempt and
//   applies the plan the closure returns through [`classify_apply`].

/// Maximum capture/apply attempts before a mutation session is reported
/// non-convergent.
///
/// The single home of the retry bound that command retry loops previously
/// copied as the literal `8`.
const MUTATION_SESSION_RETRY_LIMIT: usize = 8;

/// Byte ceiling on the repository image one command captures.
pub(super) const CAPTURE_MAX_BYTES: u64 = 512 * 1024 * 1024;

/// Terminal error for a mutation session that keeps losing its capture/apply
/// race.
///
/// Replaces the bespoke "did not converge after repeated conflicts" bails: it
/// names the operation and the number of attempts made, and maps to the generic
/// exit code through a single arm in `output::failure::classify`.
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

/// Capture `spec` and, in the same round, every request `discover` adds from
/// the image read so far, until it adds none.
///
/// Each request is read once: the session grows the image rather than
/// recapturing it, so no read is compared with an earlier one. `Ok(None)` is a
/// retryable conflict.
pub(crate) fn capture_discovering(
    session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    spec: crate::repository_state::CaptureSpec,
    discover: impl FnMut(
        &crate::repository_state::RepositoryImage,
        &mut crate::repository_state::CaptureSpec,
    ) -> Result<()>,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    match capture_or_retry(session.capture(spec))? {
        Some(image) => grow_discovering(session, image, discover),
        None => Ok(None),
    }
}

/// [`capture_discovering`] continued from an `image` the round has read.
pub(crate) fn grow_discovering(
    session: &mut (dyn crate::storage::RepositoryMutationSession + '_),
    image: crate::repository_state::RepositoryImage,
    mut discover: impl FnMut(
        &crate::repository_state::RepositoryImage,
        &mut crate::repository_state::CaptureSpec,
    ) -> Result<()>,
) -> Result<Option<crate::repository_state::RepositoryImage>> {
    let mut image = Some(image);
    while let Some(read) = image {
        let mut spec = read.capture_spec().clone();
        discover(&read, &mut spec)?;
        if &spec == read.capture_spec() {
            return Ok(Some(read));
        }
        image = capture_or_retry(session.extend(read, spec))?;
    }
    Ok(None)
}

/// Report an occupied repository export target as the caller-facing
/// already-exists failure naming the output path the invocation asked for.
///
/// The typed export error names the canonical repository path it refused; an
/// adopter asked for the path they typed. Every command publishing a file at a
/// caller-chosen output path reports that refusal in the same words, so the
/// translation lives here rather than once per command
/// (`@/inv/convention-convergence`).
pub(crate) fn map_occupied_export_error(
    result: Result<()>,
    output: &std::path::Path,
) -> Result<()> {
    match result {
        Err(error)
            if matches!(
                error.downcast_ref::<crate::repository_state::RepositoryExportError>(),
                Some(crate::repository_state::RepositoryExportError::OccupiedTarget(_))
            ) =>
        {
            Err(crate::errors::AlreadyExistsError::new(format!(
                "Output path already exists: {}",
                output.display()
            ))
            .into())
        }
        other => other,
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

/// Retry driver for the pure single-session sites: open one fresh mutation
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
        issue_id: IssueId,
        assignee: crate::domain::Assignee,
    },
    Unassign {
        issue_id: IssueId,
    },
    AddGates {
        issue_id: IssueId,
        gate_keys: Vec<GateKey>,
    },
    RemoveGates {
        issue_id: IssueId,
        gate_keys: Vec<GateKey>,
    },
    SetManualGateStatus {
        issue_id: IssueId,
        gate_key: GateKey,
        status: GateStatus,
        by: Option<crate::domain::Assignee>,
    },
}

impl CapturedIssueMutation {
    fn issue_id(&self) -> &IssueId {
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

    /// Whether the operation publishes a gate-run record: an attestation does.
    fn records_gate_run(&self) -> bool {
        matches!(self, Self::SetManualGateStatus { .. })
    }
}

enum CapturedIssueMutationOutcome {
    Changed,
    Unchanged,
    GatesAdded {
        added: Vec<GateKey>,
        already_exist: Vec<GateKey>,
    },
    GatesRemoved {
        removed: Vec<GateKey>,
        not_found: Vec<GateKey>,
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

/// Declarations and plan content captured from one mutation image, over which
/// the mutation evaluates graph rules at its timestamp.
struct CapturedGraphEvidence<'a> {
    declarations: crate::repository_state::CapturedRepositoryDeclarations,
    plan_content: std::collections::HashMap<IssueId, String>,
    context: &'a crate::repository_state::MutationContext,
}

impl<'a> CapturedGraphEvidence<'a> {
    fn capture(
        image: &crate::repository_state::RepositoryImage,
        issues: &[Issue],
        context: &'a crate::repository_state::MutationContext,
    ) -> Result<Self> {
        Ok(Self {
            declarations: crate::repository_state::declarations_from_image(image)?,
            plan_content: validate::plan_content_with_issues(image, issues)?,
            context,
        })
    }

    fn evaluate_graph(
        &self,
        rules: &[&crate::declarations::rules::Rule],
        issues: &[Issue],
    ) -> Result<Vec<crate::validation::graph::GraphFinding>> {
        let config = self.declarations.config();
        let namespaces = crate::config_manager::namespaces_from_config(config);
        let hierarchy =
            crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&namespaces)?;
        Ok(crate::validation::graph::evaluate_graph(
            rules,
            issues,
            &hierarchy,
            config.content_format(),
            self.context.timestamp(),
            &self.plan_content,
        ))
    }
}

struct CapturedTransitionEvidence<'a> {
    issues: &'a [Issue],
    graph: &'a CapturedGraphEvidence<'a>,
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
        crate::repository_state::DirectoryListing,
    >,
}

impl PrecheckEvidence {
    /// Whether `image` holds these inputs unchanged, compared over the inputs
    /// it reads: a mutation base reads no issue document reference.
    fn matches(&self, image: &crate::repository_state::RepositoryImage) -> Result<bool> {
        let inputs_unchanged = self
            .inputs
            .iter()
            .filter(|(path, _)| image.capture_spec().contains_path(path))
            .map(|(path, identity)| {
                image
                    .entry(path)
                    .map(|entry| entry.identity() == identity.as_ref())
            })
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let validation_view = self
            .validation_view
            .as_ref()
            .map(|_| PrecheckValidationView {
                listings: image.directory_listings().clone(),
            });
        Ok(inputs_unchanged.into_iter().all(|unchanged| unchanged)
            && validation_view == self.validation_view)
    }
}

struct CapturedPrecheckPlan {
    evidence: PrecheckEvidence,
    prompts: std::collections::HashMap<GateKey, String>,
}

/// One precheck execution retained across publication retries. A transition
/// reuses it whenever the bound target is unchanged, even when its captured
/// inputs moved; that drift is recorded on the reused run instead of launching
/// the checker again.
struct CachedPrecheckExecution {
    target_id: IssueId,
    /// The captured inputs are retained so a reused precheck can be recorded as
    /// stable or as having moved while its checker ran.
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

/// Whether `gate` is an automatic built-in checker that judges the whole
/// captured repository rather than one issue's slice of it.
pub(crate) fn is_broad_builtin_precheck(gate: &crate::declarations::GateDefinition) -> bool {
    use crate::declarations::GateChecker;
    gate.mode == GateMode::Auto
        && !matches!(
            gate.checker,
            Some(GateChecker::Exec { .. } | GateChecker::ReviewPlaceholder)
        )
}

/// Whether one of `issue`'s precheck gates validates the whole repository.
fn has_broad_precheck(issue: &Issue, registry: &crate::declarations::GateRegistry) -> bool {
    issue
        .gates_required
        .iter()
        .filter_map(|key| registry.gates.get(key))
        .any(|gate| {
            gate.stage == crate::declarations::GateStage::Precheck
                && is_broad_builtin_precheck(gate)
        })
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
    let broad_builtin = has_broad_precheck(issue, registry);
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
        BTreeSet::from([VirtualPath::issue_record(&issue.id), VirtualPath::GATES])
    };
    if !broad_builtin {
        paths.extend(
            issue
                .dependencies
                .iter()
                .map(VirtualPath::issue_record)
                .collect::<Vec<_>>(),
        );
        if consumes_history {
            paths.extend(named_gate_run_records(issue).map(|(_, _, path)| path));
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
        listings: image.directory_listings().clone(),
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
    paths: &crate::storage::WorktreePaths,
) -> Result<Option<crate::storage::claim_coordinator::ClaimsMutationGuard>> {
    use crate::storage::{ClaimCoordinator, FileLocker};

    if !paths.has_claims_coordination() {
        return Ok(None);
    }
    let agent = crate::agent_config::resolve_agent_id(None)
        .unwrap_or(crate::domain::Assignee::SYSTEM_LEASE_CHECK);
    let worktree_id = match crate::storage::worktree_identity::read_worktree_id(paths)? {
        Some(id) => id,
        None => {
            crate::domain::identity::WorktreeId::try_from(paths.worktree_root.display().to_string())
                .with_context(|| {
                    format!(
                        "Invalid worktree id fallback from root path '{}'",
                        paths.worktree_root.display()
                    )
                })?
        }
    };
    let coordinator = ClaimCoordinator::new(
        paths.clone(),
        FileLocker::with_configured_timeout(),
        worktree_id,
        agent,
    );
    coordinator
        .lock()
        .map(crate::storage::claim_coordinator::ClaimsMutationGuard::hold)
        .map(Some)
}

/// The lease rule every issue-record write obeys: `strict` refuses when another
/// agent holds an active lease on a target, naming each; `warn` answers the same
/// findings; `off` answers none.
fn refuse_foreign_leases(
    mode: crate::config::EnforcementMode,
    targets: &[IssueId],
    conflicting_holder: impl Fn(&IssueId) -> Result<Option<crate::domain::Assignee>>,
) -> Result<Vec<(IssueId, String)>> {
    use crate::config::EnforcementMode;

    if mode == EnforcementMode::Off {
        return Ok(Vec::new());
    }
    let findings = targets
        .iter()
        .filter_map(|id| {
            conflicting_holder(id)
                .map(|holder| {
                    holder.map(|holder| (id.clone(), format!("{id} is leased by {holder}")))
                })
                .transpose()
        })
        .collect::<Result<Vec<_>>>()?;
    if mode == EnforcementMode::Strict && !findings.is_empty() {
        let named = findings
            .iter()
            .map(|(_, finding)| finding.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "Refusing to write: {named}.\nCoordinate with the holder, or wait for the lease to end. \
             If the lease is yours, set your identity with JIT_AGENT_ID."
        );
    }
    Ok(findings)
}

/// [`refuse_foreign_leases`] over the leases a held claims guard read.
fn refuse_foreign_leases_held(
    mode: crate::config::EnforcementMode,
    targets: &[IssueId],
    guard: Option<&crate::storage::claim_coordinator::ClaimsMutationGuard>,
) -> Result<Vec<String>> {
    let agent = crate::agent_config::resolve_agent_id(None).ok();
    let findings = refuse_foreign_leases(mode, targets, |id| {
        Ok(guard
            .map(|guard| guard.conflicting_lease(id, agent.as_ref()))
            .transpose()?
            .flatten()
            .map(|lease| lease.agent_id))
    })?;
    Ok(findings.into_iter().map(|(_, finding)| finding).collect())
}

/// [`refuse_foreign_leases`] over active leases read from storage, for a
/// command that reaches it without taking the claims lock.
fn refuse_foreign_leases_read(
    paths: &crate::storage::WorktreePaths,
    mode: crate::config::EnforcementMode,
    targets: &[IssueId],
) -> Result<Vec<(IssueId, String)>> {
    use crate::storage::claim_coordinator::read_active_leases;

    if mode == crate::config::EnforcementMode::Off {
        return Ok(Vec::new());
    }
    let agent = crate::agent_config::resolve_agent_id(None).ok();
    let leases = read_active_leases(paths, chrono::Utc::now())?;
    refuse_foreign_leases(mode, targets, |id| {
        Ok(leases
            .iter()
            .find(|lease| &lease.issue_id == id && Some(&lease.agent_id) != agent.as_ref())
            .map(|lease| lease.agent_id.clone()))
    })
}

/// Refuse claiming `target` for `assignee` when anyone else leases it, whatever
/// the enforcement mode.
fn ensure_claim_available(
    guard: Option<&crate::storage::claim_coordinator::ClaimsMutationGuard>,
    target: &IssueId,
    assignee: &crate::domain::Assignee,
) -> Result<()> {
    refuse_foreign_leases(
        crate::config::EnforcementMode::Strict,
        std::slice::from_ref(target),
        |id| {
            Ok(guard
                .map(|guard| guard.conflicting_lease(id, Some(assignee)))
                .transpose()?
                .flatten()
                .map(|lease| lease.agent_id))
        },
    )
    .map(drop)
}

#[derive(Clone)]
enum CapturedLifecycleMutation {
    Claim {
        issue_id: IssueId,
        assignee: crate::domain::Assignee,
    },
    Release {
        issue_id: IssueId,
        reason: String,
        released_by: crate::domain::Assignee,
    },
    AutoReady {
        issue_id: IssueId,
    },
    AutoDone {
        issue_id: IssueId,
    },
}

impl CapturedLifecycleMutation {
    fn issue_id(&self) -> &IssueId {
        match self {
            Self::Claim { issue_id, .. }
            | Self::Release { issue_id, .. }
            | Self::AutoReady { issue_id }
            | Self::AutoDone { issue_id } => issue_id,
        }
    }
}

fn lifecycle_requires_prechecks(request: &CapturedLifecycleMutation, issue: &Issue) -> bool {
    issue.state == State::Ready && matches!(request, CapturedLifecycleMutation::Claim { .. })
}

/// The full base a precheck-running lifecycle transition reads before taking
/// the claims guard, because its precheck gates run outside the lock.
struct PrecheckPreflight {
    plan: CapturedPrecheckPlan,
    image: crate::repository_state::RepositoryImage,
    issue: Issue,
    registry: crate::declarations::GateRegistry,
}

struct CapturedLifecycleOutcome {
    issue: Issue,
    changed: bool,
}

#[derive(Clone)]
struct CapturedFieldUpdate {
    issue_id: IssueId,
    title: Option<String>,
    description: Option<DescriptionUpdate>,
    priority: Option<Priority>,
    state: Option<State>,
    add_labels: Vec<String>,
    remove_labels: Vec<String>,
    append_label: Option<String>,
    content_format: Option<Option<crate::domain::ContentFormat>>,
    issue_type: Option<String>,
    add_gates: Vec<GateKey>,
    remove_gates: Vec<GateKey>,
    assignee: Option<crate::domain::Assignee>,
    unassign: bool,
    bulk: bool,
    force: bool,
    enforce_lease: bool,
    /// Rejection: skips rule enforcement and promotes the dependents it unblocks.
    rejection: bool,
}

impl CapturedFieldUpdate {
    /// Build the bulk-update request shape for one issue. Shared by the real
    /// per-issue publication path ([`CommandExecutor::publish_captured_bulk_update`])
    /// and the no-op verification derivation
    /// ([`IssueOps::confirm_bulk_noop_candidates`]) so both run the
    /// identical authoritative request through [`derive_field_update`] --
    /// there is exactly one place that turns `UpdateOperations` into a
    /// captured request, so the two call sites cannot silently drift apart.
    fn bulk(issue_id: IssueId, operations: &UpdateOperations, force: bool) -> Result<Self> {
        Ok(Self {
            issue_id,
            title: None,
            description: None,
            priority: operations.priority,
            state: operations.state,
            add_labels: operations.add_labels.clone(),
            remove_labels: operations.remove_labels.clone(),
            append_label: None,
            content_format: None,
            issue_type: None,
            add_gates: operations.add_gates.clone(),
            remove_gates: operations.remove_gates.clone(),
            assignee: operations.assignee.clone(),
            unassign: operations.unassign,
            bulk: true,
            force,
            enforce_lease: false,
            rejection: false,
        })
    }

    /// The standalone gate and assignee operations a single-issue update
    /// composes, in the order its CLI flags apply; bulk updates fold these
    /// fields into their own `issue_updated` record instead.
    fn issue_local_mutations(&self, issue_id: &IssueId) -> Vec<CapturedIssueMutation> {
        if self.bulk {
            return Vec::new();
        }
        let issue_id = issue_id.clone();
        let assignment = if self.unassign {
            Some(CapturedIssueMutation::Unassign {
                issue_id: issue_id.clone(),
            })
        } else {
            self.assignee
                .clone()
                .map(|assignee| CapturedIssueMutation::Assign {
                    issue_id: issue_id.clone(),
                    assignee,
                })
        };
        [
            (!self.add_gates.is_empty()).then(|| CapturedIssueMutation::AddGates {
                issue_id: issue_id.clone(),
                gate_keys: self.add_gates.clone(),
            }),
            (!self.remove_gates.is_empty()).then(|| CapturedIssueMutation::RemoveGates {
                issue_id: issue_id.clone(),
                gate_keys: self.remove_gates.clone(),
            }),
            assignment,
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    fn new(issue_id: IssueId) -> Self {
        Self {
            issue_id,
            title: None,
            description: None,
            priority: None,
            state: None,
            add_labels: Vec::new(),
            remove_labels: Vec::new(),
            append_label: None,
            content_format: None,
            issue_type: None,
            add_gates: Vec::new(),
            remove_gates: Vec::new(),
            assignee: None,
            unassign: false,
            bulk: false,
            force: false,
            enforce_lease: false,
            rejection: false,
        }
    }

    /// Reject the issue, appending `resolution` when given.
    fn rejection(issue_id: IssueId, resolution: Option<String>) -> Self {
        Self {
            state: Some(State::Rejected),
            enforce_lease: true,
            rejection: true,
            append_label: resolution,
            ..Self::new(issue_id)
        }
    }
}

struct DerivedFieldUpdate {
    changed: bool,
    /// The update moved the issue from a non-terminal into a terminal state,
    /// which is what makes dependents eligible for readiness.
    entered_terminal: bool,
    warnings: Vec<String>,
    intents: Vec<crate::repository_state::MutationIntent>,
    error_after_apply: Option<crate::errors::TransitionBlockedError>,
}

#[derive(Debug)]
struct CapturedFieldUpdateOutcome {
    issue: Issue,
    changed: bool,
    entered_terminal: bool,
    warnings: Vec<String>,
}

fn derive_write_validation(
    issue: &Issue,
    declarations: &crate::repository_state::CapturedRepositoryDeclarations,
    force: bool,
) -> Result<WriteValidation> {
    let config = declarations.config();
    let repo_format = config.content_format();
    let strictness = config.strictness();
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
        warnings: evaluation.warnings().into_iter().cloned().collect(),
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
        if let Some(origin) = issue.archived_from.filter(|origin| target != *origin) {
            return Err(crate::errors::TransitionBlockedError::archived_revive(
                issue.id.clone(),
                target,
                origin,
            )
            .into());
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
                unpassed_gate_blockers(&issue, &evidence.graph.declarations.gates),
            )
            .into());
        }
    }

    let mut bypass_events = Vec::new();
    if !matches!(target, State::Rejected | State::Archived) {
        let mut projected = issue.clone();
        projected.state = target;
        let rules = evidence
            .graph
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
            let strictness = evidence.graph.declarations.config().strictness();
            let enforcing = rules
                .iter()
                .filter(|rule| rule.enforce)
                .map(|rule| rule.name.as_str())
                .collect::<HashSet<_>>();
            let findings = evidence.graph.evaluate_graph(&rules, &slice)?;
            let mut blocking = Vec::new();
            for finding in findings {
                let config_error = finding.is_config_error();
                let pertains = config_error || finding.issue_id.as_ref() == Some(&projected.id);
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
                                EventBody::draft_graph_rule_bypassed(
                                    issue.id.clone(),
                                    target,
                                    rule.clone(),
                                )
                            } else {
                                EventBody::draft_transition_blocked(
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
        EventBody::draft_issue_state_changed(issue.id.clone(), old_state, target),
    ));
    if target == State::Done {
        bypass_events.push((3, EventBody::draft_issue_completed(issue.id.clone())));
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

    // A single-issue update stages its gate and assignee changes through the
    // standalone operations, so it records their events; the inline handling
    // below then finds them already applied.
    let (original, staged, mut events) = request
        .issue_local_mutations(&original.id)
        .iter()
        .try_fold(
            (original, false, Vec::new()),
            |(issue, staged, mut events), mutation| {
                let applied = apply_captured_issue_mutation(
                    issue,
                    &evidence.graph.declarations.gates,
                    mutation,
                    evidence.graph.context,
                )?;
                events.extend(applied.events);
                Ok::<_, anyhow::Error>((applied.issue, staged || applied.changed, events))
            },
        )?;
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
        if !request.bulk && !issue.labels.contains(label) {
            return Err(crate::errors::NotFoundError::new(format!(
                "issue {} has no label '{label}' to remove",
                issue.id
            ))
            .into());
        }
        issue.labels.retain(|candidate| candidate != label);
    }
    if let Some(label) = &request.append_label {
        issue.labels.push(label.clone());
    }
    let missing_gates = request
        .add_gates
        .iter()
        .filter(|key| !evidence.graph.declarations.gates.gates.contains_key(key))
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
                unpassed_gate_blockers(&issue, &evidence.graph.declarations.gates),
            ));
            Some(State::Gated)
        }
        target => target,
    };

    let mut projected = issue.clone();
    if let Some(target) = target {
        projected.state = target;
    }
    let validation = match (&request.append_label, request.rejection) {
        (Some(_), _) | (None, true) => {
            let repo_format = evidence.graph.declarations.config().content_format();
            let evaluation = crate::validation::evaluate_local(
                &projected,
                &evidence.graph.declarations.rules,
                repo_format,
            )
            .map_err(|error| anyhow!("rule evaluation failed: {error}"))?;
            WriteValidation {
                warnings: evaluation
                    .findings()
                    .into_iter()
                    .filter(|finding| finding.severity != crate::declarations::rules::Severity::Off)
                    .cloned()
                    .collect(),
                bypassed_rules: Vec::new(),
            }
        }
        (None, false) => {
            derive_write_validation(&projected, &evidence.graph.declarations, request.force)?
        }
    };
    if request.issue_type.is_some() {
        let repo_format = evidence.graph.declarations.config().content_format();
        let explicit_type = crate::validation::evaluate_local(
            &projected,
            &evidence.graph.declarations.rules,
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

    let mut warnings = validation
        .warnings
        .iter()
        .map(|finding| format!("[{}] {}", finding.rule, finding.message))
        .collect::<Vec<_>>();
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
                    entered_terminal: false,
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

    let changed = staged || !changed_fields.is_empty() || issue.state != original.state;
    let entered_terminal = !original.state.is_terminal() && issue.state.is_terminal();
    if issue.assignee != original.assignee {
        if let Some(assignee) = issue.assignee.clone() {
            events.push((
                4,
                EventBody::draft_issue_claimed(issue.id.clone(), assignee),
            ));
        }
    }
    if !changed_fields.is_empty() && request.append_label.is_none() {
        events.push((
            5,
            EventBody::draft_issue_updated(
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
    events.extend(validation.bypassed_rules.into_iter().map(|rule| {
        (
            9,
            EventBody::draft_local_rule_bypassed(issue.id.clone(), rule),
        )
    }));
    let (promotions, blocked) = if request.rejection && entered_terminal {
        let graph = evidence
            .issues
            .iter()
            .map(|candidate| {
                if candidate.id == issue.id {
                    issue.clone()
                } else {
                    candidate.clone()
                }
            })
            .collect::<Vec<_>>();
        crate::domain::queries::issues_becoming_ready(&graph)
            .iter()
            .try_fold((Vec::new(), None), |(mut intents, blocked), id| {
                let dependent = graph
                    .iter()
                    .find(|candidate| &candidate.id == id)
                    .cloned()
                    .ok_or_else(|| anyhow!("captured dependent {id} disappeared"))?;
                let (promoted, events, error) = issue::derive_ready_promotion(
                    dependent,
                    CapturedTransitionEvidence {
                        issues: &graph,
                        ..evidence
                    },
                )?;
                intents.push(MutationIntent::UpdateIssue {
                    issue: Box::new(promoted),
                });
                intents.extend(events);
                Ok::<_, anyhow::Error>((intents, blocked.or(error)))
            })?
    } else {
        (Vec::new(), None)
    };
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
        .chain(promotions)
        .collect();
    Ok(DerivedFieldUpdate {
        changed,
        entered_terminal,
        warnings,
        intents,
        error_after_apply: gate_error.or(blocked),
    })
}

/// Derive the full-record low-level intent and its audit events from one captured
/// issue. Keeping this pure makes the retry contract directly testable: every
/// attempt receives the new image's record and cannot retain an ambient preimage.
fn derive_captured_issue_mutation(
    issue: Issue,
    registry: &crate::declarations::GateRegistry,
    request: &CapturedIssueMutation,
    context: &crate::repository_state::MutationContext,
) -> Result<DerivedCapturedIssueMutation> {
    use crate::repository_state::MutationIntent;

    let AppliedIssueMutation {
        issue,
        outcome,
        events,
        gate_runs,
        changed,
    } = apply_captured_issue_mutation(issue, registry, request, context)?;
    let intents = if changed {
        std::iter::once(MutationIntent::UpdateIssue {
            issue: Box::new(issue),
        })
        .chain(
            gate_runs
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
        .collect()
    } else {
        Vec::new()
    };
    Ok(DerivedCapturedIssueMutation { outcome, intents })
}

/// One issue-local operation applied to a captured record, with its audit events
/// and the gate-run records it publishes.
struct AppliedIssueMutation {
    issue: Issue,
    outcome: CapturedIssueMutationOutcome,
    events: Vec<(u8, EventBody)>,
    gate_runs: Vec<crate::domain::GateRunResult>,
    changed: bool,
}

fn apply_captured_issue_mutation(
    mut issue: Issue,
    registry: &crate::declarations::GateRegistry,
    request: &CapturedIssueMutation,
    context: &crate::repository_state::MutationContext,
) -> Result<AppliedIssueMutation> {
    let mut gate_runs = Vec::new();
    let (outcome, events, changed) = match request {
        CapturedIssueMutation::Assign { assignee, .. } => {
            if issue.assignee.as_ref() == Some(assignee) {
                (CapturedIssueMutationOutcome::Unchanged, Vec::new(), false)
            } else {
                issue.assignee = Some(assignee.clone());
                let event = EventBody::draft_issue_claimed(issue.id.clone(), assignee.clone());
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
                let event = EventBody::draft_issue_updated(
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
                .filter(|key| !registry.gates.contains_key(key))
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
                    run_id: None,
                });
                added.push(key.clone());
            }
            let events = added
                .iter()
                .filter_map(|key| registry.gates.get(key))
                .map(|gate| {
                    (
                        1,
                        EventBody::draft_gate_added(issue.id.clone(), gate.key.clone()),
                    )
                })
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
                .map(|key| {
                    (
                        1,
                        EventBody::draft_gate_removed(issue.id.clone(), key.clone()),
                    )
                })
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
            let run_id = context.gate_run_id_at(0);
            let replacement = GateState {
                status: *status,
                updated_by: by.clone(),
                updated_at: chrono::DateTime::default(),
                run_id: Some(run_id.clone()),
            };
            issue.gates_status.insert(gate_key.clone(), replacement);
            let event = match status {
                GateStatus::Passed => {
                    EventBody::draft_gate_passed(issue.id.clone(), gate.key.clone(), by.clone())
                }
                GateStatus::Failed => {
                    EventBody::draft_gate_failed(issue.id.clone(), gate.key.clone(), by.clone())
                }
                GateStatus::Pending => unreachable!("validated manual gate evidence status"),
            };
            gate_runs.push(crate::domain::GateRunResult::attestation(
                &run_id,
                &issue.id,
                &gate.key,
                gate.stage,
                *status,
                by.as_ref(),
            ));
            (
                CapturedIssueMutationOutcome::Changed,
                vec![(1, event)],
                true,
            )
        }
    };
    Ok(AppliedIssueMutation {
        issue,
        outcome,
        events,
        gate_runs,
        changed,
    })
}

/// The unpassed gates of `issue` paired with their current status and registry
/// mode, in the order [`Issue::get_unpassed_gates`] reports them.
///
/// A required gate with no recorded run counts as [`GateStatus::Pending`]. A
/// gate key missing from `registry` (should not normally happen) defaults to
/// [`GateMode::Manual`], matching [`GateOps::pass_gate`]'s own
/// fallback: an unregistered gate never matches the `Auto` branch there
/// either, so it is treated as requiring `--by` just the same. Shared by the
/// transition guard and the gate-diversion path so both describe a
/// gate-blocked completion with the same blockers, and so the remediation
/// hint can name the `--by <attestor>` form for a manual gate (jit:1d59070d
/// REQ-03).
fn unpassed_gate_blockers(
    issue: &Issue,
    registry: &crate::declarations::GateRegistry,
) -> Vec<(GateKey, GateStatus, GateMode)> {
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
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct CommitInfo {
    pub sha: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

/// Status summary for all issues
#[derive(Debug)]
pub struct StatusSummary {
    pub backlog: usize,
    pub ready: usize,
    pub in_progress: usize,
    pub gated: usize,
    pub done: usize,
    pub rejected: usize,
    pub blocked: usize,
    pub total: usize,
}

/// Collection key of [`DocumentListResult`].
pub enum DocumentsKey {}

impl CollectionKey for DocumentsKey {
    const KEY: &'static str = "documents";
    const SCHEMA_NAME: Option<&'static str> = Some("DocumentListResult");
}

/// Result of listing document references for an issue
pub type DocumentListResult =
    ListEnvelope<DocumentsKey, crate::domain::DocumentReference, DocumentListScope>;

/// The issue a [`DocumentListResult`] lists documents of.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentListScope {
    pub issue_id: IssueId,
}

/// Git history for a document
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentHistory {
    pub path: String,
    pub commits: Vec<CommitInfo>,
}

/// Collection key of [`AssetListResult`].
pub enum AssetsKey {}

impl CollectionKey for AssetsKey {
    const KEY: &'static str = "assets";
    const SCHEMA_NAME: Option<&'static str> = Some("AssetListResult");
}

/// Result of listing assets for a document
pub type AssetListResult = ListEnvelope<AssetsKey, crate::document::Asset, AssetListScope>;

/// Fields of an [`AssetListResult`] beside its `assets`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetListScope {
    /// Worktree root the human rendering checks asset paths against.
    #[serde(skip)]
    pub worktree_root: std::path::PathBuf,
    pub issue_id: IssueId,
    pub document_path: String,
    pub summary: AssetSummary,
}

/// Document content display result
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentContentResult {
    pub path: String,
    pub label: Option<String>,
    pub commit: String,
    pub doc_type: Option<String>,
    pub content: String,
}

/// Document diff result
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentDiffResult {
    pub path: String,
    pub from_commit: String,
    pub to_commit: String,
    pub diff: String,
}

/// Result of adding a document reference
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentAddResult {
    pub issue_id: IssueId,
    pub document: crate::domain::DocumentReference,
    /// `true` when `path` was already linked to the issue and this call
    /// refreshed that entry in place; `false` when it appended a new one.
    pub updated: bool,
}

/// Result of removing a document reference
#[derive(Debug, Serialize, JsonSchema)]
pub struct DocumentRemoveResult {
    pub issue_id: IssueId,
    pub path: String,
}

/// Summary of asset counts by category
#[derive(Debug, Serialize, JsonSchema)]
pub struct AssetSummary {
    pub total: usize,
    pub per_doc: usize,
    pub shared: usize,
    pub external: usize,
    pub missing: usize,
}

/// Result of exporting a snapshot
#[derive(Debug, Serialize, JsonSchema)]
pub struct SnapshotExportResult {
    pub path: String,
    pub issue_count: usize,
    pub document_count: usize,
    pub format: String,
    pub size_bytes: Option<u64>,
}

/// Result of checking document links
#[derive(Debug, Serialize, JsonSchema)]
pub struct LinkCheckResult {
    pub valid: bool,
    pub errors: Vec<serde_json::Value>,
    pub link_warnings: Vec<serde_json::Value>,
    #[serde(skip)]
    pub scope: String,
    pub summary: LinkCheckSummary,
}

/// Summary of link check results
#[derive(Debug, Serialize, JsonSchema)]
pub struct LinkCheckSummary {
    pub total_documents: usize,
    pub valid: usize,
    pub errors: usize,
    pub warnings: usize,
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
    /// Local-rule findings that do not block the write, to surface to the caller.
    pub warnings: Vec<crate::validation::Finding>,
    /// Names of `enforce` rules whose blocking findings were overridden by
    /// `--force`. One [`EventBody::LocalRuleBypassed`] must be logged per entry,
    /// AFTER the write commits.
    pub bypassed_rules: Vec<String>,
}

type PhasedEvents = Vec<(u8, EventBody)>;

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

/// Read a known virtual path from the captured image.
pub(crate) fn image_virtual_bytes(
    image: &crate::repository_state::RepositoryImage,
    path: &crate::repository_state::VirtualPath,
) -> Result<Option<Vec<u8>>> {
    Ok(image.file_bytes(path)?.map(<[u8]>::to_vec))
}

/// Parse the issues the captured listing names, in listing order, reporting a
/// missing record as issue-not-found.
fn captured_active_issues(image: &crate::repository_state::RepositoryImage) -> Result<Vec<Issue>> {
    use crate::repository_state::{captured_issue_ids, parse_issue_records, IssueRecordsError};

    parse_issue_records(&captured_issue_ids(image)?, |path| image.file_bytes(path))
        .map(|records| records.into_iter().map(|(issue, _)| issue).collect())
        .map_err(|error| match error {
            IssueRecordsError::Missing(id) => crate::storage::IssueNotFoundError::new(id).into(),
            error => error.into(),
        })
}

/// Each required gate of `issue` paired with the record path of the run its
/// state names, the only runs a history-consuming checker reads.
fn named_gate_run_records(
    issue: &Issue,
) -> impl Iterator<
    Item = (
        &GateKey,
        &crate::domain::identity::GateRunId,
        crate::repository_state::VirtualPath,
    ),
> {
    issue.gates_required.iter().filter_map(|key| {
        let run_id = issue.gates_status.get(key)?.run_id.as_ref()?;
        Some((
            key,
            run_id,
            crate::repository_state::VirtualPath::gate_run_record(&issue.id, run_id),
        ))
    })
}

/// Add the gate-run root and its complete listing to `spec`, then each issue
/// directory it names with its listing, then each record those listings name.
/// Names that are not issue ids, `verdicts` among them, and names not of the
/// form `<run-id>.json` are evidence only through their listing, matching the
/// typed reader. The verdict records are discovered the same way: the
/// `verdicts` directory and its listing, each gate-key directory it names and
/// its listing, then each record named there.
fn discover_gate_records(
    image: &crate::repository_state::RepositoryImage,
    spec: &mut crate::repository_state::CaptureSpec,
) -> Result<()> {
    use crate::repository_state::VirtualPath;

    let root = VirtualPath::GATE_RUNS;
    spec.discover_paths([root.clone()])?;
    spec.discover_listing(root.clone())?;
    if let Some(listing) = image.directory_listings().get(&root) {
        spec.discover_paths(
            listing
                .names()
                .iter()
                .filter_map(|name| IssueId::try_from(name.as_str()).ok())
                .map(|issue| VirtualPath::gate_run_issue_directory(&issue)),
        )?;
    }
    spec.discover_listings(
        crate::repository_state::captured_gate_run_issue_ids(image)
            .unwrap_or_default()
            .iter()
            .map(VirtualPath::gate_run_issue_directory),
    )?;
    spec.discover_paths(
        crate::repository_state::captured_gate_run_ids(image)
            .iter()
            .map(|(issue, run)| VirtualPath::gate_run_record(issue, run)),
    )?;
    let verdicts = VirtualPath::GATE_VERDICTS;
    spec.discover_paths([verdicts.clone()])?;
    spec.discover_listing(verdicts.clone())?;
    if let Some(listing) = image.directory_listings().get(&verdicts) {
        spec.discover_paths(
            listing
                .names()
                .iter()
                .filter_map(|name| crate::domain::identity::GateKey::try_from(name.as_str()).ok())
                .map(|key| VirtualPath::gate_verdict_directory(&key)),
        )?;
    }
    spec.discover_listings(
        crate::repository_state::captured_gate_verdict_keys(image)
            .iter()
            .map(VirtualPath::gate_verdict_directory),
    )?;
    spec.discover_paths(crate::repository_state::captured_gate_verdict_records(
        image,
    ))?;
    Ok(())
}

fn require_issue_in_capture(issues: &[Issue], id: &IssueId) -> Result<IssueId> {
    issues
        .iter()
        .find(|issue| &issue.id == id)
        .map(|issue| issue.id.clone())
        .ok_or_else(|| crate::storage::IssueNotFoundError::new(id.clone()).into())
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
    /// retained mutation session — reentrant only for this same canonical layout
    /// (`@/inv` layout coherence) — changes what the executor holds internally, not
    /// these call sites. `None` only when no layout was supplied; a session-opening
    /// command then reports a wiring error rather than proceeding.
    layout: Option<crate::repository_state::RepositoryLayout>,
    /// Dispatch-owned worktree classification used by lifecycle lease checks.
    ///
    /// This is the same selected-data-root authority used by claim commands.
    /// The CLI injects its dispatch-time value; non-dispatch library callers
    /// lazily derive it from their selected repository layout.
    worktree_paths: OnceLock<Result<crate::storage::WorktreePaths, String>>,
    /// Leftover redo records the opening of this repository applied.
    recovered_records: usize,
}

impl<S: IssueStore> CommandExecutor<S> {
    /// Create a new command executor with the given storage
    pub fn new(storage: S) -> Self {
        let data_root = storage.root();
        let config_layout = crate::storage::discover_repository_layout(
            data_root
                .parent()
                .unwrap_or_else(|| std::path::Path::new(".")),
            data_root,
        )
        .ok();
        let config_manager = config_layout
            .map(ConfigManager::new)
            .unwrap_or_else(ConfigManager::unbound);
        Self {
            storage,
            config_manager,
            rules: OnceLock::new(),
            effective_rules: OnceLock::new(),
            config: OnceLock::new(),
            namespaces: OnceLock::new(),
            layout: None,
            worktree_paths: OnceLock::new(),
            recovered_records: 0,
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
        self.config_manager.bind_layout(layout.clone());
        self.layout = Some(layout);
        self
    }

    /// Construct this executor with the selected data root's worktree authority.
    pub fn with_worktree_paths(mut self, paths: crate::storage::WorktreePaths) -> Self {
        self.worktree_paths = OnceLock::from(Ok(paths));
        self
    }

    /// Record the leftover redo records startup recovery applied.
    pub fn with_recovered_records(mut self, count: usize) -> Self {
        self.recovered_records = count;
        self
    }

    /// Return or lazily derive the selected-data-root worktree authority.
    pub(crate) fn require_worktree_paths(&self) -> Result<&crate::storage::WorktreePaths> {
        self.worktree_paths
            .get_or_init(|| {
                let (data_root, worktree_root) = self.layout.as_ref().map_or_else(
                    || {
                        let data_root = self.storage.root();
                        let worktree_root = data_root.parent().unwrap_or(data_root);
                        (data_root, worktree_root)
                    },
                    |layout| (layout.data_root(), layout.worktree_root()),
                );
                crate::storage::WorktreePaths::detect_for_data_root(data_root, worktree_root)
                    .map_err(|error| error.to_string())
            })
            .as_ref()
            .map_err(|message| anyhow!("{message}"))
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

        let context = production_mutation_context();
        with_mutation_session(&self.storage, layout, "repository export", |session| {
            let Some(image) = capture_or_retry(session.capture(intent.capture_spec(budget)?))?
            else {
                return Ok(SessionStep::Retry);
            };
            let plan = finalize_repository_export(&image, &context, intent)?;
            Ok(SessionStep::Apply(plan, ()))
        })
    }

    /// Capture and finalize one repository export without publishing it.
    pub(crate) fn plan_repository_export(
        &self,
        layout: &crate::repository_state::RepositoryLayout,
        intent: &crate::repository_state::RepositoryExportIntent,
        budget: crate::repository_state::CaptureBudget,
    ) -> Result<()>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::finalize_repository_export;

        with_mutation_session(
            &self.storage,
            layout,
            "repository export planning",
            |session| {
                let Some(image) = capture_or_retry(session.capture(intent.capture_spec(budget)?))?
                else {
                    return Ok(SessionStep::Retry);
                };
                finalize_repository_export(
                    &image,
                    &crate::repository_state::MutationContext::preview(),
                    intent,
                )?;
                Ok(SessionStep::Done(()))
            },
        )
    }

    /// Rebase one closed issue-local operation on a freshly captured record.
    ///
    /// The request determines its complete capture set. The caller cannot pass a
    /// record, capture closure, or arbitrary patch. A conflict reopens a session
    /// and derives again under a fresh context, so the identity and time an
    /// attempt publishes are sampled under the write lock that publishes them,
    /// so an attestation's run time is its publication time.
    fn publish_captured_issue_mutation(
        &self,
        request: CapturedIssueMutation,
    ) -> Result<(Issue, CapturedIssueMutationOutcome)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{finalize, CaptureSpec, RepositoryEntry, VirtualPath};
        use std::collections::BTreeSet;

        let layout = self.require_layout()?;
        with_mutation_session(
            &self.storage,
            &layout,
            "captured issue mutation",
            |session| {
                let context = production_mutation_context();
                let run_paths = if request.records_gate_run() {
                    gate_run_record_paths(&context, request.issue_id(), 1)
                } else {
                    Vec::new()
                };
                let mut paths = BTreeSet::from([VirtualPath::issue_record(request.issue_id())]);
                if request.captures_gate_registry() {
                    paths.insert(VirtualPath::GATES);
                }
                paths.extend(run_paths.iter().cloned());
                let base = issue_mutation_capture_budget();
                let budget = crate::repository_state::CaptureBudget {
                    max_depth: run_paths
                        .iter()
                        .map(|path| path.relative().depth())
                        .fold(base.max_depth, usize::max),
                    ..base
                };
                let mut spec = CaptureSpec::new(paths, budget)?;
                spec.discover_tail(VirtualPath::EVENTS)?;
                let Some(image) = capture_or_retry(session.capture(spec))? else {
                    return Ok(SessionStep::Retry);
                };
                let issue_path = VirtualPath::issue_record(request.issue_id());
                let issue: Issue = match image.entry(&issue_path)? {
                    RepositoryEntry::File { content, .. } => {
                        crate::repository_state::decode_issue_record(&issue_path, content.bytes())?
                    }
                    RepositoryEntry::Absent => {
                        return Err(crate::storage::IssueNotFoundError::new(
                            request.issue_id().clone(),
                        )
                        .into())
                    }
                    _ => return Err(anyhow!("captured issue path is not an ordinary file")),
                };
                if &issue.id != request.issue_id() {
                    return Err(anyhow!(
                        "captured issue identity mismatch: requested {}, found {}",
                        request.issue_id(),
                        issue.id
                    ));
                }
                let registry = if request.captures_gate_registry() {
                    let path = VirtualPath::GATES;
                    match image.entry(&path)? {
                        RepositoryEntry::File { content, .. } => {
                            crate::declarations::parse_gate_registry(content.bytes())
                                .context("failed to parse captured gate registry")?
                        }
                        RepositoryEntry::Absent => crate::declarations::GateRegistry::default(),
                        _ => return Err(anyhow!("captured gate registry is not an ordinary file")),
                    }
                } else {
                    crate::declarations::GateRegistry::default()
                };

                let derived = derive_captured_issue_mutation(issue, &registry, &request, &context)?;
                let issue_result = crate::repository_state::mutated_issue(
                    &image,
                    request.issue_id(),
                    &derived.intents,
                    context.timestamp(),
                )?;
                if derived.intents.is_empty() {
                    return Ok(SessionStep::Done((issue_result, derived.outcome)));
                }
                let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
                Ok(SessionStep::Apply(plan, (issue_result, derived.outcome)))
            },
        )
    }

    /// Publish `draft`, whose id `context` derived, as a new issue, with its
    /// advisory local-rule findings, then the graph-rule findings attributed to it.
    fn publish_issue_creation(
        &self,
        context: &crate::repository_state::MutationContext,
        draft: Issue,
        explicit_type: bool,
        force: bool,
    ) -> Result<(Issue, Vec<crate::validation::ReportedFinding>)>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::{created_issue, finalize, MutationIntent, VirtualPath};

        let layout = self.require_layout()?;
        with_mutation_session(&self.storage, &layout, "issue creation", |session| {
            let issue_id = draft.id.clone();
            let Some(crate::commands::validate::MutationBase { image, mut issues }) = self
                .capture_mutation_base(
                    session,
                    &[VirtualPath::ISSUES, VirtualPath::issue_record(&issue_id)],
                    None,
                )?
            else {
                return Ok(SessionStep::Retry);
            };
            let graph = CapturedGraphEvidence::capture(&image, &issues, context)?;
            let declarations = &graph.declarations;
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
                let repo_format = declarations.config().content_format();
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
            let validation = derive_write_validation(&final_draft, declarations, force)?;
            let intents = std::iter::once(MutationIntent::CreateIssue {
                draft: Box::new(final_draft.clone()),
            })
            .chain(
                validation
                    .bypassed_rules
                    .iter()
                    .map(|rule| MutationIntent::RecordEvent {
                        phase: 9,
                        event: Box::new(EventBody::draft_local_rule_bypassed(
                            issue_id.clone(),
                            rule.clone(),
                        )),
                    }),
            )
            .collect::<Vec<_>>();
            let created = created_issue(&final_draft, context.timestamp());
            issues.push(created.clone());
            let rules = declarations.rules.rules.iter().collect::<Vec<_>>();
            let findings = validation
                .warnings
                .iter()
                .chain(
                    graph
                        .evaluate_graph(&rules, &issues)?
                        .iter()
                        .filter(|finding| finding.issue_id.as_ref() == Some(&issue_id))
                        .map(|finding| &finding.finding),
                )
                .map(|finding| {
                    crate::validation::ReportedFinding::new(Some(issue_id.clone()), finding)
                })
                .collect();
            let plan = finalize(&layout, &image, context, &intents, None)?;
            Ok(SessionStep::Apply(plan, (created, findings)))
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
                    self.cached_config()
                        .map_err(|error| RuleConfigError::Configuration {
                            message: error.to_string(),
                        })?;
                crate::storage::ruleset_store::load_ruleset(
                    self.repository_layout()
                        .map_err(|error| RuleConfigError::Configuration {
                            message: error.to_string(),
                        })?,
                    config,
                )
            })
            .as_ref()
    }

    /// Combine authored rules with registry-derived defaults in memory, without
    /// writing files. Missing default rows apply even in an empty or custom-only
    /// file; existing rows retain their policy edits. Custom rules load their
    /// declared schemas, while default assertions derive from configuration.
    ///
    /// The result, including any configuration or rule-loading error, is cached.
    pub fn effective_rules(&self) -> Result<&RuleSet> {
        self.effective_rules
            .get_or_init(|| {
                let layout = self.require_layout().map_err(|error| error.to_string())?;
                let rules_path = layout
                    .resolve(&crate::repository_state::VirtualPath::RULES)
                    .map_err(|error| error.to_string())?;
                if rules_path.exists() {
                    let user = self.rules().map_err(|e| e.to_string())?.clone();
                    let namespaces = self.cached_namespaces().map_err(|e| e.to_string())?;
                    crate::repository_state::reconcile_default_rules_with_config(user, namespaces)
                        .map_err(|error| error.to_string())
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

    fn publish_captured_field_update(
        &self,
        request: CapturedFieldUpdate,
    ) -> Result<CapturedFieldUpdateOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        use crate::repository_state::finalize;

        let layout = self.require_layout()?;
        let context = production_mutation_context();
        with_mutation_attempts("issue update", || {
            // The claims guard does not depend on the repository, so it is taken
            // before the one session whose image decides target and lease policy.
            let claims_guard = request
                .enforce_lease
                .then(|| claims_mutation_guard(self.require_worktree_paths()?))
                .transpose()?
                .flatten();
            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(crate::commands::validate::MutationBase { image, issues }) =
                self.capture_mutation_base(session.as_mut(), &[], None)?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            let target = require_issue_in_capture(&issues, &request.issue_id)?;
            let issue = issues
                .iter()
                .find(|issue| issue.id == target)
                .cloned()
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(target.clone()))?;
            let graph = CapturedGraphEvidence::capture(&image, &issues, &context)?;
            let lease_warnings = if request.enforce_lease {
                refuse_foreign_leases_held(
                    graph.declarations.config().enforcement_mode(),
                    std::slice::from_ref(&target),
                    claims_guard.as_ref(),
                )?
            } else {
                Vec::new()
            };
            let mut effective_request = request.clone();
            effective_request.issue_id = target.clone();
            let mut derived = derive_field_update(
                issue,
                &effective_request,
                CapturedTransitionEvidence {
                    issues: &issues,
                    graph: &graph,
                },
            )?;
            let mut lease_warnings = lease_warnings;
            lease_warnings.append(&mut derived.warnings);
            derived.warnings = lease_warnings;
            let issue_result = crate::repository_state::mutated_issue(
                &image,
                &target,
                &derived.intents,
                context.timestamp(),
            )?;
            if derived.intents.is_empty() {
                return match derived.error_after_apply {
                    Some(error) => Err(error.with_warnings(derived.warnings).into()),
                    None => Ok(AttemptOutcome::Done(CapturedFieldUpdateOutcome {
                        issue: issue_result,
                        changed: derived.changed,
                        entered_terminal: derived.entered_terminal,
                        warnings: derived.warnings,
                    })),
                };
            }
            let plan = finalize(&layout, &image, &context, &derived.intents, None)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            match derived.error_after_apply {
                Some(error) => Err(error.with_warnings(derived.warnings).into()),
                None => Ok(AttemptOutcome::Done(CapturedFieldUpdateOutcome {
                    issue: issue_result,
                    changed: derived.changed,
                    entered_terminal: derived.entered_terminal,
                    warnings: derived.warnings,
                })),
            }
        })
    }

    fn publish_captured_bulk_update(
        &self,
        issue_id: &IssueId,
        operations: &UpdateOperations,
        force: bool,
    ) -> Result<CapturedFieldUpdateOutcome>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let request = CapturedFieldUpdate::bulk(issue_id.clone(), operations, force)?;
        self.publish_captured_field_update(request)
    }

    fn publish_captured_claim(
        &self,
        issue_id: &IssueId,
        assignee: crate::domain::Assignee,
    ) -> Result<Issue>
    where
        S: crate::storage::RepositoryStateStore,
    {
        self.publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Claim {
            issue_id: issue_id.clone(),
            assignee,
        })
        .map(|outcome| outcome.issue)
    }

    fn publish_captured_release(&self, issue_id: &IssueId, reason: String) -> Result<Issue>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let released_by = claim::resolve_release_actor(self.require_layout()?.worktree_root())?;
        let outcome =
            self.publish_captured_lifecycle_mutation(CapturedLifecycleMutation::Release {
                issue_id: issue_id.clone(),
                reason,
                released_by,
            })?;
        Ok(outcome.issue)
    }

    fn publish_captured_auto_transition(&self, issue_id: &IssueId, target: State) -> Result<bool>
    where
        S: crate::storage::RepositoryStateStore,
    {
        let request = match target {
            State::Ready => CapturedLifecycleMutation::AutoReady {
                issue_id: issue_id.clone(),
            },
            State::Done => CapturedLifecycleMutation::AutoDone {
                issue_id: issue_id.clone(),
            },
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
        use crate::repository_state::{finalize, MutationIntent};

        let layout = self.require_layout()?;
        let context = production_mutation_context();
        let mut cached_precheck = None::<CachedPrecheckExecution>;
        let coordinate_claims = matches!(request, CapturedLifecycleMutation::Claim { .. });
        with_mutation_attempts("lifecycle mutation", || {
            let (expected_target, expected_precheck_required, precheck) = {
                let mut preflight = self.storage.open_mutation_session(layout.clone())?;
                // Precheck gates run outside the repository lock, so a
                // transition that runs them grows its preflight to the full base.
                let Some(light) = self.capture_mutation_preflight(
                    preflight.as_mut(),
                    request.issue_id(),
                    |target| lifecycle_requires_prechecks(&request, target),
                )?
                else {
                    return Ok(AttemptOutcome::Retry);
                };
                let should_precheck = light.issues.is_some();
                let precheck = if let Some(issues) = light.issues {
                    let image = light.image;
                    let issue = issues
                        .into_iter()
                        .find(|issue| issue.id == light.target.id)
                        .ok_or_else(|| {
                            crate::storage::IssueNotFoundError::new(light.target.id.clone())
                        })?;
                    let declarations = crate::repository_state::declarations_from_image(&image)?;
                    let registry = declarations.gates;
                    // A precheck validating the whole repository runs over
                    // the validation base.
                    let image = match has_broad_precheck(&issue, &registry) {
                        false => image,
                        true => match self.capture_proposed_base(
                            preflight.as_mut(),
                            &std::collections::BTreeMap::new(),
                            &[],
                            Some(image),
                        )? {
                            Some(image) => image,
                            None => return Ok(AttemptOutcome::Retry),
                        },
                    };
                    let plan = captured_precheck_plan(&image, &issue, &registry)?;
                    Some(PrecheckPreflight {
                        plan,
                        image,
                        issue,
                        registry,
                    })
                } else {
                    None
                };
                (light.target.id, should_precheck, precheck)
            };
            if let (true, Some(_)) = (coordinate_claims, &precheck) {
                // Refuse before running prechecks; the held guard repeats this.
                let preliminary_guard = claims_mutation_guard(self.require_worktree_paths()?)?;
                if let CapturedLifecycleMutation::Claim { assignee, .. } = &request {
                    ensure_claim_available(preliminary_guard.as_ref(), &expected_target, assignee)?;
                }
            }
            let expected_precheck = precheck.as_ref().map(|precheck| &precheck.plan);
            if let Some(precheck) = &precheck {
                let reuse = cached_precheck
                    .as_ref()
                    .is_some_and(|cached| cached.target_id == expected_target);
                if !reuse {
                    cached_precheck = Some(CachedPrecheckExecution {
                        target_id: expected_target.clone(),
                        evidence: precheck.plan.evidence.clone(),
                        execution: self.execute_captured_prechecks(
                            &precheck.image,
                            &precheck.issue,
                            &precheck.registry,
                            &precheck.plan.prompts,
                            &context,
                        )?,
                    });
                }
            }
            let mut precheck_runs = cached_precheck
                .as_ref()
                .filter(|_| expected_precheck.is_some())
                .map(|cached| cached.execution.runs.clone())
                .unwrap_or_default();
            let precheck_has_error = cached_precheck
                .as_ref()
                .filter(|_| expected_precheck.is_some())
                .is_some_and(|cached| cached.execution.error.is_some());
            let precheck_run_paths =
                gate_run_record_paths(&context, &expected_target, precheck_runs.len());

            let claims_guard = coordinate_claims
                .then(|| claims_mutation_guard(self.require_worktree_paths()?))
                .transpose()?
                .flatten();

            let mut session = self.storage.open_mutation_session(layout.clone())?;
            let Some(crate::commands::validate::MutationBase { image, issues }) = self
                .capture_mutation_base(
                    session.as_mut(),
                    &precheck_run_paths,
                    Some(request.issue_id()),
                )?
            else {
                return Ok(AttemptOutcome::Retry);
            };
            if require_issue_in_capture(&issues, request.issue_id())? != expected_target {
                return Ok(AttemptOutcome::Retry);
            }
            let mut issue = issues
                .iter()
                .find(|issue| issue.id == expected_target)
                .cloned()
                .ok_or_else(|| crate::storage::IssueNotFoundError::new(expected_target.clone()))?;
            let declarations = crate::repository_state::declarations_from_image(&image)?;
            let current_precheck_required = lifecycle_requires_prechecks(&request, &issue);
            if current_precheck_required != expected_precheck_required {
                return Ok(AttemptOutcome::Retry);
            }
            let precheck_inputs_moved = if let Some(cached) = cached_precheck.as_ref() {
                let changed_since_precheck_capture = expected_precheck
                    .as_ref()
                    .is_some_and(|plan| plan.evidence != cached.evidence);
                if changed_since_precheck_capture {
                    true
                } else if cached.evidence.validation_view.is_some() {
                    !cached.evidence.matches(&image)?
                } else {
                    captured_precheck_plan(&image, &issue, &declarations.gates)?.evidence
                        != cached.evidence
                }
            } else {
                false
            };
            if precheck_inputs_moved {
                precheck_runs.iter_mut().for_each(|run| {
                    run.evaluation_stability = GateEvaluationStability::InputsMoved;
                });
            }
            if let CapturedLifecycleMutation::Claim { assignee, .. } = &request {
                ensure_claim_available(claims_guard.as_ref(), &expected_target, assignee)?;
            }
            let mut precheck_events = Vec::new();
            if expected_precheck.is_some() {
                for result in &precheck_runs {
                    let state = gate_check::gate_state_from_run(result);
                    let by = result.by.clone();
                    let status = state.status;
                    issue.gates_status.insert(result.gate_key.clone(), state);
                    precheck_events.push((
                        1,
                        if status == GateStatus::Passed {
                            EventBody::draft_gate_passed(
                                issue.id.clone(),
                                result.gate_key.clone(),
                                by,
                            )
                        } else {
                            EventBody::draft_gate_failed(
                                issue.id.clone(),
                                result.gate_key.clone(),
                                by,
                            )
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
                let plan = finalize(&layout, &image, &context, &intents, None)?;
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
            let target = match &request {
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
                                issue,
                                changed: false,
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
                    if issue.state == State::Ready {
                        State::InProgress
                    } else {
                        issue.state
                    }
                }
                CapturedLifecycleMutation::Release { .. } => {
                    if issue.state == State::InProgress {
                        State::Ready
                    } else {
                        issue.state
                    }
                }
                CapturedLifecycleMutation::AutoReady { .. } => {
                    let resolved = crate::domain::queries::build_issue_map(&issues);
                    if !issue.should_auto_transition_to_ready(&resolved) {
                        return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                            issue,
                            changed: false,
                        }));
                    }
                    State::Ready
                }
                CapturedLifecycleMutation::AutoDone { .. } => {
                    if !issue.should_auto_transition_to_done() {
                        return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                            issue,
                            changed: false,
                        }));
                    }
                    State::Done
                }
            };
            let graph = CapturedGraphEvidence::capture(&image, &issues, &context)?;
            let (mut warnings, mut events, mut update, blocked) = if issue.state == target {
                (Vec::new(), Vec::new(), None, None)
            } else {
                match derive_state_transition(
                    issue.clone(),
                    target,
                    false,
                    CapturedTransitionEvidence {
                        issues: &issues,
                        graph: &graph,
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
            if blocked.is_none() {
                let record = update.as_mut().unwrap_or(&mut issue);
                match &request {
                    CapturedLifecycleMutation::Claim { assignee, .. } => {
                        if record.assignee.as_ref() != Some(assignee) {
                            record.assignee = Some(assignee.clone());
                            events.push((
                                4,
                                EventBody::draft_issue_claimed(record.id.clone(), assignee.clone()),
                            ));
                            if update.is_none() {
                                update = Some(issue);
                            }
                        }
                    }
                    CapturedLifecycleMutation::Release {
                        reason,
                        released_by,
                        ..
                    } => {
                        if let Some(assignee) = record.assignee.take() {
                            events.push((
                                4,
                                EventBody::draft_issue_released(
                                    record.id.clone(),
                                    assignee,
                                    reason.clone(),
                                    released_by.clone(),
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
            let issue_result = crate::repository_state::mutated_issue(
                &image,
                &expected_target,
                &intents,
                context.timestamp(),
            )?;
            if intents.is_empty() {
                return Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                    issue: issue_result,
                    changed,
                }));
            }
            let plan = finalize(&layout, &image, &context, &intents, None)?;
            if let AttemptOutcome::Retry = classify_apply(session.apply(&plan), ())? {
                return Ok(AttemptOutcome::Retry);
            }
            if let Some(error) = blocked {
                return Err(error.with_warnings(std::mem::take(&mut warnings)).into());
            }
            Ok(AttemptOutcome::Done(CapturedLifecycleOutcome {
                issue: issue_result,
                changed,
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

    /// Refuse a write to any of `targets` another agent holds an active lease on
    /// ([`refuse_foreign_leases`]), by the repository's enforcement mode.
    pub(crate) fn refuse_foreign_leases(
        &self,
        targets: &[IssueId],
    ) -> Result<Vec<(IssueId, String)>> {
        let mode = self.cached_config()?.enforcement_mode();
        if mode == crate::config::EnforcementMode::Off {
            return Ok(Vec::new());
        }
        refuse_foreign_leases_read(self.require_worktree_paths()?, mode, targets)
    }

    /// [`Self::refuse_foreign_leases`] for one issue, answering its warnings.
    pub(crate) fn refuse_foreign_lease(&self, issue_id: &IssueId) -> Result<Vec<String>> {
        Ok(self
            .refuse_foreign_leases(std::slice::from_ref(issue_id))?
            .into_iter()
            .map(|(_, warning)| warning)
            .collect())
    }
}

// Helper functions for parsing command-line arguments
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::EventBody;

    #[test]
    fn test_claims_mutation_guard_rejects_invalid_root_path_fallback_and_names_source() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("worktree with spaces");
        let paths = crate::test_utils::primary_checkout_paths(&root);
        let error = claims_mutation_guard(&paths).err().unwrap();
        assert!(error.to_string().contains("root path"));
        assert!(error.to_string().contains(root.to_str().unwrap()));
        assert!(error
            .downcast_ref::<crate::domain::identity::InvalidWorktreeIdError>()
            .is_some());
    }

    #[test]
    fn test_derive_state_transition_preserves_invalid_hierarchy_error() {
        use crate::declarations::rules::{Rule, RuleScope, RuleSet, Selector, Severity};

        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let type_name = taxonomy.type_at_level(4).to_string();
        let mut issue = crate::test_utils::fixture_issue("Transition".into(), String::new());
        issue.labels = vec![format!("type:{type_name}")];

        let config_text = crate::test_taxonomy::invalid_hierarchy_config_fragment();
        let config: JitConfig = toml::from_str(&config_text).unwrap();
        let rules = RuleSet {
            rules: vec![Rule {
                name: "graph-probe".to_string(),
                origin: None,
                description: None,
                when: Selector {
                    type_: Some(type_name),
                    ..Selector::default()
                },
                severity: Severity::Error,
                enforce: false,
                assert: RuleSet::parse(
                    "[[rules]]\nname = \"coverage\"\nassert = { label-coverage = {} }",
                    None,
                    [],
                )
                .unwrap()
                .rules[0]
                    .assert
                    .clone(),
                scope: RuleScope::Graph,
            }],
        };
        let declarations = crate::repository_state::CapturedRepositoryDeclarations::from_parts(
            config,
            crate::declarations::GateRegistry::default(),
            rules,
        );
        let issues = [issue.clone()];
        let context = production_mutation_context();
        let graph = CapturedGraphEvidence {
            declarations,
            plan_content: std::collections::HashMap::new(),
            context: &context,
        };

        let error = derive_state_transition(
            issue,
            crate::domain::State::InProgress,
            false,
            CapturedTransitionEvidence {
                issues: &issues,
                graph: &graph,
            },
        )
        .err()
        .expect("invalid hierarchy must abort the transition");

        assert_eq!(
            error.downcast_ref::<crate::domain::type_taxonomy::ConfigError>(),
            Some(&crate::domain::type_taxonomy::ConfigError::InvalidLevel(0))
        );
    }

    #[test]
    fn test_production_mutation_context_factory_defaults_to_empty_annotation() {
        assert!(production_mutation_context()
            .dispatch_annotation()
            .is_none());
    }

    /// A dispatch-scoped override fact, as the linked-checkout write guard installs it.
    fn override_annotation() -> crate::repository_state::MutationContextAnnotation {
        crate::repository_state::MutationContextAnnotation::LinkedCheckoutWriteOverridden {
            checkout: std::path::PathBuf::from("/repo/.worktrees/feature"),
            declared_stance: crate::domain::LinkedCheckoutWriteStance::Refuse,
            invocation_override: crate::domain::LinkedCheckoutWriteStance::Allow,
        }
    }

    #[test]
    fn test_dispatch_mutation_annotation_scope_restores_default_after_return_and_unwind() {
        assert!(production_mutation_context()
            .dispatch_annotation()
            .is_none());

        with_dispatch_mutation_annotation(override_annotation(), || {
            assert_eq!(
                production_mutation_context().dispatch_annotation(),
                Some(&override_annotation())
            );
            with_dispatch_mutation_annotation(override_annotation(), || {
                assert!(production_mutation_context()
                    .dispatch_annotation()
                    .is_some());
            });
            assert!(production_mutation_context()
                .dispatch_annotation()
                .is_some());
            std::thread::spawn(|| {
                assert!(production_mutation_context()
                    .dispatch_annotation()
                    .is_none());
            })
            .join()
            .unwrap();
        });
        assert!(production_mutation_context()
            .dispatch_annotation()
            .is_none());

        let unwound = std::panic::catch_unwind(|| {
            with_dispatch_mutation_annotation(override_annotation(), || {
                panic!("scope cleanup probe")
            });
        });
        assert!(unwound.is_err());
        assert!(production_mutation_context()
            .dispatch_annotation()
            .is_none());
    }

    #[test]
    fn test_create_issue_under_dispatch_override_persists_the_audit_record_with_the_issue_events() {
        use crate::domain::{LinkedCheckoutWriteStance, Priority};
        use crate::storage::IssueStore;
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let storage = crate::storage::InMemoryStorage::new();
        let executor = crate::commands::test_helpers::memory_executor_declaring(
            storage.clone(),
            crate::commands::test_helpers::MemoryDeclarations {
                config: Some(&taxonomy.config_fragment()),
                ..Default::default()
            },
        );
        let events_before = storage
            .read_events()
            .expect("read the event log before the permitted mutation");
        let annotation = override_annotation();
        let expected_checkout = std::path::PathBuf::from("/repo/.worktrees/feature");
        let expected_declared_stance = LinkedCheckoutWriteStance::Refuse;
        let expected_invocation_override = LinkedCheckoutWriteStance::Allow;

        let issue_id = with_dispatch_mutation_annotation(annotation, || {
            executor.issues().create_issue(
                "Permitted mutation".to_string(),
                "Issue created while an override permitted the invocation".to_string(),
                Priority::Normal,
                Vec::new(),
                vec![crate::test_taxonomy::type_label(&taxonomy, 4)],
                None,
                None,
                false,
            )
        })
        .expect("the permitted mutation succeeds")
        .issue
        .id;

        let events_after = storage
            .read_events()
            .expect("read the event log after the permitted mutation");
        assert!(
            events_after.starts_with(&events_before),
            "the permitted mutation must preserve the prior event-log prefix"
        );
        let appended = &events_after[events_before.len()..];
        let override_records = appended
            .iter()
            .filter(|event| {
                matches!(
                    event.body(),
                    EventBody::LinkedCheckoutWriteOverridden { .. }
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            override_records.len(),
            1,
            "the permitted mutation must append exactly one linked-checkout override audit record"
        );
        let override_event = override_records[0];
        let EventBody::LinkedCheckoutWriteOverridden {
            checkout,
            declared_stance,
            invocation_override,
            ..
        } = override_event.body()
        else {
            unreachable!("the selected event is the linked-checkout override audit record");
        };
        assert_eq!(
            checkout, &expected_checkout,
            "the audit record must name the checkout selected by the dispatch site"
        );
        assert_eq!(
            declared_stance, &expected_declared_stance,
            "the audit record must preserve the declared linked-checkout write stance"
        );
        assert_eq!(
            invocation_override, &expected_invocation_override,
            "the audit record must preserve the invocation stance that permitted the mutation"
        );
        assert!(
            !override_event.id().is_empty(),
            "the finalizer must assign a non-empty identifier to the audit record"
        );

        let issue_created_timestamp = appended
            .iter()
            .find_map(|event| match event.body() {
                EventBody::IssueCreated {
                    issue_id: event_issue_id,
                    ..
                } if event_issue_id == &issue_id => Some(event.timestamp()),
                _ => None,
            })
            .expect("the issue-created event must be in the same append as the audit record");
        assert_eq!(
            override_event.timestamp(),
            issue_created_timestamp,
            "the audit record and issue-created event must share one mutation timestamp"
        );
        assert!(
            matches!(
                appended.last(),
                Some(event) if matches!(event.body(), EventBody::LinkedCheckoutWriteOverridden { .. })
            ),
            "canonical event ordering must place the override audit record last in the append"
        );
        storage
            .load_issue(&issue_id)
            .expect("the mutation explained by the audit record must be durable");
    }

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
        let occupied_issue_path = VirtualPath::issue_record(&crate::test_utils::issue_id(1));
        storage.add_data_file(occupied_issue_path.relative().as_str(), "{}");
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

    /// Seed a ready issue with one automated and one passed manual precheck gate.
    fn seed_precheck_evidence(storage: &crate::storage::InMemoryStorage) -> IssueId {
        use crate::declarations::{GateChecker, GateDefinition, GateRegistry, GateStage};
        use std::collections::HashMap;

        storage.add_worktree_file("start-prompt.md", "captured prompt");
        let mut registry = GateRegistry::default();
        registry.gates.insert(
            "auto-start".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "auto-start".parse().unwrap(),
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
                inputs: None,
            },
        );
        registry.gates.insert(
            "manual-start".parse().unwrap(),
            GateDefinition {
                version: 1,
                key: "manual-start".parse().unwrap(),
                title: "Manual start".to_string(),
                description: String::new(),
                stage: GateStage::Precheck,
                mode: GateMode::Manual,
                checker: None,
                priority: 2,
                reserved: HashMap::new(),
                inputs: None,
            },
        );
        crate::commands::test_helpers::seed_gate_registry(storage, &registry);
        let mut issue = crate::test_utils::fixture_issue("checked".to_string(), String::new());
        issue.state = State::Ready;
        issue.gates_required = vec![
            "auto-start".parse().unwrap(),
            "manual-start".parse().unwrap(),
        ];
        issue.gates_status.insert(
            "manual-start".parse().unwrap(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: chrono::Utc::now(),
                run_id: None,
            },
        );
        let issue_id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(storage, issue);
        issue_id
    }

    fn capture_precheck_plan(
        executor: &CommandExecutor<crate::storage::InMemoryStorage>,
        issue_id: &IssueId,
    ) -> CapturedPrecheckPlan {
        use crate::storage::RepositoryStateStore;
        let mut session = executor
            .storage
            .open_mutation_session(executor.require_layout().unwrap())
            .unwrap();
        let crate::commands::validate::MutationBase { image, issues } = executor
            .capture_mutation_base(session.as_mut(), &[], Some(issue_id))
            .unwrap()
            .unwrap();
        let issue = issues
            .into_iter()
            .find(|issue| &issue.id == issue_id)
            .unwrap();
        let registry = crate::repository_state::declarations_from_image(&image)
            .unwrap()
            .gates;
        captured_precheck_plan(&image, &issue, &registry).unwrap()
    }

    fn prior_precheck_run(issue_id: &IssueId) -> crate::domain::GateRunResult {
        crate::domain::GateRunResult {
            schema_version: crate::domain::GATE_RUN_SCHEMA_VERSION,
            run_id: crate::test_utils::gate_run_id(1),
            gate_key: "auto-start".parse().unwrap(),
            stage: crate::declarations::GateStage::Precheck,
            issue_id: issue_id.clone(),
            commit: None,
            branch: None,
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
            evaluation_stability: GateEvaluationStability::Stable,
        }
    }

    fn captured_issue_spec(id: &IssueId) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        let paths = std::collections::BTreeSet::from([VirtualPath::issue_record(id)]);
        let mut spec = CaptureSpec::new(
            paths,
            CaptureBudget {
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_tail(VirtualPath::EVENTS).unwrap();
        spec
    }

    /// The capture of an issue mutation over `id` whose gate-run records
    /// `context` derives ids for.
    fn captured_gate_issue_spec(
        id: &IssueId,
        context: &crate::repository_state::MutationContext,
    ) -> crate::repository_state::CaptureSpec {
        use crate::repository_state::{CaptureBudget, CaptureSpec, VirtualPath};
        let mut paths = std::collections::BTreeSet::from([
            VirtualPath::issue_record(id),
            VirtualPath::data("gates.toml").unwrap(),
        ]);
        paths.extend(gate_run_record_paths(context, id, 1));
        let mut spec = CaptureSpec::new(
            paths,
            CaptureBudget {
                max_listings: 0,
                max_bytes: 1024 * 1024,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_tail(VirtualPath::EVENTS).unwrap();
        spec
    }

    fn issue_from_image(image: &crate::repository_state::RepositoryImage, id: &IssueId) -> Issue {
        let path = crate::repository_state::VirtualPath::issue_record(id);
        serde_json::from_slice(image.file_bytes(&path).unwrap().unwrap()).unwrap()
    }

    fn event_bytes(plan: &crate::repository_state::MaterializationPlan) -> Vec<u8> {
        plan.delta().appended_event_bytes().unwrap().to_vec()
    }

    fn manual_gate_registry(key: &GateKey) -> crate::declarations::GateRegistry {
        use crate::declarations::{GateDefinition, GateMode, GateRegistry, GateStage};
        let definition = GateDefinition {
            version: 1,
            key: key.clone(),
            title: key.to_string(),
            description: String::new(),
            stage: GateStage::Postcheck,
            mode: GateMode::Manual,
            checker: None,
            priority: 100,
            reserved: std::collections::HashMap::new(),
            inputs: None,
        };
        GateRegistry {
            gates: std::collections::HashMap::from([(key.clone(), definition)]),
        }
    }

    fn gate_registry_from_image(
        image: &crate::repository_state::RepositoryImage,
    ) -> crate::declarations::GateRegistry {
        let path = crate::repository_state::VirtualPath::data("gates.toml").unwrap();
        crate::declarations::parse_gate_registry(image.file_bytes(&path).unwrap().unwrap()).unwrap()
    }

    #[test]
    fn test_captured_issue_retry_preserves_unrelated_change() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{InMemoryStorage, RepositoryStateStore, RepositoryStateStoreError};

        let storage = InMemoryStorage::new();
        let issue = crate::test_utils::fixture_issue(
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
            &context,
        )
        .unwrap();
        let first_plan = finalize(&layout, &first_image, &context, &first.intents, None).unwrap();

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
            &context,
        )
        .unwrap();
        let retry_plan = finalize(&layout, &retry_image, &context, &retry.intents, None).unwrap();
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
        let mut issue = crate::test_utils::fixture_issue(
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
            &crate::repository_state::MutationContext::preview(),
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
    fn test_manual_gate_evidence_repeats_with_fresh_identity_and_time() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{InMemoryStorage, RepositoryStateStore};

        for status in [GateStatus::Passed, GateStatus::Failed] {
            let storage = InMemoryStorage::new();
            crate::commands::test_helpers::seed_gate_registry(
                &storage,
                &manual_gate_registry(&"review".parse().unwrap()),
            );
            let mut issue = crate::test_utils::fixture_issue(
                format!("repeat-{status:?}"),
                "Repeated evidence".to_string(),
            );
            let id = issue.id.clone();
            let actor: crate::domain::Assignee = "agent:reviewer".parse().unwrap();
            issue.gates_required.push("review".parse().unwrap());
            issue.gates_status.insert(
                "review".parse().unwrap(),
                GateState {
                    status,
                    updated_by: Some(actor.clone()),
                    updated_at: chrono::DateTime::UNIX_EPOCH,
                    run_id: None,
                },
            );
            crate::commands::test_helpers::seed_issue(&storage, issue);
            let layout = storage.repository_layout();
            let request = CapturedIssueMutation::SetManualGateStatus {
                issue_id: id.clone(),
                gate_key: "review".parse().unwrap(),
                status,
                by: Some(actor.clone()),
            };
            let first_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T10:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc);
            let first_context = MutationContext::deterministic([31; 32], first_time);
            let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
            let first_image = first_session
                .capture(captured_gate_issue_spec(&id, &first_context))
                .unwrap();
            let first = derive_captured_issue_mutation(
                issue_from_image(&first_image, &id),
                &gate_registry_from_image(&first_image),
                &request,
                &first_context,
            )
            .unwrap();
            let first_plan =
                finalize(&layout, &first_image, &first_context, &first.intents, None).unwrap();
            first_session.apply(&first_plan).unwrap();
            drop(first_session);
            assert_eq!(
                storage.load_issue(&id).unwrap().gates_status[&"review".parse().unwrap()]
                    .updated_at,
                first_time
            );

            let second_time = chrono::DateTime::parse_from_rfc3339("2026-07-20T11:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc);
            let second_context = MutationContext::deterministic([32; 32], second_time);
            let mut second_session = storage.open_mutation_session(layout.clone()).unwrap();
            let second_image = second_session
                .capture(captured_gate_issue_spec(&id, &second_context))
                .unwrap();
            let second = derive_captured_issue_mutation(
                issue_from_image(&second_image, &id),
                &gate_registry_from_image(&second_image),
                &request,
                &second_context,
            )
            .unwrap();
            let second_plan = finalize(
                &layout,
                &second_image,
                &second_context,
                &second.intents,
                None,
            )
            .unwrap();
            assert_ne!(event_bytes(&first_plan), event_bytes(&second_plan));
            second_session.apply(&second_plan).unwrap();

            assert_eq!(
                storage.load_issue(&id).unwrap().gates_status[&"review".parse().unwrap()]
                    .updated_at,
                second_time
            );
            let events = storage.read_events().unwrap();
            assert_eq!(events.len(), 2);
            let evidence = events
                .iter()
                .map(|event| match (status, event.body()) {
                    (GateStatus::Passed, EventBody::GatePassed { updated_by, .. })
                    | (GateStatus::Failed, EventBody::GateFailed { updated_by, .. })
                        if updated_by.as_ref() == Some(&actor) =>
                    {
                        (event.id(), event.timestamp())
                    }
                    _ => panic!("unexpected manual gate evidence: {event:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(evidence[0].1, first_time);
            assert_eq!(evidence[1].1, second_time);
            assert_ne!(evidence[0].0, evidence[1].0);
            let mut attested_at = storage
                .list_gate_runs_for_issue(&id)
                .unwrap()
                .into_iter()
                .map(|run| run.started_at)
                .collect::<Vec<_>>();
            attested_at.sort();
            assert_eq!(
                attested_at,
                vec![first_time, second_time],
                "each attestation's run carries its own operation's time"
            );
        }
    }

    #[test]
    fn test_manual_gate_pending_status_is_rejected_before_other_validation() {
        let issue = crate::test_utils::fixture_issue(
            "pending-precedence".to_string(),
            "Pending precedence".to_string(),
        );
        let request = CapturedIssueMutation::SetManualGateStatus {
            issue_id: issue.id.clone(),
            gate_key: "missing-and-not-required".parse().unwrap(),
            status: GateStatus::Pending,
            by: None,
        };

        let error = match derive_captured_issue_mutation(
            issue,
            &crate::declarations::GateRegistry::default(),
            &request,
            &crate::repository_state::MutationContext::preview(),
        ) {
            Ok(_) => panic!("pending status unexpectedly accepted"),
            Err(error) => error,
        };

        assert_eq!(
            error.to_string(),
            "manual gate status must be passed or failed"
        );
    }

    /// The gate registry is evidence a gate addition reads and never writes, so
    /// a declaration removed after capture is caught by the next session's own
    /// derivation rather than by the first session's publication — the cost
    /// `@/charter/D-17` names.
    #[test]
    fn test_gate_add_retry_rejects_removed_registry_declaration() {
        use crate::repository_state::{finalize, MutationContext};
        use crate::storage::{GateNotFoundError, InMemoryStorage, RepositoryStateStore};

        let storage = InMemoryStorage::new();
        crate::commands::test_helpers::seed_gate_registry(
            &storage,
            &manual_gate_registry(&"review".parse().unwrap()),
        );
        let issue = crate::test_utils::fixture_issue(
            "registry-race".to_string(),
            "Registry race".to_string(),
        );
        let id = issue.id.clone();
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let layout = storage.repository_layout();
        let request = CapturedIssueMutation::AddGates {
            issue_id: id.clone(),
            gate_keys: vec!["review".parse().unwrap()],
        };
        let context = MutationContext::deterministic(
            [41; 32],
            chrono::DateTime::parse_from_rfc3339("2026-07-20T12:00:00Z")
                .unwrap()
                .with_timezone(&chrono::Utc),
        );

        let mut first_session = storage.open_mutation_session(layout.clone()).unwrap();
        let first_image = first_session
            .capture(captured_gate_issue_spec(&id, &context))
            .unwrap();
        let first = derive_captured_issue_mutation(
            issue_from_image(&first_image, &id),
            &gate_registry_from_image(&first_image),
            &request,
            &context,
        )
        .unwrap();
        drop(finalize(&layout, &first_image, &context, &first.intents, None).unwrap());
        drop(first_session);

        crate::commands::test_helpers::seed_gate_registry(
            &storage,
            &crate::declarations::GateRegistry::default(),
        );

        let mut retry_session = storage.open_mutation_session(layout).unwrap();
        let retry_image = retry_session
            .capture(captured_gate_issue_spec(&id, &context))
            .unwrap();
        let error = match derive_captured_issue_mutation(
            issue_from_image(&retry_image, &id),
            &gate_registry_from_image(&retry_image),
            &request,
            &context,
        ) {
            Ok(_) => panic!("retry must revalidate against the changed registry"),
            Err(error) => error,
        };
        assert_eq!(
            error.downcast_ref::<GateNotFoundError>(),
            Some(&GateNotFoundError::Batch(vec!["review".parse().unwrap()]))
        );
        assert!(storage.load_issue(&id).unwrap().gates_required.is_empty());
        assert!(storage.read_events().unwrap().is_empty());
    }

    #[test]
    fn test_rules_are_parsed_once_and_cached() {
        use crate::declarations::rules::Assertion;
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        let layout = storage.repository_layout();
        let rules_path = layout
            .resolve(&crate::repository_state::VirtualPath::RULES)
            .unwrap();
        std::fs::create_dir_all(rules_path.parent().unwrap()).unwrap();
        std::fs::write(
            &rules_path,
            r#"
[[rules]]
name = "first"
assert = { require-section = { heading = "Goals" } }
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage).with_layout(layout);

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
        let layout = storage.repository_layout();
        let config_path = layout
            .resolve(&crate::repository_state::VirtualPath::CONFIG)
            .unwrap();
        std::fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        std::fs::write(
            &config_path,
            r#"
[namespaces.req]
description = "Requirement tags"
unique = false
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage).with_layout(layout);

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
        let layout = storage.repository_layout();
        // No rules.toml written.

        let executor = CommandExecutor::new(storage).with_layout(layout);
        let rules = executor.rules().expect("missing file is not an error");
        assert!(rules.rules.is_empty());
    }

    #[test]
    fn test_rules_malformed_file_yields_err() {
        use crate::storage::InMemoryStorage;

        let storage = InMemoryStorage::new();
        let layout = storage.repository_layout();
        let rules_path = layout
            .resolve(&crate::repository_state::VirtualPath::RULES)
            .unwrap();
        std::fs::create_dir_all(rules_path.parent().unwrap()).unwrap();

        // A rule whose assert table has no kind is a genuine config error and
        // must NOT be downgraded to an empty (no-rules) set.
        std::fs::write(
            rules_path,
            r#"
[[rules]]
name = "broken"
assert = {}
"#,
        )
        .unwrap();

        let executor = CommandExecutor::new(storage).with_layout(layout);
        assert!(
            executor.rules().is_err(),
            "malformed rules.toml must surface an error, not an empty set"
        );
    }

    // Enforcement tests
    #[test]
    fn test_precheck_evidence_detects_manual_status_mutation() {
        use crate::storage::IssueStore;
        let storage = crate::storage::InMemoryStorage::new();
        let issue_id = seed_precheck_evidence(&storage);
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());
        let before = capture_precheck_plan(&executor, &issue_id).evidence;

        let mut issue = storage.load_issue(&issue_id).unwrap();
        issue
            .gates_status
            .get_mut(&"manual-start".parse().unwrap())
            .unwrap()
            .status = GateStatus::Failed;
        crate::commands::test_helpers::seed_issue(&storage, issue);

        assert_ne!(capture_precheck_plan(&executor, &issue_id).evidence, before);
    }

    #[test]
    fn test_precheck_evidence_detects_named_run_mutation() {
        let storage = crate::storage::InMemoryStorage::new();
        let issue_id = seed_precheck_evidence(&storage);
        let run = prior_precheck_run(&issue_id);
        let mut issue = storage.load_issue(&issue_id).unwrap();
        issue.gates_status.insert(
            run.gate_key.clone(),
            crate::commands::gate_check::gate_state_from_run(&run),
        );
        crate::commands::test_helpers::seed_issue(&storage, issue);
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());
        let before = capture_precheck_plan(&executor, &issue_id).evidence;
        storage.add_data_file(
            crate::repository_state::VirtualPath::gate_run_record(&run.issue_id, &run.run_id)
                .relative()
                .as_str(),
            &serde_json::to_string(&run).unwrap(),
        );

        assert_ne!(capture_precheck_plan(&executor, &issue_id).evidence, before);
    }

    #[test]
    fn test_precheck_evidence_detects_prompt_mutation_and_preserves_captured_bytes() {
        let storage = crate::storage::InMemoryStorage::new();
        let issue_id = seed_precheck_evidence(&storage);
        let executor = crate::commands::test_helpers::memory_executor(storage.clone());
        let before = capture_precheck_plan(&executor, &issue_id);
        assert_eq!(
            before.prompts[&"auto-start".parse().unwrap()],
            "captured prompt"
        );

        storage.add_worktree_file("start-prompt.md", "mutated prompt");
        let after = capture_precheck_plan(&executor, &issue_id);

        assert_eq!(
            after.prompts[&"auto-start".parse().unwrap()],
            "mutated prompt"
        );
        assert_ne!(after.evidence, before.evidence);
    }

    #[test]
    fn test_claims_guard_uses_explicit_layout_not_process_cwd() {
        use crate::storage::worktree_paths::WorktreePaths;
        use crate::storage::{ClaimCoordinator, FileLocker};
        use std::time::Duration;

        let repo = tempfile::TempDir::new().unwrap();
        std::fs::create_dir(repo.path().join(".jit")).unwrap();
        crate::test_cli::init_git_repository(repo.path()).unwrap();
        let paths = WorktreePaths::detect_from(repo.path()).unwrap();
        let coordinator = ClaimCoordinator::new(
            paths.clone(),
            FileLocker::new(Duration::from_secs(1)),
            "wt:test".parse().unwrap(),
            "agent:test".parse().unwrap(),
        );
        coordinator.init().unwrap();
        let issue_id = crate::test_utils::issue_id(0x4000_8000_0000_0000_0001_0f0e_0d0c);
        coordinator
            .acquire_claim_synchronized(
                &issue_id,
                600,
                Some("test lease"),
                crate::storage::claim_coordinator::ClaimAcquireLimits::new(100, 100),
                |_| Ok(()),
            )
            .unwrap();
        let guard = claims_mutation_guard(&paths).unwrap().unwrap();

        let stranger = "agent:stranger".parse().unwrap();
        assert!(guard
            .conflicting_lease(&issue_id, Some(&stranger))
            .unwrap()
            .is_some());
    }
}

#[cfg(test)]
mod mutation_session_contract_tests {
    use super::*;
    use crate::storage::RepositoryStateStoreError;

    fn retryable() -> RepositoryStateStoreError {
        let record =
            crate::repository_state::VirtualPath::issue_record(&crate::test_utils::issue_id(1));
        RepositoryStateStoreError::RetryableConflict {
            path: record.relative().as_str().to_owned(),
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
            max_listings: 0,
            max_bytes: 1024,
            max_depth: 6,
        };
        let image = session
            .capture(CaptureSpec::new([VirtualPath::data("config.toml").unwrap()], budget).unwrap())
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
        assert_eq!(attempts, 3, "each attempt opens one fresh mutation session");
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
