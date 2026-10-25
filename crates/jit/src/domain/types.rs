//! Core domain types for the issue tracker.
//!
//! This module defines the fundamental data structures used throughout the system:
//! issues, gates, events, and their associated states and priorities.

use crate::declarations::GateStage;
use crate::domain::gate_findings::GateFindings;
use crate::domain::identity::ProfileId;
use crate::domain::vocabulary::wire_vocabulary;
use crate::errors::InvalidArgumentError;
// `repository_state::path` is a self-contained identity vocabulary: it imports
// nothing from this crate and performs no I/O, so naming its root-relative path
// identity here keeps the domain layer pure while leaving one canonical type
// for a normalized repository-root-relative path.
use crate::repository_state::RootRelativePath;
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::str::FromStr;
#[cfg(test)]
use uuid::Uuid;

/// Length of short issue ID (git-style short hash)
pub const SHORT_ID_LENGTH: usize = 8;

wire_vocabulary! {
    /// Issue lifecycle state
    ///
    /// `PartialOrd`/`Ord` follow the declaration order, which is the lifecycle order
    /// (`Backlog < Ready < InProgress < Gated < Done < Rejected < Archived`); this
    /// lets states be stored in ordered collections such as `BTreeSet<State>`.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
    )]
    pub enum State {
        /// Created but not actionable yet (blocked by dependencies)
        Backlog = "backlog",
        /// All dependencies in a terminal state, ready for work (gates gate completion, not start)
        Ready = "ready",
        /// Currently being worked on
        InProgress = "in_progress",
        /// Work complete, awaiting quality gate approval
        Gated = "gated",
        /// Completed successfully
        Done = "done",
        /// Won't implement (bypasses gates)
        Rejected = "rejected",
        /// No longer relevant
        Archived = "archived",
    }
}

impl State {
    /// Check if this state is terminal (Done or Rejected)
    ///
    /// Terminal states represent closure - either successful completion (Done)
    /// or decision not to implement (Rejected). Both unblock dependent issues.
    pub fn is_terminal(self) -> bool {
        matches!(self, State::Done | State::Rejected)
    }
}

/// The terminal state an issue counts as for dependency, readiness, and
/// delivery-accounting purposes — its *effective* terminal state.
///
/// `Archived` is terminality-preserving (`jit:45a140ae`): archiving preserves
/// whatever was true before it. So the effective terminal state is:
///
/// - `Some(Done)` / `Some(Rejected)` for a literally terminal issue,
/// - `archived_from` for an `Archived` issue, but only when that recorded
///   pre-archive state was itself terminal (`Done`/`Rejected`),
/// - `None` otherwise — including an `Archived` issue parked from a non-terminal
///   state.
///
/// `state` is the issue's lifecycle state and `archived_from` its
/// [`Issue::archived_from`]; passing `archived_from` for a non-`Archived` `state`
/// is ignored.
pub fn effective_terminal_state(state: State, archived_from: Option<State>) -> Option<State> {
    match state {
        State::Done | State::Rejected => Some(state),
        State::Archived => archived_from.filter(|origin| origin.is_terminal()),
        _ => None,
    }
}

/// Whether an issue in `state` (archived from `archived_from`) is terminal for
/// dependency and readiness purposes.
///
/// The single archived-aware terminality predicate: true exactly when
/// [`effective_terminal_state`] resolves to a terminal state. Prefer the
/// convenience methods [`Issue::is_effectively_terminal`] /
/// [`MinimalIssue::is_effectively_terminal`] when a whole record is in hand.
pub fn is_effectively_terminal(state: State, archived_from: Option<State>) -> bool {
    effective_terminal_state(state, archived_from).is_some()
}

/// Whether a dependency whose target sits in `state` (archived from
/// `archived_from`) is met.
///
/// This is the single definition of dependency satisfaction: a dependency is met
/// exactly when its target reached an *effective* terminal state
/// ([`is_effectively_terminal`] — `Done`, `Rejected`, or `Archived` from one of
/// those). Every surface that asks whether a dependency still holds work back
/// routes through here: [`Issue::is_blocked`], the blocked-reason enumeration,
/// the transition blockers, and unmet-dependency rendering.
///
/// A dependency id that resolves to no issue is dangling. That is a separate
/// condition, reported on each surface's own terms, so it is not this predicate's
/// input.
pub fn is_dependency_met(state: State, archived_from: Option<State>) -> bool {
    is_effectively_terminal(state, archived_from)
}

/// Issue priority level
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// Low priority
    Low,
    /// Normal priority (default)
    Normal,
    /// High priority
    High,
    /// Critical priority
    Critical,
}

impl FromStr for Priority {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "low" => Ok(Priority::Low),
            "normal" => Ok(Priority::Normal),
            "high" => Ok(Priority::High),
            "critical" => Ok(Priority::Critical),
            _ => Err(InvalidArgumentError::new(format!("Invalid priority: {s}")).into()),
        }
    }
}

wire_vocabulary! {
    /// The content format of an issue body, selecting which [`ContentParser`] the
    /// validation projection uses to extract `sections`.
    ///
    /// When absent on an issue the repo default (`[validation].content_format`)
    /// applies; the final fallback is [`ContentFormat::DEFAULT`]. HTML/XML are
    /// only usable when the `html`/`xml` cargo features are built (see
    /// [`content_parser_for`](crate::document::content_parser_for)).
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
    pub enum ContentFormat {
        /// Markdown body (always available).
        Markdown = "markdown",
        /// HTML body (requires the `html` cargo feature to parse).
        Html = "html",
        /// XML body (requires the `xml` cargo feature to parse).
        Xml = "xml",
    }
}

impl ContentFormat {
    /// Format applied when neither the issue nor `[validation]` declares one.
    /// It must be a format that is always compiled in.
    pub const DEFAULT: Self = Self::Markdown;
}

impl Default for ContentFormat {
    fn default() -> Self {
        Self::DEFAULT
    }
}

wire_vocabulary! {
    /// Quality gate status
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
    pub enum GateStatus {
        /// Gate not yet evaluated
        Pending = "pending",
        /// Gate passed successfully
        Passed = "passed",
        /// Gate failed
        Failed = "failed",
    }
}

/// A parsed assignee in the documented `{kind}:{identifier}` form.
///
/// Issues and gate states identify the responsible agent or person with a
/// `kind:identifier` string (e.g. `agent:copilot`, `human:alice`,
/// `ci:github-actions`). This newtype is the single place that format is parsed
/// and validated: every value stored in [`Issue::assignee`] or
/// [`GateState::updated_by`] is built through [`Assignee::from_str`] (directly or
/// via deserialization), so a raw, unvalidated string can never reach storage.
///
/// The fields are private; construct an `Assignee` by parsing (`str::parse` /
/// [`FromStr`]) or take an engine-owned actor from its associated constant, and
/// read the parts via [`Assignee::kind`] / [`Assignee::identifier`]. It
/// serializes transparently as the `kind:identifier` string (through
/// [`Display`] / [`FromStr`]), so on-disk issue JSON is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Assignee {
    kind: Cow<'static, str>,
    identifier: Cow<'static, str>,
}

impl Assignee {
    /// Coordinator actor for listing active leases.
    pub const SYSTEM_LIST: Self = Self::engine_actor("system", "list");
    /// Coordinator actor for checking a lease.
    pub const SYSTEM_CHECK: Self = Self::engine_actor("system", "check");
    /// Coordinator actor for force-evicting a lease.
    pub const SYSTEM_ADMIN: Self = Self::engine_actor("system", "admin");
    /// Coordinator actor for claims and lease recovery.
    pub const SYSTEM_RECOVERY: Self = Self::engine_actor("system", "recovery");
    /// Coordinator actor guarding a repository mutation when no agent identity
    /// resolves.
    pub const SYSTEM_LEASE_CHECK: Self = Self::engine_actor("system", "lease-check");
    /// Actor stamped on the `by` field of a gate run recorded by the automated
    /// executor (as opposed to a human or agent attestation). A recorded pass
    /// whose `updated_by` equals this value is an auto-era pass with no human
    /// attestation behind it, which matters when a gate is later redefined to
    /// manual mode.
    pub const AUTO_EXECUTOR: Self = Self::engine_actor("auto", "executor");

    /// Builds an engine-owned actor without parsing. Private so that only the
    /// constants above use it; each must equal the parse of its spelling: a
    /// non-empty, colon-free `kind` and a non-empty `identifier`.
    const fn engine_actor(kind: &'static str, identifier: &'static str) -> Self {
        Self {
            kind: Cow::Borrowed(kind),
            identifier: Cow::Borrowed(identifier),
        }
    }

    /// The kind segment before the first colon (e.g. `agent` in `agent:copilot`).
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The identifier segment after the first colon (e.g. `copilot`).
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

/// Error returned when a string cannot be parsed as an [`Assignee`].
///
/// Distinguishes the failure modes so callers (e.g. agent-identity validation)
/// can map them to their own user-facing messages while reusing the one parse
/// path.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AssigneeParseError {
    /// The input was empty.
    #[error("Assignee cannot be empty")]
    Empty,
    /// The input had no `:` separating kind from identifier.
    #[error(
        "Assignee must be in format 'type:identifier' (e.g. 'agent:copilot', 'human:alice'); got '{0}'"
    )]
    MissingSeparator(String),
    /// The kind segment (before the colon) was empty.
    #[error(
        "Assignee kind cannot be empty; expected 'type:identifier' (e.g. 'agent:copilot'); got '{0}'"
    )]
    EmptyKind(String),
    /// The identifier segment (after the colon) was empty.
    #[error(
        "Assignee identifier cannot be empty; expected 'type:identifier' (e.g. 'agent:copilot'); got '{0}'"
    )]
    EmptyIdentifier(String),
}

impl FromStr for Assignee {
    type Err = AssigneeParseError;

    /// Parse a `kind:identifier` assignee, splitting on the first colon only.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(AssigneeParseError::Empty);
        }
        let (kind, identifier) = s
            .split_once(':')
            .ok_or_else(|| AssigneeParseError::MissingSeparator(s.to_string()))?;
        if kind.is_empty() {
            return Err(AssigneeParseError::EmptyKind(s.to_string()));
        }
        if identifier.is_empty() {
            return Err(AssigneeParseError::EmptyIdentifier(s.to_string()));
        }
        Ok(Assignee {
            kind: Cow::Owned(kind.to_string()),
            identifier: Cow::Owned(identifier.to_string()),
        })
    }
}

impl std::fmt::Display for Assignee {
    /// Render as the canonical `kind:identifier` string.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.kind, self.identifier)
    }
}

/// Compare an [`Assignee`] against a raw `kind:identifier` string without
/// allocating, so filter predicates can match a parsed assignee to user input.
impl PartialEq<str> for Assignee {
    fn eq(&self, other: &str) -> bool {
        match other.split_once(':') {
            Some((kind, identifier)) => self.kind == kind && self.identifier == identifier,
            None => false,
        }
    }
}

impl Serialize for Assignee {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

/// Schema for the `{type}:{identifier}` string form `Serialize` writes and
/// `Deserialize` reads, hand-written because the serde impls are (@/inv/assignee-format).
impl schemars::JsonSchema for Assignee {
    fn schema_name() -> String {
        "Assignee".to_string()
    }

    fn json_schema(generator: &mut schemars::gen::SchemaGenerator) -> schemars::schema::Schema {
        String::json_schema(generator)
    }
}

impl<'de> Deserialize<'de> for Assignee {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(serde::de::Error::custom)
    }
}

/// State of a quality gate for a specific issue
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateState {
    /// Current status of the gate
    pub status: GateStatus,
    /// Who updated the gate status (e.g., "human:alice", "ci:github-actions")
    #[schemars(with = "Option<String>")]
    pub updated_by: Option<Assignee>,
    /// When the gate was last updated
    pub updated_at: DateTime<Utc>,
    /// The gate run this status was projected from, which every latest-run view
    /// selects; absent while no run has been recorded since the gate became
    /// required.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl GateState {
    /// The run of `gate_key` among `runs` this status was projected from.
    pub fn projected_run<'a>(
        &self,
        gate_key: &str,
        runs: &'a [GateRunResult],
    ) -> Option<&'a GateRunResult> {
        let run_id = self.run_id.as_deref()?;
        runs.iter()
            .find(|run| run.run_id == run_id && run.gate_key == gate_key)
    }
}

