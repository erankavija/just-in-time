//! Core domain types for the issue tracker.
//!
//! This module defines the fundamental data structures used throughout the system:
//! issues, gates, events, and their associated states and priorities.

use crate::declarations::GateStage;
use crate::domain::gate_findings::GateFindings;
use crate::errors::InvalidArgumentError;
use anyhow::Result;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
#[cfg(test)]
use uuid::Uuid;

/// Length of short issue ID (git-style short hash)
pub const SHORT_ID_LENGTH: usize = 8;

/// Issue lifecycle state
///
/// `PartialOrd`/`Ord` follow the declaration order, which is the lifecycle order
/// (`Backlog < Ready < InProgress < Gated < Done < Rejected < Archived`); this
/// lets states be stored in ordered collections such as `BTreeSet<State>`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// Created but not actionable yet (blocked by dependencies)
    Backlog,
    /// All dependencies in a terminal state, ready for work (gates gate completion, not start)
    Ready,
    /// Currently being worked on
    InProgress,
    /// Work complete, awaiting quality gate approval
    Gated,
    /// Completed successfully
    Done,
    /// Won't implement (bypasses gates)
    Rejected,
    /// No longer relevant
    Archived,
}

impl State {
    /// Check if this state is terminal (Done or Rejected)
    ///
    /// Terminal states represent closure - either successful completion (Done)
    /// or decision not to implement (Rejected). Both unblock dependent issues.
    pub fn is_terminal(self) -> bool {
        matches!(self, State::Done | State::Rejected)
    }

    /// Check if this state is closed (Done or Rejected)
    ///
    /// This is an alias for is_terminal() for query semantics.
    pub fn is_closed(self) -> bool {
        self.is_terminal()
    }

    /// The canonical snake_case string for this state.
    ///
    /// Identical to the JSON serialization (`#[serde(rename_all = "snake_case")]`)
    /// and to what [`State::from_str`] round-trips; use it when a `&'static str`
    /// is needed (e.g. a compact one-line status render) without allocating.
    pub fn as_str(self) -> &'static str {
        match self {
            State::Backlog => "backlog",
            State::Ready => "ready",
            State::InProgress => "in_progress",
            State::Gated => "gated",
            State::Done => "done",
            State::Rejected => "rejected",
            State::Archived => "archived",
        }
    }

    /// Every `State` variant, in canonical lifecycle order.
    ///
    /// Use this to enumerate the state space exhaustively instead of hardcoding
    /// a state list at a call site (@/inv/domain-agnostic): a counts-by-state
    /// rollup, for instance, must list every state so its shape stays stable
    /// and complete as the enum evolves.
    pub const fn all() -> [State; 7] {
        [
            State::Backlog,
            State::Ready,
            State::InProgress,
            State::Gated,
            State::Done,
            State::Rejected,
            State::Archived,
        ]
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
///   state and a legacy `Archived` record whose `archived_from` was never
///   recorded (conservative default: it does not start satisfying dependents).
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

impl FromStr for State {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "backlog" => Ok(State::Backlog),
            "open" => Ok(State::Backlog), // Backward compatibility alias
            "ready" => Ok(State::Ready),
            "in_progress" | "inprogress" => Ok(State::InProgress),
            "gated" => Ok(State::Gated),
            "done" => Ok(State::Done),
            "rejected" => Ok(State::Rejected),
            "archived" => Ok(State::Archived),
            _ => Err(InvalidArgumentError::new(format!("Invalid state: {s}")).into()),
        }
    }
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

/// The content format of an issue body, selecting which [`ContentParser`] the
/// validation projection uses to extract `sections`.
///
/// Serialized lowercase (`"markdown"`, `"html"`, `"xml"`) so issue JSON stays
/// human-readable. When absent on an issue the repo default
/// (`[validation].content_format`) applies; the final fallback is `Markdown`,
/// which is always compiled in. HTML/XML are only usable when the `html`/`xml`
/// cargo features are built (see
/// [`content_parser_for`](crate::document::content_parser_for)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ContentFormat {
    /// Markdown body (the default; always available).
    #[default]
    Markdown,
    /// HTML body (requires the `html` cargo feature to parse).
    Html,
    /// XML body (requires the `xml` cargo feature to parse).
    Xml,
}

