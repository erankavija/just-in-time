//! Rule data model and `.jit/rules.toml` loader.
//!
//! This module defines the declarative validation rule model and the loader
//! that parses `.jit/rules.toml` into a [`RuleSet`]. It also implements
//! selector matching (the union of rules applicable to a given issue) and the
//! config-level guards required by the design record:
//!
//! - A raw JSON Schema assertion MUST reference a `.jit/schemas/<name>.json`
//!   file; inline raw JSON Schema in TOML is rejected (DR §8.1). TOML cannot
//!   faithfully express JSON Schema (no `null`, native datetimes, regex
//!   backslashes), so raw schemas live only in files.
//! - A rule's `assert` table holds exactly one kind. Shorthand kinds and a
//!   `json-schema` reference cannot coexist in one rule (shorthand XOR file).
//! - `enforce` absent defaults to `false` (warn only, DR §7.2).
//!
//! This module only defines the MODEL, the LOADER, selector matching, and the
//! guards. Actual JSON Schema validation, shorthand desugaring, and graph rule
//! evaluation are filled in by downstream tasks; assertion payloads are parsed
//! and stored here without being evaluated.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use crate::domain::Issue;
use crate::labels as label_utils;

/// Errors that can occur while loading and parsing `.jit/rules.toml`.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::{RuleConfigError, RuleSet};
/// use std::path::Path;
///
/// // A rule whose `assert` table names no assertion kind is a config error.
/// let toml = r#"
/// [[rules]]
/// name = "broken"
/// assert = {}
/// "#;
/// let err = RuleSet::from_toml_str(toml, Path::new(".")).unwrap_err();
/// assert!(matches!(err, RuleConfigError::InvalidAssertion { .. }));
/// ```
#[derive(Debug, Error)]
pub enum RuleConfigError {
    /// The rules file could not be read from disk.
    #[error("failed to read rules file '{path}': {source}")]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// The rules file is not valid TOML or does not match the rule schema.
    #[error("failed to parse rules file: {0}")]
    Toml(#[from] toml::de::Error),

    /// A referenced `.jit/schemas/<name>.json` file could not be read.
    #[error("rule '{rule}': failed to read schema file '{path}': {source}")]
    SchemaIo {
        /// Name of the offending rule.
        rule: String,
        /// Schema file path that failed to read.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },

    /// A referenced schema file did not contain valid JSON.
    #[error("rule '{rule}': schema file '{path}' is not valid JSON: {source}")]
    SchemaJson {
        /// Name of the offending rule.
        rule: String,
        /// Schema file path that failed to parse.
        path: PathBuf,
        /// Underlying JSON parse error.
        source: serde_json::Error,
    },

    /// A rule's `assert` table is invalid (no kind, multiple kinds, or a
    /// shorthand kind combined with a `json-schema` file reference).
    #[error("rule '{rule}': {message}")]
    InvalidAssertion {
        /// Name of the offending rule.
        rule: String,
        /// Human-readable explanation of the problem.
        message: String,
    },

    /// A `json-schema` reference does not name a safe `.jit/schemas/<name>.json`
    /// file. References MUST be relative paths under `schemas/` ending in
    /// `.json`, with no `..` traversal and no absolute paths.
    #[error("rule '{rule}': invalid schema reference '{reference}': {message}")]
    InvalidSchemaReference {
        /// Name of the offending rule.
        rule: String,
        /// The reference string as authored in `rules.toml`.
        reference: String,
        /// Human-readable explanation of why the reference was rejected.
        message: String,
    },

    /// A `when` selector names a lifecycle state that is not one of the seven
    /// valid `state_token` values. Caught at config load so a typo'd state
    /// (which would otherwise silently never match) surfaces immediately.
    #[error(
        "rule '{rule}': invalid state '{value}' in 'when' selector; \
         valid states are {valid}"
    )]
    InvalidState {
        /// Name of the offending rule.
        rule: String,
        /// The unrecognized state token as authored.
        value: String,
        /// Comma-separated list of the valid snake_case state tokens.
        valid: String,
    },

    /// Two or more rules share the same `name`. Rule names MUST be unique so
    /// that every finding attributes unambiguously to exactly one rule: a
    /// finding naming rule `"foo"` must refer to a single rule. (The engine
    /// keys its validator cache by schema identity, not by name, so this guard
    /// is about attribution clarity rather than cache correctness.)
    #[error("duplicate rule name '{name}': rule names must be unique")]
    DuplicateRuleName {
        /// The name that appeared more than once.
        name: String,
    },
}

/// Severity of a rule finding.
///
/// `off` disables the rule entirely; `warn` reports without blocking; `error`
/// reports and (when `enforce` is set) can block a write.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::Severity;
///
/// // `warn` is the default severity when none is authored.
/// assert_eq!(Severity::default(), Severity::Warn);
/// assert_ne!(Severity::Off, Severity::Error);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    /// Rule is disabled.
    Off,
    /// Rule reports a warning but never blocks.
    #[default]
    Warn,
    /// Rule reports an error (blocks writes when `enforce` is true).
    Error,
}

impl Severity {
    /// Stable snake_case token for this severity, matching the TOML grammar.
    ///
    /// Used to render severities consistently in both human and `--json`
    /// validation output.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::Severity;
    ///
    /// assert_eq!(Severity::Off.token(), "off");
    /// assert_eq!(Severity::Warn.token(), "warn");
    /// assert_eq!(Severity::Error.token(), "error");
    /// ```
    pub fn token(self) -> &'static str {
        match self {
            Severity::Off => "off",
            Severity::Warn => "warn",
            Severity::Error => "error",
        }
    }
}

/// Evaluation scope of a rule, derived from its assertion kind.
///
/// `Local` rules are pure predicates over a single projected issue and run on
/// write. `Graph` rules need the whole store and run only in `jit validate` and
/// gate checkers, never on write.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::{RuleSet, Scope};
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "local"
/// assert = { require-label = { label = "type:*" } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
/// assert_eq!(set.rules[0].scope, Scope::Local);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// Per-issue, runs on write.
    Local,
    /// Aggregate/graph, runs only on demand.
    Graph,
}

/// Selector matching the issue dimensions a rule applies to.
///
/// All present dimensions are AND-combined. Matching reads only cheap `Issue`
/// fields (labels, state) and never parses the description. The `label`
/// dimension supports `ns:*` wildcards via [`label_utils::matches_pattern`].
///
/// # Examples
///
/// ```
/// use jit::validation::rules::Selector;
/// use jit::domain::Issue;
///
/// let selector = Selector {
///     type_: Some("epic".to_string()),
///     ..Default::default()
/// };
/// let mut epic = Issue::new("An epic".to_string(), String::new());
/// epic.labels = vec!["type:epic".to_string()];
/// assert!(selector.matches(&epic));
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Selector {
    /// Match issues of this `type:*` label value (e.g. `"epic"`).
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
    /// Match issues carrying this label, supporting `ns:*` wildcards.
    #[serde(default)]
    pub label: Option<String>,
    /// Match issues in any of these lifecycle states (serde snake_case, e.g.
    /// `"ready"`). Authored as a single string or a list of strings; matching
    /// is true when the issue's state token is in the set. State names are
    /// validated at config load (see [`RuleConfigError::InvalidState`]).
    #[serde(default)]
    pub state: Option<StatePredicate>,
    /// Match issues that have a document with this `doc_type`.
    #[serde(rename = "has_doc_type", alias = "has-doc-type", default)]
    pub has_doc_type: Option<String>,
}

impl Selector {
    /// Returns whether this selector matches the given issue.
    ///
    /// Matching is AND across all present dimensions. An empty selector (no
    /// dimensions) matches every issue. Only cheap fields are read; the
    /// description is never parsed.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::Selector;
    /// use jit::domain::Issue;
    ///
    /// // An empty selector matches every issue.
    /// let any = Selector::default();
    /// let issue = Issue::new("title".to_string(), String::new());
    /// assert!(any.matches(&issue));
    /// ```
    pub fn matches(&self, issue: &Issue) -> bool {
        self.matches_type(issue)
            && self.matches_label(issue)
            && self.matches_state(issue)
            && self.matches_doc_type(issue)
    }