/// An issue representing a unit of work
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "IssueWire")]
pub struct Issue {
    /// Unique identifier (UUID)
    pub id: String,
    /// Short summary of the issue
    pub title: String,
    /// Detailed description and acceptance criteria
    pub description: String,
    /// Current lifecycle state
    pub state: State,
    /// Priority level
    pub priority: Priority,
    /// Assigned agent or person (format: "type:identifier")
    #[schemars(with = "Option<String>")]
    pub assignee: Option<Assignee>,
    /// IDs of issues that must be done first
    pub dependencies: Vec<String>,
    /// Gate keys that must pass before ready/done
    pub gates_required: Vec<String>,
    /// Current status of each required gate
    pub gates_status: HashMap<String, GateState>,
    /// Flexible key-value storage for agent-specific data
    pub context: HashMap<String, String>,
    /// References to design documents, notes, and artifacts
    pub documents: Vec<DocumentReference>,
    /// Labels for categorization and hierarchy (format: "namespace:value")
    pub labels: Vec<String>,
    /// Content format of the `description` body, selecting the parser used to
    /// extract `sections` during validation. Absent (`None`) means inherit the
    /// repo default (`[validation].content_format`), falling back to
    /// [`ContentFormat::DEFAULT`].
    /// Existing issue files without this field deserialize as `None` and are NOT
    /// rewritten (skipped on serialize when absent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_format: Option<ContentFormat>,
    /// When the issue was created (serialized as an RFC 3339 timestamp)
    pub created_at: DateTime<Utc>,
    /// When the issue was last updated (serialized as an RFC 3339 timestamp)
    pub updated_at: DateTime<Utc>,
    /// When the issue FIRST entered [`State::Ready`] (RFC 3339 timestamp).
    ///
    /// Set once, at the first Ready transition (including the auto-promotion of a
    /// dependency-free issue at creation); a later cycle back through Ready leaves
    /// the original value intact ("first occurrence" semantics). Absent (`None`)
    /// for an issue that never reached Ready, and for issues predating this field
    /// whose event log carries no `issue_state_changed` into Ready. Skipped on
    /// serialize when absent, so old issue files round-trip unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_ready_at: Option<DateTime<Utc>>,
    /// When the issue was FIRST claimed/assigned (RFC 3339 timestamp).
    ///
    /// Set once, at the first assignment (`jit issue claim`/`assign`); a later
    /// re-assignment leaves the original value intact. Absent (`None`) for an
    /// unclaimed issue and for pre-existing issues with no claim in the event log.
    /// Skipped on serialize when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claimed_at: Option<DateTime<Utc>>,
    /// When the issue FIRST reached [`State::Done`] (RFC 3339 timestamp).
    ///
    /// Set once, at the first Done transition; re-opening and re-completing does
    /// NOT overwrite it ("first occurrence" semantics). Absent (`None`) for an
    /// issue never completed and for pre-existing issues with no completion in the
    /// event log. Skipped on serialize when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub done_at: Option<DateTime<Utc>>,
    /// The lifecycle state this issue held immediately before entering
    /// [`State::Archived`], making `Archived` terminality-preserving
    /// (`jit:45a140ae`).
    ///
    /// Set when the issue transitions into `Archived` (recording the state it
    /// left) and cleared on revive. It drives the issue's effective terminal
    /// state ([`Issue::effective_terminal_state`]) and constrains revive to the
    /// recorded origin. Absent (`None`) for every non-`Archived` issue; decoding
    /// an `Archived` record without it fails. Skipped on serialize when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_from: Option<State>,
}

/// Decode-boundary mirror of [`Issue`]; [`TryFrom`] enforces that an `Archived`
/// record carries `archived_from`.
#[derive(Deserialize)]
struct IssueWire {
    id: String,
    title: String,
    description: String,
    state: State,
    priority: Priority,
    assignee: Option<Assignee>,
    dependencies: Vec<String>,
    gates_required: Vec<String>,
    gates_status: HashMap<String, GateState>,
    context: HashMap<String, String>,
    documents: Vec<DocumentReference>,
    labels: Vec<String>,
    #[serde(default)]
    content_format: Option<ContentFormat>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(default)]
    first_ready_at: Option<DateTime<Utc>>,
    #[serde(default)]
    claimed_at: Option<DateTime<Utc>>,
    #[serde(default)]
    done_at: Option<DateTime<Utc>>,
    #[serde(default)]
    archived_from: Option<State>,
}

impl TryFrom<IssueWire> for Issue {
    type Error = String;

    fn try_from(wire: IssueWire) -> std::result::Result<Self, Self::Error> {
        if wire.state == State::Archived && wire.archived_from.is_none() {
            return Err(format!(
                "missing field `archived_from` on archived issue {}",
                wire.id
            ));
        }
        Ok(Self {
            id: wire.id,
            title: wire.title,
            description: wire.description,
            state: wire.state,
            priority: wire.priority,
            assignee: wire.assignee,
            dependencies: wire.dependencies,
            gates_required: wire.gates_required,
            gates_status: wire.gates_status,
            context: wire.context,
            documents: wire.documents,
            labels: wire.labels,
            content_format: wire.content_format,
            created_at: wire.created_at,
            updated_at: wire.updated_at,
            first_ready_at: wire.first_ready_at,
            claimed_at: wire.claimed_at,
            done_at: wire.done_at,
            archived_from: wire.archived_from,
        })
    }
}

#[cfg(test)]
pub(crate) fn fixture_issue(title: String, description: String) -> Issue {
    let mut issue = Issue::draft(title, description);
    issue.id = Uuid::new_v4().to_string();
    issue
}

/// The readiness change the dependency graph demands for a stored issue.
///
/// Readiness is stored on the issue and is also derivable from the graph, so the
/// two views must agree. [`Issue::derive_readiness_correction`] names the change
/// that restores agreement, and every dependency-mutating path applies the
/// direction it owns: an edge addition can only demote, an edge removal or a
/// dependency reaching a terminal state can only promote.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadinessCorrection {
    /// A [`State::Backlog`] issue nothing blocks belongs in [`State::Ready`].
    Promote,
    /// A [`State::Ready`] issue with at least one unmet dependency belongs in
    /// [`State::Backlog`].
    Demote,
}

impl Issue {
    /// Draft an issue carrying no authoritative id or lifecycle timestamps.
    ///
    /// The id is empty and the lifecycle timestamps hold a sentinel epoch; the
    /// `repository_state` finalizer assigns the real id and stamps `created_at`,
    /// `updated_at`, and `first_ready_at`. Command producers build drafts so time
    /// and identity authority stay solely in the finalizer, never in a command.
    pub fn draft(title: String, description: String) -> Self {
        let sentinel = DateTime::from_timestamp(0, 0).expect("epoch is representable");
        Self {
            id: String::new(),
            title,
            description,
            state: State::Backlog,
            priority: Priority::Normal,
            assignee: None,
            dependencies: Vec::new(),
            gates_required: Vec::new(),
            gates_status: HashMap::new(),
            context: HashMap::new(),
            documents: Vec::new(),
            labels: Vec::new(),
            content_format: None,
            created_at: sentinel,
            updated_at: sentinel,
            first_ready_at: None,
            claimed_at: None,
            done_at: None,
            archived_from: None,
        }
    }

    /// Get short ID (first 8 characters of UUID)
    ///
    /// Returns a git-style short hash for human-readable output.
    /// Minimum length is 8 characters for reasonable collision resistance.
    pub fn short_id(&self) -> String {
        self.id.chars().take(SHORT_ID_LENGTH).collect()
    }

    /// The terminal state this issue counts as for dependency, readiness, and
    /// delivery accounting — its effective terminal state
    /// ([`effective_terminal_state`]). `Some(Done)`/`Some(Rejected)` when
    /// literally terminal or archived from one of those; `None` otherwise.
    pub fn effective_terminal_state(&self) -> Option<State> {
        effective_terminal_state(self.state, self.archived_from)
    }

    /// Whether this issue is terminal for dependency and readiness purposes,
    /// accounting for terminality-preserving `Archived`
    /// ([`is_effectively_terminal`]).
    pub fn is_effectively_terminal(&self) -> bool {
        is_effectively_terminal(self.state, self.archived_from)
    }

    /// Check if this issue is blocked by unmet dependencies
    ///
    /// Returns true if any dependency is unmet by [`is_dependency_met`], including
    /// a dependency whose id resolves to no issue.
    /// Note: Gates do not block work from starting, only from completing.
    pub fn is_blocked(&self, resolved_issues: &HashMap<String, &Issue>) -> bool {
        self.dependencies.iter().any(|dep_id| {
            !matches!(resolved_issues.get(dep_id), Some(issue) if is_dependency_met(issue.state, issue.archived_from))
        })
    }

    /// Check if this issue has unpassed gates
    ///
    /// Returns true if any required gate hasn't passed.
    /// Used to determine if issue can transition to Done.
    pub fn has_unpassed_gates(&self) -> bool {
        self.gates_required
            .iter()
            .any(|gate_key| !matches!(self.gates_status.get(gate_key), Some(gate_state) if gate_state.status == GateStatus::Passed))
    }

    /// Get list of unpassed gates
    ///
    /// Returns a vector of gate keys that have not yet passed.
    pub fn get_unpassed_gates(&self) -> Vec<String> {
        self.gates_required
            .iter()
            .filter(|gate_key| !matches!(self.gates_status.get(*gate_key), Some(gate_state) if gate_state.status == GateStatus::Passed))
            .cloned()
            .collect()
    }

    /// Derive the readiness correction the dependency graph demands for this
    /// issue, or `None` when the stored state already agrees with the graph.
    ///
    /// This is the single derivation of readiness from the graph: every path
    /// that mutates a dependency edge — `jit dep add`/`remove`, graph-template
    /// application, and the `jit validate --fix` repair — decides here, so no two
    /// of them can drift (`@/invariant/derived-state-coherence`). The
    /// whole-repository check reports a [`ReadinessCorrection::Demote`] as a
    /// violation.
    ///
    /// Only [`State::Backlog`] and [`State::Ready`] are graph-derived. Every
    /// other state is owned by an explicit transition — an issue already claimed,
    /// gated, or terminal is not re-derived from its dependencies — and yields
    /// `None`.
    pub fn derive_readiness_correction(
        &self,
        resolved_issues: &HashMap<String, &Issue>,
    ) -> Option<ReadinessCorrection> {
        match (self.state, self.is_blocked(resolved_issues)) {
            (State::Backlog, false) => Some(ReadinessCorrection::Promote),
            (State::Ready, true) => Some(ReadinessCorrection::Demote),
            _ => None,
        }
    }

    /// Check if this issue should auto-transition to Ready state
    /// A Backlog issue transitions to Ready when it becomes unblocked
    pub fn should_auto_transition_to_ready(
        &self,
        resolved_issues: &HashMap<String, &Issue>,
    ) -> bool {
        self.derive_readiness_correction(resolved_issues) == Some(ReadinessCorrection::Promote)
    }

    /// Check if this issue should auto-transition to Done state
    /// A Gated issue transitions to Done when all required gates pass
    pub fn should_auto_transition_to_done(&self) -> bool {
        self.state == State::Gated && !self.has_unpassed_gates()
    }

    /// Stamp [`first_ready_at`](Self::first_ready_at) at the FIRST Ready
    /// transition; a later cycle back through Ready is a no-op.
    ///
    /// Uses [`Option::get_or_insert`], so it records `at` only when the field is
    /// still absent — the "first occurrence" semantics the lifecycle timestamps
    /// require. Call at every path that lands [`State::Ready`].
    pub fn mark_first_ready(&mut self, at: DateTime<Utc>) {
        self.first_ready_at.get_or_insert(at);
    }

    /// Stamp [`claimed_at`](Self::claimed_at) at the FIRST claim/assignment; a
    /// later re-assignment is a no-op.
    pub fn mark_claimed(&mut self, at: DateTime<Utc>) {
        self.claimed_at.get_or_insert(at);
    }

    /// Stamp [`done_at`](Self::done_at) at the FIRST Done transition; re-opening
    /// and re-completing does NOT overwrite it.
    pub fn mark_done(&mut self, at: DateTime<Utc>) {
        self.done_at.get_or_insert(at);
    }
}

/// Minimal issue representation for efficient list queries
///
/// Returns only essential fields to reduce token usage. Use `jit issue show`
/// for full details.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MinimalIssue {
    pub id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    /// Assigned agent or person (optional for context)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    /// Labels for categorization (optional for context)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    /// Pre-archive origin state, carried so a dependency projection can decide
    /// effective terminality without the full record. Mirrors
    /// [`Issue::archived_from`]: `Some` exactly for an `Archived` issue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_from: Option<State>,
}