impl FromStr for ContentFormat {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "markdown" | "md" => Ok(ContentFormat::Markdown),
            "html" => Ok(ContentFormat::Html),
            "xml" => Ok(ContentFormat::Xml),
            _ => Err(InvalidArgumentError::new(format!(
                "Invalid content format: '{s}' (expected markdown, html, or xml)"
            ))
            .into()),
        }
    }
}

/// Quality gate status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GateStatus {
    /// Gate not yet evaluated
    Pending,
    /// Gate passed successfully
    Passed,
    /// Gate failed
    Failed,
}

impl GateStatus {
    /// The canonical snake_case string for this status.
    ///
    /// Identical to the JSON serialization (`#[serde(rename_all = "snake_case")]`);
    /// use it when a `&'static str` is needed (e.g. a compact `key=status`
    /// render) without allocating.
    pub fn as_str(self) -> &'static str {
        match self {
            GateStatus::Pending => "pending",
            GateStatus::Passed => "passed",
            GateStatus::Failed => "failed",
        }
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
/// [`FromStr`]) and read the parts via [`Assignee::kind`] /
/// [`Assignee::identifier`]. It serializes transparently as the
/// `kind:identifier` string (through [`Display`] / [`FromStr`]), so on-disk
/// issue JSON is unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Assignee {
    kind: String,
    identifier: String,
}

impl Assignee {
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
            kind: kind.to_string(),
            identifier: identifier.to_string(),
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
}

