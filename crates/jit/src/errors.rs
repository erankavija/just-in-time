//! Actionable error formatting for improved user experience.
//!
//! This module provides utilities for creating error messages with:
//! - Clear error description
//! - Possible causes (diagnostics)
//! - Remediation steps (actionable fixes)
//!
//! Designed to help users understand what went wrong and how to fix it.

use crate::domain::identity::GateKey;
use std::fmt;

use crate::declarations::GateMode;
use crate::domain::identity::IssueId;
use crate::domain::{GateStatus, Issue, State};

/// An error with diagnostic context and remediation steps.
///
/// This struct wraps an error message with additional context to help users
/// diagnose and fix the problem.
#[derive(Debug, Clone)]
pub struct ActionableError {
    /// The main error message
    error: String,
    /// Possible causes (diagnostic hints)
    causes: Vec<String>,
    /// Remediation steps (how to fix)
    remediation: Vec<String>,
}

impl ActionableError {
    /// Create a new actionable error with the given message.
    pub fn new(error: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            causes: Vec::new(),
            remediation: Vec::new(),
        }
    }

    /// Add a possible cause (diagnostic hint).
    ///
    /// This helps users understand why the error might have occurred.
    pub fn with_cause(mut self, cause: impl Into<String>) -> Self {
        self.causes.push(cause.into());
        self
    }

    /// Add a remediation step (actionable fix).
    ///
    /// This tells users what they can do to fix the problem.
    pub fn with_remedy(mut self, remedy: impl Into<String>) -> Self {
        self.remediation.push(remedy.into());
        self
    }

    /// Convert to a formatted error message suitable for display.
    ///
    /// Renders the error text verbatim, with no "Error: " prefix: the single
    /// prefix is added exactly once, by the top-level CLI printer
    /// (`main::main`), so wrapping this in another error type (as
    /// [`RequiresGitError`] and [`LeaseNotFoundError`] do) never produces
    /// a doubled "Error: Error: " line.
    pub fn to_error_message(&self) -> String {
        let mut msg = format!("{}\n", self.error);

        if !self.causes.is_empty() {
            msg.push_str("\nPossible causes:\n");
            for cause in &self.causes {
                msg.push_str(&format!("  • {}\n", cause));
            }
        }

        if !self.remediation.is_empty() {
            msg.push_str("\nTo fix:\n");
            for remedy in &self.remediation {
                msg.push_str(&format!("  • {}\n", remedy));
            }
        }

        msg
    }
}

impl fmt::Display for ActionableError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_error_message())
    }
}

impl std::error::Error for ActionableError {}

/// A user-supplied argument or configuration value that failed to parse or
/// validate.
///
/// This is the shared typed carrier for "invalid argument" conditions that were
/// previously raised as bare `anyhow!("Invalid ...")` strings. It carries the
/// original human-readable message so `Display` is byte-for-byte unchanged from
/// the old form; the CLI downcasts to this type to classify the failure as an
/// invalid-argument condition (exit code `2`) without scanning the message text.
///
/// Use [`InvalidArgumentError::new`] at every origin whose error message the CLI
/// maps to exit code `2` (enum `FromStr` parse failures, config parse failures,
/// malformed CLI values), so the classification is driven by type rather than by
/// substring matching.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InvalidArgumentError {
    message: String,
}

impl InvalidArgumentError {
    /// Build an [`InvalidArgumentError`] carrying the user-facing message.
    ///
    /// The message is reproduced verbatim by `Display`, so passing the exact
    /// string that the previous `anyhow!` origin produced keeps user-facing
    /// output unchanged.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The user-facing message describing what was invalid.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A malformed label filter supplied to a query command.
///
/// This dedicated type preserves the established `INVALID_LABEL_PATTERN`
/// machine-readable code while allowing the top-level CLI classifier to select
/// it structurally. Its display text remains the exact validation message
/// produced by [`crate::labels::validate_label_pattern`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InvalidLabelPatternError {
    message: String,
}

impl InvalidLabelPatternError {
    /// Build an [`InvalidLabelPatternError`] carrying the user-facing message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The user-facing explanation of the malformed label filter.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A generic "resource not found" failure carrying its user-facing message.
///
/// This is the shared typed carrier for the long tail of lookup failures that do
/// not warrant a bespoke type (document references, gate keys, plan documents,
/// git paths, the `jit-server` binary, …). The structured storage resources keep
/// their dedicated errors (e.g. [`crate::storage::IssueNotFoundError`]); this
/// covers the rest. It carries the original message so `Display` is unchanged,
/// and the CLI downcasts to it to classify the failure as a not-found condition
/// (exit code `3`) without scanning the message text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct NotFoundError {
    message: String,
}

impl NotFoundError {
    /// Build a [`NotFoundError`] carrying the user-facing message verbatim.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The user-facing not-found message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// An addressed item that no scope declares.
///
/// The CLI downcasts to this type to classify the failure as
/// [`ErrorCode::ItemNotFound`](crate::output::ErrorCode::ItemNotFound) without
/// scanning the message text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ItemNotFoundError {
    message: String,
}