// Custom serialization to add computed short_id
impl Serialize for MinimalIssue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("MinimalIssue", 8)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("short_id", &self.short_id())?;
        state.serialize_field("title", &self.title)?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("priority", &self.priority)?;
        if let Some(ref assignee) = self.assignee {
            state.serialize_field("assignee", assignee)?;
        } else {
            state.serialize_field("assignee", &None::<String>)?;
        }
        if !self.labels.is_empty() {
            state.serialize_field("labels", &self.labels)?;
        }
        if let Some(archived_from) = self.archived_from {
            state.serialize_field("archived_from", &archived_from)?;
        }
        state.end()
    }
}

impl From<&Issue> for MinimalIssue {
    fn from(issue: &Issue) -> Self {
        Self {
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee.as_ref().map(Assignee::to_string),
            labels: issue.labels.clone(),
            archived_from: issue.archived_from,
        }
    }
}

impl MinimalIssue {
    /// Get short ID (first 8 characters of UUID)
    pub fn short_id(&self) -> String {
        self.id.chars().take(SHORT_ID_LENGTH).collect()
    }

    /// Whether this issue is terminal for dependency and readiness purposes,
    /// accounting for terminality-preserving `Archived`
    /// ([`is_effectively_terminal`]).
    pub fn is_effectively_terminal(&self) -> bool {
        is_effectively_terminal(self.state, self.archived_from)
    }
}

/// Minimal blocked issue for queries - includes blocking reasons
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct MinimalBlockedIssue {
    pub id: String,
    pub title: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub blocked_reasons: Vec<String>,
}

// Custom serialization to add computed short_id (mirrors MinimalIssue)
impl Serialize for MinimalBlockedIssue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("MinimalBlockedIssue", 8)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("short_id", &self.short_id())?;
        state.serialize_field("title", &self.title)?;
        state.serialize_field("state", &self.state)?;
        state.serialize_field("priority", &self.priority)?;
        if let Some(ref assignee) = self.assignee {
            state.serialize_field("assignee", assignee)?;
        } else {
            state.serialize_field("assignee", &None::<String>)?;
        }
        if !self.labels.is_empty() {
            state.serialize_field("labels", &self.labels)?;
        }
        state.serialize_field("blocked_reasons", &self.blocked_reasons)?;
        state.end()
    }
}

impl MinimalBlockedIssue {
    pub fn short_id(&self) -> String {
        self.id.chars().take(SHORT_ID_LENGTH).collect()
    }
}

impl From<(&Issue, Vec<String>)> for MinimalBlockedIssue {
    fn from((issue, blocked_reasons): (&Issue, Vec<String>)) -> Self {
        Self {
            id: issue.id.clone(),
            title: issue.title.clone(),
            state: issue.state,
            priority: issue.priority,
            assignee: issue.assignee.as_ref().map(Assignee::to_string),
            labels: issue.labels.clone(),
            blocked_reasons,
        }
    }
}

/// Implement GraphNode for Issue to enable dependency graph operations
impl crate::graph::GraphNode for Issue {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> impl Iterator<Item = &str> + '_ {
        self.dependencies.iter().map(String::as_str)
    }
}

/// A reference to a document (design doc, notes, artifact) in the repository
///
/// Documents can reference files at HEAD or specific git commits for
/// version-aware knowledge management.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DocumentReference {
    /// Path relative to repository root (e.g., "docs/api-design.md")
    pub path: String,
    /// Optional git commit hash (None = HEAD, Some("a1b2c3d") = specific commit)
    pub commit: Option<String>,
    /// Human-readable label (e.g., "API Design Document")
    pub label: Option<String>,
    /// Document type hint (e.g., "design", "implementation", "notes")
    pub doc_type: Option<String>,
    /// Document format (e.g., "markdown", "asciidoc", "rst")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Assets referenced by this document (images, diagrams, etc.)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<crate::document::Asset>,
}

/// Structured context passed to gate checker processes
///
/// When `pass_context` is enabled on a gate checker, this context is serialized
/// to a JSON file and made available via the `JIT_CONTEXT_FILE` env var.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateContext {
    /// Schema version for forward compatibility
    pub schema_version: u32,
    /// Resolved prompt (from inline `prompt` or `prompt_file`)
    pub prompt: Option<String>,
    /// Full issue data as JSON, excluding the current gate's stale pre-run
    /// projection; the current gate is identified by [`Self::gate`].
    pub issue: serde_json::Value,
    /// Gate definition as JSON value
    pub gate: serde_json::Value,
    /// Latest run for this gate+issue pair, or an empty vector when none exists.
    /// Structured runs retain findings and metadata but omit stdout and stderr;
    /// legacy unstructured runs retain stdout as a compatibility fallback.
    pub run_history: Vec<GateRunResult>,
}

/// Record-format version stamped into every [`GateRunResult`] a gate execution
/// records, and the version this binary writes into
/// `.jit/gate-runs/<run-id>/result.json`.
pub const GATE_RUN_SCHEMA_VERSION: u32 = 1;

/// Result of a gate execution, as persisted to
/// `.jit/gate-runs/<run-id>/result.json`.
///
/// `JsonSchema` is derived so the storage reference's freshness guard can read
/// this record's field set off the derived schema
/// (`storage::reference::tests::test_gate_run_fields_match_derived_schema`): a
/// field added here appears there, and the projection fails until the field is
/// documented.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GateRunResult {
    /// Schema version for future evolution ([`GATE_RUN_SCHEMA_VERSION`])
    pub schema_version: u32,
    /// Unique run identifier
    pub run_id: String,
    /// Gate key that was executed
    pub gate_key: String,
    /// Stage at which gate was executed
    pub stage: GateStage,
    /// Issue ID
    pub issue_id: String,
    /// Git commit the checker was launched at, when one was available. `None`
    /// for a run that launched no checker — a verdict taken from an earlier run
    /// ([`origin`](Self::origin)) ran at no commit of its own, and the run it
    /// names carries the one its checker was launched at.
    pub commit: Option<String>,
    /// Git branch the checker was launched on, under the same conditions as
    /// [`commit`](Self::commit).
    pub branch: Option<String>,
    /// Result status
    pub status: GateRunStatus,
    /// When execution started
    pub started_at: DateTime<Utc>,
    /// When execution completed
    pub completed_at: Option<DateTime<Utc>>,
    /// Duration in milliseconds
    pub duration_ms: Option<u64>,
    /// Exit code (for command execution)
    pub exit_code: Option<i32>,
    /// Standard output
    pub stdout: String,
    /// Standard error
    pub stderr: String,
    /// Command that was executed
    pub command: String,
    /// Who triggered this execution
    pub by: Option<Assignee>,
    /// Optional message
    pub message: Option<String>,
    /// Structured findings parsed from the checker's machine-readable block, if
    /// one was emitted. `None` for plain-text checkers and for runs recorded
    /// before this field existed; the raw [`stdout`](Self::stdout) is always
    /// kept alongside. See [`parse_gate_findings`](crate::domain::gate_findings::parse_gate_findings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<GateFindings>,
    /// Digest of the repository files the gate declares its checker reads, taken
    /// before the verdict was obtained.
    ///
    /// `None` when the gate declares no inputs, or when the repository the run
    /// was recorded against offers no worktree to read. Two runs of one gate
    /// carrying the same digest read byte-identical content at byte-identical
    /// paths, which is what makes [`origin`](Self::origin) able to carry a
    /// verdict from one to the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs_digest: Option<crate::domain::repository_inputs::InputsDigest>,
    /// Where this record's verdict came from: this run's own checker execution,
    /// an earlier run over the same declared inputs, or a manual attestation.
    #[serde(default)]
    pub origin: GateVerdictOrigin,
    /// Whether the evaluation inputs this verdict was derived over still
    /// matched the repository when the run was recorded.
    #[serde(default)]
    pub evaluation_stability: GateEvaluationStability,
}

impl GateRunResult {
    /// Whether an evaluation of `gate_key` over inputs digesting to `digest`
    /// may carry this run's verdict.
    ///
    /// Three things must hold together. The digests must match, which is what
    /// makes the two evaluations questions about identical content. The run
    /// must have executed its own checker, so the record a reuse names is
    /// always the execution behind the verdict rather than another reference
    /// to it. And the run must have reached a verdict — `passed` or `failed`;
    /// an errored run reports the checker never getting there (a timeout, a
    /// missing command), which says nothing about the inputs.
    pub fn is_reusable_for(
        &self,
        gate_key: &str,
        digest: &crate::domain::repository_inputs::InputsDigest,
    ) -> bool {
        self.gate_key == gate_key
            && self.inputs_digest.as_ref() == Some(digest)
            && self.origin.is_executed()
            && matches!(self.status, GateRunStatus::Passed | GateRunStatus::Failed)
    }

    /// The draft record of a manual gate's attestation of `status` by `by`.
    ///
    /// No checker ran, so the record carries no command, exit code, report
    /// text, or inputs digest, and its [`GateVerdictOrigin::Attested`] origin
    /// keeps it out of verdict reuse.
    pub fn attestation(
        issue_id: &str,
        gate_key: &str,
        stage: GateStage,
        status: GateStatus,
        by: Option<&Assignee>,
    ) -> Self {
        Self {
            schema_version: GATE_RUN_SCHEMA_VERSION,
            // Repository mutation finalization assigns the durable run identity
            // and stamps the attestation time.
            run_id: String::new(),
            gate_key: gate_key.to_string(),
            stage,
            issue_id: issue_id.to_string(),
            commit: None,
            branch: None,
            status: match status {
                GateStatus::Passed => GateRunStatus::Passed,
                GateStatus::Failed => GateRunStatus::Failed,
                GateStatus::Pending => GateRunStatus::Pending,
            },
            started_at: DateTime::UNIX_EPOCH,
            completed_at: None,
            duration_ms: None,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
            command: String::new(),
            by: by.cloned(),
            message: None,
            findings: None,
            inputs_digest: None,
            origin: GateVerdictOrigin::Attested,
            evaluation_stability: GateEvaluationStability::Stable,
        }
    }
}

/// The most recently started run among `runs`, or `None` when empty.
///
/// Ties on the start instant fall back to the run id so one repository state
/// always selects one run.
pub fn latest_gate_run(runs: Vec<GateRunResult>) -> Option<GateRunResult> {
    runs.into_iter().max_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then_with(|| left.run_id.cmp(&right.run_id))
    })
}

/// Where a recorded gate verdict came from.
///
/// A reused verdict is evidence of one checker execution read a second time,
/// not of a second verification. Keeping the distinction in the record is what
/// stops a container whose issues all report a passed gate from being read as
/// that many independent verifications, and the named run is where the report
/// text and findings that produced the verdict live.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "derivation",
    content = "source_run",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum GateVerdictOrigin {
    /// This run executed the checker and the verdict is its own.
    #[default]
    Executed,
    /// The verdict was taken from the run with this id, whose declared inputs
    /// digested to the same value. That run executed the checker.
    Reused(String),
    /// The verdict is a manual gate's attestation: `by` names the attestor and
    /// `started_at` the time. No checker ran.
    Attested,
}

/// Whether the evaluation inputs a verdict was derived over still matched the
/// repository when the run was recorded.
///
/// A checker is launched against one captured repository image. `InputsMoved`
/// says the repository changed under a checker that had already started, so the
/// verdict is the checker's own, taken over the state captured before it ran,
/// rather than over the state the record was published against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateEvaluationStability {
    /// The evaluation inputs held still from capture through recording.
    #[default]
    Stable,
    /// The evaluation inputs moved while the checker ran.
    InputsMoved,
}

impl GateVerdictOrigin {
    /// The earlier run this verdict was taken from, or `None` when the verdict
    /// is this run's own.
    pub fn source_run(&self) -> Option<&str> {
        match self {
            Self::Executed | Self::Attested => None,
            Self::Reused(run_id) => Some(run_id),
        }
    }

    /// Whether this run executed the checker that produced its verdict.
    pub fn is_executed(&self) -> bool {
        matches!(self, Self::Executed)
    }
}

/// Gate run status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateRunStatus {
    /// Check succeeded
    Passed,
    /// Check failed (expected failure, e.g., tests failed)
    Failed,
    /// Unexpected error (timeout, command not found, crash)
    Error,
    /// Not yet run (for manual gates)
    Pending,
}

/// Origin vocabulary persisted for an installed profile package.
///
/// The location a package was read from is carried by the origin itself, so a
/// persisted origin cannot name a repository location without saying which. The
/// payload is [`RootRelativePath`], the repository's own normalized
/// root-relative path identity, whose parse and deserialization reject an
/// absolute, traversing, or otherwise non-canonical spelling; a location is
/// relative to the worktree root, so a stored record stays readable wherever
/// the repository is checked out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(
    tag = "source",
    content = "location",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ProfileOrigin {
    /// Package bytes were read from this worktree-relative directory.
    Directory(RootRelativePath),
}

