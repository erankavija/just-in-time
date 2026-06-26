//! Actionable error formatting for improved user experience.
//!
//! This module provides utilities for creating error messages with:
//! - Clear error description
//! - Possible causes (diagnostics)
//! - Remediation steps (actionable fixes)
//!
//! Designed to help users understand what went wrong and how to fix it.

use std::fmt;

use crate::domain::{GateStatus, Issue, State, SHORT_ID_LENGTH};

/// An error with diagnostic context and remediation steps.
///
/// This struct wraps an error message with additional context to help users
/// diagnose and fix the problem.
///
/// # Example
///
/// ```
/// use jit::errors::ActionableError;
///
/// let error = ActionableError::new("Lease abc123 not found")
///     .with_cause("The lease may have expired")
///     .with_cause("The lease ID may be incorrect")
///     .with_remedy("List all active leases: jit claim list --json | jq -r '.data.leases[].lease_id'")
///     .with_remedy("Check if the issue is still claimed: jit claim status --issue <issue-id>");
///
/// eprintln!("{}", error);
/// ```
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
    pub fn to_error_message(&self) -> String {
        let mut msg = format!("Error: {}\n", self.error);

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
///
/// # Examples
///
/// ```
/// use jit::errors::InvalidArgumentError;
///
/// let err = InvalidArgumentError::new("Invalid priority: urgent");
/// assert_eq!(err.to_string(), "Invalid priority: urgent");
/// assert_eq!(err.message(), "Invalid priority: urgent");
///
/// // Downcastable through anyhow for exit-code classification.
/// let any: anyhow::Error = err.into();
/// assert!(any.downcast_ref::<InvalidArgumentError>().is_some());
/// ```
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

/// A generic "resource not found" failure carrying its user-facing message.
///
/// This is the shared typed carrier for the long tail of lookup failures that do
/// not warrant a bespoke type (document references, gate keys, plan documents,
/// git paths, the `jit-server` binary, …). The structured storage resources keep
/// their dedicated errors (e.g. [`crate::storage::IssueNotFoundError`]); this
/// covers the rest. It carries the original message so `Display` is unchanged,
/// and the CLI downcasts to it to classify the failure as a not-found condition
/// (exit code `3`) without scanning the message text.
///
/// # Examples
///
/// ```
/// use jit::errors::NotFoundError;
///
/// let err = NotFoundError::new("Document reference docs/x.md not found in issue ab12");
/// assert!(err.to_string().contains("not found"));
///
/// let any: anyhow::Error = err.into();
/// assert!(any.downcast_ref::<NotFoundError>().is_some());
/// ```
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

/// A generic "resource already exists" failure carrying its user-facing message.
///
/// Shared typed carrier for already-exists conditions outside the gate registry
/// (which uses [`crate::storage::GateAlreadyExistsError`]), e.g. a snapshot output
/// path that is already occupied. Carries the message verbatim; the CLI downcasts
/// to it to classify the failure as an already-exists condition (exit code `6`).
///
/// # Examples
///
/// ```
/// use jit::errors::AlreadyExistsError;
///
/// let err = AlreadyExistsError::new("Output path already exists: /tmp/snap");
/// assert!(err.to_string().contains("already exists"));
///
/// let any: anyhow::Error = err.into();
/// assert!(any.downcast_ref::<AlreadyExistsError>().is_some());
/// ```
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

/// A validation failure that rejects an operation: a repository-integrity
/// violation found by `jit validate`'s silent pass (broken dependency, undefined
/// required gate, dangling document reference, cyclic/isolated DAG, …) or a
/// write-time rule rejection (an `enforce` rule blocking a create/update).
///
/// These are validation failures by design (exit code `4`): this typed wrapper
/// carries the underlying message verbatim so `Display` is unchanged, while the
/// CLI downcasts to it to classify the failure without scanning the message text.
/// Structured validation failures keep their dedicated types (e.g.
/// [`TransitionBlockedError`], [`crate::GraphError`]); this covers the
/// message-only cases.
///
/// # Examples
///
/// ```
/// use jit::errors::ValidationFailedError;
///
/// let err = ValidationFailedError::new(
///     "Invalid dependency: issue 'a' depends on 'b' which does not exist",
/// );
/// assert!(err.to_string().contains("Invalid dependency"));
///
/// let any: anyhow::Error = err.into();
/// assert!(any.downcast_ref::<ValidationFailedError>().is_some());
/// ```
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

/// A lease-not-found failure.
///
/// Most lease lookups previously stringified an [`ActionableError`] into an
/// `anyhow!("{}", ...)` (causes + remediation); one path used a plain
/// `anyhow!("Lease {} not found", id)`. The CLI could only classify either by
/// scanning for "not found". This dedicated type makes a lease-not-found
/// downcastable (exit code `3`) instead, while each constructor reproduces its
/// origin's message verbatim through `Display`.
///
/// # Examples
///
/// ```
/// use jit::errors::{no_active_lease, LeaseNotFoundError};
///
/// let err = LeaseNotFoundError::new(&no_active_lease("abc12345"));
/// // The rich remediation text is preserved verbatim.
/// assert!(err.to_string().contains("abc12345"));
/// assert!(err.to_string().contains("not found"));
///
/// // The plain "Lease <id> not found" form for the bare lookup path.
/// assert_eq!(
///     LeaseNotFoundError::by_id("abc12345").to_string(),
///     "Lease abc12345 not found"
/// );
///
/// // Downcastable through anyhow for exit-code classification.
/// let any: anyhow::Error = err.into();
/// assert!(any.downcast_ref::<LeaseNotFoundError>().is_some());
/// ```
#[derive(Debug, Clone, thiserror::Error)]
#[error("{message}")]
pub struct LeaseNotFoundError {
    message: String,
}

impl LeaseNotFoundError {
    /// Build a [`LeaseNotFoundError`] capturing the full rendered message of an
    /// [`ActionableError`] (e.g. from [`no_active_lease`] or [`lease_not_found`]).
    pub fn new(error: &ActionableError) -> Self {
        Self {
            message: error.to_string(),
        }
    }

    /// Build a [`LeaseNotFoundError`] for a bare lookup miss, using the plain
    /// "Lease <id> not found" phrasing (no remediation block).
    pub fn by_id(lease_id: impl std::fmt::Display) -> Self {
        Self {
            message: format!("Lease {} not found", lease_id),
        }
    }

    /// The fully-rendered, user-facing lease-not-found message.
    pub fn message(&self) -> &str {
        &self.message
    }
}

/// A state-transition failure with structured blocker details.
///
/// Command logic uses this error to report why an issue cannot move to the
/// requested lifecycle state without embedding CLI or JSON rendering in the
/// command layer. Human-readable formatting is provided through `Display`, while
/// JSON serialization is handled by `crate::output`.
///
/// # Examples
///
/// ```ignore
/// // Returned by issue transition commands when blockers prevent progress.
/// let error = transition_result.unwrap_err();
/// assert!(error.to_string().contains("To fix:"));
/// ```
#[derive(Debug, Clone)]
pub struct TransitionBlockedError {
    issue_id: String,
    requested_state: State,
    actual_state: State,
    blockers: Vec<TransitionBlocker>,
    /// Non-blocking findings (warn-severity or non-enforcing rules) observed
    /// while evaluating the blocked transition. Carried on the error so paths
    /// that must return an error (e.g. the gate diversion) still surface them
    /// in both the rendered message and the JSON details.
    warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) enum TransitionBlocker {
    Dependency {
        issue_id: String,
        title: String,
        state: State,
    },
    MissingDependency {
        issue_id: String,
    },
    Gate {
        gate_key: String,
        status: GateStatus,
    },
    /// An enforcing graph rule (`enforce = true`, severity error) produced a
    /// finding attributed to this issue in its target state, blocking the
    /// transition (CC-2). Carries the rule name and the rule's finding message.
    GraphRule {
        rule: String,
        message: String,
    },
}

impl TransitionBlockedError {
    pub(crate) fn dependencies(
        issue_id: String,
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
        issue_id: String,
        requested_state: State,
        actual_state: State,
        gates: Vec<(String, GateStatus)>,
    ) -> Self {
        Self {
            issue_id,
            requested_state,
            actual_state,
            blockers: gates
                .into_iter()
                .map(|(gate_key, status)| TransitionBlocker::Gate { gate_key, status })
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
        issue_id: String,
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

    /// Attach non-blocking findings observed during the blocked transition.
    pub(crate) fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    pub(crate) fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub(crate) fn issue_id(&self) -> &str {
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

    pub(crate) fn error_code(&self) -> &'static str {
        if self.blockers.iter().all(|blocker| {
            matches!(
                blocker,
                TransitionBlocker::Dependency { .. } | TransitionBlocker::MissingDependency { .. }
            )
        }) {
            crate::output::ErrorCode::BLOCKED
        } else {
            crate::output::ErrorCode::VALIDATION_FAILED
        }
    }

    pub(crate) fn summary(&self) -> String {
        let requested = state_name(self.requested_state);
        if self
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
                "Cannot transition to '{}': issue blocked by {} incomplete dependencies",
                requested,
                self.blockers.len()
            )
        }
    }

    pub(crate) fn remediation_commands(&self) -> Vec<String> {
        let inspect_command = match self.blockers.first() {
            Some(TransitionBlocker::Gate { .. }) => {
                format!("jit gate check-all {}", self.issue_id)
            }
            Some(TransitionBlocker::GraphRule { .. }) => {
                format!("jit validate --explain {}", self.issue_id)
            }
            _ => format!("jit graph deps {}", self.issue_id),
        };

        // When the transition is blocked by an incomplete (or missing)
        // dependency, surface how to assign the issue without starting work.
        // This is the actionable answer to a failed `jit issue claim`.
        let assign_hint = self.is_dependency_blocked().then(|| {
            format!(
                "To assign without starting work (no state change): jit issue assign {} <assignee>",
                self.issue_id
            )
        });

        std::iter::once(inspect_command)
            .chain(self.blockers.iter().map(|blocker| match blocker {
                TransitionBlocker::Dependency { issue_id, .. } => {
                    format!("jit issue show {}", issue_id)
                }
                TransitionBlocker::MissingDependency { issue_id } => {
                    format!("jit validate --json  # missing dependency: {}", issue_id)
                }
                TransitionBlocker::Gate { gate_key, .. } => {
                    format!("jit gate pass {} {}", self.issue_id, gate_key)
                }
                TransitionBlocker::GraphRule { rule, .. } => {
                    format!(
                        "jit issue update {} ...  # satisfy or fix rule '{}', or re-run with --force",
                        self.issue_id, rule
                    )
                }
            }))
            .chain(assign_hint)
            .collect()
    }

    /// True when any blocker is an incomplete or missing dependency.
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

    pub(crate) fn missing_dependency(issue_id: String) -> Self {
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
            } => write!(
                f,
                "{} {} [{}]",
                short_id(issue_id),
                title,
                state_name(*state)
            ),
            Self::MissingDependency { issue_id } => {
                write!(f, "{} (missing issue) [missing]", short_id(issue_id))
            }
            Self::Gate { gate_key, status } => {
                write!(f, "{} [{}]", gate_key, gate_status_name(*status))
            }
            Self::GraphRule { rule, message } => {
                write!(f, "[{}] {}", rule, message)
            }
        }
    }
}