impl ItemNotFoundError {
    /// Build an [`ItemNotFoundError`] carrying the user-facing message verbatim.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A generic "resource already exists" failure carrying its user-facing message.
///
/// Shared typed carrier for already-exists conditions outside the gate registry
/// (which uses [`crate::storage::GateAlreadyExistsError`]), e.g. a snapshot output
/// path that is already occupied. Carries the message verbatim; the CLI downcasts
/// to it to classify the failure as an already-exists condition (exit code `6`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct AlreadyExistsError {
    message: String,
}

impl AlreadyExistsError {
    /// Build an [`AlreadyExistsError`] carrying the user-facing message verbatim.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The user-facing already-exists message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// Error returned when `jit issue delete` is refused for missing operator
/// confirmation.
///
/// Deletion is a destructive, discouraged operation (Phase 3 safety): the
/// caller's process environment must set `JIT_ALLOW_DELETION=1` or the delete
/// is refused before anything is written (jit:0daba57d). Refusing used to
/// `anyhow::bail!` a plain string, which fell through the failure classifier to
/// the generic-error code; a caller checking for `exit 0` (or treating any
/// nonzero as equivalent) had no reliable way to branch on the refusal. This
/// typed wrapper is downcastable in `output::failure::classify` (→
/// `ExitCode::InvalidArgument`, exit `2`) and renders as `DELETION_NOT_CONFIRMED`
/// under `--json`, so both output modes let a script distinguish "refused" from
/// "deleted" reliably.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct DeletionNotConfirmedError {
    issue_id: IssueId,
    message: String,
}

impl DeletionNotConfirmedError {
    /// Build the refusal for `issue_id`, naming it in the example remediation
    /// command.
    pub fn new(issue_id: IssueId) -> Self {
        let actionable = ActionableError::new(
            "Issue deletion is discouraged and requires explicit confirmation.",
        )
        .with_cause("Deletion is a destructive operation")
        .with_remedy(format!(
            "Set JIT_ALLOW_DELETION=1 environment variable to proceed: \
             JIT_ALLOW_DELETION=1 jit issue delete {issue_id}"
        ))
        .with_remedy("Consider closing issues instead of deleting them");
        Self {
            issue_id,
            message: actionable.to_error_message(),
        }
    }

    /// The issue whose deletion was refused.
    pub fn issue_id(&self) -> &IssueId {
        &self.issue_id
    }

    /// The fully-rendered, user-facing refusal message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A state-mutating invocation refused because it runs inside a linked
/// non-primary checkout whose effective write stance is
/// [`LinkedCheckoutWriteStance::Refuse`](crate::domain::LinkedCheckoutWriteStance::Refuse).
///
/// The refusal is raised at dispatch, before the repository mutation session
/// opens, so it is not a state change: nothing is written, locked, leased, or
/// appended, and there is no residue for recovery to converge (`@/inv/event-log`
/// is preserved rather than excepted).
///
/// It carries the three facts a caller needs to act — the checkout it refused
/// in, the stance that refused it, and how to permit the operation — and the CLI
/// renders those same three into the `--json` envelope's `details` and
/// `suggestions`, so the two output modes cannot drift.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LinkedCheckoutWriteRefusedError {
    message: String,
    checkout: std::path::PathBuf,
    stance: crate::domain::LinkedCheckoutWriteStance,
}

impl LinkedCheckoutWriteRefusedError {
    /// Both ways to permit the refused operation, in the order they are offered:
    /// declare the allowing stance for every linked checkout of the repository,
    /// or supply it for one invocation.
    ///
    /// This is the single source both output modes render — the rendered
    /// message's remediation list and the `--json` envelope's `suggestions` — so
    /// neither can name a surface the other does not.
    ///
    /// The per-invocation override is offered second because it is the narrower
    /// of the two, and because it is the one that works in the case the first
    /// does not cover: an invocation whose own stance is the one that refused
    /// cannot be permitted by editing the declaration it already outranks.
    pub fn permit_guidance() -> [String; 2] {
        [
            "Declare write_policy = \"allow\" under [worktree] in .jit/config.toml to permit \
             state-mutating commands in this repository's linked checkouts"
                .to_owned(),
            format!(
                "Set {}=allow to permit this one invocation",
                crate::config_manager::LINKED_CHECKOUT_WRITE_STANCE_ENV
            ),
        ]
    }

    /// Build the refusal for a state-mutating invocation in `checkout`, whose
    /// effective write stance resolved to `stance`.
    pub fn new(
        checkout: impl Into<std::path::PathBuf>,
        stance: crate::domain::LinkedCheckoutWriteStance,
    ) -> Self {
        let checkout = checkout.into();
        let message = Self::permit_guidance()
            .into_iter()
            .fold(
                ActionableError::new(format!(
                    "Refusing a state-mutating command inside the linked checkout at {}.",
                    checkout.display()
                ))
                .with_cause(format!(
                    "The linked-checkout write stance in force for this invocation is '{}'",
                    stance.as_token()
                )),
                ActionableError::with_remedy,
            )
            .to_error_message();
        Self {
            message,
            checkout,
            stance,
        }
    }

    /// The linked checkout the invocation was refused in.
    pub fn checkout(&self) -> &std::path::Path {
        &self.checkout
    }

    /// The write stance that refused the invocation.
    pub const fn stance(&self) -> crate::domain::LinkedCheckoutWriteStance {
        self.stance
    }
}