/// The lifecycle operation requested by a profile mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileLifecycleOperation {
    /// Create a repository and install its selected profiles.
    Initialize,
    /// Apply a selected package closure.
    Apply,
}

/// The action observed for one profile in a lifecycle mutation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileLifecycleStatus {
    /// The operation published the profile's content.
    Installed,
    /// The profile and all of its materializations were already current.
    Unchanged,
}

/// One profile's lifecycle result within an aggregate mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProfileLifecycleProfile {
    /// Stable profile identifier.
    pub id: ProfileId,
    /// The package version applied.
    pub version: String,
    /// The action observed for this profile.
    pub status: ProfileLifecycleStatus,
    /// The variable values applied, by name.
    pub variables: std::collections::BTreeMap<crate::profile::ProfileVariableName, String>,
}

/// The stance on state-mutating commands inside a linked non-primary checkout.
///
/// One vocabulary carries both the repository's declared stance and the
/// per-invocation override that can outrank it, so an audit record names the
/// two with one type instead of two parallel ones.
///
/// This type is also the parser for both sources of that vocabulary: the
/// `[worktree] write_policy` TOML key deserializes through
/// [`FromStr`](std::str::FromStr) (case-insensitively), and so does the
/// per-invocation override read from the environment, so neither source can
/// drift from the other's accepted tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LinkedCheckoutWriteStance {
    /// State-mutating commands are refused inside a linked checkout.
    Refuse,
    /// State-mutating commands may run inside a linked checkout.
    Allow,
}

/// Failure to parse a linked-checkout write stance token.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkedCheckoutWriteStanceError {
    /// The value is not one of `"refuse"` or `"allow"`.
    #[error("invalid linked-checkout write stance '{value}'; expected 'refuse' or 'allow'")]
    Invalid {
        /// The unrecognised value.
        value: String,
    },
}

impl std::str::FromStr for LinkedCheckoutWriteStance {
    type Err = LinkedCheckoutWriteStanceError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.to_lowercase().as_str() {
            "refuse" => Ok(LinkedCheckoutWriteStance::Refuse),
            "allow" => Ok(LinkedCheckoutWriteStance::Allow),
            _ => Err(LinkedCheckoutWriteStanceError::Invalid {
                value: value.to_string(),
            }),
        }
    }
}

impl<'de> Deserialize<'de> for LinkedCheckoutWriteStance {
    fn deserialize<D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Self, <D as serde::Deserializer<'de>>::Error> {
        let token = String::deserialize(deserializer)?;
        token
            .parse::<LinkedCheckoutWriteStance>()
            .map_err(serde::de::Error::custom)
    }
}

impl LinkedCheckoutWriteStance {
    /// Default stance when a repository declares none — whether because
    /// `[worktree]` omits `write_policy` or the `[worktree]` section is
    /// absent altogether. Refusing is the safe default: the adopter who
    /// never considered the question is the one the divergent-store hazard
    /// hurts.
    pub const DEFAULT: LinkedCheckoutWriteStance = LinkedCheckoutWriteStance::Refuse;

    /// Resolve the effective stance for one invocation.
    ///
    /// `invocation_override`, when present, always takes precedence over
    /// `declared` — an explicit per-invocation choice overrides whatever the
    /// repository declares. This is the ONE place this precedence is
    /// stated; callers must not re-decide it themselves.
    pub fn resolve(
        declared: LinkedCheckoutWriteStance,
        invocation_override: Option<LinkedCheckoutWriteStance>,
    ) -> LinkedCheckoutWriteStance {
        invocation_override.unwrap_or(declared)
    }

    /// The token naming this stance: the inverse of
    /// [`FromStr`](std::str::FromStr), and the same text the type serializes to,
    /// so prose naming a stance and a machine-readable field carrying one agree.
    pub const fn as_token(self) -> &'static str {
        match self {
            LinkedCheckoutWriteStance::Refuse => "refuse",
            LinkedCheckoutWriteStance::Allow => "allow",
        }
    }
}

impl std::fmt::Display for LinkedCheckoutWriteStance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_token())
    }
}

/// System event types for audit log
///
/// `JsonSchema` is derived so the event catalog's freshness guard can read the
/// serde `type` tag of every variant off the derived schema
/// (`event_catalog::tests::test_event_variant_tags_are_all_cataloged`): a new
/// variant appears there whether or not it was given a distinct [`EventTag`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    /// A new issue was created
    IssueCreated {
        /// Event ID
        id: String,
        /// Issue that was created
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Issue title
        title: String,
        /// Issue priority
        priority: Priority,
    },
    /// An issue was claimed by an agent
    IssueClaimed {
        /// Event ID
        id: String,
        /// Issue that was claimed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Who claimed it (format: "type:identifier")
        assignee: Assignee,
    },
    /// Issue state transitioned
    IssueStateChanged {
        /// Event ID
        id: String,
        /// Issue that changed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Previous state
        from: State,
        /// New state
        to: State,
    },
    /// A quality gate passed
    GatePassed {
        /// Event ID
        id: String,
        /// Issue with the gate
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that passed
        gate_key: String,
        /// Who marked it as passed (format: "type:identifier")
        updated_by: Option<Assignee>,
    },
    /// A quality gate failed
    GateFailed {
        /// Event ID
        id: String,
        /// Issue with the gate
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that failed
        gate_key: String,
        /// Who marked it as failed (format: "type:identifier")
        updated_by: Option<Assignee>,
    },
    /// A quality gate was added to an issue
    GateAdded {
        /// Event ID
        id: String,
        /// Issue to which gate was added
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that was added
        gate_key: String,
    },
    /// A quality gate was removed from an issue
    GateRemoved {
        /// Event ID
        id: String,
        /// Issue from which gate was removed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Gate that was removed
        gate_key: String,
    },
    /// Issue was completed
    IssueCompleted {
        /// Event ID
        id: String,
        /// Issue that completed
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
    },
    /// Issue was permanently deleted
    IssueDeleted {
        /// Event ID
        id: String,
        /// Issue that was deleted
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
    },
    /// Issue was released from assignee
    IssueReleased {
        /// Event ID
        id: String,
        /// Issue that was released
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Previous assignee (format: "type:identifier")
        assignee: Assignee,
        /// Reason for release
        reason: String,
    },
    /// A dependency-aware archive execution reached its durable commit point.
    ArtifactArchiveExecuted {
        /// Event ID.
        id: String,
        /// When the commit point was recorded.
        timestamp: DateTime<Utc>,
        /// Container or document selected for archival.
        target: crate::domain::artifact_plan::PlanTarget,
        /// Repository-relative mirror root used by the operation.
        destination_root: String,
        /// New or adopted publications made durable before this record.
        publications: Vec<crate::domain::artifact_execution::ArchivePublication>,
        /// Exact issue-document reference changes made durable before this record.
        reference_changes: Vec<crate::domain::artifact_plan::ReferenceChange>,
        /// Identity-guarded removals attempted after this record.
        planned_deletions: Vec<crate::domain::artifact_plan::PendingDeletion>,
        /// Whether the record reconciles durable state left by an earlier failed append.
        reconciling: bool,
    },
    /// Issue was updated (labels, priority, assignee, etc.)
    IssueUpdated {
        /// Event ID
        id: String,
        /// Issue that was updated
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Who updated it (e.g., "bulk-update", "human:alice", "agent:copilot")
        updated_by: String,
        /// Fields that changed
        fields: Vec<String>,
    },
    /// Redundant dependencies were removed by transitive reduction during validate --fix
    DependencyReduced {
        /// Event ID
        id: String,
        /// Issue whose dependencies were reduced
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Number of dependencies before reduction
        old_count: usize,
        /// Number of dependencies after reduction
        new_count: usize,
        /// IDs of the removed (redundant) dependencies
        removed_deps: Vec<String>,
    },
    /// A blocking validation rule was bypassed via `--force` on a write.
    ///
    /// Emitted once per enforce-rule whose `error` finding was overridden by
    /// `--force` during an issue create/update/bulk write. This is the
    /// audit-sensitive override (DR §7.6): ordinary rule rejections and
    /// read-only `jit validate` runs are NOT logged, only the deliberate bypass.
    LocalRuleBypassed {
        /// Event ID
        id: String,
        /// Issue whose write bypassed the rule
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Name of the enforce rule that was bypassed
        rule: String,
    },
    /// A state transition was blocked by an enforcing graph rule (CC-2).
    ///
    /// Emitted once per blocking rule, BEFORE the transition error is returned —
    /// the attempted transition is the auditable act. Distinct from a gate or
    /// dependency block: this records that a `Scope::Graph` rule with `enforce =
    /// true` produced an `error` finding attributed to the issue in its target
    /// state, so the transition into `target` was refused.
    TransitionBlocked {
        /// Event ID
        id: String,
        /// Issue whose transition was blocked
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// State the issue was attempting to enter
        target: State,
        /// Name of the enforcing graph rule that blocked the transition
        rule: String,
    },
    /// An enforcing graph rule was bypassed via `--force` at a state transition
    /// (CC-2).
    ///
    /// The transition-path counterpart of [`Event::LocalRuleBypassed`]: emitted
    /// once per enforce graph rule whose `error` finding was overridden by
    /// `--force` during a `--state` transition. Unlike `LocalRuleBypassed` (which
    /// the write path emits AFTER its save commits), this event is appended inside
    /// `enforce_transition_graph_rules`, i.e. just BEFORE the caller persists the
    /// issue save — the enforcement and its audit entry are produced together,
    /// then the caller commits the transition. Kept distinct from the write-path
    /// bypass so the audit log can tell apart a forced write from a forced
    /// transition.
    GraphRuleBypassed {
        /// Event ID
        id: String,
        /// Issue whose transition bypassed the rule
        issue_id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// State the issue transitioned into
        target: State,
        /// Name of the enforce graph rule that was bypassed
        rule: String,
    },
    /// A gate definition in the registry was edited (`jit gate update`).
    ///
    /// Registry-scoped: it carries no issue id
    /// because it mutates the shared gate registry rather than a single issue.
    GateDefinitionUpdated {
        /// Event ID
        id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Registry key of the gate that was updated
        gate_key: String,
    },
    /// A gate definition was added to the registry (`jit gate define`).
    ///
    /// Registry-scoped, like [`Event::GateDefinitionUpdated`]: it carries no
    /// issue id because it mutates the shared gate registry rather than a
    /// single issue.
    GateDefinitionCreated {
        /// Event ID
        id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Registry key of the gate that was created
        gate_key: String,
    },
    /// A gate definition was removed from the registry (`jit gate remove`).
    ///
    /// Registry-scoped, like [`Event::GateDefinitionUpdated`]: it carries no
    /// issue id because it mutates the shared gate registry rather than a
    /// single issue.
    GateDefinitionRemoved {
        /// Event ID
        id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Registry key of the gate that was removed
        gate_key: String,
    },
    /// A complete profile lifecycle mutation reached its durable commit point.
    ///
    /// One record summarizes the requested operation and every profile in the
    /// dependency-first aggregate, with each package's id, version and applied
    /// variable values.
    ProfileLifecycle {
        /// Event ID.
        id: String,
        /// When the lifecycle operation was recorded.
        timestamp: DateTime<Utc>,
        /// Requested lifecycle operation.
        operation: ProfileLifecycleOperation,
        /// Per-profile outcomes in stable aggregate order.
        profiles: Vec<ProfileLifecycleProfile>,
        /// Whether this append isolated a pre-existing non-newline,
        /// malformed event tail immediately before this record.
        isolated_torn_tail: bool,
    },
    /// An explicit per-invocation override permitted a state-mutating command
    /// inside a linked non-primary checkout.
    ///
    /// Repository-scoped, like [`Event::ProfileLifecycle`]: the override is
    /// decided for the whole invocation before any issue is known, so the record
    /// carries no issue id. It exists because an ordinary mutation record cannot
    /// show that the invocation was permitted by an override rather than by a
    /// permissive declared stance: the pair of stances is the audit fact.
    ///
    /// The finalizer's event pass composes the record into the same
    /// materialization plan as the mutation the override permitted, from the
    /// invocation-scoped annotation the dispatch site installs, so the record and
    /// the mutation it explains become durable together or not at all.
    LinkedCheckoutWriteOverridden {
        /// Event ID
        id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Root of the linked checkout whose store the invocation mutated.
        checkout: std::path::PathBuf,
        /// The stance declared for the repository, which would have refused the
        /// invocation.
        declared_stance: LinkedCheckoutWriteStance,
        /// The per-invocation stance that outranked the declaration and
        /// permitted the invocation.
        invocation_override: LinkedCheckoutWriteStance,
    },
}