pub(crate) fn state_name(state: State) -> &'static str {
    match state {
        State::Backlog => "backlog",
        State::Ready => "ready",
        State::InProgress => "in_progress",
        State::Gated => "gated",
        State::Done => "done",
        State::Rejected => "rejected",
        State::Archived => "archived",
    }
}

pub(crate) fn gate_status_name(status: GateStatus) -> &'static str {
    match status {
        GateStatus::Pending => "pending",
        GateStatus::Passed => "passed",
        GateStatus::Failed => "failed",
    }
}

pub(crate) fn short_id(issue_id: &str) -> String {
    issue_id.chars().take(SHORT_ID_LENGTH).collect()
}

/// Helper to create lease not found errors with standard remediation.
pub fn lease_not_found(lease_id: &str) -> ActionableError {
    ActionableError::new(format!("Lease {} not found", lease_id))
        .with_cause("The lease may have expired")
        .with_cause("The lease ID may be incorrect")
        .with_cause("The lease may have been released or evicted")
        .with_remedy(
            "List all active leases: jit claim list --json | jq -r '.data.leases[].lease_id'",
        )
        .with_remedy("Check lease status: jit claim status --json")
}

/// Helper to create "no acting identity" errors with standard remediation.
///
/// Used by `jit claim release <issue-id>` when neither a configured agent id
/// (`JIT_AGENT_ID` / `~/.config/jit/agent.toml`) nor a git `user.name` is
/// available. A release must be attributable in the audit trail, so the command
/// errors rather than fabricating a placeholder identity.
///
/// # Examples
///
/// ```
/// use jit::errors::no_acting_identity;
///
/// let err = no_acting_identity();
/// assert!(err.to_error_message().contains("no acting identity"));
/// ```
pub fn no_acting_identity() -> ActionableError {
    ActionableError::new("Cannot release: no acting identity for the audit trail")
        .with_cause("No agent identity is configured (JIT_AGENT_ID or ~/.config/jit/agent.toml)")
        .with_cause("Git user.name is not set")
        .with_remedy("Set an agent identity: export JIT_AGENT_ID=agent:your-name")
        .with_remedy("Or configure git: git config user.name \"Your Name\"")
}