/// A validation failure that rejects an operation: a repository-integrity
/// violation found by `jit validate`'s silent pass (broken dependency, undefined
/// required gate, dangling document reference, cyclic/isolated DAG, …) or a
/// write-time rule rejection (an `enforce` rule blocking a create/update).
///
/// These are validation failures by design (exit code `4`): this typed wrapper
/// carries the underlying message verbatim so `Display` is unchanged, while the
/// CLI downcasts to it to classify the failure without scanning the message text.
/// Structured validation failures keep their dedicated types (e.g.
/// [`TransitionBlockedError`], [`crate::graph::GraphError`]); this covers the
/// message-only cases.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ValidationFailedError {
    message: String,
}

impl ValidationFailedError {
    /// Build a [`ValidationFailedError`] carrying the violation message verbatim.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The user-facing validation-failure message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A lease-not-found failure, downcastable to exit code `3`, whose `Display`
/// is the rendered [`ActionableError`] verbatim.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct LeaseNotFoundError {
    message: String,
}

impl LeaseNotFoundError {
    /// Build a [`LeaseNotFoundError`] capturing the full rendered message of an
    /// [`ActionableError`] (e.g. from [`no_active_lease`]).
    pub fn new(error: &ActionableError) -> Self {
        Self {
            message: error.to_string(),
        }
    }

    /// The fully-rendered, user-facing lease-not-found message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// An active lease another agent holds on an issue a write targets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{issue_id} is leased by {holder}")]
pub struct ForeignLease {
    pub issue_id: IssueId,
    pub holder: crate::domain::Assignee,
}

/// A `strict`-mode refusal of a write to issues other agents lease.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub struct ForeignLeaseError {
    leases: Vec<ForeignLease>,
}

impl ForeignLeaseError {
    pub fn new(leases: Vec<ForeignLease>) -> Self {
        Self { leases }
    }

    /// Each refused issue with its holder.
    pub fn leases(&self) -> &[ForeignLease] {
        &self.leases
    }
}

impl fmt::Display for ForeignLeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let named = self
            .leases
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        write!(
            f,
            "Refusing to write: {named}.\nCoordinate with the holder, or wait for the lease to end. \
             If the lease is yours, set your identity with JIT_AGENT_ID."
        )
    }
}

/// The missing Git precondition, determining the appropriate remediation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitRequirementGap {
    /// The current directory is not inside a git repository at all.
    NoRepository,
    /// The directory is a git repository, but it has no commits yet, so
    /// `HEAD` does not resolve to a branch.
    NoCommits,
}

/// A command requires a usable Git repository (external dependency failure, exit 10).
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct RequiresGitError {
    message: String,
}

impl RequiresGitError {
    /// Tailor remediation to a missing repository or an unborn branch.
    pub fn new(gap: GitRequirementGap) -> Self {
        let base = ActionableError::new("This command requires a git repository");
        let actionable = match gap {
            GitRequirementGap::NoRepository => base
                .with_cause("Current directory is not inside a git repository")
                .with_remedy("Initialize a git repository: git init")
                .with_remedy("Change to a directory that is inside a git repository"),
            GitRequirementGap::NoCommits => base
                .with_cause(
                    "Current directory is a git repository, but it has no commits yet, \
                     so HEAD does not resolve to a branch",
                )
                .with_remedy("Create an initial commit: git commit --allow-empty -m \"init\"")
                .with_remedy(
                    "Change to a directory inside a git repository that has at least one commit",
                ),
        };
        Self {
            message: actionable.to_error_message(),
        }
    }

    /// The fully-rendered, user-facing message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A `jit dep add` rejected because the edge would break transitive reduction.
///
/// Cycle detection is a write-time guard (@/inv/dag-acyclic); this error makes the
/// transitive-reduction property a write-time guard too. `jit dep add` refuses,
/// by default, any edge that shadows an existing direct edge or is itself already
/// reachable through other dependencies — the exact property `jit validate`
/// enforces — so a silent write can no longer resurface as a distant validation
/// error. Downcastable in `output::failure::classify` (→ `ExitCode::ValidationFailed`,
/// exit 4), matching both cycle detection and the read-time validator. Re-run
/// `jit dep add --reduce` to drop the now-redundant edge(s) in the same operation.
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct RedundantDependencyError {
    message: String,
}

impl RedundantDependencyError {
    /// Build a [`RedundantDependencyError`] for an `attempted = (from, to)` edge
    /// that would leave `redundant_edges` (each a `(from, to)` pair) redundant.
    /// When the attempted edge is itself the redundant one it appears in
    /// `redundant_edges`. Ids are shortened with `short_id()` in the message.
    pub fn new(attempted: (IssueId, IssueId), redundant_edges: Vec<(IssueId, IssueId)>) -> Self {
        let (from, to) = (attempted.0.short_id(), attempted.1.short_id());
        let pairs = redundant_edges
            .iter()
            .map(|(f, t)| format!("{}→{}", f.short_id(), t.short_id()))
            .collect::<Vec<_>>()
            .join(", ");
        let message = format!(
            "Refusing to add dependency {from}→{to}: it would leave the graph not \
             transitively reduced. Redundant edge(s): {pairs}. Re-run with \
             `jit dep add --reduce` to drop the now-redundant edge(s) in the same \
             operation, or run `jit validate --fix`."
        );
        Self { message }
    }