impl Event {
    /// This event's identity, as assigned by [`Self::assign_identity`].
    ///
    /// A drafted event that has not reached the mutation finalizer carries an
    /// empty id.
    pub fn id(&self) -> &str {
        match self {
            Event::IssueCreated { id, .. }
            | Event::IssueClaimed { id, .. }
            | Event::IssueStateChanged { id, .. }
            | Event::GatePassed { id, .. }
            | Event::GateFailed { id, .. }
            | Event::GateAdded { id, .. }
            | Event::GateRemoved { id, .. }
            | Event::IssueCompleted { id, .. }
            | Event::IssueDeleted { id, .. }
            | Event::IssueReleased { id, .. }
            | Event::ArtifactArchiveExecuted { id, .. }
            | Event::IssueUpdated { id, .. }
            | Event::DependencyReduced { id, .. }
            | Event::LocalRuleBypassed { id, .. }
            | Event::TransitionBlocked { id, .. }
            | Event::GraphRuleBypassed { id, .. }
            | Event::GateDefinitionUpdated { id, .. }
            | Event::GateDefinitionCreated { id, .. }
            | Event::GateDefinitionRemoved { id, .. }
            | Event::ProfileLifecycle { id, .. }
            | Event::LinkedCheckoutWriteOverridden { id, .. } => id,
        }
    }

    /// Overwrite this event's id and timestamp with finalizer-assigned values.
    ///
    /// The `repository_state` mutation finalizer is the sole authority over event
    /// identity and time: it assigns a deterministic id and stamps the single
    /// mutation timestamp, overriding whatever an intermediate constructor set.
    pub fn assign_identity(&mut self, new_id: String, new_timestamp: DateTime<Utc>) {
        match self {
            Event::IssueCreated { id, timestamp, .. }
            | Event::IssueClaimed { id, timestamp, .. }
            | Event::IssueStateChanged { id, timestamp, .. }
            | Event::GatePassed { id, timestamp, .. }
            | Event::GateFailed { id, timestamp, .. }
            | Event::GateAdded { id, timestamp, .. }
            | Event::GateRemoved { id, timestamp, .. }
            | Event::IssueCompleted { id, timestamp, .. }
            | Event::IssueDeleted { id, timestamp, .. }
            | Event::IssueReleased { id, timestamp, .. }
            | Event::ArtifactArchiveExecuted { id, timestamp, .. }
            | Event::IssueUpdated { id, timestamp, .. }
            | Event::DependencyReduced { id, timestamp, .. }
            | Event::LocalRuleBypassed { id, timestamp, .. }
            | Event::TransitionBlocked { id, timestamp, .. }
            | Event::GraphRuleBypassed { id, timestamp, .. }
            | Event::GateDefinitionUpdated { id, timestamp, .. }
            | Event::GateDefinitionCreated { id, timestamp, .. }
            | Event::GateDefinitionRemoved { id, timestamp, .. }
            | Event::ProfileLifecycle { id, timestamp, .. }
            | Event::LinkedCheckoutWriteOverridden { id, timestamp, .. } => {
                *id = new_id;
                *timestamp = new_timestamp;
            }
        }
    }

    /// Create an issue created event
    pub fn draft_issue_created(issue: &Issue) -> Self {
        Event::IssueCreated {
            id: String::new(),
            issue_id: issue.id.clone(),
            timestamp: DateTime::UNIX_EPOCH,
            title: issue.title.clone(),
            priority: issue.priority,
        }
    }

    /// Create an issue claimed event.
    ///
    /// Takes a typed [`Assignee`] so a malformed actor can never be logged; the
    /// caller parses (and thereby validates) the actor before constructing the
    /// event.
    pub fn draft_issue_claimed(issue_id: String, assignee: Assignee) -> Self {
        Event::IssueClaimed {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            assignee,
        }
    }

    /// Create an issue state changed event
    pub fn draft_issue_state_changed(issue_id: String, from: State, to: State) -> Self {
        Event::IssueStateChanged {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            from,
            to,
        }
    }

    /// Create a gate passed event.
    ///
    /// `updated_by` is a typed [`Assignee`] (the actor who passed the gate), so
    /// no malformed actor can be logged.
    pub fn draft_gate_passed(
        issue_id: String,
        gate_key: String,
        updated_by: Option<Assignee>,
    ) -> Self {
        Event::GatePassed {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
            updated_by,
        }
    }

    /// Create a gate failed event.
    ///
    /// `updated_by` is a typed [`Assignee`] (the actor who failed the gate), so
    /// no malformed actor can be logged.
    pub fn draft_gate_failed(
        issue_id: String,
        gate_key: String,
        updated_by: Option<Assignee>,
    ) -> Self {
        Event::GateFailed {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
            updated_by,
        }
    }

    /// Create a gate added event
    pub fn draft_gate_added(issue_id: String, gate_key: String) -> Self {
        Event::GateAdded {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
        }
    }

    /// Create a gate removed event
    pub fn draft_gate_removed(issue_id: String, gate_key: String) -> Self {
        Event::GateRemoved {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
        }
    }

    /// Create an issue completed event
    pub fn draft_issue_completed(issue_id: String) -> Self {
        Event::IssueCompleted {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
        }
    }

    /// Create an issue released event.
    ///
    /// `assignee` is the typed [`Assignee`] being released; callers emit this
    /// event only when a prior assignee existed, so the actor is always a valid
    /// `kind:identifier`.
    pub fn draft_issue_released(issue_id: String, assignee: Assignee, reason: String) -> Self {
        Event::IssueReleased {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            assignee,
            reason,
        }
    }

    /// Create the durable commit record for dependency-aware archival.
    pub fn draft_artifact_archive_executed(
        target: crate::domain::artifact_plan::PlanTarget,
        destination_root: String,
        publications: Vec<crate::domain::artifact_execution::ArchivePublication>,
        reference_changes: Vec<crate::domain::artifact_plan::ReferenceChange>,
        planned_deletions: Vec<crate::domain::artifact_plan::PendingDeletion>,
        reconciling: bool,
    ) -> Self {
        Event::ArtifactArchiveExecuted {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            target,
            destination_root,
            publications,
            reference_changes,
            planned_deletions,
            reconciling,
        }
    }

    /// Create a dependency reduced event
    pub fn draft_dependency_reduced(
        issue_id: String,
        old_count: usize,
        new_count: usize,
        removed_deps: Vec<String>,
    ) -> Self {
        Event::DependencyReduced {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            old_count,
            new_count,
            removed_deps,
        }
    }

    /// Create an issue updated event
    pub fn draft_issue_updated(issue_id: String, updated_by: String, fields: Vec<String>) -> Self {
        Event::IssueUpdated {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            updated_by,
            fields,
        }
    }

    /// Create an issue deleted event.
    ///
    /// Records that an issue was permanently deleted, preserving an audit trail
    /// of the removal in the event log (the issue file itself is gone).
    pub fn draft_issue_deleted(issue_id: String) -> Self {
        Event::IssueDeleted {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
        }
    }

    /// Create a local-rule-bypassed event.
    ///
    /// Records that a `--force` write deliberately bypassed an `enforce` rule
    /// whose `error` finding would otherwise have blocked the write (DR §7.6).
    pub fn draft_local_rule_bypassed(issue_id: String, rule: String) -> Self {
        Event::LocalRuleBypassed {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            rule,
        }
    }

    /// Create a transition-blocked event.
    ///
    /// Records that an enforcing graph rule blocked the issue's transition into
    /// `target` (CC-2). Appended before the blocking error is returned, one per
    /// blocking rule.
    pub fn draft_transition_blocked(issue_id: String, target: State, rule: String) -> Self {
        Event::TransitionBlocked {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            target,
            rule,
        }
    }

    /// Create a graph-rule-bypassed event.
    ///
    /// Records that a `--force` transition deliberately bypassed an enforcing
    /// graph rule whose `error` finding would otherwise have blocked the
    /// transition into `target` (CC-2).
    pub fn draft_graph_rule_bypassed(issue_id: String, target: State, rule: String) -> Self {
        Event::GraphRuleBypassed {
            id: String::new(),
            issue_id,
            timestamp: DateTime::UNIX_EPOCH,
            target,
            rule,
        }
    }

    /// Create a gate-definition-updated event.
    ///
    /// Registry-scoped (issue-less): records that the
    /// gate registry entry `gate_key` was edited via `jit gate update`.
    pub fn draft_gate_definition_updated(gate_key: String) -> Self {
        Event::GateDefinitionUpdated {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
        }
    }

    /// Create a gate-definition-created event.
    ///
    /// Registry-scoped (issue-less, like
    /// [`draft_gate_definition_updated`](Self::draft_gate_definition_updated)):
    /// records that `gate_key` was added to the gate registry via `jit gate
    /// define`.
    pub fn draft_gate_definition_created(gate_key: String) -> Self {
        Event::GateDefinitionCreated {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
        }
    }

    /// Create a gate-definition-removed event.
    ///
    /// Registry-scoped (issue-less, like
    /// [`draft_gate_definition_updated`](Self::draft_gate_definition_updated)):
    /// records that `gate_key` was removed from the gate registry via `jit
    /// gate remove`.
    pub fn draft_gate_definition_removed(gate_key: String) -> Self {
        Event::GateDefinitionRemoved {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            gate_key,
        }
    }

    /// Create the aggregate profile lifecycle audit event.
    pub fn draft_profile_lifecycle(
        operation: ProfileLifecycleOperation,
        profiles: Vec<ProfileLifecycleProfile>,
    ) -> Self {
        Event::ProfileLifecycle {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            operation,
            profiles,
            isolated_torn_tail: false,
        }
    }

    /// Draft the record of an override that permitted a mutation inside a linked
    /// checkout.
    ///
    /// `checkout` is the linked checkout's root as reported by the selected-root
    /// worktree authority; `declared_stance` is the repository's declaration and
    /// `invocation_override` the per-invocation stance that outranked it under
    /// [`LinkedCheckoutWriteStance::resolve`].
    pub fn draft_linked_checkout_write_overridden(
        checkout: std::path::PathBuf,
        declared_stance: LinkedCheckoutWriteStance,
        invocation_override: LinkedCheckoutWriteStance,
    ) -> Self {
        Event::LinkedCheckoutWriteOverridden {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            checkout,
            declared_stance,
            invocation_override,
        }
    }

    /// Get the issue ID associated with this event
    pub fn get_issue_id(&self) -> &str {
        match self {
            Event::IssueCreated { issue_id, .. } => issue_id,
            Event::IssueClaimed { issue_id, .. } => issue_id,
            Event::IssueStateChanged { issue_id, .. } => issue_id,
            Event::GatePassed { issue_id, .. } => issue_id,
            Event::GateFailed { issue_id, .. } => issue_id,
            Event::GateAdded { issue_id, .. } => issue_id,
            Event::GateRemoved { issue_id, .. } => issue_id,
            Event::IssueCompleted { issue_id, .. } => issue_id,
            Event::IssueDeleted { issue_id, .. } => issue_id,
            Event::IssueReleased { issue_id, .. } => issue_id,
            Event::IssueUpdated { issue_id, .. } => issue_id,
            Event::ArtifactArchiveExecuted { .. } => "", // Repository-scoped
            Event::DependencyReduced { issue_id, .. } => issue_id,
            Event::LocalRuleBypassed { issue_id, .. } => issue_id,
            Event::TransitionBlocked { issue_id, .. } => issue_id,
            Event::GraphRuleBypassed { issue_id, .. } => issue_id,
            Event::GateDefinitionUpdated { .. } => "", // No associated issue (registry-scoped)
            Event::GateDefinitionCreated { .. } => "", // No associated issue (registry-scoped)
            Event::GateDefinitionRemoved { .. } => "", // No associated issue (registry-scoped)
            Event::ProfileLifecycle { .. } => "",      // No associated issue (repo-scoped)
            Event::LinkedCheckoutWriteOverridden { .. } => "", // No associated issue (repo-scoped)
        }
    }