/// Helper to create "issue has no active lease" errors with standard remediation.
///
/// Used by `jit claim release <issue-id>` when the issue has no active lease to
/// release. The message contains "not found" so callers that map error text to
/// exit codes treat it like other lookup failures.
///
/// # Examples
///
/// ```
/// use jit::errors::no_active_lease;
///
/// let err = no_active_lease("abc12345");
/// let msg = err.to_error_message();
/// assert!(msg.contains("abc12345"));
/// assert!(msg.contains("not found"));
/// ```
pub fn no_active_lease(issue_id: &str) -> ActionableError {
    ActionableError::new(format!(
        "Issue {} has no active lease to release (not found)",
        issue_id
    ))
    .with_cause("No agent currently holds a lease on this issue")
    .with_cause("The lease may have already expired or been released")
    .with_remedy(format!(
        "Check active leases for this issue: jit claim status --issue {} --json",
        issue_id
    ))
    .with_remedy("List all active leases: jit claim list --json")
}

/// Helper to create already claimed errors with standard remediation.
pub fn already_claimed(issue_id: &str, agent_id: &str, expires_info: &str) -> ActionableError {
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
    .with_remedy("If the agent crashed, force evict with: jit claim force-evict <lease-id> --reason \"<reason>\"")
}

/// Helper to create git repository detection errors with standard remediation.
pub fn not_in_git_repo() -> ActionableError {
    ActionableError::new("Not in a git repository")
        .with_cause("Current directory is not part of a git repository")
        .with_cause("Git is not installed or not in PATH")
        .with_remedy("Initialize a git repository: git init")
        .with_remedy("Change to a directory inside a git repository")
        .with_remedy("Verify git is installed: git --version")
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

        assert!(msg.contains("Error: Test error"));
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

        assert!(msg.contains("Error: Simple error"));
        assert!(!msg.contains("Possible causes:"));
        assert!(msg.contains("To fix:"));
        assert!(msg.contains("• Just fix it"));
    }

    #[test]
    fn test_error_without_remediation() {
        let error = ActionableError::new("Diagnostic only").with_cause("Something went wrong");

        let msg = error.to_error_message();

        assert!(msg.contains("Error: Diagnostic only"));
        assert!(msg.contains("Possible causes:"));
        assert!(msg.contains("• Something went wrong"));
        assert!(!msg.contains("To fix:"));
    }

    #[test]
    fn test_graph_rule_block_renders_rule_and_message_and_validation_code() {
        let error = TransitionBlockedError::graph_rules(
            "issue-123".to_string(),
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
            crate::output::ErrorCode::VALIDATION_FAILED
        );

        let rendered = error.to_string();
        assert!(rendered.contains("Graph rule validation failed"));
        assert!(rendered.contains("epic-done-needs-design-dep"));
        assert!(rendered.contains("must depend on a type:design issue"));
        // The remediation suggests explaining the rule and a forced override.
        assert!(rendered.contains("jit validate --explain issue-123"));
        assert!(rendered.contains("--force"));
        // It is NOT misreported as a gate or dependency block.
        assert!(!rendered.contains("incomplete dependencies"));
        assert!(!rendered.contains("gate(s) not passed"));
    }

    #[test]
    fn test_dependency_block_names_assign_in_remediation() {
        let error = TransitionBlockedError::dependencies(
            "issue-123".to_string(),
            State::InProgress,
            State::Backlog,
            vec![TransitionBlocker::MissingDependency {
                issue_id: "dep-456".to_string(),
            }],
        );

        let rendered = error.to_string();
        assert!(rendered.contains("jit issue assign issue-123 <assignee>"));
        assert!(rendered.contains("no state change"));

        // The same hint is present in the remediation list (the JSON path).
        assert!(error
            .remediation_commands()
            .iter()
            .any(|cmd| cmd.contains("jit issue assign issue-123")));
    }

    #[test]
    fn test_gate_block_does_not_name_assign_in_remediation() {
        let error = TransitionBlockedError::gates(
            "issue-123".to_string(),
            State::Done,
            State::Gated,
            vec![("code-review".to_string(), GateStatus::Pending)],
        );

        assert!(!error.to_string().contains("jit issue assign"));
    }

    #[test]
    fn test_lease_not_found_helper() {
        let error = lease_not_found("abc123");
        let msg = error.to_error_message();

        assert!(msg.contains("Lease abc123 not found"));
        assert!(msg.contains("jit claim list"));
        assert!(msg.contains("jit claim status"));
    }

    #[test]
    fn test_already_claimed_helper() {
        let error = already_claimed("issue-1", "agent:worker-1", "until 2026-01-15");
        let msg = error.to_error_message();

        assert!(msg.contains("Issue issue-1 already claimed"));
        assert!(msg.contains("agent:worker-1"));
        assert!(msg.contains("jit claim status --issue"));
        assert!(msg.contains("force-evict"));
    }

    #[test]
    fn test_not_in_git_repo_helper() {
        let error = not_in_git_repo();
        let msg = error.to_error_message();

        assert!(msg.contains("Not in a git repository"));
        assert!(msg.contains("git init"));
        assert!(msg.contains("git --version"));
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
        let actionable = no_active_lease("abc12345");
        let expected = actionable.to_string();
        let err = LeaseNotFoundError::new(&actionable);
        assert_eq!(err.to_string(), expected);
        assert_eq!(err.message(), expected);

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), expected);
        assert!(any.downcast_ref::<LeaseNotFoundError>().is_some());
    }

    #[test]
    fn test_lease_not_found_error_by_id_is_plain_message() {
        // The bare lookup path keeps its plain "Lease <id> not found" phrasing
        // (no remediation block) while still being the dedicated, downcastable type.
        let err = LeaseNotFoundError::by_id("xyz789");
        assert_eq!(err.to_string(), "Lease xyz789 not found");
        assert_eq!(err.message(), "Lease xyz789 not found");

        let any: anyhow::Error = err.into();
        assert_eq!(any.to_string(), "Lease xyz789 not found");
        assert!(any.downcast_ref::<LeaseNotFoundError>().is_some());
    }
}