    /// The user-facing rejection message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A `jit dep add` batch where at least one requested edge failed validation.
///
/// [`add_dependencies_with_policy`](crate::commands::GraphOps::add_dependencies_with_policy)
/// validates every requested edge against the would-be-final graph BEFORE
/// writing anything: id resolution (ambiguous or not found) and graph
/// validation (cycle detection, and — under
/// [`RedundancyPolicy::Reject`](crate::commands::RedundancyPolicy::Reject) —
/// the transitive-reduction check) both run for every edge first. If any edge
/// fails either check, this error is returned and NONE of the batch's edges
/// are applied and no event is logged for any of them (jit:c8518f2a, REQ-01).
/// Every rejected edge is named, not only the first (REQ-02). The CLI raises
/// it too, naming every target it cannot resolve before calling the executor.
///
/// Each rejected edge keeps its own originating typed error so a caller (the
/// CLI) can still classify it individually — by downcast, never by message
/// text — for a per-edge JSON `code`, while `output::failure::classify`
/// can downcast this wrapper directly and pick a single dominant exit code
/// across the whole batch.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct DependencyBatchRejectedError {
    message: String,
    from_id: IssueId,
    rejected: Vec<(String, anyhow::Error)>,
}

impl DependencyBatchRejectedError {
    /// Pair each rejected target's as-supplied request text with its typed
    /// error, in request order.
    pub fn new(from_id: IssueId, rejected: Vec<(String, anyhow::Error)>) -> Self {
        let mut message = format!(
            "Rejected {} dependency edge{} for {}; none were added:\n",
            rejected.len(),
            if rejected.len() == 1 { "" } else { "s" },
            from_id.short_id(),
        );
        for (requested, error) in &rejected {
            message.push_str(&format!("  - {}: {}\n", requested, error));
        }
        Self {
            message,
            from_id,
            rejected,
        }
    }

    /// The issue the batch targeted (the `<from>` of `jit dep add`).
    pub fn from_id(&self) -> &IssueId {
        &self.from_id
    }

    /// Every rejected edge: the as-supplied target request text paired with the
    /// typed error that rejected it, in request order.
    pub fn rejected(&self) -> &[(String, anyhow::Error)] {
        &self.rejected
    }
}

/// A state-transition failure with structured blocker details.
///
/// Command logic uses this error to report why an issue cannot move to the
/// requested lifecycle state without embedding CLI or JSON rendering in the
/// command layer. Human-readable formatting is provided through `Display`, while
/// JSON serialization is handled by `crate::output`.
#[derive(Debug, Clone)]
pub struct TransitionBlockedError {
    issue_id: IssueId,
    requested_state: State,
    actual_state: State,
    blockers: Vec<TransitionBlocker>,
    /// Non-blocking findings (warn-severity or non-enforcing rules) observed
    /// while evaluating the blocked transition. Carried on the error so paths
    /// that must return an error (e.g. the gate diversion) still surface them
    /// in both the rendered message and the JSON details.
    warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TransitionBlocker {
    Dependency {
        issue_id: IssueId,
        title: String,
        state: State,
    },
    MissingDependency {
        issue_id: IssueId,
    },
    Gate {
        gate_key: GateKey,
        status: GateStatus,
        /// The gate's registry mode, so the remediation hint can name the
        /// `--by <attestor>` form for a manual gate (jit:1d59070d REQ-03)
        /// rather than a bare `evaluate` that now fails without one.
        mode: GateMode,
    },
    /// An enforcing graph rule (`enforce = true`, severity error) produced a
    /// finding attributed to this issue in its target state, blocking the
    /// transition (CC-2). Carries the rule name and the rule's finding message.
    GraphRule {
        rule: String,
        message: String,
    },
    /// A revive out of [`State::Archived`] targeted a state other than the
    /// recorded pre-archive origin. `Archived` is terminality-preserving
    /// (`jit:45a140ae`): reviving restores the pre-archive state exactly, so
    /// the round-trip cannot resurrect a completed issue into the active
    /// lifecycle. Carries the only legal revive target.
    ArchivedRevive {
        /// The recorded pre-archive origin, the sole permitted revive target.
        origin: State,
    },
}

impl TransitionBlockedError {
    pub(crate) fn dependencies(
        issue_id: IssueId,
        requested_state: State,
        actual_state: State,
        blockers: Vec<TransitionBlocker>,
    ) -> Self {
        Self {
            issue_id,
            requested_state,
            actual_state,
            blockers,
            warnings: Vec::new(),
        }
    }

    pub(crate) fn gates(
        issue_id: IssueId,
        requested_state: State,
        actual_state: State,
        gates: Vec<(GateKey, GateStatus, GateMode)>,
    ) -> Self {
        Self {
            issue_id,
            requested_state,
            actual_state,
            blockers: gates
                .into_iter()
                .map(|(gate_key, status, mode)| TransitionBlocker::Gate {
                    gate_key,
                    status,
                    mode,
                })
                .collect(),
            warnings: Vec::new(),
        }
    }