    /// Get the event type as a string.
    ///
    /// The string is the value serde writes into the record's `type` field, and
    /// the value `jit events query --event-type` matches against. It is supplied
    /// by [`EventTag::as_str`](crate::domain::event_catalog::EventTag::as_str), the single
    /// definition of the tag vocabulary, via [`Event::tag`].
    pub fn get_type(&self) -> &str {
        self.tag().as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{document_reference, document_reference_at, document_reference_typed};

    fn issue_record(state: State, archived_from: Option<State>) -> serde_json::Value {
        let mut issue = fixture_issue("Archived".to_string(), String::new());
        issue.state = state;
        issue.archived_from = archived_from;
        serde_json::to_value(&issue).unwrap()
    }

    #[test]
    fn test_issue_decode_rejects_archived_record_without_archived_from() {
        let error = serde_json::from_value::<Issue>(issue_record(State::Archived, None))
            .expect_err("an archived record without an origin must not decode");
        assert!(error.to_string().contains("archived_from"), "{error}");

        let decoded =
            serde_json::from_value::<Issue>(issue_record(State::Archived, Some(State::Done)))
                .unwrap();
        assert_eq!(decoded.archived_from, Some(State::Done));
        assert!(serde_json::from_value::<Issue>(issue_record(State::Done, None)).is_ok());
    }

    #[test]
    fn test_issue_schema_describes_archived_from() {
        let schema = serde_json::to_value(schemars::schema_for!(Issue)).unwrap();
        assert!(schema["properties"].get("archived_from").is_some());
    }

    #[test]
    fn test_is_reusable_for_rejects_an_attestation_over_a_matching_digest() {
        let digest =
            crate::domain::repository_inputs::InputsDigestBuilder::new(b"gate declaration")
                .finish();
        let attestor: Assignee = "human:reviewer".parse().unwrap();
        let mut attested = GateRunResult::attestation(
            "issue",
            "review",
            GateStage::Postcheck,
            GateStatus::Passed,
            Some(&attestor),
        );
        attested.inputs_digest = Some(digest.clone());

        assert!(!attested.is_reusable_for("review", &digest));
    }

    #[test]
    fn test_gate_run_result_by_round_trips_through_json_byte_identically() {
        let mut run = GateRunResult::attestation(
            "issue",
            "review",
            GateStage::Postcheck,
            GateStatus::Passed,
            None,
        );
        run.by = Some(Assignee::AUTO_EXECUTOR);

        let encoded = serde_json::to_string(&run).unwrap();
        let decoded: GateRunResult = serde_json::from_str(&encoded).unwrap();

        assert_eq!(decoded.by, run.by);
        assert_eq!(serde_json::to_string(&decoded).unwrap(), encoded);
    }

    #[test]
    fn test_linked_checkout_write_stance_from_json_tokens() {
        assert_eq!(
            serde_json::from_str::<LinkedCheckoutWriteStance>("\"refuse\"").unwrap(),
            LinkedCheckoutWriteStance::Refuse
        );
        assert_eq!(
            serde_json::from_str::<LinkedCheckoutWriteStance>("\"allow\"").unwrap(),
            LinkedCheckoutWriteStance::Allow
        );
    }

    #[test]
    fn test_linked_checkout_write_stance_as_token_round_trips_through_every_representation() {
        for stance in [
            LinkedCheckoutWriteStance::Refuse,
            LinkedCheckoutWriteStance::Allow,
        ] {
            let token = stance.as_token();
            assert_eq!(
                token.parse::<LinkedCheckoutWriteStance>().unwrap(),
                stance,
                "the token a stance renders as must be one its own parser accepts"
            );
            assert_eq!(
                serde_json::to_value(stance).unwrap(),
                serde_json::Value::String(token.to_owned()),
                "prose naming a stance and a machine-readable field carrying one must agree"
            );
            assert_eq!(
                stance.to_string(),
                token,
                "Display must render the same token"
            );
        }
    }

    #[test]
    fn test_linked_checkout_write_stance_from_str_accepts_tokens_case_insensitively() {
        for (token, expected) in [
            ("refuse", LinkedCheckoutWriteStance::Refuse),
            ("REFUSE", LinkedCheckoutWriteStance::Refuse),
            ("allow", LinkedCheckoutWriteStance::Allow),
            ("Allow", LinkedCheckoutWriteStance::Allow),
        ] {
            assert_eq!(
                token.parse::<LinkedCheckoutWriteStance>().unwrap(),
                expected,
                "case must not change which stance a token names"
            );
        }
    }

    #[test]
    fn test_linked_checkout_write_stance_from_str_rejects_unknown_token_naming_accepted_ones() {
        let error = "sometimes"
            .parse::<LinkedCheckoutWriteStance>()
            .expect_err("a token outside the vocabulary is not a stance");
        let message = error.to_string();
        for expected in ["sometimes", "refuse", "allow"] {
            assert!(
                message.contains(expected),
                "the rejection must name the offending token and every accepted one; got: {message}"
            );
        }
    }

    #[test]
    fn test_linked_checkout_write_stance_deserializes_through_its_own_parser() {
        // The TOML key and the per-invocation override share one parser, so a
        // token either source accepts is accepted by the other.
        assert_eq!(
            serde_json::from_str::<LinkedCheckoutWriteStance>("\"ALLOW\"").unwrap(),
            "ALLOW".parse::<LinkedCheckoutWriteStance>().unwrap(),
            "deserialization must not apply case rules the string parser does not"
        );
        let error = serde_json::from_str::<LinkedCheckoutWriteStance>("\"sometimes\"")
            .expect_err("a token outside the vocabulary is not a stance");
        assert!(
            error.to_string().contains("sometimes"),
            "the deserialization error must carry the parser's message; got: {error}"
        );
    }

    #[test]
    fn test_linked_checkout_write_stance_resolve_prefers_override_over_declared() {
        assert_eq!(
            LinkedCheckoutWriteStance::resolve(
                LinkedCheckoutWriteStance::Refuse,
                Some(LinkedCheckoutWriteStance::Allow)
            ),
            LinkedCheckoutWriteStance::Allow
        );
        assert_eq!(
            LinkedCheckoutWriteStance::resolve(
                LinkedCheckoutWriteStance::Allow,
                Some(LinkedCheckoutWriteStance::Refuse)
            ),
            LinkedCheckoutWriteStance::Refuse
        );
    }

    #[test]
    fn test_linked_checkout_write_stance_resolve_falls_back_to_declared_without_override() {
        assert_eq!(
            LinkedCheckoutWriteStance::resolve(LinkedCheckoutWriteStance::Refuse, None),
            LinkedCheckoutWriteStance::Refuse
        );
        assert_eq!(
            LinkedCheckoutWriteStance::resolve(LinkedCheckoutWriteStance::Allow, None),
            LinkedCheckoutWriteStance::Allow
        );
    }

    #[test]
    fn test_new_issue_has_correct_defaults() {
        let issue =
            crate::domain::fixture_issue("Test Issue".to_string(), "Description".to_string());

        assert_eq!(issue.title, "Test Issue");
        assert_eq!(issue.description, "Description");
        assert_eq!(issue.state, State::Backlog);
        assert_eq!(issue.priority, Priority::Normal);
        assert_eq!(issue.assignee, None);
        assert!(issue.dependencies.is_empty());
        assert!(issue.gates_required.is_empty());
        assert!(issue.gates_status.is_empty());
        assert!(issue.context.is_empty());
        assert!(!issue.id.is_empty());
        assert_eq!(issue.first_ready_at, None);
        assert_eq!(issue.claimed_at, None);
        assert_eq!(issue.done_at, None);
    }

    #[test]
    fn test_mark_lifecycle_timestamps_are_first_occurrence_only() {
        let mut issue = crate::domain::fixture_issue("t".to_string(), String::new());
        let first = Utc::now();
        let later = first + chrono::Duration::hours(1);

        issue.mark_first_ready(first);
        issue.mark_first_ready(later);
        issue.mark_claimed(first);
        issue.mark_claimed(later);
        issue.mark_done(first);
        issue.mark_done(later);

        assert_eq!(issue.first_ready_at, Some(first));
        assert_eq!(issue.claimed_at, Some(first));
        assert_eq!(issue.done_at, Some(first));
    }

    #[test]
    fn test_old_issue_file_without_timestamps_loads_and_roundtrips() {
        // An issue file written before the lifecycle-timestamp fields existed has
        // none of the three keys. It must deserialize (fields default to None)
        // and re-serialize WITHOUT introducing the keys, so old files round-trip
        // byte-stable (skip_serializing_if = Option::is_none).
        let json = r#"{
            "id": "11111111-2222-3333-4444-555555555555",
            "title": "Legacy",
            "description": "",
            "state": "ready",
            "priority": "normal",
            "assignee": null,
            "dependencies": [],
            "gates_required": [],
            "gates_status": {},
            "context": {},
            "documents": [],
            "labels": [],
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z"
        }"#;

        let issue: Issue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.first_ready_at, None);
        assert_eq!(issue.claimed_at, None);
        assert_eq!(issue.done_at, None);