    /// Explain why this selector does NOT match the issue, or `None` if it does.
    ///
    /// Returns a human-readable reason naming the selector dimension(s) that
    /// excluded the issue, joined by `"; "` when more than one dimension fails.
    /// The state dimension is called out explicitly (the issue's current state
    /// token versus the predicate's authored tokens) so `--explain` can show
    /// "state predicate did not match". `None` is returned exactly when
    /// [`Selector::matches`] is `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::{Selector, StatePredicate};
    /// use jit::domain::{Issue, State};
    ///
    /// let selector = Selector {
    ///     state: Some(StatePredicate::Single("done".to_string())),
    ///     ..Default::default()
    /// };
    /// let mut issue = Issue::new("t".to_string(), String::new());
    /// issue.state = State::InProgress;
    /// let reason = selector.match_failure(&issue).unwrap();
    /// assert!(reason.contains("in_progress"));
    /// assert!(reason.contains("done"));
    /// ```
    pub fn match_failure(&self, issue: &Issue) -> Option<String> {
        let mut reasons: Vec<String> = Vec::new();
        if !self.matches_type(issue) {
            let want = self.type_.as_deref().unwrap_or("");
            reasons.push(format!(
                "type predicate did not match (issue is not 'type:{want}')"
            ));
        }
        if !self.matches_label(issue) {
            let want = self.label.as_deref().unwrap_or("");
            reasons.push(format!(
                "label predicate did not match (issue lacks label '{want}')"
            ));
        }
        if !self.matches_state(issue) {
            let want = self
                .state
                .as_ref()
                .map(|p| p.tokens().join("|"))
                .unwrap_or_default();
            reasons.push(format!(
                "state predicate did not match (issue is '{}', wants '{}')",
                state_token(issue.state),
                want
            ));
        }
        if !self.matches_doc_type(issue) {
            let want = self.has_doc_type.as_deref().unwrap_or("");
            reasons.push(format!(
                "has_doc_type predicate did not match (issue has no '{want}' document)"
            ));
        }
        if reasons.is_empty() {
            None
        } else {
            Some(reasons.join("; "))
        }
    }

    fn matches_type(&self, issue: &Issue) -> bool {
        match &self.type_ {
            None => true,
            Some(ty) => {
                let needle = format!("type:{ty}");
                issue.labels.iter().any(|l| l == &needle)
            }
        }
    }

    fn matches_label(&self, issue: &Issue) -> bool {
        match &self.label {
            None => true,
            Some(pattern) => label_utils::matches_pattern(&issue.labels, pattern),
        }
    }

    fn matches_state(&self, issue: &Issue) -> bool {
        match &self.state {
            None => true,
            Some(predicate) => predicate.matches(issue.state),
        }
    }

    fn matches_doc_type(&self, issue: &Issue) -> bool {
        match &self.has_doc_type {
            None => true,
            Some(doc_type) => issue
                .documents
                .iter()
                .any(|d| d.doc_type.as_deref() == Some(doc_type.as_str())),
        }
    }
}

/// Serde snake_case token for a lifecycle state, matching the selector grammar.
fn state_token(state: crate::domain::State) -> &'static str {
    use crate::domain::State;
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

/// The seven valid lifecycle-state tokens, in lifecycle order, that a `when`
/// state predicate may name. Used both to validate authored predicates at load
/// and to list the valid values in [`RuleConfigError::InvalidState`].
const VALID_STATE_TOKENS: [&str; 7] = [
    "backlog",
    "ready",
    "in_progress",
    "gated",
    "done",
    "rejected",
    "archived",
];

/// A `when` state predicate: one or more lifecycle states a rule applies to.
///
/// Authored in TOML as either a single string or a list of strings, both of
/// which deserialize into the same set-of-states predicate:
///
/// ```toml
/// when = { state = "in_progress" }                       # single
/// when = { state = ["ready", "in_progress", "gated"] }   # list
/// ```
///
/// Matching is membership: [`StatePredicate::matches`] is true when the issue's
/// state token is one of the predicate's tokens. The stored tokens are the
/// authored strings (lowercased on comparison); they are validated against the
/// seven valid tokens at config load via [`StatePredicate::validate`].
///
/// # Examples
///
/// ```
/// use jit::validation::rules::Selector;
/// use jit::domain::{Issue, State};
///
/// let toml = r#"
/// [[rules]]
/// name = "lifecycle"
/// when = { state = ["ready", "in_progress"] }
/// assert = { require-section = { heading = "Plan" } }
/// "#;
/// let set =
///     jit::validation::rules::RuleSet::from_toml_str(toml, std::path::Path::new("/x")).unwrap();
/// let mut issue = Issue::new("t".to_string(), String::new());
/// issue.state = State::InProgress;
/// assert!(set.rules[0].when.matches(&issue));
/// issue.state = State::Done;
/// assert!(!set.rules[0].when.matches(&issue));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum StatePredicate {
    /// A single authored state token (e.g. `"in_progress"`).
    Single(String),
    /// A list of authored state tokens (e.g. `["ready", "in_progress"]`).
    List(Vec<String>),
}

impl StatePredicate {
    /// The authored state tokens this predicate carries, in authored order.
    pub fn tokens(&self) -> &[String] {
        match self {
            StatePredicate::Single(s) => std::slice::from_ref(s),
            StatePredicate::List(v) => v.as_slice(),
        }
    }

    /// Returns whether the given lifecycle state is in this predicate's set.
    ///
    /// Comparison is case-insensitive against the snake_case state tokens.
    pub fn matches(&self, state: crate::domain::State) -> bool {
        let needle = state_token(state);
        self.tokens().iter().any(|t| t.to_lowercase() == needle)
    }

    /// Validate that this predicate is non-empty and every token names one of
    /// the seven valid lifecycle states.
    ///
    /// An empty list (`state = []`) is rejected because it silently never
    /// matches any issue — a rule scoped to zero states is always a config
    /// error, not a useful no-op.
    ///
    /// Returns [`RuleConfigError::InvalidState`] naming the first unrecognized
    /// token (or the empty-list sentinel `"<empty list>"`) together with the
    /// owning rule and the list of valid tokens.
    fn validate(&self, rule: &str) -> Result<(), RuleConfigError> {
        if self.tokens().is_empty() {
            return Err(RuleConfigError::InvalidState {
                rule: rule.to_string(),
                value: "<empty list>".to_string(),
                valid: VALID_STATE_TOKENS.join(", "),
            });
        }
        for token in self.tokens() {
            if !VALID_STATE_TOKENS.contains(&token.to_lowercase().as_str()) {
                return Err(RuleConfigError::InvalidState {
                    rule: rule.to_string(),
                    value: token.clone(),
                    valid: VALID_STATE_TOKENS.join(", "),
                });
            }
        }
        Ok(())
    }
}

/// Source of a raw JSON Schema assertion.
///
/// Raw JSON Schema lives ONLY in `.jit/schemas/<name>.json` files; the relative
/// reference and the transcoded schema value are both retained. The `schema`
/// value is loaded eagerly at parse time so downstream compilation never has to
/// touch the filesystem.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::SchemaSource;
/// use std::path::PathBuf;
///
/// let source = SchemaSource {
///     reference: "schemas/epic-body.json".to_string(),
///     path: PathBuf::from("/repo/.jit/schemas/epic-body.json"),
///     schema: serde_json::json!({ "type": "object" }),
/// };
/// assert_eq!(source.reference, "schemas/epic-body.json");
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaSource {
    /// The reference string as authored in `rules.toml` (e.g.
    /// `"schemas/epic-body.json"`).
    pub reference: String,
    /// The schema file's path, resolved relative to the `.jit` root.
    pub path: PathBuf,
    /// The parsed JSON Schema document.
    pub schema: serde_json::Value,
}