    /// A transition blocked by one or more enforcing graph rules (CC-2).
    ///
    /// Each `(rule, message)` pair becomes a [`TransitionBlocker::GraphRule`].
    /// The error maps to exit 4 via [`TransitionBlockedError::error_code`] and
    /// the existing downcast in `main.rs`.
    pub(crate) fn graph_rules(
        issue_id: IssueId,
        requested_state: State,
        actual_state: State,
        rules: Vec<(String, String)>,
    ) -> Self {
        Self {
            issue_id,
            requested_state,
            actual_state,
            blockers: rules
                .into_iter()
                .map(|(rule, message)| TransitionBlocker::GraphRule { rule, message })
                .collect(),
            warnings: Vec::new(),
        }
    }

    /// A revive out of [`State::Archived`] refused because it targeted a state
    /// other than the recorded pre-archive origin (`jit:45a140ae`).
    ///
    /// `requested_state` is the attempted target, `origin` the only legal revive
    /// target. Maps to exit 4 like the other transition refusals.
    pub(crate) fn archived_revive(
        issue_id: IssueId,
        requested_state: State,
        origin: State,
    ) -> Self {
        Self {
            issue_id,
            requested_state,
            actual_state: State::Archived,
            blockers: vec![TransitionBlocker::ArchivedRevive { origin }],
            warnings: Vec::new(),
        }
    }

    /// Attach non-blocking findings observed during the blocked transition.
    pub(crate) fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(crate) fn issue_id(&self) -> &IssueId {
        &self.issue_id
    }

    pub(crate) fn requested_state(&self) -> State {
        self.requested_state
    }

    pub(crate) fn actual_state(&self) -> State {
        self.actual_state
    }

    pub(crate) fn blockers(&self) -> &[TransitionBlocker] {
        &self.blockers
    }

    pub(crate) fn error_code(&self) -> crate::output::ErrorCode {
        if self.blockers.iter().all(|blocker| {
            matches!(
                blocker,
                TransitionBlocker::Dependency { .. } | TransitionBlocker::MissingDependency { .. }
            )
        }) {
            crate::output::ErrorCode::Blocked
        } else {
            crate::output::ErrorCode::ValidationFailed
        }
    }

    pub(crate) fn summary(&self) -> String {
        let requested = self.requested_state.as_str();
        if let Some(TransitionBlocker::ArchivedRevive { origin }) = self
            .blockers
            .iter()
            .find(|blocker| matches!(blocker, TransitionBlocker::ArchivedRevive { .. }))
        {
            format!(
                "Cannot revive archived issue to '{}': an archived issue only revives to its \
                 pre-archive state '{}'",
                requested,
                origin.as_str()
            )
        } else if self
            .blockers
            .iter()
            .any(|blocker| matches!(blocker, TransitionBlocker::Gate { .. }))
        {
            format!(
                "Gate validation failed: Cannot transition to '{}': {} gate(s) not passed",
                requested,
                self.blockers.len()
            )
        } else if self
            .blockers
            .iter()
            .all(|blocker| matches!(blocker, TransitionBlocker::GraphRule { .. }))
        {
            format!(
                "Graph rule validation failed: Cannot transition to '{}': {} enforcing rule(s) failed",
                requested,
                self.blockers.len()
            )
        } else {
            format!(
                "Cannot transition to '{}': issue blocked by {} unmet dependencies",
                requested,
                self.blockers.len()
            )
        }
    }

    pub(crate) fn remediation_commands(&self) -> Vec<String> {
        let inspect_command = match self.blockers.first() {
            Some(TransitionBlocker::Gate { .. }) => {
                format!("jit gate status-all {}", self.issue_id)
            }
            Some(TransitionBlocker::GraphRule { .. }) => {
                format!("jit validate --explain {}", self.issue_id)
            }
            Some(TransitionBlocker::ArchivedRevive { origin }) => {
                format!(
                    "jit issue update {} --state {}  # revive restores the pre-archive state",
                    self.issue_id,
                    origin.as_str()
                )
            }
            _ => format!("jit graph deps {}", self.issue_id),
        };

        // When the transition is blocked by an unmet (or missing)
        // dependency, surface how to assign the issue without starting work.
        // This is the actionable answer to a failed `jit issue claim`.
        let assign_hint = self.is_dependency_blocked().then(|| {
            format!(
                "To assign without starting work (no state change): jit issue assign {} <assignee>",
                self.issue_id
            )
        });

        std::iter::once(inspect_command)
            .chain(self.blockers.iter().flat_map(|blocker| match blocker {
                TransitionBlocker::Dependency { issue_id, .. } => {
                    vec![format!("jit issue show {}", issue_id)]
                }
                TransitionBlocker::MissingDependency { issue_id } => {
                    vec![format!(
                        "jit validate --json  # missing dependency: {}",
                        issue_id
                    )]
                }
                TransitionBlocker::Gate { gate_key, mode, .. } => {
                    // A manual gate's evaluate now requires --by (jit:1d59070d
                    // REQ-03); name the attested form so the remediation stays
                    // actionable rather than pointing at a call that fails.
                    let evaluate_hint = match mode {
                        GateMode::Manual => format!(
                            "jit gate evaluate {} {} --by <attestor>",
                            self.issue_id, gate_key
                        ),
                        GateMode::Auto => {
                            format!("jit gate evaluate {} {}", self.issue_id, gate_key)
                        }
                    };
                    vec![
                        evaluate_hint,
                        format!(
                            "jit gate status {} {} --all  # run history",
                            self.issue_id, gate_key
                        ),
                    ]
                }
                TransitionBlocker::GraphRule { rule, .. } => {
                    vec![format!(
                        "jit issue update {} ...  # satisfy or fix rule '{}', or re-run with --force",
                        self.issue_id, rule
                    )]
                }
                TransitionBlocker::ArchivedRevive { origin } => {
                    vec![format!(
                        "jit issue update {} --state {}  # the only legal revive target",
                        self.issue_id,
                        origin.as_str()
                    )]
                }
            }))
            .chain(assign_hint)
            .collect()
    }