        let reserialized = serde_json::to_string(&issue).unwrap();
        assert!(!reserialized.contains("first_ready_at"));
        assert!(!reserialized.contains("claimed_at"));
        assert!(!reserialized.contains("done_at"));
    }

    #[test]
    fn test_issue_with_timestamps_serializes_the_fields() {
        let mut issue = crate::domain::fixture_issue("t".to_string(), String::new());
        let at = Utc::now();
        issue.mark_first_ready(at);
        issue.mark_claimed(at);
        issue.mark_done(at);

        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("first_ready_at"));
        assert!(json.contains("claimed_at"));
        assert!(json.contains("done_at"));

        let roundtripped: Issue = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.first_ready_at, Some(at));
        assert_eq!(roundtripped.claimed_at, Some(at));
        assert_eq!(roundtripped.done_at, Some(at));
    }

    #[test]
    fn test_issue_not_blocked_with_no_dependencies_or_gates() {
        let issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_unmet_dependency() {
        let mut issue = crate::domain::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let dependency = crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_when_dependency_is_done() {
        let mut issue = crate::domain::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let mut dependency =
            crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Done;

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_by_unpassed_gate() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_by_pending_gate() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
                run_id: None,
            },
        );

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_by_failed_gate() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: Utc::now(),
                run_id: None,
            },
        );

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_when_gate_passed() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: Utc::now(),
                run_id: None,
            },
        );

        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
        assert!(!issue.has_unpassed_gates());
    }

    #[test]
    fn test_document_reference_serialization() {
        let doc = DocumentReference {
            label: Some("Design Doc".to_string()),
            ..document_reference("docs/design.md")
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
    }

    #[test]
    fn test_issue_with_documents() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        assert_eq!(issue.documents.len(), 0);

        issue.documents.push(document_reference("docs/design.md"));
        assert_eq!(issue.documents.len(), 1);
        assert_eq!(issue.documents[0].path, "docs/design.md");
    }

    #[test]
    fn test_issue_serialization_with_documents() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        issue.documents.push(DocumentReference {
            label: Some("Design".to_string()),
            ..document_reference_at("docs/design.md", "abc123")
        });

        let json = serde_json::to_string(&issue).unwrap();
        let deserialized: Issue = serde_json::from_str(&json).unwrap();

        assert_eq!(issue.documents.len(), deserialized.documents.len());
        assert_eq!(issue.documents[0], deserialized.documents[0]);
    }

    // State model tests for Backlog and Gated states

    #[test]
    fn test_new_issue_starts_in_backlog() {
        let issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        assert_eq!(issue.state, State::Backlog);
    }

    #[test]
    fn test_backlog_issue_should_auto_transition_to_ready_when_unblocked() {
        let issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        let resolved = HashMap::new();

        assert_eq!(issue.state, State::Backlog);
        assert!(issue.should_auto_transition_to_ready(&resolved));
    }

    #[test]
    fn test_backlog_issue_should_not_transition_to_ready_when_blocked() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        let dependency = crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert_eq!(issue.state, State::Backlog);
        assert!(!issue.should_auto_transition_to_ready(&resolved));
    }

    #[test]
    fn test_derive_readiness_correction_demotes_a_ready_issue_an_unmet_dependency_blocks() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        let dependency = crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        issue.state = State::Ready;
        issue.dependencies.push(dependency.id.clone());
        let resolved = HashMap::from([(dependency.id.clone(), &dependency)]);

        assert_eq!(
            issue.derive_readiness_correction(&resolved),
            Some(ReadinessCorrection::Demote)
        );
    }

    #[test]
    fn test_derive_readiness_correction_promotes_a_backlog_issue_nothing_blocks() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        let mut dependency =
            crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Done;
        issue.dependencies.push(dependency.id.clone());
        let resolved = HashMap::from([(dependency.id.clone(), &dependency)]);

        assert_eq!(
            issue.derive_readiness_correction(&resolved),
            Some(ReadinessCorrection::Promote)
        );
    }

    #[test]
    fn test_derive_readiness_correction_leaves_a_state_that_already_agrees_with_the_graph() {
        let mut ready = crate::domain::fixture_issue("Ready".to_string(), "Desc".to_string());
        ready.state = State::Ready;
        let mut blocked = crate::domain::fixture_issue("Blocked".to_string(), "Desc".to_string());
        let dependency = crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        blocked.dependencies.push(dependency.id.clone());
        let resolved = HashMap::from([(dependency.id.clone(), &dependency)]);

        assert_eq!(ready.derive_readiness_correction(&resolved), None);
        assert_eq!(blocked.derive_readiness_correction(&resolved), None);
    }

    #[test]
    fn test_derive_readiness_correction_leaves_a_state_no_dependency_owns() {
        let dependency = crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        let resolved = HashMap::from([(dependency.id.clone(), &dependency)]);
        let blocked_by_unmet_dependency = [
            State::InProgress,
            State::Gated,
            State::Done,
            State::Rejected,
            State::Archived,
        ]
        .map(|state| {
            let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
            issue.state = state;
            issue.dependencies.push(dependency.id.clone());
            issue
        });

        for issue in &blocked_by_unmet_dependency {
            assert_eq!(
                issue.derive_readiness_correction(&resolved),
                None,
                "{:?} is owned by an explicit transition, not derived from dependencies",
                issue.state
            );
        }
    }

    #[test]
    fn test_gated_issue_should_auto_transition_to_done_when_gates_pass() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: Utc::now(),
                run_id: None,
            },
        );

        assert!(issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_gated_issue_should_not_transition_to_done_when_gates_pending() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
                run_id: None,
            },
        );

        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_gated_issue_should_not_transition_to_done_when_gates_failed() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("ci:tests".parse().unwrap()),
                updated_at: Utc::now(),
                run_id: None,
            },
        );

        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_in_progress_issue_should_not_auto_transition() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::InProgress;

        let resolved = HashMap::new();
        assert!(!issue.should_auto_transition_to_ready(&resolved));
        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_state_enumeration_preserves_lifecycle_order() {
        let lifecycle = [
            State::Backlog,
            State::Ready,
            State::InProgress,
            State::Gated,
            State::Done,
            State::Rejected,
            State::Archived,
        ];
        assert_eq!(State::ALL, lifecycle);
        assert!(lifecycle.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn test_new_issue_has_empty_labels() {
        let issue = crate::domain::fixture_issue("Test".to_string(), "Description".to_string());
        assert!(issue.labels.is_empty());
    }

    #[test]
    fn test_issue_serialization_with_labels() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.labels.push("milestone:v1.0".to_string());
        issue.labels.push("epic:auth".to_string());
        issue.labels.push("type:task".to_string());

        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"labels\""));
        assert!(json.contains("milestone:v1.0"));

        let deserialized: Issue = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.labels.len(), 3);
        assert!(deserialized.labels.contains(&"milestone:v1.0".to_string()));
        assert!(deserialized.labels.contains(&"epic:auth".to_string()));
        assert!(deserialized.labels.contains(&"type:task".to_string()));
    }

    #[test]
    fn test_issue_labels_can_be_modified() {
        let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
        assert!(issue.labels.is_empty());

        issue.labels.push("component:backend".to_string());
        assert_eq!(issue.labels.len(), 1);

        issue.labels.push("priority:high".to_string());
        assert_eq!(issue.labels.len(), 2);

        issue.labels.retain(|l| l != "component:backend");
        assert_eq!(issue.labels.len(), 1);
        assert_eq!(issue.labels[0], "priority:high");
    }

    // Tests for Rejected state
    #[test]
    fn test_rejected_state_serialization() {
        let json = serde_json::to_string(&State::Rejected).unwrap();
        assert_eq!(json, "\"rejected\"");
    }

    #[test]
    fn test_rejected_state_deserialization() {
        let state: State = serde_json::from_str("\"rejected\"").unwrap();
        assert_eq!(state, State::Rejected);
    }

    #[test]
    fn test_is_terminal_returns_true_for_done() {
        assert!(State::Done.is_terminal());
    }

    #[test]
    fn test_is_terminal_returns_true_for_rejected() {
        assert!(State::Rejected.is_terminal());
    }

    #[test]
    fn test_is_terminal_returns_false_for_non_terminal_states() {
        assert!(!State::Backlog.is_terminal());
        assert!(!State::Ready.is_terminal());
        assert!(!State::InProgress.is_terminal());
        assert!(!State::Gated.is_terminal());
        assert!(!State::Archived.is_terminal());
    }

    #[test]
    fn test_is_dependency_met_accepts_terminal_states() {
        assert!(is_dependency_met(State::Done, None));
        assert!(is_dependency_met(State::Rejected, None));
    }

    #[test]
    fn test_is_dependency_met_rejects_non_terminal_states() {
        assert!(!is_dependency_met(State::Backlog, None));
        assert!(!is_dependency_met(State::Ready, None));
        assert!(!is_dependency_met(State::InProgress, None));
        assert!(!is_dependency_met(State::Gated, None));
    }

    #[test]
    fn test_effective_terminality_preserves_archived_origin() {
        // Archived is terminality-preserving: a dependency archived from a
        // terminal state stays met; one archived from a non-terminal state does
        // not.
        assert!(is_dependency_met(State::Archived, Some(State::Done)));
        assert!(is_dependency_met(State::Archived, Some(State::Rejected)));
        assert!(!is_dependency_met(State::Archived, Some(State::InProgress)));

        assert_eq!(
            effective_terminal_state(State::Archived, Some(State::Done)),
            Some(State::Done)
        );
        assert_eq!(
            effective_terminal_state(State::Archived, Some(State::Rejected)),
            Some(State::Rejected)
        );
        assert_eq!(
            effective_terminal_state(State::Archived, Some(State::Gated)),
            None
        );
        // A non-archived state ignores any archived_from value.
        assert_eq!(
            effective_terminal_state(State::Done, None),
            Some(State::Done)
        );
        assert_eq!(
            effective_terminal_state(State::Ready, Some(State::Done)),
            None
        );
    }

    #[test]
    fn test_issue_not_blocked_when_dependency_is_rejected() {
        let mut issue = crate::domain::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let mut dependency =
            crate::domain::fixture_issue("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Rejected;

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        // Rejected dependencies should unblock, like Done
        assert!(!issue.is_blocked(&resolved));
    }

    // Tests for extended DocumentReference schema with format and assets

    #[test]
    fn test_document_reference_with_format_and_assets() {
        use crate::document::Asset;
        use std::path::PathBuf;

        let doc = DocumentReference {
            label: Some("Design Doc".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![Asset {
                original_path: "./logo.png".to_string(),
                resolved_path: Some(PathBuf::from("docs/logo.png")),
                asset_type: crate::document::AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: Some("sha256:abc123".to_string()),
                is_shared: false,
            }],
            ..document_reference_typed("docs/design.md", "design")
        };

        assert_eq!(doc.format, Some("markdown".to_string()));
        assert_eq!(doc.assets.len(), 1);
        assert_eq!(doc.assets[0].original_path, "./logo.png");
    }

    #[test]
    fn test_document_reference_serialization_with_new_fields() {
        use crate::document::Asset;
        use std::path::PathBuf;

        let doc = DocumentReference {
            label: Some("Design Doc".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![Asset {
                original_path: "./logo.png".to_string(),
                resolved_path: Some(PathBuf::from("docs/logo.png")),
                asset_type: crate::document::AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: Some("sha256:abc123".to_string()),
                is_shared: false,
            }],
            ..document_reference_typed("docs/design.md", "design")
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
        assert_eq!(deserialized.format, Some("markdown".to_string()));
        assert_eq!(deserialized.assets.len(), 1);
    }

    #[test]
    fn test_document_reference_backward_compatibility() {
        // Old JSON without format and assets fields
        let old_json = r#"{
            "path": "docs/design.md",
            "commit": null,
            "label": "Design Doc",
            "doc_type": "design"
        }"#;

        let doc: DocumentReference = serde_json::from_str(old_json).unwrap();

        assert_eq!(doc.path, "docs/design.md");
        assert_eq!(doc.label, Some("Design Doc".to_string()));
        assert_eq!(doc.doc_type, Some("design".to_string()));
        // New fields should have default values
        assert_eq!(doc.format, None);
        assert_eq!(doc.assets.len(), 0);
    }

    #[test]
    fn test_document_reference_forward_compatibility() {
        use crate::document::Asset;
        use std::path::PathBuf;

        // New JSON with format and assets
        let doc = DocumentReference {
            label: Some("Design".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![
                Asset {
                    original_path: "./arch.png".to_string(),
                    resolved_path: Some(PathBuf::from("docs/arch.png")),
                    asset_type: crate::document::AssetType::Local,
                    mime_type: Some("image/png".to_string()),
                    content_hash: Some("sha256:def456".to_string()),
                    is_shared: false,
                },
                Asset {
                    original_path: "https://example.com/logo.svg".to_string(),
                    resolved_path: None,
                    asset_type: crate::document::AssetType::External,
                    mime_type: Some("image/svg+xml".to_string()),
                    content_hash: None,
                    is_shared: false,
                },
            ],
            ..document_reference_typed("docs/design.md", "design")
        };

        let json = serde_json::to_string_pretty(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.format, Some("markdown".to_string()));
        assert_eq!(deserialized.assets.len(), 2);
        assert_eq!(
            deserialized.assets[0].mime_type,
            Some("image/png".to_string())
        );
        assert_eq!(
            deserialized.assets[1].original_path,
            "https://example.com/logo.svg"
        );
    }

    #[test]
    fn test_document_reference_empty_assets() {
        let doc = DocumentReference {
            format: Some("markdown".to_string()),
            ..document_reference_typed("docs/notes.md", "notes")
        };

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
        assert_eq!(deserialized.assets.len(), 0);
    }

    // FromStr trait tests
    mod fromstr_tests {
        use super::*;
        use crate::declarations::GateMode;
        use std::str::FromStr;

        #[test]
        fn test_priority_from_str_valid_lowercase() {
            assert_eq!(Priority::from_str("low").unwrap(), Priority::Low);
            assert_eq!(Priority::from_str("normal").unwrap(), Priority::Normal);
            assert_eq!(Priority::from_str("high").unwrap(), Priority::High);
            assert_eq!(Priority::from_str("critical").unwrap(), Priority::Critical);
        }

        #[test]
        fn test_priority_from_str_valid_uppercase() {
            assert_eq!(Priority::from_str("LOW").unwrap(), Priority::Low);
            assert_eq!(Priority::from_str("NORMAL").unwrap(), Priority::Normal);
            assert_eq!(Priority::from_str("HIGH").unwrap(), Priority::High);
            assert_eq!(Priority::from_str("CRITICAL").unwrap(), Priority::Critical);
        }

        #[test]
        fn test_priority_from_str_valid_mixedcase() {
            assert_eq!(Priority::from_str("Low").unwrap(), Priority::Low);
            assert_eq!(Priority::from_str("Normal").unwrap(), Priority::Normal);
        }

        #[test]
        fn test_priority_from_str_invalid() {
            assert!(Priority::from_str("invalid").is_err());
            assert!(Priority::from_str("").is_err());
            assert!(Priority::from_str("medium").is_err());
        }

        #[test]
        fn test_priority_parse_method() {
            // Test using str::parse() method
            let priority: Priority = "high".parse().unwrap();
            assert_eq!(priority, Priority::High);
        }

        // GateStage FromStr/Display/as_str

        #[test]
        fn test_gate_stage_from_str_accepts_valid() {
            assert_eq!(
                GateStage::from_str("precheck").unwrap(),
                GateStage::Precheck
            );
            assert_eq!(
                GateStage::from_str("postcheck").unwrap(),
                GateStage::Postcheck
            );
        }

        #[test]
        fn test_gate_stage_from_str_rejects_invalid() {
            assert!(GateStage::from_str("invalid").is_err());
            assert!(GateStage::from_str("").is_err());
            assert!(GateStage::from_str("PRECHECK").is_err());
        }

        #[test]
        fn test_gate_stage_display_and_as_str_round_trip() {
            assert_eq!(GateStage::Precheck.to_string(), "precheck");
            assert_eq!(GateStage::Postcheck.to_string(), "postcheck");
            assert_eq!(GateStage::Precheck.as_str(), "precheck");
            assert_eq!(GateStage::Postcheck.as_str(), "postcheck");
        }

        #[test]
        fn test_gate_stage_display_equals_serde() {
            // Display/as_str must match what serde produces (snake_case).
            let json_pre = serde_json::to_string(&GateStage::Precheck).unwrap();
            let json_post = serde_json::to_string(&GateStage::Postcheck).unwrap();
            assert_eq!(json_pre, format!("\"{}\"", GateStage::Precheck));
            assert_eq!(json_post, format!("\"{}\"", GateStage::Postcheck));
        }

        // GateMode FromStr/Display/as_str

        #[test]
        fn test_gate_mode_from_str_accepts_valid() {
            assert_eq!(GateMode::from_str("manual").unwrap(), GateMode::Manual);
            assert_eq!(GateMode::from_str("auto").unwrap(), GateMode::Auto);
        }

        #[test]
        fn test_gate_mode_from_str_rejects_invalid() {
            assert!(GateMode::from_str("invalid").is_err());
            assert!(GateMode::from_str("").is_err());
            assert!(GateMode::from_str("MANUAL").is_err());
        }

        #[test]
        fn test_gate_mode_display_and_as_str_round_trip() {
            assert_eq!(GateMode::Manual.to_string(), "manual");
            assert_eq!(GateMode::Auto.to_string(), "auto");
            assert_eq!(GateMode::Manual.as_str(), "manual");
            assert_eq!(GateMode::Auto.as_str(), "auto");
        }

        #[test]
        fn test_gate_mode_display_equals_serde() {
            // Display/as_str must match what serde produces (snake_case).
            let json_manual = serde_json::to_string(&GateMode::Manual).unwrap();
            let json_auto = serde_json::to_string(&GateMode::Auto).unwrap();
            assert_eq!(json_manual, format!("\"{}\"", GateMode::Manual));
            assert_eq!(json_auto, format!("\"{}\"", GateMode::Auto));
        }
    }

    mod assignee_tests {
        use super::*;
        use std::str::FromStr;

        #[test]
        fn test_assignee_from_str_accepts_valid() {
            let a = Assignee::from_str("agent:copilot").unwrap();
            assert_eq!(a.kind(), "agent");
            assert_eq!(a.identifier(), "copilot");
        }

        #[test]
        fn test_assignee_from_str_splits_on_first_colon_only() {
            let a = Assignee::from_str("ci:job:42").unwrap();
            assert_eq!(a.kind(), "ci");
            assert_eq!(a.identifier(), "job:42");
        }

        #[test]
        fn test_assignee_from_str_rejects_empty() {
            assert_eq!(Assignee::from_str(""), Err(AssigneeParseError::Empty));
        }

        #[test]
        fn test_assignee_from_str_rejects_missing_separator() {
            assert!(matches!(
                Assignee::from_str("nocolon"),
                Err(AssigneeParseError::MissingSeparator(_))
            ));
        }

        #[test]
        fn test_assignee_from_str_rejects_empty_parts() {
            assert!(matches!(
                Assignee::from_str(":identifier"),
                Err(AssigneeParseError::EmptyKind(_))
            ));
            assert!(matches!(
                Assignee::from_str("kind:"),
                Err(AssigneeParseError::EmptyIdentifier(_))
            ));
        }

        #[test]
        fn test_assignee_display_round_trips_through_from_str() {
            for raw in [
                "agent:copilot",
                "human:alice",
                "ci:github-actions",
                "ci:job:42",
            ] {
                let parsed = Assignee::from_str(raw).unwrap();
                assert_eq!(parsed.to_string(), raw);
                assert_eq!(Assignee::from_str(&parsed.to_string()).unwrap(), parsed);
            }
        }

        #[test]
        fn test_assignee_system_actor_constants_equal_parsed_spelling() {
            for (constant, spelling) in [
                (Assignee::SYSTEM_LIST, "system:list"),
                (Assignee::SYSTEM_CHECK, "system:check"),
                (Assignee::SYSTEM_ADMIN, "system:admin"),
                (Assignee::SYSTEM_RECOVERY, "system:recovery"),
                (Assignee::SYSTEM_LEASE_CHECK, "system:lease-check"),
                (Assignee::AUTO_EXECUTOR, "auto:executor"),
            ] {
                assert_eq!(Assignee::from_str(spelling).unwrap(), constant);
            }
        }

        #[test]
        fn test_assignee_partial_eq_str() {
            let a = Assignee::from_str("agent:copilot").unwrap();
            assert!(a == *"agent:copilot");
            assert!(a != *"agent:other");
            assert!(a != *"nocolon");
        }

        #[test]
        fn test_assignee_serializes_as_plain_string() {
            let a = Assignee::from_str("human:alice").unwrap();
            assert_eq!(serde_json::to_string(&a).unwrap(), "\"human:alice\"");
        }

        #[test]
        fn test_assignee_deserializes_from_string_round_trip() {
            let a: Assignee = serde_json::from_str("\"ci:github-actions\"").unwrap();
            assert_eq!(a, Assignee::from_str("ci:github-actions").unwrap());
        }

        #[test]
        fn test_assignee_deserialize_rejects_malformed_string() {
            assert!(serde_json::from_str::<Assignee>("\"nocolon\"").is_err());
        }

        #[test]
        fn test_issue_assignee_serializes_unchanged() {
            let mut issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());
            issue.assignee = Some(Assignee::from_str("agent:copilot").unwrap());

            let json = serde_json::to_string(&issue).unwrap();
            assert!(json.contains("\"assignee\":\"agent:copilot\""));

            let deserialized: Issue = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.assignee, issue.assignee);
        }
    }

    #[test]
    fn test_issue_timestamps_serialize_as_rfc3339() {
        let issue = crate::domain::fixture_issue("Test".to_string(), "Desc".to_string());

        let value: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&issue).unwrap()).unwrap();
        let created = value["created_at"].as_str().unwrap();

        // chrono's default serde emits RFC 3339, parseable back to the same instant.
        let reparsed = DateTime::parse_from_rfc3339(created).unwrap();
        assert_eq!(reparsed.with_timezone(&Utc), issue.created_at);
    }

    #[test]
    fn test_issue_loads_legacy_offset_timestamp() {
        // Issue files predating the DateTime migration store `+00:00` offsets
        // rather than chrono's `Z`; both are RFC 3339 and must still load.
        let json = r#"{
            "id": "11111111-2222-3333-4444-555555555555",
            "title": "Legacy",
            "description": "Body",
            "state": "backlog",
            "priority": "normal",
            "assignee": "agent:copilot",
            "dependencies": [],
            "gates_required": [],
            "gates_status": {},
            "context": {},
            "documents": [],
            "labels": [],
            "created_at": "2026-06-22T21:59:04.464226946+00:00",
            "updated_at": "2026-06-22T21:59:04.464226946+00:00"
        }"#;

        let issue: Issue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.assignee.unwrap().to_string(), "agent:copilot");
        assert_eq!(issue.created_at.timezone(), Utc);
    }

    mod event_assignee_tests {
        use super::*;
        use std::str::FromStr;

        #[test]
        fn test_issue_claimed_assignee_serde_round_trips_as_string() {
            let actor = Assignee::from_str("agent:copilot").unwrap();
            let event = Event::draft_issue_claimed("issue-1".to_string(), actor);

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"assignee\":\"agent:copilot\""));

            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
        }

        #[test]
        fn test_issue_released_assignee_serde_round_trips_as_string() {
            let prev = Assignee::from_str("copilot:session-1").unwrap();
            let event =
                Event::draft_issue_released("issue-1".to_string(), prev, "timeout".to_string());

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"assignee\":\"copilot:session-1\""));

            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
        }

        #[test]
        fn test_gate_event_updated_by_serde_round_trips_as_string() {
            let by = Assignee::from_str("ci:runner").unwrap();
            let event =
                Event::draft_gate_passed("issue-1".to_string(), "tests".to_string(), Some(by));

            let json = serde_json::to_string(&event).unwrap();
            assert!(json.contains("\"updated_by\":\"ci:runner\""));

            let back: Event = serde_json::from_str(&json).unwrap();
            assert_eq!(back, event);
        }

        #[test]
        fn test_event_actor_rejected_before_construction() {
            // The constructors only accept an already-parsed `Assignee`, so a
            // malformed actor is rejected at parse time and can never reach an
            // event. (`"bulk-update"` is the kind of non-assignee actor that the
            // string-typed `IssueUpdated.updated_by` still carries.)
            assert!(Assignee::from_str("bulk-update").is_err());
            assert!("nocolon".parse::<Assignee>().is_err());
        }

        #[test]
        fn test_event_deserialize_rejects_malformed_assignee() {
            let json = r#"{"type":"issue_claimed","id":"e1","issue_id":"i1","timestamp":"2026-01-01T00:00:00Z","assignee":"nocolon"}"#;
            assert!(serde_json::from_str::<Event>(json).is_err());
        }
    }
}