/// A single assertion kind. Exactly one kind is present per rule.
///
/// Shorthand kinds carry simple scalars and desugar to JSON Schema downstream.
/// [`Assertion::JsonSchema`] carries a raw schema loaded from a file. Graph
/// kinds run only on demand. Payloads are parsed/stored but not yet evaluated.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::{Assertion, Scope};
///
/// let assertion = Assertion::RequireSection {
///     heading: "Success Criteria".to_string(),
/// };
/// assert_eq!(assertion.scope(), Scope::Local);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub enum Assertion {
    /// Require a label (by exact value or `ns:*` wildcard) with optional
    /// cardinality bounds. Shorthand; local scope.
    RequireLabel {
        /// Label or `ns:*` wildcard that must be present.
        label: String,
        /// Minimum number of matching labels (inclusive).
        min: Option<u32>,
        /// Maximum number of matching labels (inclusive).
        max: Option<u32>,
    },
    /// Require a section with the given heading in the description. Shorthand;
    /// local scope.
    RequireSection {
        /// Heading text that must be present.
        heading: String,
    },
    /// Require an attached document of the given `doc_type`. Shorthand; local
    /// scope.
    RequireDocType {
        /// Document type that must be present.
        doc_type: String,
    },
    /// Require label values in a namespace to match a regex. Shorthand; local
    /// scope. The regex is authored as a TOML literal string and stored as-is.
    LabelValuePattern {
        /// Namespace whose values are constrained (e.g. `"req"`).
        namespace: String,
        /// Regex the values must match (raw, not yet compiled).
        regex: String,
    },
    /// Validate the projection against a raw JSON Schema from a file. Raw;
    /// local scope.
    JsonSchema(SchemaSource),
    /// Run an external checker command (escape hatch). Local scope.
    CheckerCommand(String),
    /// Every source criterion is satisfied by at least one child. Graph scope.
    LabelCoverage {
        /// Raw configuration table for the coverage rule (evaluated downstream).
        config: toml::value::Table,
    },
    /// A from-reference resolves to a declared source. Graph scope.
    LabelReference {
        /// Raw configuration table for the reference rule (evaluated downstream).
        config: toml::value::Table,
    },
    /// A selector must/should depend on a target selector. Graph scope.
    DependencyShape {
        /// Raw configuration table for the dependency-shape rule.
        config: toml::value::Table,
    },
    /// An issue's recorded gate results must be no older than a configured age.
    /// Graph scope. Age is computed against `GateState.updated_at` at evaluation
    /// time, with the clock injected into
    /// [`evaluate_graph`](crate::validation::graph::evaluate_graph) (never read
    /// from wall-clock inside the engine).
    GateRecency {
        /// Maximum permitted age of a gate result, in whole hours. Authored as
        /// `max-age-days` (multiplied by 24) or `max-age-hours`.
        max_age_hours: u64,
        /// Which gate keys to check. Empty means "all of the issue's
        /// `gates_required`".
        gates: Vec<String>,
    },
    /// A built-in type-hierarchy warning (orphan-leaf or strategic-consistency).
    /// Graph scope. Authorable in `rules.toml` via the `type-hierarchy` assert
    /// kind, and also constructed programmatically as a built-in default rule
    /// (see [`default_ruleset`](crate::validation::defaults::default_ruleset)).
    /// Evaluation reuses the existing [`crate::type_hierarchy`] domain functions
    /// rather than reimplementing the hierarchy logic; the repo's
    /// [`HierarchyConfig`] is NOT stored in the parsed rule — it is injected by
    /// the graph evaluator at evaluation time (see
    /// [`evaluate_graph`](crate::validation::graph::evaluate_graph)).
    TypeHierarchy {
        /// Which legacy hierarchy check this rule performs.
        kind: TypeHierarchyKind,
    },
}

/// The legacy type-hierarchy warning expressed by an
/// [`Assertion::TypeHierarchy`].
///
/// Each variant reuses one existing domain function over the whole issue set:
/// `OrphanLeaf` -> [`crate::type_hierarchy::validate_orphans`],
/// `StrategicConsistency` -> [`crate::type_hierarchy::validate_strategic_labels`].
///
/// # Examples
///
/// ```
/// use jit::validation::rules::{Assertion, Scope, TypeHierarchyKind};
///
/// let assertion = Assertion::TypeHierarchy {
///     kind: TypeHierarchyKind::OrphanLeaf,
/// };
/// // Type-hierarchy checks need the whole issue set, so they are graph-scoped.
/// assert_eq!(assertion.scope(), Scope::Graph);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeHierarchyKind {
    /// A leaf-level issue (e.g. a task) carries no parent association label.
    OrphanLeaf,
    /// A strategic-type issue (e.g. an epic) is missing its identifying label.
    StrategicConsistency,
}

impl Assertion {
    /// The evaluation scope implied by this assertion kind.
    ///
    /// Shorthand and file-schema kinds are [`Scope::Local`]; the aggregate graph
    /// kinds are [`Scope::Graph`].
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::{Assertion, Scope};
    ///
    /// let local = Assertion::RequireDocType {
    ///     doc_type: "design".to_string(),
    /// };
    /// assert_eq!(local.scope(), Scope::Local);
    /// ```
    pub fn scope(&self) -> Scope {
        match self {
            Assertion::LabelCoverage { .. }
            | Assertion::LabelReference { .. }
            | Assertion::DependencyShape { .. }
            | Assertion::GateRecency { .. }
            | Assertion::TypeHierarchy { .. } => Scope::Graph,
            _ => Scope::Local,
        }
    }

    /// Whether this graph assertion has REPO-WIDE semantics that make it unsafe
    /// to evaluate over only a neighborhood slice at transition time (CC-2a).
    ///
    /// Transition-time enforcement evaluates rules over the issue's dependency
    /// neighborhood, not the whole repository. A rule that resolves references
    /// against ANY issue in the repo (`label-reference` with the default
    /// `scope = "global"`) would produce false "dangling reference" findings on a
    /// slice that omits the declaring issue, so it is skipped at transition time
    /// and remains a `jit validate` concern. A `scope = "linked"` reference rule
    /// resolves only against linked issues, all of which the neighborhood slice
    /// includes, so it runs.
    ///
    /// `type-hierarchy` checks (orphan-leaf / strategic-consistency) reason about
    /// each issue against the WHOLE set's parent/child relations, so they are
    /// likewise treated as repo-wide and validate-only. The per-issue graph kinds
    /// (`label-coverage`, `dependency-shape`, `gate-recency`) only need the
    /// issue's neighborhood and so run at transition time.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::RuleSet;
    /// use std::path::Path;
    ///
    /// let toml = r#"
    /// [[rules]]
    /// name = "global-ref"
    /// assert = { label-reference = { from = "satisfies", to = "req" } }
    ///
    /// [[rules]]
    /// name = "linked-ref"
    /// assert = { label-reference = { from = "satisfies", to = "req", scope = "linked" } }
    /// "#;
    /// let set = RuleSet::from_toml_str(toml, Path::new("/x")).unwrap();
    /// assert!(set.rules[0].assert.is_repo_wide_at_transition());
    /// assert!(!set.rules[1].assert.is_repo_wide_at_transition());
    /// ```
    pub fn is_repo_wide_at_transition(&self) -> bool {
        match self {
            // A label-reference rule is repo-wide unless explicitly scoped to
            // linked issues (which the neighborhood slice fully contains).
            Assertion::LabelReference { config } => {
                config.get("scope").and_then(|v| v.as_str()) != Some("linked")
            }
            // Type-hierarchy checks reason over the whole set's relations.
            Assertion::TypeHierarchy { .. } => true,
            // The remaining graph kinds are per-issue / neighborhood-local.
            _ => false,
        }
    }
}

/// A fully parsed validation rule.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::{RuleSet, Scope, Severity};
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "epic-needs-req"
/// when = { type = "epic" }
/// severity = "error"
/// enforce = true
/// assert = { require-label = { label = "req:*", min = 1 } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
/// let rule = &set.rules[0];
/// assert_eq!(rule.name, "epic-needs-req");
/// assert_eq!(rule.severity, Severity::Error);
/// assert!(rule.enforce);
/// assert_eq!(rule.scope, Scope::Local);
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Unique, human-readable rule name.
    pub name: String,
    /// Selector deciding which issues the rule applies to.
    pub when: Selector,
    /// Reporting severity.
    pub severity: Severity,
    /// Whether an `error` finding blocks writes (default false).
    pub enforce: bool,
    /// The assertion to evaluate.
    pub assert: Assertion,
    /// Evaluation scope, derived from the assertion kind.
    pub scope: Scope,
}