/// An issue representing a unit of work
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    /// repo default (`[validation].content_format`), with a Markdown fallback.
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
    /// left) and cleared on revive, so it is `Some` only while `state ==
    /// Archived`. It drives the issue's effective terminal state
    /// ([`Issue::effective_terminal_state`]) and constrains revive to the
    /// recorded origin. Absent (`None`) for every non-`Archived` issue and for a
    /// legacy record archived before this field existed; a legacy `Archived`
    /// issue is treated as non-terminal (the pre-change behavior). Skipped on
    /// serialize when absent, so old issue files round-trip unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_from: Option<State>,
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
    /// [`Issue::archived_from`]: `Some` only for an `Archived` issue that
    /// recorded its origin, absent otherwise.
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

    /// Get state symbol for human-readable output
    /// - ✓ for effectively terminal states (done/rejected, or archived from one)
    /// - ○ for active states
    pub fn state_symbol(&self) -> &str {
        if self.is_effectively_terminal() {
            "✓"
        } else {
            "○"
        }
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

    fn dependencies(&self) -> &[String] {
        &self.dependencies
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

impl DocumentReference {
    /// Create a new document reference pointing to HEAD
    #[allow(dead_code)]
    pub fn new(path: String) -> Self {
        Self {
            path,
            commit: None,
            label: None,
            doc_type: None,
            format: None,
            assets: Vec::new(),
        }
    }

    /// Create a reference to a document at a specific commit
    #[allow(dead_code)]
    pub fn at_commit(path: String, commit: String) -> Self {
        Self {
            path,
            commit: Some(commit),
            label: None,
            doc_type: None,
            format: None,
            assets: Vec::new(),
        }
    }

    /// Builder method to add a label
    #[allow(dead_code)]
    pub fn with_label(mut self, label: String) -> Self {
        self.label = Some(label);
        self
    }

    /// Builder method to add a document type
    #[allow(dead_code)]
    pub fn with_type(mut self, doc_type: String) -> Self {
        self.doc_type = Some(doc_type);
        self
    }

    /// Builder method to add format
    #[allow(dead_code)]
    pub fn with_format(mut self, format: String) -> Self {
        self.format = Some(format);
        self
    }

    /// Builder method to set assets
    #[allow(dead_code)]
    pub fn with_assets(mut self, assets: Vec<crate::document::Asset>) -> Self {
        self.assets = assets;
        self
    }
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
    /// Git commit (if available)
    pub commit: Option<String>,
    /// Git branch (if available)
    pub branch: Option<String>,
    /// Whether the working tree differed from the named [`commit`](Self::commit)
    /// when the checker started. `Some(true)` means the tree carried uncommitted
    /// or untracked changes, so the run evidences that modified tree rather than
    /// the commit alone; `Some(false)` means the tree matched the commit exactly;
    /// `None` means there was no commit to compare against (the working directory
    /// is not a git repository, or the repository has no commits yet) — a clean
    /// tree is never fabricated in that case. Defaulted so run records written
    /// before this field existed parse as `None`.
    #[serde(default)]
    pub tree_dirty: Option<bool>,
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
    pub by: Option<String>,
    /// Optional message
    pub message: Option<String>,
    /// Structured findings parsed from the checker's machine-readable block, if
    /// one was emitted. `None` for plain-text checkers and for runs recorded
    /// before this field existed; the raw [`stdout`](Self::stdout) is always
    /// kept alongside. See [`parse_gate_findings`](crate::domain::parse_gate_findings).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub findings: Option<GateFindings>,
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
    /// Not applicable (future: for conditional gates)
    Skipped,
}

/// Origin vocabulary persisted for an installed profile package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProfileOrigin {
    /// Package bytes were compiled into the running JIT binary.
    Embedded,
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
    /// The one-time lifecycle-timestamp backfill migration ran
    /// (`jit migrate lifecycle-timestamps`).
    ///
    /// Repository-scoped, like [`Event::ArtifactArchiveExecuted`]: it carries no issue
    /// id because it records a whole-repository migration rather than a single
    /// issue's change. `issues_updated` is the number of issue files the run
    /// wrote (issues that gained at least one derived timestamp); a re-run over an
    /// already-migrated repository writes nothing and appends no event.
    LifecycleTimestampsBackfilled {
        /// Event ID
        id: String,
        /// When this occurred
        timestamp: DateTime<Utc>,
        /// Number of issues whose lifecycle timestamps were backfilled
        issues_updated: usize,
    },
    /// An embedded profile package was transactionally applied.
    ///
    /// Repository-scoped: package targets, the minimal installed record, and
    /// this event become durable in one recoverable transaction.
    ProfileApplied {
        /// Event ID.
        id: String,
        /// When the transaction was constructed.
        timestamp: DateTime<Utc>,
        /// Stable profile identifier.
        profile_id: String,
        /// Applied semantic version.
        version: String,
        /// Package discovery origin.
        origin: ProfileOrigin,
        /// Hash of the complete canonical package.
        package_hash: String,
        /// Package contribution hashes keyed by repository target.
        target_hashes: std::collections::BTreeMap<String, String>,
        /// Whether the transaction isolated a pre-existing non-newline,
        /// malformed event tail immediately before this record.
        isolated_torn_tail: bool,
    },
}

impl Event {
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
            | Event::LifecycleTimestampsBackfilled { id, timestamp, .. }
            | Event::ProfileApplied { id, timestamp, .. } => {
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

    /// Create a lifecycle-timestamp backfill event recording how many issues the
    /// one-time migration updated.
    pub fn draft_lifecycle_timestamps_backfilled(issues_updated: usize) -> Self {
        Event::LifecycleTimestampsBackfilled {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            issues_updated,
        }
    }

    /// Create a repository-scoped profile application event.
    pub fn draft_profile_applied(
        profile_id: String,
        version: String,
        origin: ProfileOrigin,
        package_hash: String,
        target_hashes: std::collections::BTreeMap<String, String>,
        isolated_torn_tail: bool,
    ) -> Self {
        Event::ProfileApplied {
            id: String::new(),
            timestamp: DateTime::UNIX_EPOCH,
            profile_id,
            version,
            origin,
            package_hash,
            target_hashes,
            isolated_torn_tail,
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
            Event::LifecycleTimestampsBackfilled { .. } => "", // No associated issue (repo-scoped)
            Event::ProfileApplied { .. } => "",        // No associated issue (repo-scoped)
        }
    }

    /// Get the event type as a string.
    ///
    /// The string is the value serde writes into the record's `type` field, and
    /// the value `jit events query --event-type` matches against. It is supplied
    /// by [`EventTag::as_str`](crate::domain::EventTag::as_str), the single
    /// definition of the tag vocabulary, via [`Event::tag`].
    pub fn get_type(&self) -> &str {
        self.tag().as_str()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_issue_has_correct_defaults() {
        let issue = crate::domain::types::fixture_issue(
            "Test Issue".to_string(),
            "Description".to_string(),
        );

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
        let mut issue = crate::domain::types::fixture_issue("t".to_string(), String::new());
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
        let mut issue = crate::domain::types::fixture_issue("t".to_string(), String::new());
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
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_blocked_by_unmet_dependency() {
        let mut issue =
            crate::domain::types::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_when_dependency_is_done() {
        let mut issue =
            crate::domain::types::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let mut dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
        dependency.state = State::Done;

        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert!(!issue.is_blocked(&resolved));
    }

    #[test]
    fn test_issue_not_blocked_by_unpassed_gate() {
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());

        let resolved = HashMap::new();

        // Gates don't block work from starting
        assert!(!issue.is_blocked(&resolved));
        // But gates do prevent completion
        assert!(issue.has_unpassed_gates());
    }

    #[test]
    fn test_issue_not_blocked_by_pending_gate() {
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
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
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: Utc::now(),
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
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: Utc::now(),
            },
        );

        let resolved = HashMap::new();

        assert!(!issue.is_blocked(&resolved));
        assert!(!issue.has_unpassed_gates());
    }

    #[test]
    fn test_document_reference_new() {
        let doc = DocumentReference::new("docs/design.md".to_string());
        assert_eq!(doc.path, "docs/design.md");
        assert_eq!(doc.commit, None);
        assert_eq!(doc.label, None);
        assert_eq!(doc.doc_type, None);
    }

    #[test]
    fn test_document_reference_at_commit() {
        let doc = DocumentReference::at_commit("docs/design.md".to_string(), "a1b2c3d".to_string());
        assert_eq!(doc.path, "docs/design.md");
        assert_eq!(doc.commit, Some("a1b2c3d".to_string()));
    }

    #[test]
    fn test_document_reference_builder() {
        let doc = DocumentReference::new("docs/design.md".to_string())
            .with_label("API Design".to_string())
            .with_type("design".to_string());

        assert_eq!(doc.label, Some("API Design".to_string()));
        assert_eq!(doc.doc_type, Some("design".to_string()));
    }

    #[test]
    fn test_document_reference_serialization() {
        let doc = DocumentReference::new("docs/design.md".to_string())
            .with_label("Design Doc".to_string());

        let json = serde_json::to_string(&doc).unwrap();
        let deserialized: DocumentReference = serde_json::from_str(&json).unwrap();

        assert_eq!(doc, deserialized);
    }

    #[test]
    fn test_issue_with_documents() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        assert_eq!(issue.documents.len(), 0);

        issue
            .documents
            .push(DocumentReference::new("docs/design.md".to_string()));
        assert_eq!(issue.documents.len(), 1);
        assert_eq!(issue.documents[0].path, "docs/design.md");
    }

    #[test]
    fn test_issue_serialization_with_documents() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        issue.documents.push(
            DocumentReference::at_commit("docs/design.md".to_string(), "abc123".to_string())
                .with_label("Design".to_string()),
        );

        let json = serde_json::to_string(&issue).unwrap();
        let deserialized: Issue = serde_json::from_str(&json).unwrap();

        assert_eq!(issue.documents.len(), deserialized.documents.len());
        assert_eq!(issue.documents[0], deserialized.documents[0]);
    }

    // State model tests for Backlog and Gated states

    #[test]
    fn test_new_issue_starts_in_backlog() {
        let issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        assert_eq!(issue.state, State::Backlog);
    }

    #[test]
    fn test_backlog_issue_should_auto_transition_to_ready_when_unblocked() {
        let issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        let resolved = HashMap::new();

        assert_eq!(issue.state, State::Backlog);
        assert!(issue.should_auto_transition_to_ready(&resolved));
    }

    #[test]
    fn test_backlog_issue_should_not_transition_to_ready_when_blocked() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        let dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
        issue.dependencies.push(dependency.id.clone());

        let mut resolved = HashMap::new();
        resolved.insert(dependency.id.clone(), &dependency);

        assert_eq!(issue.state, State::Backlog);
        assert!(!issue.should_auto_transition_to_ready(&resolved));
    }

    #[test]
    fn test_derive_readiness_correction_demotes_a_ready_issue_an_unmet_dependency_blocks() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        let dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
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
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        let mut dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
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
        let mut ready =
            crate::domain::types::fixture_issue("Ready".to_string(), "Desc".to_string());
        ready.state = State::Ready;
        let mut blocked =
            crate::domain::types::fixture_issue("Blocked".to_string(), "Desc".to_string());
        let dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
        blocked.dependencies.push(dependency.id.clone());
        let resolved = HashMap::from([(dependency.id.clone(), &dependency)]);

        assert_eq!(ready.derive_readiness_correction(&resolved), None);
        assert_eq!(blocked.derive_readiness_correction(&resolved), None);
    }

    #[test]
    fn test_derive_readiness_correction_leaves_a_state_no_dependency_owns() {
        let dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
        let resolved = HashMap::from([(dependency.id.clone(), &dependency)]);
        let blocked_by_unmet_dependency = [
            State::InProgress,
            State::Gated,
            State::Done,
            State::Rejected,
            State::Archived,
        ]
        .map(|state| {
            let mut issue =
                crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
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
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("human:reviewer".parse().unwrap()),
                updated_at: Utc::now(),
            },
        );

        assert!(issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_gated_issue_should_not_transition_to_done_when_gates_pending() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Pending,
                updated_by: None,
                updated_at: Utc::now(),
            },
        );

        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_gated_issue_should_not_transition_to_done_when_gates_failed() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::Gated;
        issue.gates_required.push("review".to_string());
        issue.gates_status.insert(
            "review".to_string(),
            GateState {
                status: GateStatus::Failed,
                updated_by: Some("ci:tests".parse().unwrap()),
                updated_at: Utc::now(),
            },
        );

        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_in_progress_issue_should_not_auto_transition() {
        let mut issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        issue.state = State::InProgress;

        let resolved = HashMap::new();
        assert!(!issue.should_auto_transition_to_ready(&resolved));
        assert!(!issue.should_auto_transition_to_done());
    }

    #[test]
    fn test_state_serialization_backlog() {
        let state = State::Backlog;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"backlog\"");

        let deserialized: State = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn test_state_serialization_gated() {
        let state = State::Gated;
        let json = serde_json::to_string(&state).unwrap();
        assert_eq!(json, "\"gated\"");

        let deserialized: State = serde_json::from_str(&json).unwrap();
        assert_eq!(state, deserialized);
    }

    #[test]
    fn test_new_issue_has_empty_labels() {
        let issue =
            crate::domain::types::fixture_issue("Test".to_string(), "Description".to_string());
        assert!(issue.labels.is_empty());
    }

    #[test]
    fn test_issue_serialization_with_labels() {
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
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
        let mut issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
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
        // Legacy archived (no recorded origin) stays non-terminal.
        assert!(!is_dependency_met(State::Archived, None));
    }

    #[test]
    fn test_effective_terminality_preserves_archived_origin() {
        // Archived is terminality-preserving: a dependency archived from a
        // terminal state stays met; one archived from a non-terminal state or
        // with no recorded origin (legacy) does not.
        assert!(is_dependency_met(State::Archived, Some(State::Done)));
        assert!(is_dependency_met(State::Archived, Some(State::Rejected)));
        assert!(!is_dependency_met(State::Archived, Some(State::InProgress)));
        assert!(!is_dependency_met(State::Archived, None));

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
        assert_eq!(effective_terminal_state(State::Archived, None), None);
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
        let mut issue =
            crate::domain::types::fixture_issue("Dependent".to_string(), "Desc".to_string());
        let mut dependency =
            crate::domain::types::fixture_issue("Dependency".to_string(), "Desc".to_string());
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
            path: "docs/design.md".to_string(),
            commit: None,
            label: Some("Design Doc".to_string()),
            doc_type: Some("design".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![Asset {
                original_path: "./logo.png".to_string(),
                resolved_path: Some(PathBuf::from("docs/logo.png")),
                asset_type: crate::document::AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: Some("sha256:abc123".to_string()),
                is_shared: false,
            }],
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
            path: "docs/design.md".to_string(),
            commit: None,
            label: Some("Design Doc".to_string()),
            doc_type: Some("design".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![Asset {
                original_path: "./logo.png".to_string(),
                resolved_path: Some(PathBuf::from("docs/logo.png")),
                asset_type: crate::document::AssetType::Local,
                mime_type: Some("image/png".to_string()),
                content_hash: Some("sha256:abc123".to_string()),
                is_shared: false,
            }],
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
            path: "docs/design.md".to_string(),
            commit: None,
            label: Some("Design".to_string()),
            doc_type: Some("design".to_string()),
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
            path: "docs/notes.md".to_string(),
            commit: None,
            label: None,
            doc_type: Some("notes".to_string()),
            format: Some("markdown".to_string()),
            assets: vec![],
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
        fn test_state_from_str_valid_lowercase() {
            assert_eq!(State::from_str("backlog").unwrap(), State::Backlog);
            assert_eq!(State::from_str("ready").unwrap(), State::Ready);
            assert_eq!(State::from_str("in_progress").unwrap(), State::InProgress);
            assert_eq!(State::from_str("gated").unwrap(), State::Gated);
            assert_eq!(State::from_str("done").unwrap(), State::Done);
            assert_eq!(State::from_str("rejected").unwrap(), State::Rejected);
            assert_eq!(State::from_str("archived").unwrap(), State::Archived);
        }

        #[test]
        fn test_state_from_str_valid_uppercase() {
            assert_eq!(State::from_str("BACKLOG").unwrap(), State::Backlog);
            assert_eq!(State::from_str("READY").unwrap(), State::Ready);
            assert_eq!(State::from_str("IN_PROGRESS").unwrap(), State::InProgress);
        }

        #[test]
        fn test_state_from_str_valid_mixedcase() {
            assert_eq!(State::from_str("Backlog").unwrap(), State::Backlog);
            assert_eq!(State::from_str("Ready").unwrap(), State::Ready);
        }

        #[test]
        fn test_state_from_str_aliases() {
            // Backward compatibility alias
            assert_eq!(State::from_str("open").unwrap(), State::Backlog);
            // Alternative in_progress format
            assert_eq!(State::from_str("inprogress").unwrap(), State::InProgress);
        }

        #[test]
        fn test_state_from_str_invalid() {
            assert!(State::from_str("invalid").is_err());
            assert!(State::from_str("").is_err());
            assert!(State::from_str("pending").is_err());
        }

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
        fn test_state_parse_method() {
            // Test using str::parse() method
            let state: State = "ready".parse().unwrap();
            assert_eq!(state, State::Ready);
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
            let mut issue =
                crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());
            issue.assignee = Some(Assignee::from_str("agent:copilot").unwrap());

            let json = serde_json::to_string(&issue).unwrap();
            assert!(json.contains("\"assignee\":\"agent:copilot\""));

            let deserialized: Issue = serde_json::from_str(&json).unwrap();
            assert_eq!(deserialized.assignee, issue.assignee);
        }
    }

    #[test]
    fn test_issue_timestamps_serialize_as_rfc3339() {
        let issue = crate::domain::types::fixture_issue("Test".to_string(), "Desc".to_string());

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

    /// Check if a namespace exists
    #[allow(dead_code)] // May be used in future
    pub fn contains(&self, name: &str) -> bool {
        self.namespaces.contains_key(name)
    }
}