/// Label namespace configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelNamespace {
    /// Human-readable description
    pub description: String,
    /// Whether only one label from this namespace can be applied per issue
    pub unique: bool,
}

impl LabelNamespace {
    /// Create a new namespace with given properties
    pub fn new(description: impl Into<String>, unique: bool) -> Self {
        Self {
            description: description.into(),
            unique,
        }
    }
}

/// Container for all label namespaces
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelNamespaces {
    /// Schema version for future migrations
    pub schema_version: u32,
    /// Map of namespace name to configuration
    pub namespaces: HashMap<String, LabelNamespace>,
    /// Type hierarchy the repository declared, absent when it declared none
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_hierarchy: Option<HashMap<String, u8>>,
    /// Label associations for membership namespaces (type_name -> namespace)
    /// e.g., "epic" -> "epic", "release" -> "milestone"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label_associations: Option<HashMap<String, String>>,
    /// List of type names that are considered strategic (optional)
    /// e.g., ["milestone", "epic"]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategic_types: Option<Vec<String>>,
}

impl LabelNamespaces {
    /// A registry declaring nothing: no namespace, no type hierarchy, no
    /// membership association and no strategic type.
    ///
    /// Equivalent to what
    /// [`namespaces_from_config`](crate::config_manager::namespaces_from_config)
    /// builds for a `config.toml` that declares neither `[namespaces]` nor
    /// `[type_hierarchy]`, so the rules derived from it are exactly the ones
    /// that need no declaration.
    pub fn empty(schema_version: u32) -> Self {
        Self {
            schema_version,
            namespaces: HashMap::new(),
            type_hierarchy: None,
            label_associations: None,
            strategic_types: None,
        }
    }

    /// Ensure namespaces exist for all membership labels in label_associations.
    /// Dynamically creates namespace entries for custom type names.
    pub fn sync_membership_namespaces(&mut self) {
        if let Some(ref associations) = self.label_associations {
            for (type_name, namespace) in associations {
                // Only create namespace if it doesn't already exist
                if !self.namespaces.contains_key(namespace) {
                    self.namespaces.insert(
                        namespace.clone(),
                        LabelNamespace::new(
                            format!("{} organizational grouping", type_name),
                            false,
                        ),
                    );
                }
            }
        }
    }

    /// `self` with the type hierarchy and membership associations of the
    /// declared test vocabulary ([`test_taxonomy`](crate::test_taxonomy::test_taxonomy))
    /// added, for the suites whose subject is a rule keyed on a declared
    /// hierarchy.
    ///
    /// Derived from that one declaration rather than restating it, and reachable
    /// only under `cfg(test)` or `feature = "test-support"`, so no adopter build
    /// compiles the vocabulary and no repository can receive it.
    #[cfg(any(test, feature = "test-support"))]
    pub fn declaring_test_hierarchy(self) -> Self {
        let vocabulary = crate::test_taxonomy::test_taxonomy().hierarchy_config();
        Self {
            type_hierarchy: Some(
                vocabulary
                    .types()
                    .map(|(name, level)| (name.clone(), *level))
                    .collect(),
            ),
            label_associations: Some(
                vocabulary
                    .membership_namespaces()
                    .map(|(type_name, namespace)| (type_name.clone(), namespace.clone()))
                    .collect(),
            ),
            ..self
        }
    }

    /// The type hierarchy this registry declares, as a type-name to level map.
    ///
    /// Empty when the repository declared no `[type_hierarchy]`, which is what
    /// keeps the rules keyed on it — the `type:` value enumeration and the two
    /// hierarchy graph warnings — out of such a repository's rule set.
    pub fn declared_type_hierarchy(&self) -> HashMap<String, u8> {
        self.type_hierarchy.clone().unwrap_or_default()
    }

    /// Add or update a namespace
    pub fn add(&mut self, name: String, namespace: LabelNamespace) {
        self.namespaces.insert(name, namespace);
    }

    /// Get a namespace by name
    pub fn get(&self, name: &str) -> Option<&LabelNamespace> {
        self.namespaces.get(name)
    }
}