/// A parsed set of rules, ready for selector matching.
///
/// # Examples
///
/// ```
/// use jit::validation::rules::RuleSet;
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "ready-needs-criteria"
/// when = { state = "ready" }
/// assert = { require-section = { heading = "Success Criteria" } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
/// assert_eq!(set.rules.len(), 1);
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSet {
    /// The rules in authored order.
    pub rules: Vec<Rule>,
}

impl RuleSet {
    /// An empty rule set (used when no `rules.toml` exists).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::RuleSet;
    ///
    /// let set = RuleSet::empty();
    /// assert!(set.rules.is_empty());
    /// ```
    pub fn empty() -> Self {
        Self::default()
    }

    /// Load and parse `.jit/rules.toml` relative to the given `.jit` root.
    ///
    /// Returns an empty [`RuleSet`] when the file does not exist. Referenced
    /// schema files are read relative to `jit_root` and transcoded to JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::RuleSet;
    ///
    /// // A directory with no `rules.toml` loads as an empty rule set.
    /// let dir = tempfile::tempdir().unwrap();
    /// let set = RuleSet::load(dir.path()).unwrap();
    /// assert!(set.rules.is_empty());
    /// ```
    pub fn load(jit_root: &Path) -> Result<Self, RuleConfigError> {
        let path = jit_root.join("rules.toml");
        if !path.exists() {
            return Ok(Self::empty());
        }
        let content = std::fs::read_to_string(&path).map_err(|source| RuleConfigError::Io {
            path: path.clone(),
            source,
        })?;
        Self::from_toml_str(&content, jit_root)
    }

    /// Parse a `rules.toml` string. `jit_root` resolves schema file references.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::RuleSet;
    /// use std::path::Path;
    ///
    /// let toml = r#"
    /// [[rules]]
    /// name = "task-needs-design"
    /// when = { type = "task" }
    /// assert = { require-doc-type = { doc-type = "design" } }
    /// "#;
    /// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
    /// assert_eq!(set.rules[0].name, "task-needs-design");
    /// ```
    pub fn from_toml_str(content: &str, jit_root: &Path) -> Result<Self, RuleConfigError> {
        let raw: RawRulesFile = toml::from_str(content)?;
        let rules = raw
            .rules
            .into_iter()
            .map(|r| r.into_rule(jit_root))
            .collect::<Result<Vec<_>, _>>()?;

        // Enforce the documented "Unique" invariant on `Rule::name` so every
        // finding attributes to exactly one rule. (The engine keys its validator
        // cache by schema identity, not name, so this is about attribution
        // clarity, not cache correctness.) Detect the first collision via a
        // `HashSet` insert.
        let mut seen = std::collections::HashSet::new();
        if let Some(rule) = rules.iter().find(|r| !seen.insert(r.name.as_str())) {
            return Err(RuleConfigError::DuplicateRuleName {
                name: rule.name.clone(),
            });
        }

        Ok(Self { rules })
    }

    /// Returns the union of rules whose selector matches the issue.
    ///
    /// Only cheap `Issue` fields are read (no description parse). Rules are
    /// returned in authored order.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::rules::RuleSet;
    /// use jit::domain::Issue;
    /// use std::path::Path;
    ///
    /// let toml = r#"
    /// [[rules]]
    /// name = "epic-rule"
    /// when = { type = "epic" }
    /// assert = { require-section = { heading = "Goals" } }
    /// "#;
    /// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
    ///
    /// let mut epic = Issue::new("An epic".to_string(), String::new());
    /// epic.labels = vec!["type:epic".to_string()];
    /// assert_eq!(set.matching_rules(&epic).len(), 1);
    ///
    /// let mut task = Issue::new("A task".to_string(), String::new());
    /// task.labels = vec!["type:task".to_string()];
    /// assert!(set.matching_rules(&task).is_empty());
    /// ```
    pub fn matching_rules(&self, issue: &Issue) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| rule.when.matches(issue))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Raw deserialization layer
// ---------------------------------------------------------------------------

/// Top-level `rules.toml` document.
#[derive(Debug, Deserialize)]
struct RawRulesFile {
    #[serde(default, rename = "rules")]
    rules: Vec<RawRule>,
}

/// A rule as authored, before validation of the `assert` table.
#[derive(Debug, Deserialize)]
struct RawRule {
    name: String,
    #[serde(default)]
    when: Selector,
    #[serde(default)]
    severity: Severity,
    #[serde(default)]
    enforce: bool,
    assert: RawAssert,
}

/// The `assert` table with every possible kind optional, so we can enforce the
/// exactly-one and shorthand-XOR-file guards ourselves.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAssert {
    #[serde(default, rename = "require-label")]
    require_label: Option<RawRequireLabel>,
    #[serde(default, rename = "require-section")]
    require_section: Option<RawRequireSection>,
    #[serde(default, rename = "require-doc-type")]
    require_doc_type: Option<RawRequireDocType>,
    #[serde(default, rename = "label-value-pattern")]
    label_value_pattern: Option<RawLabelValuePattern>,
    #[serde(default, rename = "json-schema")]
    json_schema: Option<String>,
    #[serde(default, rename = "checker-command")]
    checker_command: Option<String>,
    #[serde(default, rename = "label-coverage")]
    label_coverage: Option<toml::value::Table>,
    #[serde(default, rename = "label-reference")]
    label_reference: Option<toml::value::Table>,
    #[serde(default, rename = "dependency-shape")]
    dependency_shape: Option<toml::value::Table>,
    #[serde(default, rename = "gate-recency")]
    gate_recency: Option<RawGateRecency>,
    #[serde(default, rename = "type-hierarchy")]
    type_hierarchy: Option<RawTypeHierarchy>,
}

/// The `gate-recency` assert payload: a max age plus an optional gate filter.
///
/// Exactly one of `max-age-days` / `max-age-hours` MUST be set; both-or-neither
/// is a config error (caught in [`RawGateRecency::into_assertion`]).
/// `deny_unknown_fields` rejects a stray key so a typo surfaces at load.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGateRecency {
    #[serde(default, rename = "max-age-days")]
    max_age_days: Option<u64>,
    #[serde(default, rename = "max-age-hours")]
    max_age_hours: Option<u64>,
    #[serde(default)]
    gates: Option<Vec<String>>,
}

impl RawGateRecency {
    /// Lower into an [`Assertion::GateRecency`], normalizing the age to whole
    /// hours and rejecting a missing/ambiguous/zero age at load.
    fn into_assertion(self, rule: &str) -> Result<Assertion, RuleConfigError> {
        let max_age_hours = match (self.max_age_days, self.max_age_hours) {
            (Some(_), Some(_)) => {
                return Err(RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: "gate-recency must set exactly one of 'max-age-days' or \
                              'max-age-hours', not both"
                        .to_string(),
                });
            }
            (None, None) => {
                return Err(RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: "gate-recency requires 'max-age-days' or 'max-age-hours'".to_string(),
                });
            }
            (Some(days), None) => {
                days.checked_mul(24)
                    .ok_or_else(|| RuleConfigError::InvalidAssertion {
                        rule: rule.to_string(),
                        message: "gate-recency 'max-age-days' is too large".to_string(),
                    })?
            }
            (None, Some(hours)) => hours,
        };
        if max_age_hours == 0 {
            return Err(RuleConfigError::InvalidAssertion {
                rule: rule.to_string(),
                message: "gate-recency max age must be greater than zero".to_string(),
            });
        }
        // The evaluator compares against `chrono` hour counts, which are i64.
        // Reject ages beyond that range at load so the comparison can never
        // wrap a huge-but-parseable age into a negative threshold.
        if max_age_hours > i64::MAX as u64 {
            return Err(RuleConfigError::InvalidAssertion {
                rule: rule.to_string(),
                message: "gate-recency max age is too large".to_string(),
            });
        }
        Ok(Assertion::GateRecency {
            max_age_hours,
            gates: self.gates.unwrap_or_default(),
        })
    }
}