    /// True when any blocker is an unmet or missing dependency.
    ///
    /// Used to gate the "assign without starting work" remediation so it only
    /// appears for the dependency-blocked (claim-blocked) case, not for gate or
    /// graph-rule failures.
    fn is_dependency_blocked(&self) -> bool {
        self.blockers.iter().any(|blocker| {
            matches!(
                blocker,
                TransitionBlocker::Dependency { .. } | TransitionBlocker::MissingDependency { .. }
            )
        })
    }
}

impl TransitionBlocker {
    pub(crate) fn dependency(issue: Issue) -> Self {
        Self::Dependency {
            issue_id: issue.id,
            title: issue.title,
            state: issue.state,
        }
    }

    pub(crate) fn missing_dependency(issue_id: IssueId) -> Self {
        Self::MissingDependency { issue_id }
    }
}

impl fmt::Display for TransitionBlockedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "{}", self.summary())?;

        writeln!(f, "\nBlockers:")?;
        for blocker in &self.blockers {
            writeln!(f, "  - {}", blocker)?;
        }

        if !self.warnings.is_empty() {
            writeln!(f, "\nWarnings (non-blocking):")?;
            for warning in &self.warnings {
                writeln!(f, "  - {}", warning)?;
            }
        }

        writeln!(f, "\nTo fix:")?;
        for command in self.remediation_commands() {
            writeln!(f, "  - {}", command)?;
        }

        if self.actual_state == State::Gated {
            writeln!(
                f,
                "\nIssue automatically transitioned to 'gated' and will move to 'done' when all gates pass."
            )?;
        }

        Ok(())
    }
}

impl std::error::Error for TransitionBlockedError {}

impl fmt::Display for TransitionBlocker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Dependency {
                issue_id,
                title,
                state,
            } => write!(f, "{} {} [{}]", issue_id.short_id(), title, state.as_str()),
            Self::MissingDependency { issue_id } => {
                write!(f, "{} (missing issue) [missing]", issue_id.short_id())
            }
            Self::Gate {
                gate_key, status, ..
            } => {
                write!(f, "{} [{}]", gate_key, status)
            }
            Self::GraphRule { rule, message } => {
                write!(f, "[{}] {}", rule, message)
            }
            Self::ArchivedRevive { origin } => {
                write!(
                    f,
                    "archived issue only revives to its pre-archive state [{}]",
                    origin.as_str()
                )
            }
        }
    }
}

/// Helper to create "no acting identity" errors with standard remediation.
///
/// Used by `jit claim release <issue-id>` when neither a configured agent id
/// (`JIT_AGENT_ID` / `~/.config/jit/agent.toml`) nor a git `user.name` is
/// available. A release must be attributable in the audit trail, so the command
/// errors rather than fabricating a placeholder identity.
pub fn no_acting_identity() -> ActionableError {
    ActionableError::new("Cannot release: no acting identity for the audit trail")
        .with_cause("No agent identity is configured (JIT_AGENT_ID or ~/.config/jit/agent.toml)")
        .with_cause("Git user.name is not set")
        .with_remedy("Set an agent identity: export JIT_AGENT_ID=agent:your-name")
        .with_remedy("Or configure git: git config user.name \"Your Name\"")
}

/// Helper to create "issue has no active lease" errors with standard remediation.
///
/// Used by the lease commands addressed by issue (`jit claim release`, `renew`,
/// `heartbeat` and `force-evict`) when the issue has no unexpired lease.
pub fn no_active_lease(issue: impl std::fmt::Display) -> ActionableError {
    ActionableError::new(format!("Issue {issue} has no active lease (not found)"))
        .with_cause("No agent currently holds a lease on this issue")
        .with_cause("The lease may have already expired or been released")
        .with_remedy(format!(
            "Check active leases for this issue: jit claim status --issue {issue} --json"
        ))
        .with_remedy("List all active leases: jit claim list --json")
}

/// Helper to create already claimed errors with standard remediation.
pub fn already_claimed(issue_id: &IssueId, agent_id: &str, expires_info: &str) -> ActionableError {
    ActionableError::new(format!(
        "Issue {} already claimed by {} {}",
        issue_id, agent_id, expires_info
    ))
    .with_cause("Another agent is currently working on this issue")
    .with_cause("The previous agent may have crashed without releasing the lease")
    .with_cause("The issue may still be in progress")
    .with_remedy(format!(
        "Wait for the lease to expire or be released: jit claim status --issue {} --json",
        issue_id
    ))
    .with_remedy(format!(
        "Contact the agent owner to coordinate: {}",
        agent_id
    ))
    .with_remedy(format!(
        "If the agent crashed, force evict with: jit claim force-evict {issue_id} --reason \"<reason>\""
    ))
}