/// The `type-hierarchy` assert payload: a single `kind` discriminator selecting
/// one of the two built-in hierarchy warnings. `deny_unknown_fields` rejects any
/// stray key so a typo surfaces as a parse error.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTypeHierarchy {
    kind: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequireLabel {
    label: String,
    #[serde(default)]
    min: Option<u32>,
    #[serde(default)]
    max: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequireSection {
    heading: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequireDocType {
    #[serde(rename = "doc-type", alias = "doc_type")]
    doc_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLabelValuePattern {
    namespace: String,
    regex: String,
}

impl RawRule {
    fn into_rule(self, jit_root: &Path) -> Result<Rule, RuleConfigError> {
        // Validate the `when` state predicate at load so a typo'd state (which
        // would otherwise silently never match any issue) is rejected with an
        // error naming the rule and the valid tokens.
        if let Some(state) = &self.when.state {
            state.validate(&self.name)?;
        }
        let assert = self.assert.into_assertion(&self.name, jit_root)?;
        // A `checker-command` is the validate/gate ESCAPE HATCH (DR §4.3) and is
        // never evaluated on the write path, so it can NEVER block a write.
        // Authoring `enforce = true` on one would silently be a no-op as a
        // blocker; reject it at load time so the contradiction is explicit.
        if self.enforce && matches!(assert, Assertion::CheckerCommand(_)) {
            return Err(RuleConfigError::InvalidAssertion {
                rule: self.name,
                message: "checker-command rules cannot block writes; remove enforce".to_string(),
            });
        }
        let scope = assert.scope();
        Ok(Rule {
            name: self.name,
            when: self.when,
            severity: self.severity,
            enforce: self.enforce,
            assert,
            scope,
        })
    }
}

impl RawAssert {
    fn into_assertion(self, rule: &str, jit_root: &Path) -> Result<Assertion, RuleConfigError> {
        // Collect which kinds were provided, partitioned into shorthand vs raw.
        let shorthand_present = self.require_label.is_some()
            || self.require_section.is_some()
            || self.require_doc_type.is_some()
            || self.label_value_pattern.is_some()
            || self.checker_command.is_some()
            || self.label_coverage.is_some()
            || self.label_reference.is_some()
            || self.dependency_shape.is_some()
            || self.gate_recency.is_some()
            || self.type_hierarchy.is_some();
        let raw_schema_present = self.json_schema.is_some();

        // Shorthand XOR file-schema: combining them in one rule is a config
        // error (DR §8.1).
        if shorthand_present && raw_schema_present {
            return Err(RuleConfigError::InvalidAssertion {
                rule: rule.to_string(),
                message:
                    "a rule cannot combine a 'json-schema' file reference with a shorthand kind; \
                     use exactly one"
                        .to_string(),
            });
        }

        // Count the total number of kinds so we can require exactly one.
        let total = [
            self.require_label.is_some(),
            self.require_section.is_some(),
            self.require_doc_type.is_some(),
            self.label_value_pattern.is_some(),
            self.json_schema.is_some(),
            self.checker_command.is_some(),
            self.label_coverage.is_some(),
            self.label_reference.is_some(),
            self.dependency_shape.is_some(),
            self.gate_recency.is_some(),
            self.type_hierarchy.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count();

        if total == 0 {
            return Err(RuleConfigError::InvalidAssertion {
                rule: rule.to_string(),
                message: "the 'assert' table must contain exactly one assertion kind, found none"
                    .to_string(),
            });
        }
        if total > 1 {
            return Err(RuleConfigError::InvalidAssertion {
                rule: rule.to_string(),
                message: format!(
                    "the 'assert' table must contain exactly one assertion kind, found {total}"
                ),
            });
        }

        // Exactly one kind is present; build it.
        if let Some(rl) = self.require_label {
            return Ok(Assertion::RequireLabel {
                label: rl.label,
                min: rl.min,
                max: rl.max,
            });
        }
        if let Some(rs) = self.require_section {
            return Ok(Assertion::RequireSection {
                heading: rs.heading,
            });
        }
        if let Some(rd) = self.require_doc_type {
            return Ok(Assertion::RequireDocType {
                doc_type: rd.doc_type,
            });
        }
        if let Some(lvp) = self.label_value_pattern {
            return Ok(Assertion::LabelValuePattern {
                namespace: lvp.namespace,
                regex: lvp.regex,
            });
        }
        if let Some(reference) = self.json_schema {
            return load_schema_source(rule, jit_root, reference).map(Assertion::JsonSchema);
        }
        if let Some(cmd) = self.checker_command {
            return Ok(Assertion::CheckerCommand(cmd));
        }
        if let Some(config) = self.label_coverage {
            // Validate `child-state` at load so a typo'd state (which would
            // otherwise silently never match any child, producing spurious
            // "criterion not satisfied" findings) is caught immediately.
            if let Some(toml::Value::String(s)) = config.get("child-state") {
                StatePredicate::Single(s.clone()).validate(rule)?;
            }
            return Ok(Assertion::LabelCoverage { config });
        }
        if let Some(config) = self.label_reference {
            return Ok(Assertion::LabelReference { config });
        }
        if let Some(config) = self.dependency_shape {
            return Ok(Assertion::DependencyShape { config });
        }
        if let Some(gr) = self.gate_recency {
            return gr.into_assertion(rule);
        }
        if let Some(th) = self.type_hierarchy {
            let kind = match th.kind.as_str() {
                "orphan-leaf" => TypeHierarchyKind::OrphanLeaf,
                "strategic-consistency" => TypeHierarchyKind::StrategicConsistency,
                other => {
                    return Err(RuleConfigError::InvalidAssertion {
                        rule: rule.to_string(),
                        message: format!(
                            "type-hierarchy 'kind' must be 'orphan-leaf' or \
                             'strategic-consistency', found '{other}'"
                        ),
                    });
                }
            };
            return Ok(Assertion::TypeHierarchy { kind });
        }

        // Unreachable: total == 1 guarantees one branch above matched.
        Err(RuleConfigError::InvalidAssertion {
            rule: rule.to_string(),
            message: "internal error: assertion kind not handled".to_string(),
        })
    }
}

/// Validate that a `json-schema` reference names a safe file under `schemas/`.
///
/// A reference MUST be a relative path that starts with `schemas/`, ends with
/// `.json`, and contains no `..` (parent) component. Absolute paths are
/// rejected because [`Path::join`] would discard the `.jit` root, letting a rule
/// read an arbitrary file; `..` is rejected to prevent traversal out of
/// `<jit_root>/schemas/`. After these checks the resolved path is guaranteed to
/// live under `<jit_root>/schemas/`.
fn validate_schema_reference(rule: &str, reference: &str) -> Result<(), RuleConfigError> {
    let reject = |message: &str| RuleConfigError::InvalidSchemaReference {
        rule: rule.to_string(),
        reference: reference.to_string(),
        message: message.to_string(),
    };

    let ref_path = Path::new(reference);

    if ref_path.is_absolute() {
        return Err(reject("reference must be a relative path, not absolute"));
    }
    if ref_path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(reject("reference must not contain a '..' path component"));
    }
    if !reference.starts_with("schemas/") {
        return Err(reject("reference must start with 'schemas/'"));
    }
    if !reference.ends_with(".json") {
        return Err(reject("reference must end with '.json'"));
    }
    Ok(())
}

/// Load a `.jit/schemas/<name>.json` file referenced by a `json-schema` rule and
/// transcode it into a `serde_json::Value`.
fn load_schema_source(
    rule: &str,
    jit_root: &Path,
    reference: String,
) -> Result<SchemaSource, RuleConfigError> {
    validate_schema_reference(rule, &reference)?;
    let path = jit_root.join(&reference);
    let content = std::fs::read_to_string(&path).map_err(|source| RuleConfigError::SchemaIo {
        rule: rule.to_string(),
        path: path.clone(),
        source,
    })?;
    let schema: serde_json::Value =
        serde_json::from_str(&content).map_err(|source| RuleConfigError::SchemaJson {
            rule: rule.to_string(),
            path: path.clone(),
            source,
        })?;
    Ok(SchemaSource {
        reference,
        path,
        schema,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, Issue, State};

    fn issue_with(labels: &[&str], state: State) -> Issue {
        let mut issue = Issue::new("t".to_string(), String::new());
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue.state = state;
        issue
    }

    // --- Selector matching -------------------------------------------------

    #[test]
    fn test_selector_empty_matches_everything() {
        let sel = Selector::default();
        assert!(sel.matches(&issue_with(&[], State::Backlog)));
        assert!(sel.matches(&issue_with(&["type:epic"], State::Done)));
    }

    #[test]
    fn test_selector_type_matches_type_label() {
        let sel = Selector {
            type_: Some("epic".to_string()),
            ..Default::default()
        };
        assert!(sel.matches(&issue_with(&["type:epic"], State::Ready)));
        assert!(!sel.matches(&issue_with(&["type:task"], State::Ready)));
        assert!(!sel.matches(&issue_with(&[], State::Ready)));
    }

    #[test]
    fn test_selector_label_supports_wildcard() {
        let sel = Selector {
            label: Some("req:*".to_string()),
            ..Default::default()
        };
        assert!(sel.matches(&issue_with(&["req:REQ-01"], State::Ready)));
        assert!(sel.matches(&issue_with(&["req:anything", "type:task"], State::Ready)));
        assert!(!sel.matches(&issue_with(&["type:task"], State::Ready)));
    }

    #[test]
    fn test_selector_label_exact_match() {
        let sel = Selector {
            label: Some("profile:sdd".to_string()),
            ..Default::default()
        };
        assert!(sel.matches(&issue_with(&["profile:sdd"], State::Ready)));
        assert!(!sel.matches(&issue_with(&["profile:other"], State::Ready)));
    }

    #[test]
    fn test_selector_state_matches_snake_case() {
        let sel = Selector {
            state: Some(StatePredicate::Single("in_progress".to_string())),
            ..Default::default()
        };
        assert!(sel.matches(&issue_with(&[], State::InProgress)));
        assert!(!sel.matches(&issue_with(&[], State::Ready)));
    }

    #[test]
    fn test_selector_state_single_predicate_from_toml() {
        // A single-string `state` deserializes into a one-element predicate that
        // matches only the named state.
        let toml = r#"
[[rules]]
name = "single"
when = { state = "in_progress" }
assert = { require-section = { heading = "Plan" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let sel = &set.rules[0].when;
        assert!(sel.matches(&issue_with(&[], State::InProgress)));
        assert!(!sel.matches(&issue_with(&[], State::Ready)));
    }

    #[test]
    fn test_selector_state_list_predicate_matches_any_member() {
        // A list `state` matches an issue in any listed state and nothing else.
        let toml = r#"
[[rules]]
name = "list"
when = { state = ["ready", "in_progress", "gated"] }
assert = { require-section = { heading = "Plan" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let sel = &set.rules[0].when;
        assert!(sel.matches(&issue_with(&[], State::Ready)));
        assert!(sel.matches(&issue_with(&[], State::InProgress)));
        assert!(sel.matches(&issue_with(&[], State::Gated)));
        // Not a member of the list.
        assert!(!sel.matches(&issue_with(&[], State::Done)));
        assert!(!sel.matches(&issue_with(&[], State::Backlog)));
    }

    #[test]
    fn test_selector_state_list_combines_with_type() {
        // The state predicate AND-combines with the type selector.
        let toml = r#"
[[rules]]
name = "epic-lifecycle"
when = { type = "epic", state = ["ready", "in_progress"] }
assert = { require-section = { heading = "Plan" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let sel = &set.rules[0].when;
        // type + state both match.
        assert!(sel.matches(&issue_with(&["type:epic"], State::InProgress)));
        // state matches but type does not.
        assert!(!sel.matches(&issue_with(&["type:task"], State::InProgress)));
        // type matches but state does not.
        assert!(!sel.matches(&issue_with(&["type:epic"], State::Done)));
    }

    #[test]
    fn test_selector_state_invalid_name_in_list_is_rejected_at_load() {
        // An unknown state token anywhere in a `when` list is rejected at config
        // load with an error naming the rule, the bad value, and the valid set.
        let toml = r#"
[[rules]]
name = "typo"
when = { state = ["ready", "in_progres"] }
assert = { require-section = { heading = "Plan" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidState { rule, value, valid } => {
                assert_eq!(rule, "typo");
                assert_eq!(value, "in_progres");
                // The valid list names every legal token.
                for token in [
                    "backlog",
                    "ready",
                    "in_progress",
                    "gated",
                    "done",
                    "rejected",
                    "archived",
                ] {
                    assert!(
                        valid.contains(token),
                        "valid list missing '{token}': {valid}"
                    );
                }
            }
            other => panic!("expected InvalidState, got {other:?}"),
        }
    }

    #[test]
    fn test_selector_state_invalid_single_name_is_rejected_at_load() {
        let toml = r#"
[[rules]]
name = "typo-single"
when = { state = "nope" }
assert = { require-section = { heading = "Plan" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidState { rule, value, .. } => {
                assert_eq!(rule, "typo-single");
                assert_eq!(value, "nope");
            }
            other => panic!("expected InvalidState, got {other:?}"),
        }
    }

    #[test]
    fn test_selector_state_empty_list_is_rejected_at_load() {
        // `when = { state = [] }` deserializes to `StatePredicate::List(vec![])`.
        // It must be rejected at load because it silently never matches any issue.
        let toml = r#"
[[rules]]
name = "empty-list"
when = { state = [] }
assert = { require-section = { heading = "Plan" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidState { rule, value, valid } => {
                assert_eq!(rule, "empty-list");
                // The sentinel value must make the problem explicit.
                assert!(
                    value.contains("empty"),
                    "value should indicate empty list, got: {value}"
                );
                // The valid list must name every legal token.
                for token in VALID_STATE_TOKENS {
                    assert!(
                        valid.contains(token),
                        "valid list missing '{token}': {valid}"
                    );
                }
            }
            other => panic!("expected InvalidState, got {other:?}"),
        }
    }

    #[test]
    fn test_label_coverage_invalid_child_state_rejected_at_load() {
        // A typo'd `child-state` in a label-coverage config silently never matches
        // any child, producing spurious "criterion not satisfied" findings.
        // It must be rejected at load, naming the rule, the bad token, and valid states.
        let toml = r#"
[[rules]]
name = "typo-child-state"
when = { type = "epic" }
assert = { label-coverage = { source = "req", child-state = "don" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidState { rule, value, valid } => {
                assert_eq!(rule, "typo-child-state");
                assert_eq!(value, "don");
                // The valid list names every legal token.
                for token in VALID_STATE_TOKENS {
                    assert!(
                        valid.contains(token),
                        "valid list missing '{token}': {valid}"
                    );
                }
            }
            other => panic!("expected InvalidState, got {other:?}"),
        }
    }

    #[test]
    fn test_label_coverage_valid_child_state_accepted_at_load() {
        // A correctly spelled `child-state` must load without error.
        let toml = r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { child-state = "done" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules.len(), 1);
        assert_eq!(set.rules[0].name, "coverage");
    }

    #[test]
    fn test_selector_has_doc_type() {
        let sel = Selector {
            has_doc_type: Some("design".to_string()),
            ..Default::default()
        };
        let mut issue = issue_with(&[], State::Ready);
        let mut doc = DocumentReference::new("docs/d.md".to_string());
        doc.doc_type = Some("design".to_string());
        issue.documents.push(doc);
        assert!(sel.matches(&issue));

        let issue_no_doc = issue_with(&[], State::Ready);
        assert!(!sel.matches(&issue_no_doc));
    }

    #[test]
    fn test_selector_is_and_across_dimensions() {
        let sel = Selector {
            type_: Some("epic".to_string()),
            state: Some(StatePredicate::Single("ready".to_string())),
            ..Default::default()
        };
        // Both dimensions satisfied.
        assert!(sel.matches(&issue_with(&["type:epic"], State::Ready)));
        // Type matches but state does not.
        assert!(!sel.matches(&issue_with(&["type:epic"], State::Backlog)));
        // State matches but type does not.
        assert!(!sel.matches(&issue_with(&["type:task"], State::Ready)));
    }

    #[test]
    fn test_matching_rules_returns_union() {
        let toml = r#"
[[rules]]
name = "epic-rule"
when = { type = "epic" }
assert = { require-label = { label = "req:*", min = 1 } }

[[rules]]
name = "ready-rule"
when = { state = "ready" }
assert = { require-section = { heading = "Success Criteria" } }

[[rules]]
name = "task-rule"
when = { type = "task" }
assert = { require-doc-type = { doc-type = "design" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let issue = issue_with(&["type:epic"], State::Ready);
        let matched: Vec<&str> = set
            .matching_rules(&issue)
            .iter()
            .map(|r| r.name.as_str())
            .collect();
        // epic-rule (type) and ready-rule (state) both match; task-rule does not.
        assert_eq!(matched, vec!["epic-rule", "ready-rule"]);
    }

    // --- Loader: severity / enforce defaults -------------------------------

    #[test]
    fn test_enforce_defaults_to_false() {
        let toml = r#"
[[rules]]
name = "r"
when = { type = "epic" }
assert = { require-section = { heading = "Goals" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules.len(), 1);
        assert!(!set.rules[0].enforce);
    }

    #[test]
    fn test_severity_defaults_to_warn_and_parses_levels() {
        let toml = r#"
[[rules]]
name = "default-sev"
assert = { require-section = { heading = "A" } }

[[rules]]
name = "error-sev"
severity = "error"
enforce = true
assert = { require-section = { heading = "B" } }

[[rules]]
name = "off-sev"
severity = "off"
assert = { require-section = { heading = "C" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules[0].severity, Severity::Warn);
        assert_eq!(set.rules[1].severity, Severity::Error);
        assert!(set.rules[1].enforce);
        assert_eq!(set.rules[2].severity, Severity::Off);
    }

    #[test]
    fn test_load_missing_file_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let set = RuleSet::load(dir.path()).unwrap();
        assert!(set.rules.is_empty());
    }

    #[test]
    fn test_scope_is_derived_from_assertion_kind() {
        let toml = r#"
[[rules]]
name = "local"
assert = { require-label = { label = "type:*" } }

[[rules]]
name = "graph"
assert = { label-coverage = { source = "req", child-state = "done" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules[0].scope, Scope::Local);
        assert_eq!(set.rules[1].scope, Scope::Graph);
    }

    // --- Guards ------------------------------------------------------------

    #[test]
    fn test_shorthand_and_file_schema_is_rejected() {
        let toml = r#"
[[rules]]
name = "mixed"
assert = { require-label = { label = "req:*" }, json-schema = "schemas/x.json" }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "mixed");
                assert!(message.contains("json-schema"));
                assert!(message.contains("shorthand"));
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_checker_command_with_enforce_true_is_rejected() {
        // A checker-command is the validate/gate escape hatch (DR §4.3); it can
        // never block a write, so `enforce = true` on one is a contradiction and
        // must be rejected at LOAD time rather than silently demoted (finding #2).
        let toml = r#"
[[rules]]
name = "escape"
severity = "error"
enforce = true
assert = { checker-command = "scripts/check.sh" }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "escape");
                assert!(
                    message.contains("checker-command") && message.contains("enforce"),
                    "message was: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_checker_command_with_enforce_false_is_ok() {
        // The same rule with enforce = false (the only valid form) must load.
        let toml = r#"
[[rules]]
name = "escape"
severity = "warn"
enforce = false
assert = { checker-command = "scripts/check.sh" }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules.len(), 1);
        assert!(!set.rules[0].enforce);
        assert!(matches!(set.rules[0].assert, Assertion::CheckerCommand(_)));
    }

    #[test]
    fn test_empty_assert_is_rejected() {
        let toml = r#"
[[rules]]
name = "empty"
assert = {}
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { message, .. } => {
                assert!(message.contains("exactly one"));
                assert!(message.contains("none"));
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_multiple_shorthand_kinds_is_rejected() {
        let toml = r#"
[[rules]]
name = "two"
assert = { require-section = { heading = "A" }, require-doc-type = { doc-type = "design" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { message, .. } => {
                assert!(message.contains("exactly one"));
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_inline_raw_schema_object_is_rejected() {
        // An inline schema TABLE under json-schema must NOT deserialize as a
        // schema; json-schema only accepts a string file reference, so an inline
        // object is a parse error (raw JSON Schema may live only in files).
        let toml = r#"
[[rules]]
name = "inline"
assert = { json-schema = { type = "object" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    #[test]
    fn test_duplicate_rule_names_are_rejected() {
        // Two rules sharing a name violate the documented uniqueness invariant
        // that keeps finding attribution unambiguous. The loader rejects them
        // even when their schemas differ.
        let toml = r#"
[[rules]]
name = "dup"
assert = { require-section = { heading = "A" } }

[[rules]]
name = "dup"
assert = { require-doc-type = { doc-type = "design" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::DuplicateRuleName { name } => assert_eq!(name, "dup"),
            other => panic!("expected DuplicateRuleName, got {other:?}"),
        }
    }

    #[test]
    fn test_default_prefix_name_is_now_accepted() {
        // The `default:` reservation was removed (DR §8.2): the default rules now
        // live in the file and are user-editable, so a `default:*` name loads
        // like any other (only the uniqueness guard remains).
        let toml = r#"
[[rules]]
name = "default:my-rule"
assert = { require-section = { heading = "A" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules[0].name, "default:my-rule");
    }

    // --- type-hierarchy assert kind ----------------------------------------

    #[test]
    fn test_type_hierarchy_orphan_leaf_parses_as_graph_rule() {
        let toml = r#"
[[rules]]
name = "default:orphan-leaf"
assert = { type-hierarchy = { kind = "orphan-leaf" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules[0].scope, Scope::Graph);
        assert!(matches!(
            set.rules[0].assert,
            Assertion::TypeHierarchy {
                kind: TypeHierarchyKind::OrphanLeaf
            }
        ));
    }

    #[test]
    fn test_type_hierarchy_strategic_consistency_parses() {
        let toml = r#"
[[rules]]
name = "default:strategic-consistency"
assert = { type-hierarchy = { kind = "strategic-consistency" } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert!(matches!(
            set.rules[0].assert,
            Assertion::TypeHierarchy {
                kind: TypeHierarchyKind::StrategicConsistency
            }
        ));
    }

    #[test]
    fn test_type_hierarchy_unknown_kind_is_rejected() {
        let toml = r#"
[[rules]]
name = "bad"
assert = { type-hierarchy = { kind = "not-a-kind" } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "bad");
                assert!(message.contains("orphan-leaf"));
                assert!(message.contains("strategic-consistency"));
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_type_hierarchy_unknown_field_is_rejected() {
        // `deny_unknown_fields` on RawTypeHierarchy catches a stray key.
        let toml = r#"
[[rules]]
name = "bad"
assert = { type-hierarchy = { kind = "orphan-leaf", extra = 1 } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    // --- gate-recency assert kind ------------------------------------------

    #[test]
    fn test_gate_recency_max_age_days_parses_as_graph_rule() {
        let toml = r#"
[[rules]]
name = "fresh-evidence"
when = { state = "done" }
severity = "error"
enforce = true
assert = { gate-recency = { max-age-days = 7, gates = ["code-review"] } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        assert_eq!(set.rules[0].scope, Scope::Graph);
        match &set.rules[0].assert {
            Assertion::GateRecency {
                max_age_hours,
                gates,
            } => {
                assert_eq!(*max_age_hours, 7 * 24);
                assert_eq!(gates, &["code-review".to_string()]);
            }
            other => panic!("expected GateRecency, got {other:?}"),
        }
    }

    #[test]
    fn test_gate_recency_max_age_hours_parses() {
        let toml = r#"
[[rules]]
name = "fresh"
assert = { gate-recency = { max-age-hours = 12 } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        match &set.rules[0].assert {
            Assertion::GateRecency {
                max_age_hours,
                gates,
            } => {
                assert_eq!(*max_age_hours, 12);
                // `gates` defaults to empty (= all of the issue's gates_required).
                assert!(gates.is_empty());
            }
            other => panic!("expected GateRecency, got {other:?}"),
        }
    }

    #[test]
    fn test_gate_recency_both_ages_is_rejected() {
        let toml = r#"
[[rules]]
name = "ambiguous"
assert = { gate-recency = { max-age-days = 1, max-age-hours = 1 } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "ambiguous");
                assert!(message.contains("not both"), "message was: {message}");
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_gate_recency_missing_age_is_rejected() {
        let toml = r#"
[[rules]]
name = "no-age"
assert = { gate-recency = { gates = ["code-review"] } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "no-age");
                assert!(
                    message.contains("max-age-days") && message.contains("max-age-hours"),
                    "message was: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_gate_recency_zero_age_is_rejected() {
        let toml = r#"
[[rules]]
name = "zero"
assert = { gate-recency = { max-age-days = 0 } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { message, .. } => {
                assert!(
                    message.contains("greater than zero"),
                    "message was: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_gate_recency_overflowing_age_is_rejected() {
        // A huge-but-parseable age must be rejected at load: a `max-age-days`
        // whose hour normalization exceeds i64::MAX would otherwise wrap into a
        // negative threshold in the evaluator (which compares chrono's i64 hour
        // counts) and report every fresh result as stale. TOML integers are
        // i64, so the days form is the only way such an age can parse.
        let toml = r#"
[[rules]]
name = "huge"
assert = { gate-recency = { max-age-days = 384307168202282326 } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { message, .. } => {
                assert!(message.contains("too large"), "message was: {message}");
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_gate_recency_unknown_field_is_rejected() {
        let toml = r#"
[[rules]]
name = "bad"
assert = { gate-recency = { max-age-days = 7, extra = 1 } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    #[test]
    fn test_unknown_assertion_kind_is_rejected() {
        let toml = r#"
[[rules]]
name = "bogus"
assert = { not-a-real-kind = { x = 1 } }
"#;
        let err = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    // --- File-schema loading -----------------------------------------------

    #[test]
    fn test_json_schema_file_is_loaded_and_transcoded() {
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(
            schemas.join("epic-body.json"),
            r#"{ "type": "object", "required": ["sections"] }"#,
        )
        .unwrap();

        let toml = r#"
[[rules]]
name = "epic-body"
when = { type = "epic" }
assert = { json-schema = "schemas/epic-body.json" }
"#;
        let set = RuleSet::from_toml_str(toml, dir.path()).unwrap();
        match &set.rules[0].assert {
            Assertion::JsonSchema(src) => {
                assert_eq!(src.reference, "schemas/epic-body.json");
                assert_eq!(src.schema["type"], serde_json::json!("object"));
                assert_eq!(src.schema["required"], serde_json::json!(["sections"]));
            }
            other => panic!("expected JsonSchema, got {other:?}"),
        }
    }

    #[test]
    fn test_missing_json_schema_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let toml = r#"
[[rules]]
name = "epic-body"
assert = { json-schema = "schemas/does-not-exist.json" }
"#;
        let err = RuleSet::from_toml_str(toml, dir.path()).unwrap_err();
        assert!(matches!(err, RuleConfigError::SchemaIo { .. }));
    }

    #[test]
    fn test_invalid_json_schema_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(schemas.join("bad.json"), "{ not valid json").unwrap();
        let toml = r#"
[[rules]]
name = "epic-body"
assert = { json-schema = "schemas/bad.json" }
"#;
        let err = RuleSet::from_toml_str(toml, dir.path()).unwrap_err();
        assert!(matches!(err, RuleConfigError::SchemaJson { .. }));
    }

    // --- Schema reference safety -------------------------------------------

    fn schema_rule_toml(reference: &str) -> String {
        format!("[[rules]]\nname = \"epic-body\"\nassert = {{ json-schema = \"{reference}\" }}\n")
    }

    #[test]
    fn test_schema_reference_absolute_path_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let toml = schema_rule_toml("/etc/passwd.json");
        let err = RuleSet::from_toml_str(&toml, dir.path()).unwrap_err();
        match err {
            RuleConfigError::InvalidSchemaReference {
                rule, reference, ..
            } => {
                assert_eq!(rule, "epic-body");
                assert_eq!(reference, "/etc/passwd.json");
            }
            other => panic!("expected InvalidSchemaReference, got {other:?}"),
        }
    }

    #[test]
    fn test_schema_reference_parent_traversal_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let toml = schema_rule_toml("schemas/../escape.json");
        let err = RuleSet::from_toml_str(&toml, dir.path()).unwrap_err();
        assert!(matches!(
            err,
            RuleConfigError::InvalidSchemaReference { .. }
        ));
    }

    #[test]
    fn test_schema_reference_not_under_schemas_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let toml = schema_rule_toml("other/x.json");
        let err = RuleSet::from_toml_str(&toml, dir.path()).unwrap_err();
        match err {
            RuleConfigError::InvalidSchemaReference { message, .. } => {
                assert!(message.contains("schemas/"));
            }
            other => panic!("expected InvalidSchemaReference, got {other:?}"),
        }
    }

    #[test]
    fn test_schema_reference_non_json_extension_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let toml = schema_rule_toml("schemas/x.txt");
        let err = RuleSet::from_toml_str(&toml, dir.path()).unwrap_err();
        match err {
            RuleConfigError::InvalidSchemaReference { message, .. } => {
                assert!(message.contains(".json"));
            }
            other => panic!("expected InvalidSchemaReference, got {other:?}"),
        }
    }

    #[test]
    fn test_schema_reference_valid_under_schemas_is_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(schemas.join("x.json"), r#"{ "type": "object" }"#).unwrap();

        let toml = schema_rule_toml("schemas/x.json");
        let set = RuleSet::from_toml_str(&toml, dir.path()).unwrap();
        match &set.rules[0].assert {
            Assertion::JsonSchema(src) => assert_eq!(src.reference, "schemas/x.json"),
            other => panic!("expected JsonSchema, got {other:?}"),
        }
    }

    // --- Regex round-trip (TOML literal string -> serde_json) --------------

    #[test]
    fn test_regex_literal_string_round_trips_to_serde_json() {
        // A regex-bearing shorthand authored as a TOML literal string ('...')
        // must round-trip through serde_json intact, including backslashes.
        let toml = r#"
[[rules]]
name = "req-format"
when = { label = "req:*" }
assert = { label-value-pattern = { namespace = "req", regex = '^REQ-[0-9]+$' } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let regex = match &set.rules[0].assert {
            Assertion::LabelValuePattern { namespace, regex } => {
                assert_eq!(namespace, "req");
                regex.clone()
            }
            other => panic!("expected LabelValuePattern, got {other:?}"),
        };
        assert_eq!(regex, "^REQ-[0-9]+$");

        // Round-trip through serde_json: the regex survives JSON encode/decode.
        let json = serde_json::to_value(&regex).unwrap();
        assert_eq!(json, serde_json::json!("^REQ-[0-9]+$"));
        let back: String = serde_json::from_value(json).unwrap();
        assert_eq!(back, "^REQ-[0-9]+$");
    }

    #[test]
    fn test_regex_with_backslashes_round_trips() {
        // Backslash-heavy regex authored as a TOML literal string survives the
        // transcode to serde_json without mangling.
        let toml = r#"
[[rules]]
name = "marker"
assert = { label-value-pattern = { namespace = "sc", regex = '^\[hard\]\s+\w+' } }
"#;
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        let regex = match &set.rules[0].assert {
            Assertion::LabelValuePattern { regex, .. } => regex.clone(),
            other => panic!("expected LabelValuePattern, got {other:?}"),
        };
        assert_eq!(regex, r"^\[hard\]\s+\w+");
        let json = serde_json::to_value(&regex).unwrap();
        assert_eq!(json, serde_json::json!(r"^\[hard\]\s+\w+"));
    }
}