/// Helper to create git command failure errors with standard remediation.
pub fn git_command_failed(command: &str, stderr: &str) -> ActionableError {
    ActionableError::new(format!("Git command failed: {}", command))
        .with_cause(format!("Git error: {}", stderr.trim()))
        .with_cause("Repository may be in an invalid state")
        .with_cause("Git configuration may be incorrect")
        .with_remedy("Check repository status: git status")
        .with_remedy("Verify git configuration: git config --list")
        .with_remedy(format!("Try running the command manually: {}", command))
}

/// Helper to create ownership errors with standard remediation.
pub fn not_owner(resource: &str, owner: &str, requester: &str) -> ActionableError {
    ActionableError::new(format!(
        "Cannot modify {}: owned by {}, not {}",
        resource, owner, requester
    ))
    .with_cause("You are not the owner of this resource")
    .with_cause("The wrong agent ID may be configured")
    .with_remedy("Check your agent configuration: jit config show-agent")
    .with_remedy(format!("Use the correct agent ID: {}", owner))
    .with_remedy("If you need to override, use force-evict (requires reason)")
}

/// A leftover redo record could not be applied, or `jit recover` could not run
/// its repairs; the source is that failure.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct RecoveryFailedError(pub anyhow::Error);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_actionable_error_formatting() {
        let error = ActionableError::new("Test error")
            .with_cause("First cause")
            .with_cause("Second cause")
            .with_remedy("First remedy")
            .with_remedy("Second remedy");

        let msg = error.to_error_message();

        // `to_error_message` renders the text verbatim; the top-level CLI
        // printer is the sole place that adds the "Error: " prefix (REQ-01),
        // so no prefix should appear here.
        assert!(!msg.contains("Error:"));
        assert!(msg.contains("Test error"));
        assert!(msg.contains("Possible causes:"));
        assert!(msg.contains("• First cause"));
        assert!(msg.contains("• Second cause"));
        assert!(msg.contains("To fix:"));
        assert!(msg.contains("• First remedy"));
        assert!(msg.contains("• Second remedy"));
    }

    #[test]
    fn test_error_without_causes() {
        let error = ActionableError::new("Simple error").with_remedy("Just fix it");

        let msg = error.to_error_message();

        assert!(!msg.contains("Error:"));
        assert!(msg.contains("Simple error"));
        assert!(!msg.contains("Possible causes:"));
        assert!(msg.contains("To fix:"));
        assert!(msg.contains("• Just fix it"));
    }

    #[test]
    fn test_error_without_remediation() {
        let error = ActionableError::new("Diagnostic only").with_cause("Something went wrong");

        let msg = error.to_error_message();

        assert!(!msg.contains("Error:"));
        assert!(msg.contains("Diagnostic only"));
        assert!(msg.contains("Possible causes:"));
        assert!(msg.contains("• Something went wrong"));
        assert!(!msg.contains("To fix:"));
    }

    #[test]
    fn test_graph_rule_block_renders_rule_and_message_and_validation_code() {
        let id = crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_1234_5678);
        let error = TransitionBlockedError::graph_rules(
            id.clone(),
            State::Done,
            State::InProgress,
            vec![(
                "epic-done-needs-design-dep".to_string(),
                "must depend on a type:design issue".to_string(),
            )],
        );

        // A graph-rule block is a validation failure (exit 4 via error_code).
        assert_eq!(
            error.error_code(),
            crate::output::ErrorCode::ValidationFailed
        );

        let rendered = error.to_string();
        assert!(rendered.contains("Graph rule validation failed"));
        assert!(rendered.contains("epic-done-needs-design-dep"));
        assert!(rendered.contains("must depend on a type:design issue"));
        // The remediation suggests explaining the rule and a forced override.
        assert!(rendered.contains(&format!("jit validate --explain {id}")));
        assert!(rendered.contains("--force"));
        // It is NOT misreported as a gate or dependency block.
        assert!(!rendered.contains("unmet dependencies"));
        assert!(!rendered.contains("gate(s) not passed"));
    }

    #[test]
    fn test_dependency_block_names_assign_in_remediation() {
        let id = crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_1234_5678);
        let error = TransitionBlockedError::dependencies(
            id.clone(),
            State::InProgress,
            State::Backlog,
            vec![TransitionBlocker::MissingDependency {
                issue_id: crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_8765_4321),
            }],
        );

        let rendered = error.to_string();
        assert!(rendered.contains(&format!("jit issue assign {id} <assignee>")));
        assert!(rendered.contains("no state change"));

        // The same hint is present in the remediation list (the JSON path).
        assert!(error
            .remediation_commands()
            .iter()
            .any(|cmd| cmd.contains(&format!("jit issue assign {id}"))));
    }

    #[test]
    fn test_gate_block_does_not_name_assign_in_remediation() {
        let error = TransitionBlockedError::gates(
            crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_1234_5678),
            State::Done,
            State::Gated,
            vec![(
                "code-review".parse().unwrap(),
                GateStatus::Pending,
                GateMode::Auto,
            )],
        );

        assert!(!error.to_string().contains("jit issue assign"));
    }

    /// REQ-03 (jit:1d59070d): a manual gate's remediation names the attested
    /// `--by` form, since a bare `gate evaluate` on it is now a usage error.
    #[test]
    fn test_gate_block_remediation_names_by_for_manual_gate() {
        let id = crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_1234_5678);
        let error = TransitionBlockedError::gates(
            id.clone(),
            State::Done,
            State::Gated,
            vec![(
                "code-review".parse().unwrap(),
                GateStatus::Pending,
                GateMode::Manual,
            )],
        );

        let commands = error.remediation_commands();
        assert!(commands
            .iter()
            .any(|cmd| cmd == &format!("jit gate evaluate {id} code-review --by <attestor>")));
    }

    /// The auto-gate counterpart: no `--by` in the remediation, since the
    /// checker supplies the verdict.
    #[test]
    fn test_gate_block_remediation_omits_by_for_auto_gate() {
        let id = crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_1234_5678);
        let error = TransitionBlockedError::gates(
            id.clone(),
            State::Done,
            State::Gated,
            vec![(
                "tests".parse().unwrap(),
                GateStatus::Pending,
                GateMode::Auto,
            )],
        );

        let commands = error.remediation_commands();
        assert!(commands
            .iter()
            .any(|cmd| cmd == &format!("jit gate evaluate {id} tests")));
        assert!(!commands.iter().any(|cmd| cmd.contains("--by")));
    }

    #[test]
    fn test_already_claimed_helper() {
        let id = crate::test_utils::issue_id(0x1234_4234_8234_1234_5678_9abc_1234_5678);
        let error = already_claimed(&id.clone(), "agent:worker-1", "until 2026-01-15");
        let msg = error.to_error_message();

        assert!(msg.contains(&format!("Issue {id} already claimed")));
        assert!(msg.contains("agent:worker-1"));
        assert!(msg.contains("jit claim status --issue"));
        assert!(msg.contains(&format!("jit claim force-evict {id} ")));
    }

    // The generic message-carrier errors exist so the top-level exit-code
    // classifier can downcast (rather than scan the message text) while leaving
    // the user-facing `Display` byte-identical to the original `anyhow!`/`bail!`
    // string. These tests pin both halves of that contract: `Display` equals the
    // message verbatim, and the type survives an `anyhow::Error` round trip.

    #[test]
    fn test_invalid_argument_error_display_is_message_verbatim() {
        let msg = "invalid field: unknown field 'nope'";
        let err = InvalidArgumentError::new(msg);
        assert_eq!(err.to_string(), msg);
        assert_eq!(err.message(), msg);

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), msg);
        assert!(any.downcast_ref::<InvalidArgumentError>().is_some());
    }

    #[test]
    fn test_not_found_error_display_is_message_verbatim() {
        let msg = "Gate 'ci' not found";
        let err = NotFoundError::new(msg);
        assert_eq!(err.to_string(), msg);

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), msg);
        assert!(any.downcast_ref::<NotFoundError>().is_some());
    }

    #[test]
    fn test_already_exists_error_display_is_message_verbatim() {
        let msg = "Output path already exists: ./snapshot.json";
        let err = AlreadyExistsError::new(msg);
        assert_eq!(err.to_string(), msg);

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), msg);
        assert!(any.downcast_ref::<AlreadyExistsError>().is_some());
    }

    #[test]
    fn test_deletion_not_confirmed_error_names_issue_and_env_var() {
        let id = crate::test_utils::issue_id(1);
        let err = DeletionNotConfirmedError::new(id.clone());
        let msg = err.to_string();

        assert_eq!(msg, err.message());
        assert!(msg.contains("discouraged"));
        assert!(msg.contains("JIT_ALLOW_DELETION=1"));
        assert!(msg.contains(&format!("jit issue delete {id}")));
        // ActionableError's rendering, not a bespoke format: no doubled prefix.
        assert!(!msg.contains("Error:"));

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), msg);
        assert!(any.downcast_ref::<DeletionNotConfirmedError>().is_some());
    }

    #[test]
    fn test_validation_failed_error_display_is_message_verbatim() {
        let msg = "Invalid dependency: issue 'a' depends on 'b' which does not exist";
        let err = ValidationFailedError::new(msg);
        assert_eq!(err.to_string(), msg);
        assert_eq!(err.message(), msg);

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), msg);
        assert!(any.downcast_ref::<ValidationFailedError>().is_some());
    }

    #[test]
    fn test_lease_not_found_error_renders_actionable_message_verbatim() {
        // LeaseNotFoundError captures the full rendered ActionableError text so
        // the rich remediation block is preserved while remaining downcastable.
        let actionable = no_active_lease(crate::test_utils::issue_id(
            0x123442348234123456789abcabc12345,
        ));
        let expected = actionable.to_string();
        let err = LeaseNotFoundError::new(&actionable);
        assert_eq!(err.to_string(), expected);
        assert_eq!(err.message(), expected);

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), expected);
        assert!(any.downcast_ref::<LeaseNotFoundError>().is_some());
    }
}
