//! Pure rule declarations and `.jit/rules.toml` parsing.
//!
//! This module defines the declarative validation rule model and pure parsing
//! of already-captured `rules.toml`, configuration, and schema bytes into a
//! [`RuleSet`]. The captured declaration parse supplies those bytes from one
//! repository image; validation only evaluates the parsed declarations. This
//! module also implements
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
//! This module only defines the model, pure parser, selector matching, and the
//! guards. Actual JSON Schema validation, shorthand desugaring, and graph rule
//! evaluation are filled in by downstream tasks; assertion payloads are parsed
//! and stored here without being evaluated.

use crate::domain::identity::GateKey;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::config::JitConfig;
use crate::domain::id_pattern::IdPattern;
use crate::domain::vocabulary::wire_vocabulary;
use crate::domain::{Issue, State};
use crate::labels as label_utils;
use crate::repository_state::{RepositoryLayoutError, VirtualPath};

/// Default regex extracting a criterion id from an item's text (e.g. `REQ-01`).
pub const DEFAULT_CRITERION_ID_PATTERN: &str = "[A-Z][A-Z0-9]*-[0-9]+";

/// Errors that can occur while loading and parsing `.jit/rules.toml`.
#[derive(Debug, Error)]
pub enum RuleConfigError {
    /// A failure derived once and shared by every reader of that derivation.
    #[error(transparent)]
    Shared(Arc<RuleConfigError>),

    /// The repository layout could not resolve a rules or schema path.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),

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

    /// Pure parsing required schema bytes that were not supplied.
    #[error("rule '{rule}': schema bytes for '{reference}' were not supplied")]
    MissingSchema {
        /// Name of the offending rule.
        rule: String,
        /// Safe schema reference requested by the declaration.
        reference: String,
    },

    /// Kind-based coverage expansion requires an explicit parsed configuration.
    #[error("rule '{rule}': explicit JitConfig is required for {purpose}")]
    MissingConfiguration {
        /// Name of the offending rule.
        rule: String,
        /// Pure expansion that required configuration.
        purpose: &'static str,
    },

    /// The boundary could not supply a valid explicit repository configuration.
    #[error("failed to load explicit rule configuration: {message}")]
    Configuration {
        /// Boundary configuration failure.
        message: String,
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

    /// A `when` selector names a lifecycle state that is not one of the
    /// canonical state tokens. Caught at config load so a typo'd state
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

    /// A rule's `name` contains a `:`. Colon is reserved exclusively for the
    /// label `namespace:value` separator (see [`Rule::name`]); a rule's
    /// self-id must be a colon-free slug, with provenance carried in the
    /// separate `origin` field rather than an embedded prefix.
    #[error(
        "rule name '{name}' contains ':'; colon is reserved for the label \
         namespace:value separator — put provenance in the 'origin' field instead"
    )]
    InvalidRuleName {
        /// The offending name as authored.
        name: String,
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

    /// A custom rule occupies a registry-derived default rule's address.
    #[error(
        "custom rule '{name}' conflicts with a derived default rule; \
         set severity = \"off\" on the default row (origin = \"default\") instead"
    )]
    DefaultRuleNameCollision {
        /// The reserved default rule name.
        name: String,
    },
}

wire_vocabulary! {
    /// Severity of a rule finding.
    ///
    /// Each variant's doc states its effect; only [`Severity::Error`] (with
    /// `enforce` set) can block a write.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize, schemars::JsonSchema,
    )]
    pub enum Severity {
        /// Rule is disabled.
        Off = "off",
        /// Rule reports a warning but never blocks.
        #[default]
        Warn = "warn",
        /// Rule reports an error (blocks writes when `enforce` is true).
        Error = "error",
    }
}

wire_vocabulary! {
    /// Evaluation scope of a rule, derived from its assertion kind.
    ///
    /// `Local` rules are pure predicates over a single projected issue and run
    /// on write. `Graph` rules need cross-issue context and run in `jit validate`,
    /// gate checkers, and at state transitions (where an enforcing failure
    /// blocks the transition); they never run on plain field writes.
    #[derive(
        Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema,
    )]
    pub enum RuleScope {
        /// Per-issue, runs on write.
        Local = "local",
        /// Aggregate/graph, runs only on demand.
        Graph = "graph",
    }
}

/// Selector matching the issue dimensions a rule applies to.
///
/// All present dimensions are AND-combined. Matching reads only cheap `Issue`
/// fields (labels, state) and never parses the description. The `label`
/// dimension supports `ns:*` wildcards via [`label_utils::matches_pattern`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
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
    #[serde(default)]
    pub has_doc_type: Option<String>,
}

impl Selector {
    /// Returns whether this selector matches the given issue.
    ///
    /// Matching is AND across all present dimensions. An empty selector (no
    /// dimensions) matches every issue. Only cheap fields are read; the
    /// description is never parsed.
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
    pub fn match_failure(&self, issue: &Issue) -> Option<String> {
        let mut reasons: Vec<String> = Vec::new();
        if !self.matches_type(issue) {
            let want = self.type_.as_deref().unwrap_or("");
            let want_label = label_utils::type_label(want);
            reasons.push(format!(
                "type predicate did not match (issue is not '{want_label}')"
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
                issue.state.as_str(),
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
            Some(ty) => issue
                .labels
                .iter()
                .any(|l| label_utils::type_value_of(l) == Some(ty.as_str())),
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

/// Comma-separated canonical state tokens, for [`RuleConfigError::InvalidState`].
fn valid_state_tokens() -> String {
    State::ALL.map(State::as_str).join(", ")
}

/// A `when` state predicate: one or more lifecycle states a rule applies to.
///
/// Authored in TOML as either a single string or a list of strings, both of
/// which deserialize into the same set-of-states predicate:
///
/// ```toml
/// when = { state = "in_progress" }     # single
/// when = { state = ["in_progress"] }   # list
/// ```
///
/// The list form may hold any number of states.
///
/// Matching is membership: [`StatePredicate::matches`] is true when the issue's
/// state is one of the predicate's states. The authored tokens are parsed once,
/// when the predicate is built, into a typed [`BTreeSet<State>`] (see
/// [`State::from_str`](std::str::FromStr)); `matches` is then a direct set lookup with no per-call
/// string round-trip. The authored strings are retained for diagnostics and
/// validated against the canonical tokens at config load via
/// `StatePredicate::validate`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(from = "AuthoredStates")]
pub struct StatePredicate {
    /// Authored state tokens, in authored order, retained for diagnostics and
    /// load-time validation.
    authored: Vec<String>,
    /// The recognized lifecycle states, parsed once from `authored` via the
    /// canonical state parser. Unrecognized tokens are omitted here and reported by
    /// [`StatePredicate::validate`].
    states: BTreeSet<State>,
}

/// Untagged deserialization shape for a `state` predicate: a single token or a
/// list of tokens. Converted into [`StatePredicate`] (which parses the typed
/// state set) via `#[serde(from = ...)]`.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum AuthoredStates {
    Single(String),
    List(Vec<String>),
}

impl From<AuthoredStates> for StatePredicate {
    fn from(authored: AuthoredStates) -> Self {
        let authored = match authored {
            AuthoredStates::Single(s) => vec![s],
            AuthoredStates::List(v) => v,
        };
        StatePredicate::from_tokens(authored)
    }
}

impl StatePredicate {
    /// Build a predicate from authored tokens, parsing the typed state set once.
    ///
    /// Tokens that name a valid lifecycle state populate the set; any
    /// unrecognized tokens are retained in `authored` (for diagnostics) but
    /// omitted from the set, where [`StatePredicate::validate`] reports them.
    fn from_tokens(authored: Vec<String>) -> Self {
        let states = authored.iter().filter_map(|t| t.parse().ok()).collect();
        StatePredicate { authored, states }
    }

    /// Build a predicate from a single authored token.
    pub fn single(token: impl Into<String>) -> Self {
        StatePredicate::from_tokens(vec![token.into()])
    }

    /// Build a predicate from a list of authored tokens.
    pub fn list<I, S>(tokens: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        StatePredicate::from_tokens(tokens.into_iter().map(Into::into).collect())
    }

    /// The authored state tokens this predicate carries, in authored order.
    pub fn tokens(&self) -> &[String] {
        &self.authored
    }

    /// Returns whether the given lifecycle state is in this predicate's set.
    ///
    /// A direct membership check against the pre-parsed [`BTreeSet<State>`]; no
    /// per-call string conversion happens.
    pub fn matches(&self, state: State) -> bool {
        self.states.contains(&state)
    }

    /// Validate that this predicate is non-empty and every token names a
    /// canonical lifecycle state.
    ///
    /// An empty list (`state = []`) is rejected because it silently never
    /// matches any issue — a rule scoped to zero states is always a config
    /// error, not a useful no-op.
    ///
    /// Returns [`RuleConfigError::InvalidState`] naming the first unrecognized
    /// token (or the empty-list sentinel `"<empty list>"`) together with the
    /// owning rule and the list of valid tokens.
    fn validate(&self, rule: &str) -> Result<(), RuleConfigError> {
        if self.authored.is_empty() {
            return Err(RuleConfigError::InvalidState {
                rule: rule.to_string(),
                value: "<empty list>".to_string(),
                valid: valid_state_tokens(),
            });
        }
        for token in &self.authored {
            if token.parse::<State>().is_err() {
                return Err(RuleConfigError::InvalidState {
                    rule: rule.to_string(),
                    value: token.clone(),
                    valid: valid_state_tokens(),
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
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaSource {
    /// The reference string as authored in `rules.toml` (e.g.
    /// `"schemas/epic-body.json"`).
    pub reference: String,
    /// A diagnostic-only path identifying the schema, formed by joining the
    /// reference onto a synthetic `<captured>` base. The schema bytes come from
    /// the captured (or storage-loaded) schema set, so this path is never re-read
    /// and is not a live `.jit`-root filesystem path.
    pub path: PathBuf,
    /// The parsed JSON Schema document.
    pub schema: serde_json::Value,
}

/// One schema file required by captured rule declarations.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SchemaRequest {
    /// Owning rule name for typed boundary errors.
    pub rule: String,
    /// Safe repository-data-relative schema reference.
    pub reference: String,
    /// Whether a missing/malformed source is fatal at the boundary.
    pub required: bool,
}

/// A single assertion kind. Exactly one kind is present per rule.
///
/// Shorthand kinds carry simple scalars and desugar to JSON Schema downstream.
/// [`Assertion::JsonSchema`] carries a raw schema loaded from a file. Graph
/// kinds run only on demand. Payloads are parsed/stored but not yet evaluated.
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
        /// The coverage rule's configuration, decoded at load.
        config: LabelCoverageConfig,
    },
    /// A from-reference resolves to a declared source. Graph scope.
    LabelReference {
        /// The reference rule's configuration, decoded at load.
        config: LabelReferenceConfig,
    },
    /// A selector must/should depend on a target selector. Graph scope.
    DependencyShape {
        /// The shape rule's configuration, decoded at load.
        config: DependencyShapeConfig,
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
        gates: Vec<GateKey>,
    },
    /// A type-hierarchy warning, one [`TypeHierarchyKind`] check.
    /// Graph scope. Authorable in `rules.toml` via the `type-hierarchy` assert
    /// kind; the workflow profile contributes the standard instances.
    /// Evaluation reuses the existing [`crate::domain::type_taxonomy`] domain functions
    /// rather than reimplementing the hierarchy logic; the repo's
    /// [`crate::domain::type_taxonomy::HierarchyConfig`] is NOT stored in the parsed rule — it is injected by
    /// the graph evaluator at evaluation time (see
    /// [`evaluate_graph`](crate::validation::graph::evaluate_graph)).
    TypeHierarchy {
        /// Which legacy hierarchy check this rule performs.
        kind: TypeHierarchyKind,
    },
    /// Compare a label namespace's values against criterion ids extracted from a
    /// configured section of the issue body. Graph scope (per-issue; the body
    /// projection is needed but no other issues are). CC-3.
    ///
    /// A `namespace:<value>` label whose `<value>` is NOT in the extracted
    /// criterion-id set yields a stray-label finding, distinct from the
    /// declared-but-unsatisfied finding that `label-coverage` / `label-reference`
    /// emit for uncovered criteria or dangling references.
    CriteriaLabelMatch {
        /// Label namespace whose values are checked (e.g. `"req"`). Required.
        namespace: String,
        /// Projection slug of the section holding the criteria (optional;
        /// defaults to `"success_criteria"`).
        criteria_section: String,
        /// When set, only items whose text begins with this marker count.
        /// Items without the marker are not present in the extracted id set,
        /// so a label matching an unmarked id is a stray. Optional.
        marker: Option<String>,
        /// Extracts a criterion id from an item's text. Optional; defaults to
        /// [`DEFAULT_CRITERION_ID_PATTERN`].
        id_pattern: IdPattern,
    },
    /// Every criterion in a configured section maps to a check label.
    /// Graph scope. A criterion is checked when the issue carries a label
    /// `<check-namespace>:<own-short-id>/<id>` naming its own criterion.
    /// Criteria and marker filtering reuse the same `criterion_ids` extractor
    /// as `label-coverage`.
    ///
    /// # Config shape in `rules.toml`
    ///
    /// ```toml
    /// assert = { criteria-to-check = {
    ///   criteria-section = "success_criteria",  # optional, default success_criteria
    ///   marker = "[hard]",                       # optional: only marked items
    ///   id-pattern = "REQ-[0-9]+",               # optional, default DEFAULT_CRITERION_ID_PATTERN
    ///   check-namespace = "checks",              # required
    /// } }
    /// ```
    CriteriaToCheck {
        /// Projection slug of the section holding criteria (default
        /// `"success_criteria"`).
        criteria_section: String,
        /// Optional marker filter; when set, only items starting with this
        /// marker contribute criterion ids.
        marker: Option<String>,
        /// Extracts the criterion id from an item's text (default
        /// [`DEFAULT_CRITERION_ID_PATTERN`]).
        id_pattern: IdPattern,
        /// Required label namespace. A criterion `<id>` is checked when the
        /// issue carries `"<check_namespace>:<own-short-id>/<id>"`.
        check_namespace: String,
    },
    /// Assert that each value in a label namespace is owned by at most one issue
    /// across the entire repository. Graph scope, repo-wide.
    ///
    /// Config shape:
    /// ```toml
    /// assert = { label-uniqueness = { namespace = "req", scope = "all" } }
    /// ```
    ///
    /// Semantics: group all issues matching the rule's `when` selector by each
    /// `namespace:<value>` label they carry; any value declared by two or more
    /// issues yields one finding naming the value and the colliding short-ids.
    ///
    /// `scope = "all"` is required for this kind. Repo-wide rules run only in
    /// `jit validate` (not at transition time), because transition enforcement
    /// evaluates only the issue's dependency neighborhood.
    LabelUniqueness {
        /// The label namespace whose values must be unique across all matching
        /// issues (e.g. `"req"`).
        namespace: String,
    },
}

wire_vocabulary! {
    /// The legacy type-hierarchy warning expressed by an
    /// [`Assertion::TypeHierarchy`].
    ///
    /// Each variant reuses one existing domain function over the whole issue set:
    /// `OrphanLeaf` -> [`crate::domain::type_taxonomy::validate_orphans`],
    /// `StrategicConsistency` -> [`crate::domain::type_taxonomy::validate_strategic_labels`].
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum TypeHierarchyKind {
        /// A leaf-level issue (e.g. a task) carries no parent association label.
        OrphanLeaf = "orphan-leaf",
        /// A strategic-type issue (e.g. an epic) is missing its identifying label.
        StrategicConsistency = "strategic-consistency",
    }
}

/// The `label-coverage` payload after kind sugar expansion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelCoverageConfig {
    /// Authored criteria section; defaults through [`Self::criteria_section`].
    pub authored_criteria_section: Option<String>,
    /// Only criteria beginning with this marker require coverage.
    pub marker: Option<String>,
    /// Authored pattern, retained so serialization preserves its presence.
    pub authored_id_pattern: Option<IdPattern>,
    default_id_pattern: IdPattern,
    /// Authored credit namespace; defaults through [`Self::satisfies_namespace`].
    pub authored_satisfies_namespace: Option<String>,
    /// A satisfying child must be in this state, when specified.
    pub child_state: Option<State>,
    /// Authored relationship; defaults through [`Self::child_link`].
    pub authored_child_link: Option<ChildLink>,
    /// Excluded types are also traversal boundaries. Authored order is preserved.
    pub child_type_exclude: Option<Vec<String>>,
    /// Label namespace naming the criteria-bearing container.
    pub container_from_label: Option<String>,
}

impl LabelCoverageConfig {
    /// The authored criteria section, or `success_criteria`.
    pub fn criteria_section(&self) -> &str {
        self.authored_criteria_section
            .as_deref()
            .unwrap_or("success_criteria")
    }

    /// The compiled authored pattern, or the default criterion pattern.
    pub fn id_pattern(&self) -> &IdPattern {
        self.authored_id_pattern
            .as_ref()
            .unwrap_or(&self.default_id_pattern)
    }

    /// The authored credit namespace, or `satisfies`.
    pub fn satisfies_namespace(&self) -> &str {
        self.authored_satisfies_namespace
            .as_deref()
            .unwrap_or("satisfies")
    }

    /// The authored relationship, or dependents.
    pub fn child_link(&self) -> ChildLink {
        self.authored_child_link.unwrap_or_default()
    }
}

wire_vocabulary! {
    /// How a coverage child relates to its container.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub enum ChildLink {
        /// Issues depending on the container, transitively.
        #[default]
        Dependents = "dependents",
        /// Issues the container depends on, transitively.
        Dependencies = "dependencies",
        /// Any other issue, regardless of dependency edges.
        Any = "any",
    }
}

/// The `label-reference` assert payload.
///
/// Every `from`-namespace label value on a matching issue must be declared as a
/// `to`-namespace label value within `scope`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelReferenceConfig {
    /// Namespace of the reference labels (e.g. `"satisfies"`).
    pub from: String,
    /// Namespace that declares the valid sources (e.g. `"req"`).
    pub to: String,
    /// The authored `scope`, kept as written so serialization emits the
    /// authored keys; read the effective scope through [`Self::scope`].
    pub authored_scope: Option<LabelReferenceScope>,
}

impl LabelReferenceConfig {
    /// Where a declaration may live: the authored scope, or
    /// [`LabelReferenceScope::Global`] when none was authored.
    pub fn scope(&self) -> LabelReferenceScope {
        self.authored_scope.unwrap_or_default()
    }
}

/// The `dependency-shape` assert payload.
///
/// Every issue matching the rule's selector must (or should) depend on an issue
/// matching `target`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DependencyShapeConfig {
    /// The issues the source must depend on; its state tokens are validated at load.
    pub target: Selector,
    /// The authored `mode`, kept as written so serialization emits the authored
    /// keys; read the effective mode through [`Self::mode`].
    pub authored_mode: Option<DependencyShapeMode>,
    /// The authored `transitive`, kept as written; read the effective value
    /// through [`Self::is_transitive`].
    pub authored_transitive: Option<bool>,
}

impl DependencyShapeConfig {
    /// The authored mode, or [`DependencyShapeMode::Must`] when none was authored.
    pub fn mode(&self) -> DependencyShapeMode {
        self.authored_mode.unwrap_or_default()
    }

    /// Whether a transitive dependency satisfies the rule; a direct dependency
    /// is required when none was authored.
    pub fn is_transitive(&self) -> bool {
        self.authored_transitive.unwrap_or(false)
    }
}

wire_vocabulary! {
    /// The `mode` of a `dependency-shape` assert. It shapes the finding message
    /// only; the blocking weight comes from the rule's severity.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub enum DependencyShapeMode {
        /// The dependency is required.
        #[default]
        Must = "must",
        /// The dependency is expected.
        Should = "should",
    }
}

wire_vocabulary! {
    /// The `scope` of a `label-reference` assert.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
    pub enum LabelReferenceScope {
        /// Any issue in the evaluated set.
        #[default]
        Global = "global",
        /// Only issues linked to the referencing issue by a dependency edge in
        /// either direction.
        Linked = "linked",
    }
}

wire_vocabulary! {
    /// The `scope` of a `label-uniqueness` assert. Uniqueness is inherently
    /// repo-wide, so `all` is the only scope.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum LabelUniquenessScope {
        /// Every issue matching the rule's selector.
        All = "all",
    }
}

impl Assertion {
    /// The evaluation scope implied by this assertion kind.
    ///
    /// Shorthand and file-schema kinds are [`RuleScope::Local`]; the aggregate graph
    /// kinds are [`RuleScope::Graph`].
    pub fn scope(&self) -> RuleScope {
        match self {
            Assertion::LabelCoverage { .. }
            | Assertion::LabelReference { .. }
            | Assertion::DependencyShape { .. }
            | Assertion::GateRecency { .. }
            | Assertion::TypeHierarchy { .. }
            | Assertion::CriteriaLabelMatch { .. }
            | Assertion::LabelUniqueness { .. } => RuleScope::Graph,
            Assertion::CriteriaToCheck { .. } => RuleScope::Graph,
            _ => RuleScope::Local,
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
    /// `type-hierarchy` checks ([`TypeHierarchyKind`]) reason about
    /// each issue against the WHOLE set's parent/child relations, so they are
    /// likewise treated as repo-wide and validate-only. The per-issue graph kinds
    /// (`label-coverage`, `dependency-shape`, `gate-recency`) only need the
    /// issue's neighborhood and so run at transition time.
    pub fn is_repo_wide_at_transition(&self) -> bool {
        match self {
            Assertion::LabelUniqueness { .. } => true,
            // A label-reference rule is repo-wide unless explicitly scoped to
            // linked issues (which the neighborhood slice fully contains).
            Assertion::LabelReference { config } => config.scope() != LabelReferenceScope::Linked,
            // Type-hierarchy checks reason over the whole set's relations.
            Assertion::TypeHierarchy { .. } => true,
            // The remaining graph kinds are per-issue / neighborhood-local.
            // `CriteriaLabelMatch` only needs the issue's own body projection
            // and labels, so it is safe at transition time.
            _ => false,
        }
    }
}

/// The provenance marker the FIXED built-in default rule set carries in
/// [`Rule::origin`]. Default-origin rules are a config projection: their
/// assertion (and the family's membership) is reconciled from the
/// declared registry at load, so their on-disk schema files are never the
/// validation authority.
pub(crate) const DEFAULT_ORIGIN: &str = "default";

/// A fully parsed validation rule.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Unique, human-readable rule name. Colon-free: `:` is reserved solely for
    /// the label `namespace:value` separator, so a rule's provenance lives in
    /// [`origin`](Rule::origin) rather than an embedded prefix.
    pub name: String,
    /// Provenance marker, e.g. `"default"` for the FIXED built-in rule set
    /// ([`default_ruleset`](crate::repository_state::default_ruleset)) or
    /// `"bracket"` for the planning-bracket coverage-preview rule. `None` for a
    /// plain repo-authored rule.
    pub origin: Option<String>,
    /// Optional free-text explanation of what the rule enforces and why. A
    /// rule's [`name`](Rule::name) is its only mandatory human-readable content;
    /// this carries the richer prose that consumers (the rules/gates reference
    /// projection, `jit item show`) render, falling back to the name when
    /// absent. Existing description-less rules load unchanged.
    pub description: Option<String>,
    /// Selector deciding which issues the rule applies to.
    pub when: Selector,
    /// Reporting severity.
    pub severity: Severity,
    /// Whether an `error` finding blocks writes (default false).
    pub enforce: bool,
    /// The assertion to evaluate.
    pub assert: Assertion,
    /// Evaluation scope, derived from the assertion kind.
    pub scope: RuleScope,
}

/// A parsed set of rules, ready for selector matching.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RuleSet {
    /// The rules in authored order.
    pub rules: Vec<Rule>,
}

impl RuleSet {
    /// An empty rule set (used when no `rules.toml` exists).
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parse captured rules using explicit configuration and schema bytes.
    pub fn parse(
        content: &str,
        config: Option<&JitConfig>,
        schemas: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<Self, RuleConfigError> {
        RulesDocument::parse(content).rule_set(config, schemas)
    }

    /// Discover safe schema-byte requests without reading them.
    pub fn schema_requests(content: &str) -> Result<Vec<SchemaRequest>, RuleConfigError> {
        RulesDocument::parse(content)
            .schema_requests()
            .map(<[SchemaRequest]>::to_vec)
    }

    /// Returns the union of rules whose selector matches the issue.
    ///
    /// Only cheap `Issue` fields are read (no description parse). Rules are
    /// returned in authored order.
    pub fn matching_rules(&self, issue: &Issue) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| rule.when.matches(issue))
            .collect()
    }
}

/// One syntax parse of `rules.toml`, from which its schema requests and its
/// rule set are both derived.
pub(crate) struct RulesDocument {
    raw: Result<RawRulesFile, toml::de::Error>,
    /// The schema requests of `raw`, derived on first use.
    schema_requests: std::sync::OnceLock<Result<Vec<SchemaRequest>, Arc<RuleConfigError>>>,
}

impl RulesDocument {
    pub(crate) fn parse(content: &str) -> Self {
        #[cfg(test)]
        super::count_parse(super::ParsedSource::Rules);
        Self {
            raw: toml::from_str(content),
            schema_requests: std::sync::OnceLock::new(),
        }
    }

    fn raw(&self) -> Result<&RawRulesFile, RuleConfigError> {
        self.raw.as_ref().map_err(|error| error.clone().into())
    }

    /// The rule set over explicit configuration and schema bytes.
    pub(crate) fn rule_set(
        &self,
        config: Option<&JitConfig>,
        schemas: impl IntoIterator<Item = (String, Vec<u8>)>,
    ) -> Result<RuleSet, RuleConfigError> {
        let schemas = schemas.into_iter().collect::<BTreeMap<_, _>>();
        let rules = self
            .raw()?
            .rules
            .iter()
            .cloned()
            .map(|r| r.into_rule(config, &schemas))
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

        Ok(RuleSet { rules })
    }

    /// Safe schema-byte requests, without reading them, derived once per
    /// parse with their failure.
    pub(crate) fn schema_requests(&self) -> Result<&[SchemaRequest], RuleConfigError> {
        self.schema_requests
            .get_or_init(|| {
                #[cfg(test)]
                super::count_parse(super::ParsedSource::SchemaRequests);
                self.derive_schema_requests().map_err(Arc::new)
            })
            .as_deref()
            .map_err(|error| RuleConfigError::Shared(Arc::clone(error)))
    }

    fn derive_schema_requests(&self) -> Result<Vec<SchemaRequest>, RuleConfigError> {
        self.raw()?
            .rules
            .iter()
            .filter_map(|rule| {
                rule.assert.json_schema.as_ref().map(|reference| {
                    validate_schema_reference(&rule.name, reference)?;
                    Ok(SchemaRequest {
                        rule: rule.name.clone(),
                        reference: reference.clone(),
                        required: rule.origin.as_deref() != Some(DEFAULT_ORIGIN),
                    })
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Raw deserialization layer
// ---------------------------------------------------------------------------

/// Top-level `rules.toml` document.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRulesFile {
    #[serde(default, rename = "rules")]
    rules: Vec<RawRule>,
}

/// A rule as authored, before validation of the `assert` table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    name: String,
    /// Provenance marker (see [`Rule::origin`]); absent for a plain
    /// repo-authored rule.
    #[serde(default)]
    origin: Option<String>,
    /// Optional free-text explanation (see [`Rule::description`]); absent field
    /// falls back to the name in every consumer, so existing repos stay valid.
    #[serde(default)]
    description: Option<String>,
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
#[derive(Debug, Clone, Default, Deserialize)]
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
    #[serde(default, rename = "criteria-label-match")]
    criteria_label_match: Option<RawCriteriaLabelMatch>,
    #[serde(default, rename = "criteria-to-check")]
    criteria_to_check: Option<toml::value::Table>,
    #[serde(default, rename = "label-uniqueness")]
    label_uniqueness: Option<RawLabelUniqueness>,
}

/// The `gate-recency` assert payload: a max age plus an optional gate filter.
///
/// Exactly one of `max-age-days` / `max-age-hours` MUST be set; both-or-neither
/// is a config error (caught in [`RawGateRecency::into_assertion`]).
/// `deny_unknown_fields` rejects a stray key so a typo surfaces at load.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawGateRecency {
    #[serde(default, rename = "max-age-days")]
    max_age_days: Option<u64>,
    #[serde(default, rename = "max-age-hours")]
    max_age_hours: Option<u64>,
    #[serde(default)]
    gates: Option<Vec<GateKey>>,
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
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTypeHierarchy {
    kind: String,
}

/// The `criteria-label-match` assert payload: compares a label namespace's
/// values against criterion ids extracted from a configured section (CC-3).
/// `deny_unknown_fields` rejects stray keys so a typo surfaces at load.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCriteriaLabelMatch {
    /// Label namespace whose values are checked against criterion ids. Required.
    namespace: String,
    /// Projection slug of the criteria section. Optional; default `success_criteria`.
    #[serde(default, rename = "criteria-section")]
    criteria_section: Option<String>,
    /// Marker prefix; only items starting with it contribute ids. Optional.
    #[serde(default)]
    marker: Option<String>,
    /// Regex extracting the criterion id from an item. Optional; default pattern.
    #[serde(default, rename = "id-pattern")]
    id_pattern: Option<String>,
}

/// The `criteria-to-check` assert payload. `check-namespace` is required, and
/// `deny_unknown_fields` rejects unsupported configuration with rule context.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCriteriaToCheck {
    #[serde(default, rename = "criteria-section")]
    criteria_section: Option<String>,
    #[serde(default)]
    marker: Option<String>,
    #[serde(default, rename = "id-pattern")]
    id_pattern: Option<String>,
    #[serde(rename = "check-namespace")]
    check_namespace: String,
}

impl RawCriteriaToCheck {
    /// Lower into an [`Assertion::CriteriaToCheck`], applying defaults.
    fn into_assertion(self, rule: &str) -> Result<Assertion, RuleConfigError> {
        Ok(Assertion::CriteriaToCheck {
            criteria_section: self
                .criteria_section
                .unwrap_or_else(|| "success_criteria".to_string()),
            marker: self.marker,
            id_pattern: compile_id_pattern(rule, self.id_pattern)?,
            check_namespace: self.check_namespace,
        })
    }
}

/// Compile an authored `id-pattern`, or the default when absent.
fn compile_id_pattern(rule: &str, source: Option<String>) -> Result<IdPattern, RuleConfigError> {
    IdPattern::new(source.unwrap_or_else(|| DEFAULT_CRITERION_ID_PATTERN.to_string())).map_err(
        |error| RuleConfigError::InvalidAssertion {
            rule: rule.to_string(),
            message: format!("'id-pattern': {error}"),
        },
    )
}

/// The `label-uniqueness` assert payload.
///
/// `namespace` names the label namespace whose values must be unique across all
/// matching issues. `scope` must name a [`LabelUniquenessScope`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLabelUniqueness {
    namespace: String,
    scope: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequireLabel {
    label: String,
    #[serde(default)]
    min: Option<u32>,
    #[serde(default)]
    max: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequireSection {
    heading: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRequireDocType {
    #[serde(rename = "doc-type")]
    doc_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLabelValuePattern {
    namespace: String,
    regex: String,
}

/// The `label-coverage` payload, lowered at load after kind expansion.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RawLabelCoverage {
    criteria_section: Option<String>,
    marker: Option<String>,
    id_pattern: Option<IdPattern>,
    satisfies_namespace: Option<String>,
    child_state: Option<State>,
    child_link: Option<ChildLink>,
    child_type_exclude: Option<Vec<String>>,
    container_from_label: Option<String>,
    /// An `[item_kinds]` name standing in for the inline criteria triple.
    kind: Option<String>,
}

impl RawLabelCoverage {
    fn into_assertion(
        mut self,
        rule: &str,
        loaded_config: Option<&JitConfig>,
    ) -> Result<Assertion, RuleConfigError> {
        if let Some(kind) = &self.kind {
            if self.criteria_section.is_some() || self.marker.is_some() || self.id_pattern.is_some()
            {
                return Err(RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: "label-coverage key 'kind' cannot be combined with \
                              'criteria-section', 'marker' or 'id-pattern'"
                        .to_string(),
                });
            }
            let registry = loaded_config
                .ok_or_else(|| RuleConfigError::MissingConfiguration {
                    rule: rule.to_string(),
                    purpose: "label-coverage kind expansion",
                })?
                .item_kinds
                .as_ref();
            let triple = crate::domain::item::expand_kind_triple(registry, kind).map_err(|e| {
                RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: e.to_string(),
                }
            })?;
            self.criteria_section = Some(triple.section);
            self.marker = triple.marker;
            self.id_pattern = Some(IdPattern::new(triple.id_pattern).map_err(|e| {
                RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: e.to_string(),
                }
            })?);
        }
        let default_id_pattern = IdPattern::new(DEFAULT_CRITERION_ID_PATTERN).map_err(|e| {
            RuleConfigError::InvalidAssertion {
                rule: rule.to_string(),
                message: e.to_string(),
            }
        })?;
        Ok(Assertion::LabelCoverage {
            config: LabelCoverageConfig {
                authored_criteria_section: self.criteria_section,
                marker: self.marker,
                authored_id_pattern: self.id_pattern,
                default_id_pattern,
                authored_satisfies_namespace: self.satisfies_namespace,
                child_state: self.child_state,
                authored_child_link: self.child_link,
                child_type_exclude: self.child_type_exclude,
                container_from_label: self.container_from_label,
            },
        })
    }
}

/// The `label-reference` assert payload, lowered by
/// [`RawLabelReference::into_assertion`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawLabelReference {
    from: String,
    to: String,
    scope: Option<String>,
}

impl RawLabelReference {
    /// Lower into an [`Assertion::LabelReference`], rejecting an unknown scope.
    fn into_assertion(self, rule: &str) -> Result<Assertion, RuleConfigError> {
        let authored_scope = self
            .scope
            .map(|scope| {
                scope.parse::<LabelReferenceScope>().map_err(|e| {
                    RuleConfigError::InvalidAssertion {
                        rule: rule.to_string(),
                        message: format!("label-reference 'scope': {e}"),
                    }
                })
            })
            .transpose()?;
        Ok(Assertion::LabelReference {
            config: LabelReferenceConfig {
                from: self.from,
                to: self.to,
                authored_scope,
            },
        })
    }
}

/// The `dependency-shape` assert payload, lowered by
/// [`RawDependencyShape::into_assertion`].
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawDependencyShape {
    target: Selector,
    mode: Option<String>,
    transitive: Option<bool>,
}

impl RawDependencyShape {
    /// Lower into an [`Assertion::DependencyShape`], rejecting an unknown mode
    /// or an invalid state token in the target.
    fn into_assertion(self, rule: &str) -> Result<Assertion, RuleConfigError> {
        if let Some(state) = &self.target.state {
            state.validate(rule)?;
        }
        let authored_mode = self
            .mode
            .map(|mode| {
                mode.parse::<DependencyShapeMode>()
                    .map_err(|e| RuleConfigError::InvalidAssertion {
                        rule: rule.to_string(),
                        message: format!("dependency-shape 'mode': {e}"),
                    })
            })
            .transpose()?;
        Ok(Assertion::DependencyShape {
            config: DependencyShapeConfig {
                target: self.target,
                authored_mode,
                authored_transitive: self.transitive,
            },
        })
    }
}

/// Check a graph-assertion payload against its raw struct `T`, so an unknown or
/// missing key, or a mistyped value, fails the load naming the key.
fn check_payload<T: serde::de::DeserializeOwned>(
    rule: &str,
    kind: &str,
    config: &toml::value::Table,
) -> Result<T, RuleConfigError> {
    toml::Value::Table(config.clone())
        .try_into()
        .map_err(|e: toml::de::Error| RuleConfigError::InvalidAssertion {
            rule: rule.to_string(),
            message: format!("{kind}: {}", e.message()),
        })
}

impl RawRule {
    fn into_rule(
        self,
        config: Option<&JitConfig>,
        schemas: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Rule, RuleConfigError> {
        // A rule's name is its addressable self-id (`@/rule/<name>`); colon is
        // reserved for the label namespace:value separator, so a colon-bearing
        // name would mint an unaddressable label value. Reject at load time
        // rather than let it silently produce a broken address.
        if self.name.contains(':') {
            return Err(RuleConfigError::InvalidRuleName { name: self.name });
        }
        // Validate the `when` state predicate at load so a typo'd state (which
        // would otherwise silently never match any issue) is rejected with an
        // error naming the rule and the valid tokens.
        if let Some(state) = &self.when.state {
            state.validate(&self.name)?;
        }
        // A default-origin rule's `schemas/default-*.json` file is a rebuildable
        // PROJECTION, not the authority: `reconcile_default_rules_with_config`
        // re-derives its assertion from config at load. So a MISSING or MALFORMED
        // projection must not fail the load — fall back to a placeholder the
        // reconciliation replaces, keeping the projection out of the validation
        // path for default rules. Custom rules keep strict file-read semantics
        // (REQ-03): only a `default`-origin rule's schema read/parse failure is
        // tolerated, and only that failure class (a bad reference still errors).
        let default_schema_ref = if self.origin.as_deref() == Some(DEFAULT_ORIGIN) {
            self.assert.json_schema.clone()
        } else {
            None
        };
        let assert = match self.assert.into_assertion(&self.name, config, schemas) {
            Ok(assert) => assert,
            Err(err) => match (default_schema_ref, &err) {
                (
                    Some(reference),
                    RuleConfigError::SchemaIo { .. } | RuleConfigError::SchemaJson { .. },
                ) => placeholder_default_schema_assertion(reference),
                (Some(reference), RuleConfigError::MissingSchema { .. }) => {
                    placeholder_default_schema_assertion(reference)
                }
                _ => return Err(err),
            },
        };
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
            origin: self.origin,
            description: self.description,
            when: self.when,
            severity: self.severity,
            enforce: self.enforce,
            assert,
            scope,
        })
    }
}

impl RawAssert {
    fn into_assertion(
        self,
        rule: &str,
        loaded_config: Option<&JitConfig>,
        schemas: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Assertion, RuleConfigError> {
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
            || self.type_hierarchy.is_some()
            || self.criteria_label_match.is_some()
            || self.criteria_to_check.is_some()
            || self.label_uniqueness.is_some();
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
            self.criteria_label_match.is_some(),
            self.criteria_to_check.is_some(),
            self.label_uniqueness.is_some(),
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
            validate_schema_reference(rule, &reference)?;
            let bytes = schemas
                .get(&reference)
                .ok_or_else(|| RuleConfigError::MissingSchema {
                    rule: rule.to_string(),
                    reference: reference.clone(),
                })?;
            let path = PathBuf::from("<captured>").join(&reference);
            return schema_source_from_bytes(rule, reference, path, bytes)
                .map(Assertion::JsonSchema);
        }
        if let Some(cmd) = self.checker_command {
            return Ok(Assertion::CheckerCommand(cmd));
        }
        if let Some(config) = self.label_coverage {
            return check_payload::<RawLabelCoverage>(rule, "label-coverage", &config)?
                .into_assertion(rule, loaded_config);
        }
        if let Some(config) = self.label_reference {
            return check_payload::<RawLabelReference>(rule, "label-reference", &config)?
                .into_assertion(rule);
        }
        if let Some(config) = self.dependency_shape {
            return check_payload::<RawDependencyShape>(rule, "dependency-shape", &config)?
                .into_assertion(rule);
        }
        if let Some(gr) = self.gate_recency {
            return gr.into_assertion(rule);
        }
        if let Some(th) = self.type_hierarchy {
            let kind = th.kind.parse::<TypeHierarchyKind>().map_err(|e| {
                RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: format!("type-hierarchy 'kind': {e}"),
                }
            })?;
            return Ok(Assertion::TypeHierarchy { kind });
        }
        if let Some(clm) = self.criteria_label_match {
            // `namespace` is required. An empty namespace would match all labels,
            // which is nonsensical, so reject it explicitly.
            if clm.namespace.is_empty() {
                return Err(RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: "criteria-label-match 'namespace' must be a non-empty string"
                        .to_string(),
                });
            }
            return Ok(Assertion::CriteriaLabelMatch {
                namespace: clm.namespace,
                criteria_section: clm
                    .criteria_section
                    .unwrap_or_else(|| "success_criteria".to_string()),
                marker: clm.marker,
                id_pattern: compile_id_pattern(rule, clm.id_pattern)?,
            });
        }
        if let Some(ctc) = self.criteria_to_check {
            return check_payload::<RawCriteriaToCheck>(rule, "criteria-to-check", &ctc)?
                .into_assertion(rule);
        }
        if let Some(lu) = self.label_uniqueness {
            // Scope is required, not defaulted; `all` is the only value.
            lu.scope.parse::<LabelUniquenessScope>().map_err(|e| {
                RuleConfigError::InvalidAssertion {
                    rule: rule.to_string(),
                    message: format!("label-uniqueness 'scope': {e}"),
                }
            })?;
            return Ok(Assertion::LabelUniqueness {
                namespace: lu.namespace,
            });
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
/// The shared path constructor enforces the `schemas/<name>.json` shape and
/// confines the reference to one file-name component.
fn validate_schema_reference(rule: &str, reference: &str) -> Result<(), RuleConfigError> {
    let reject = |message: &str| RuleConfigError::InvalidSchemaReference {
        rule: rule.to_string(),
        reference: reference.to_string(),
        message: message.to_string(),
    };
    VirtualPath::schema_reference(reference)
        .map_err(|_| reject("reference must be one relative 'schemas/<name>.json' file"))?;
    Ok(())
}

/// The stand-in assertion for a default-origin rule whose `schemas/default-*.json`
/// projection is missing or malformed.
///
/// A permissive JSON Schema (`{}` matches anything) carrying the declared file
/// identity for diagnostics. [`reconcile_default_rules_with_config`](crate::repository_state::reconcile_default_rules_with_config)
/// replaces it with the config-derived assertion at load, so the rebuildable
/// projection is never load-bearing for a default rule's validation and this
/// placeholder is not evaluated once reconciliation has run.
fn placeholder_default_schema_assertion(reference: String) -> Assertion {
    let path = PathBuf::from("<captured>").join(&reference);
    Assertion::JsonSchema(SchemaSource {
        reference,
        path,
        schema: serde_json::json!({}),
    })
}

/// Parse injected JSON Schema bytes without consulting ambient state.
fn schema_source_from_bytes(
    rule: &str,
    reference: String,
    path: PathBuf,
    content: &[u8],
) -> Result<SchemaSource, RuleConfigError> {
    validate_schema_reference(rule, &reference)?;
    let schema: serde_json::Value =
        serde_json::from_slice(content).map_err(|source| RuleConfigError::SchemaJson {
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
    use crate::domain::{Issue, State};
    use crate::test_utils::{document_reference, fixture_issue, toml_string_array};

    fn parse(content: &str) -> Result<RuleSet, RuleConfigError> {
        RuleSet::parse(content, None, [])
    }

    fn parse_with_config(content: &str, config: &JitConfig) -> Result<RuleSet, RuleConfigError> {
        RuleSet::parse(content, Some(config), [])
    }

    fn issue_with(labels: &[&str], state: State) -> Issue {
        let mut issue = fixture_issue("t".to_string(), String::new());
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
            state: Some(StatePredicate::single("in_progress")),
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
        let set = parse(toml).unwrap();
        let sel = &set.rules[0].when;
        assert!(sel.matches(&issue_with(&[], State::InProgress)));
        assert!(!sel.matches(&issue_with(&[], State::Ready)));
    }

    #[test]
    fn test_selector_state_list_predicate_matches_any_member() {
        // A list `state` matches an issue in any listed state and nothing else.
        let toml = format!(
            r#"
[[rules]]
name = "list"
when = {{ state = {states} }}
assert = {{ require-section = {{ heading = "Plan" }} }}
"#,
            states = toml_string_array(&[State::Ready, State::InProgress, State::Gated]),
        );
        let set = parse(&toml).unwrap();
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
        let toml = format!(
            r#"
[[rules]]
name = "epic-lifecycle"
when = {{ type = "epic", state = {states} }}
assert = {{ require-section = {{ heading = "Plan" }} }}
"#,
            states = toml_string_array(&[State::Ready, State::InProgress]),
        );
        let set = parse(&toml).unwrap();
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
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::InvalidState { rule, value, valid } => {
                assert_eq!(rule, "typo");
                assert_eq!(value, "in_progres");
                // The valid list names every legal token.
                for token in State::ALL.map(State::as_str) {
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
        let err = parse(toml).unwrap_err();
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
        // `when = { state = [] }` deserializes to a predicate with no tokens.
        // It must be rejected at load because it silently never matches any issue.
        let toml = r#"
[[rules]]
name = "empty-list"
when = { state = [] }
assert = { require-section = { heading = "Plan" } }
"#;
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::InvalidState { rule, value, valid } => {
                assert_eq!(rule, "empty-list");
                // The sentinel value must make the problem explicit.
                assert!(
                    value.contains("empty"),
                    "value should indicate empty list, got: {value}"
                );
                // The valid list must name every legal token.
                for token in State::ALL.map(State::as_str) {
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
    fn test_state_predicate_matches_typed_set_equals_token_set() {
        // The pre-parsed set membership must agree with the authored tokens for
        // every lifecycle state.
        let chosen = [State::Ready, State::InProgress, State::Done];
        let pred = StatePredicate::list(chosen.map(State::as_str));
        for state in State::ALL {
            let want = chosen.contains(&state);
            assert_eq!(pred.matches(state), want, "mismatch for {state:?}");
        }
    }

    #[test]
    fn test_selector_has_doc_type() {
        let sel = Selector {
            has_doc_type: Some("design".to_string()),
            ..Default::default()
        };
        let mut issue = issue_with(&[], State::Ready);
        let mut doc = document_reference("docs/d.md");
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
            state: Some(StatePredicate::single("ready")),
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
        let set = parse(toml).unwrap();
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
        let set = parse(toml).unwrap();
        assert_eq!(set.rules.len(), 1);
        assert!(!set.rules[0].enforce);
    }

    #[test]
    fn test_severity_defaults_to_warn_and_parses_levels() {
        let toml = format!(
            r#"
[[rules]]
name = "default-sev"
assert = {{ require-section = {{ heading = "A" }} }}

[[rules]]
name = "error-sev"
severity = "{error}"
enforce = true
assert = {{ require-section = {{ heading = "B" }} }}

[[rules]]
name = "off-sev"
severity = "{off}"
assert = {{ require-section = {{ heading = "C" }} }}
"#,
            error = Severity::Error,
            off = Severity::Off,
        );
        let set = parse(&toml).unwrap();
        assert_eq!(set.rules[0].severity, Severity::Warn);
        assert_eq!(set.rules[1].severity, Severity::Error);
        assert!(set.rules[1].enforce);
        assert_eq!(set.rules[2].severity, Severity::Off);
    }

    #[test]
    fn test_scope_is_derived_from_assertion_kind() {
        let toml = r#"
[[rules]]
name = "label-rule"
assert = { require-label = { label = "type:*" } }

[[rules]]
name = "coverage-rule"
assert = { label-coverage = { child-state = "done" } }
"#;
        let set = parse(toml).unwrap();
        assert_eq!(set.rules[0].scope, RuleScope::Local);
        assert_eq!(set.rules[1].scope, RuleScope::Graph);
    }

    // --- Guards ------------------------------------------------------------

    #[test]
    fn test_shorthand_and_file_schema_is_rejected() {
        let toml = r#"
[[rules]]
name = "mixed"
assert = { require-label = { label = "req:*" }, json-schema = "schemas/x.json" }
"#;
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let set = parse(toml).unwrap();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::DuplicateRuleName { name } => assert_eq!(name, "dup"),
            other => panic!("expected DuplicateRuleName, got {other:?}"),
        }
    }

    #[test]
    fn test_origin_field_parses_when_present_and_defaults_to_none() {
        // A rule authoring an `origin` string carries it through parsing; a rule
        // that omits `origin` gets `None` (a plain repo-authored rule), not an
        // empty string.
        let toml = r#"
[[rules]]
name = "with-origin"
origin = "bracket"
assert = { require-section = { heading = "A" } }

[[rules]]
name = "without-origin"
assert = { require-section = { heading = "A" } }
"#;
        let set = parse(toml).unwrap();
        assert_eq!(set.rules[0].origin.as_deref(), Some("bracket"));
        assert_eq!(set.rules[1].origin, None);
    }

    #[test]
    fn test_colon_bearing_name_is_rejected() {
        // The `default:` reservation was removed (DR §8.2), but colon itself
        // stays reserved for the label `namespace:value` separator: a rule's
        // name is its addressable self-id (`@/rule/<name>`), so a colon-bearing
        // name would mint a label value the address grammar cannot express.
        // Provenance belongs in the separate `origin` field, not a `prefix:`.
        let toml = r#"
[[rules]]
name = "my:rule"
assert = { require-section = { heading = "A" } }
"#;
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::InvalidRuleName { name } => assert_eq!(name, "my:rule"),
            other => panic!("expected InvalidRuleName, got {other:?}"),
        }
    }

    // --- type-hierarchy assert kind ----------------------------------------

    #[test]
    fn test_type_hierarchy_orphan_leaf_parses_as_graph_rule() {
        let toml = r#"
[[rules]]
name = "orphan-leaf-fixture"
assert = { type-hierarchy = { kind = "orphan-leaf" } }
"#;
        let set = parse(toml).unwrap();
        assert_eq!(set.rules[0].scope, RuleScope::Graph);
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
name = "strategic-consistency-fixture"
assert = { type-hierarchy = { kind = "strategic-consistency" } }
"#;
        let set = parse(toml).unwrap();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let set = parse(toml).unwrap();
        assert_eq!(set.rules[0].scope, RuleScope::Graph);
        match &set.rules[0].assert {
            Assertion::GateRecency {
                max_age_hours,
                gates,
            } => {
                assert_eq!(*max_age_hours, 7 * 24);
                assert_eq!(gates, &["code-review".parse::<GateKey>().unwrap()]);
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
        let set = parse(toml).unwrap();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
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
        let err = parse(toml).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    // --- criteria-label-match assert kind ------------------------------------

    #[test]
    fn test_criteria_label_match_minimal_parses_as_graph_rule() {
        // The only required field is `namespace`; all others are optional.
        let toml = r#"
[[rules]]
name = "stray-req"
when = { type = "epic" }
severity = "error"
assert = { criteria-label-match = { namespace = "req" } }
"#;
        let set = parse(toml).unwrap();
        assert_eq!(set.rules[0].scope, RuleScope::Graph);
        match &set.rules[0].assert {
            Assertion::CriteriaLabelMatch {
                namespace,
                criteria_section,
                marker,
                id_pattern,
            } => {
                assert_eq!(namespace, "req");
                assert_eq!(criteria_section, "success_criteria"); // default
                assert!(marker.is_none());
                assert_eq!(id_pattern.as_str(), DEFAULT_CRITERION_ID_PATTERN);
            }
            other => panic!("expected CriteriaLabelMatch, got {other:?}"),
        }
    }

    #[test]
    fn test_criteria_label_match_all_fields_parse() {
        let toml = r#"
[[rules]]
name = "stray-req-full"
when = { type = "epic" }
severity = "error"
assert = { criteria-label-match = { namespace = "req", criteria-section = "success_criteria", marker = "[hard]", id-pattern = 'REQ-[0-9]+' } }
"#;
        let set = parse(toml).unwrap();
        match &set.rules[0].assert {
            Assertion::CriteriaLabelMatch {
                namespace,
                criteria_section,
                marker,
                id_pattern,
            } => {
                assert_eq!(namespace, "req");
                assert_eq!(criteria_section, "success_criteria");
                assert_eq!(marker.as_deref(), Some("[hard]"));
                assert_eq!(id_pattern.as_str(), "REQ-[0-9]+");
            }
            other => panic!("expected CriteriaLabelMatch, got {other:?}"),
        }
    }

    #[test]
    fn test_criteria_label_match_empty_namespace_is_rejected() {
        let toml = r#"
[[rules]]
name = "bad"
assert = { criteria-label-match = { namespace = "" } }
"#;
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "bad");
                assert!(message.contains("namespace"), "message was: {message}");
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_criteria_label_match_unknown_field_is_rejected() {
        // `deny_unknown_fields` on RawCriteriaLabelMatch rejects stray keys.
        let toml = r#"
[[rules]]
name = "bad"
assert = { criteria-label-match = { namespace = "req", bogus-key = 1 } }
"#;
        let err = parse(toml).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    #[test]
    fn test_parse_invalid_criteria_id_pattern_fails_naming_the_rule() {
        for payload in [
            "criteria-label-match = { namespace = \"req\", id-pattern = 'REQ-(' }",
            "criteria-to-check = { check-namespace = \"checks\", id-pattern = 'REQ-(' }",
        ] {
            let toml = format!("[[rules]]\nname = \"bad-pattern\"\nassert = {{ {payload} }}\n");
            match parse(&toml).unwrap_err() {
                RuleConfigError::InvalidAssertion { rule, message } => {
                    assert_eq!(rule, "bad-pattern");
                    assert!(message.contains("REQ-("), "message was: {message}");
                }
                other => panic!("expected InvalidAssertion for {payload}, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_criteria_to_check_requires_check_namespace() {
        let toml = "[[rules]]\nname = \"missing-check-namespace\"\n\
                    assert = { criteria-to-check = {} }\n";
        match parse(toml).unwrap_err() {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "missing-check-namespace");
                assert!(
                    message.contains("check-namespace"),
                    "error must name the required label namespace: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_criteria_to_check_rejects_gate_prefix_naming_rule() {
        let toml = "[[rules]]\nname = \"criteria-labels-only\"\n\
                    assert = { criteria-to-check = { gate-prefix = \"unused\", \
                    check-namespace = \"checks\" } }\n";
        let error = parse(toml).unwrap_err();
        assert!(matches!(
            &error,
            RuleConfigError::InvalidAssertion { rule, .. } if rule == "criteria-labels-only"
        ));
        assert!(
            error.to_string().contains("rule 'criteria-labels-only'"),
            "config error must name the rule: {error}"
        );
        assert!(
            error.to_string().contains("unknown field"),
            "error must reject the unsupported key: {error}"
        );
    }

    #[test]
    fn test_unknown_assertion_kind_is_rejected() {
        let toml = r#"
[[rules]]
name = "bogus"
assert = { not-a-real-kind = { x = 1 } }
"#;
        let err = parse(toml).unwrap_err();
        assert!(matches!(err, RuleConfigError::Toml(_)));
    }

    // --- File-schema loading -----------------------------------------------

    #[test]
    fn test_json_schema_bytes_are_injected_and_transcoded() {
        let toml = r#"
[[rules]]
name = "epic-body"
when = { type = "epic" }
assert = { json-schema = "schemas/epic-body.json" }
"#;
        let set = RuleSet::parse(
            toml,
            None,
            [(
                "schemas/epic-body.json".into(),
                br#"{ "type": "object", "required": ["sections"] }"#.to_vec(),
            )],
        )
        .unwrap();
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
    fn test_missing_injected_json_schema_errors() {
        let toml = r#"
[[rules]]
name = "epic-body"
assert = { json-schema = "schemas/does-not-exist.json" }
"#;
        let err = parse(toml).unwrap_err();
        assert!(matches!(err, RuleConfigError::MissingSchema { .. }));
    }

    #[test]
    fn test_invalid_injected_json_schema_errors() {
        let toml = r#"
[[rules]]
name = "epic-body"
assert = { json-schema = "schemas/bad.json" }
"#;
        let err = RuleSet::parse(
            toml,
            None,
            [("schemas/bad.json".into(), b"{ not valid json".to_vec())],
        )
        .unwrap_err();
        assert!(matches!(err, RuleConfigError::SchemaJson { .. }));
    }

    #[test]
    fn test_default_origin_missing_schema_falls_back_to_placeholder() {
        // A default-origin rule whose `schemas/default-*.json` projection is ABSENT
        // must still load (the projection is not the authority): the assertion
        // becomes a placeholder that reconciliation replaces from config.
        let toml = r#"
[[rules]]
name = "namespace-registry"
origin = "default"
assert = { json-schema = "schemas/default-namespace-registry.json" }
"#;
        let set = parse(toml).unwrap();
        match &set.rules[0].assert {
            Assertion::JsonSchema(src) => {
                assert_eq!(src.reference, "schemas/default-namespace-registry.json");
                assert_eq!(src.schema, serde_json::json!({}), "permissive placeholder");
            }
            other => panic!("expected a JsonSchema placeholder, got {other:?}"),
        }
    }

    #[test]
    fn test_default_origin_malformed_schema_falls_back_to_placeholder() {
        // A default-origin rule whose projection is present but MALFORMED likewise
        // falls back rather than failing the load.
        let toml = r#"
[[rules]]
name = "label-format"
origin = "default"
assert = { json-schema = "schemas/default-label-format.json" }
"#;
        let set = RuleSet::parse(
            toml,
            None,
            [(
                "schemas/default-label-format.json".into(),
                b"{ not json".to_vec(),
            )],
        )
        .unwrap();
        match &set.rules[0].assert {
            Assertion::JsonSchema(src) => assert_eq!(src.schema, serde_json::json!({})),
            other => panic!("expected a JsonSchema placeholder, got {other:?}"),
        }
    }

    #[test]
    fn test_custom_origin_missing_schema_still_errors() {
        // REQ-03: a non-default rule keeps strict file-read semantics — a missing
        // schema is a hard load error, exactly as before.
        let toml = r#"
[[rules]]
name = "custom-shape"
origin = "bracket"
assert = { json-schema = "schemas/missing.json" }
"#;
        let err = parse(toml).unwrap_err();
        assert!(matches!(err, RuleConfigError::MissingSchema { .. }));
    }

    #[test]
    fn test_default_origin_bad_schema_reference_still_errors() {
        // Tolerance is scoped to a read/parse failure: a default rule with an
        // UNSAFE reference (outside `schemas/`) is still rejected, not placeheld.
        let toml = r#"
[[rules]]
name = "namespace-registry"
origin = "default"
assert = { json-schema = "/etc/passwd.json" }
"#;
        let err = parse(toml).unwrap_err();
        assert!(matches!(
            err,
            RuleConfigError::InvalidSchemaReference { .. }
        ));
    }

    // --- Schema reference safety -------------------------------------------

    fn schema_rule_toml(reference: &str) -> String {
        format!("[[rules]]\nname = \"epic-body\"\nassert = {{ json-schema = \"{reference}\" }}\n")
    }

    #[test]
    fn test_schema_reference_absolute_path_is_rejected() {
        let toml = schema_rule_toml("/etc/passwd.json");
        let err = parse(&toml).unwrap_err();
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
        let toml = schema_rule_toml("schemas/../escape.json");
        let err = parse(&toml).unwrap_err();
        assert!(matches!(
            err,
            RuleConfigError::InvalidSchemaReference { .. }
        ));
    }

    #[test]
    fn test_schema_reference_nested_file_name_is_rejected() {
        let toml = schema_rule_toml("schemas/nested/x.json");
        let err = parse(&toml).unwrap_err();
        assert!(matches!(
            err,
            RuleConfigError::InvalidSchemaReference { .. }
        ));
    }

    #[test]
    fn test_schema_reference_not_under_schemas_is_rejected() {
        let toml = schema_rule_toml("other/x.json");
        let err = parse(&toml).unwrap_err();
        match err {
            RuleConfigError::InvalidSchemaReference { message, .. } => {
                assert!(message.contains("schemas/"));
            }
            other => panic!("expected InvalidSchemaReference, got {other:?}"),
        }
    }

    #[test]
    fn test_schema_reference_non_json_extension_is_rejected() {
        let toml = schema_rule_toml("schemas/x.txt");
        let err = parse(&toml).unwrap_err();
        match err {
            RuleConfigError::InvalidSchemaReference { message, .. } => {
                assert!(message.contains(".json"));
            }
            other => panic!("expected InvalidSchemaReference, got {other:?}"),
        }
    }

    #[test]
    fn test_schema_reference_valid_under_schemas_is_accepted() {
        let toml = schema_rule_toml("schemas/x.json");
        let set = RuleSet::parse(
            &toml,
            None,
            [("schemas/x.json".into(), br#"{ "type": "object" }"#.to_vec())],
        )
        .unwrap();
        match &set.rules[0].assert {
            Assertion::JsonSchema(src) => assert_eq!(src.reference, "schemas/x.json"),
            other => panic!("expected JsonSchema, got {other:?}"),
        }
    }

    // --- Regex round-trip (TOML literal string -> serde_json) --------------

    #[test]
    fn test_rule_description_loads_when_present_and_absent() {
        // REQ-01: the schema accepts an optional `description`. A rule that
        // authors one loads it; a description-less rule loads unchanged with
        // `None`, and a consumer wanting display text falls back to the name.
        let toml = r#"
[[rules]]
name = "described"
description = "Every label must be namespace:value."
assert = { require-section = { heading = "Goals" } }

[[rules]]
name = "bare"
assert = { require-section = { heading = "Goals" } }
"#;
        let set = parse(toml).unwrap();
        assert_eq!(
            set.rules[0].description.as_deref(),
            Some("Every label must be namespace:value.")
        );
        assert_eq!(set.rules[1].description, None);
        // Name fallback: a consumer reads description-or-name for each rule.
        let display = |r: &Rule| r.description.as_deref().unwrap_or(&r.name).to_string();
        assert_eq!(
            display(&set.rules[0]),
            "Every label must be namespace:value."
        );
        assert_eq!(display(&set.rules[1]), "bare");
    }

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
        let set = parse(toml).unwrap();
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
        let set = parse(toml).unwrap();
        let regex = match &set.rules[0].assert {
            Assertion::LabelValuePattern { regex, .. } => regex.clone(),
            other => panic!("expected LabelValuePattern, got {other:?}"),
        };
        assert_eq!(regex, r"^\[hard\]\s+\w+");
        let json = serde_json::to_value(&regex).unwrap();
        assert_eq!(json, serde_json::json!(r"^\[hard\]\s+\w+"));
    }

    #[test]
    fn test_label_uniqueness_scope_all_parses_as_graph_rule() {
        let toml = r#"
[[rules]]
name = "unique-req"
when = { type = "epic" }
severity = "error"
assert = { label-uniqueness = { namespace = "req", scope = "all" } }
"#;
        let set = parse(toml).unwrap();
        assert_eq!(set.rules[0].scope, RuleScope::Graph);
        match &set.rules[0].assert {
            Assertion::LabelUniqueness { namespace } => {
                assert_eq!(namespace, "req");
            }
            other => panic!("expected LabelUniqueness, got {other:?}"),
        }
    }

    #[test]
    fn test_label_uniqueness_missing_scope_is_rejected() {
        // scope is required for label-uniqueness.
        let toml = r#"
[[rules]]
name = "no-scope"
assert = { label-uniqueness = { namespace = "req" } }
"#;
        let err = parse(toml).unwrap_err();
        // Missing required field "scope" is a TOML parse error (deny_unknown_fields
        // does not apply here, but the missing field surfaces via serde).
        assert!(
            matches!(err, RuleConfigError::Toml(_)),
            "missing scope must be a parse error, got {err:?}"
        );
    }

    #[test]
    fn test_label_uniqueness_scope_linked_is_rejected() {
        // scope = "linked" is explicitly not supported for this kind.
        let toml = r#"
[[rules]]
name = "linked-scope"
assert = { label-uniqueness = { namespace = "req", scope = "linked" } }
"#;
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "linked-scope");
                assert!(
                    message.contains("linked") && message.contains("all"),
                    "error must mention 'linked' and 'all': {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_label_uniqueness_unknown_scope_is_rejected() {
        let toml = r#"
[[rules]]
name = "bad-scope"
assert = { label-uniqueness = { namespace = "req", scope = "global" } }
"#;
        let err = parse(toml).unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "bad-scope");
                assert!(
                    message.contains("global") && message.contains("all"),
                    "error must mention the bad value and the valid value: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_label_uniqueness_is_repo_wide_at_transition() {
        let toml = r#"
[[rules]]
name = "unique-req"
assert = { label-uniqueness = { namespace = "req", scope = "all" } }
"#;
        let set = parse(toml).unwrap();
        assert!(
            set.rules[0].assert.is_repo_wide_at_transition(),
            "label-uniqueness must be skipped at transition time"
        );
    }

    // --- label-coverage `kind =` sugar (REQ-02, REQ-03) --------------------

    /// Build explicit captured configuration declaring a custom kind for
    /// `kind =` expansion.
    fn item_kind_config() -> JitConfig {
        toml::from_str(
            r#"
[item_kinds.custom-requirement]
section = "success_criteria"
id-pattern = "CR-[0-9]+"
markers = ["[hard]"]
link-namespaces = ["satisfies"]
scope = "issue"
source-of-truth = "markdown-first"
"#,
        )
        .unwrap()
    }

    fn rule_from_config(rule_toml: &str) -> Rule {
        let config = item_kind_config();
        let set = parse_with_config(rule_toml, &config).unwrap();
        set.rules.into_iter().next().unwrap()
    }

    /// Extract the resolved coverage configuration.
    fn coverage_config(rule: &Rule) -> &LabelCoverageConfig {
        match &rule.assert {
            Assertion::LabelCoverage { config } => config,
            other => panic!("expected LabelCoverage, got {other:?}"),
        }
    }

    #[test]
    fn test_label_coverage_kind_sugar_expands_to_inline_triple() {
        // A `kind = "custom-requirement"` rule lowers to the configuration an
        // equivalent inline `(section, marker, id-pattern)` rule carries.
        let sugared = rule_from_config(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { kind = "custom-requirement", child-state = "done" } }
"#,
        );
        let inline = rule_from_config(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "CR-[0-9]+", child-state = "done" } }
"#,
        );
        // Kind sugar and inline criteria have identical effective configuration.
        assert_eq!(coverage_config(&sugared), coverage_config(&inline));
    }

    /// Assert that loading `rule_toml` fails with an error whose message names
    /// `key`.
    fn assert_rejects_key(rule_toml: &str, key: &str) {
        let err = parse_with_config(rule_toml, &item_kind_config()).unwrap_err();
        assert!(
            err.to_string().contains(key),
            "error must name the rejected key '{key}': {err}"
        );
    }

    #[test]
    fn test_label_coverage_inline_rule_with_extra_unknown_key_is_rejected() {
        assert_rejects_key(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", future-key = "ignored" } }
"#,
            "future-key",
        );
    }

    #[test]
    fn test_label_coverage_inline_rule_with_kind_key_is_rejected() {
        // `kind` and the inline triple are alternative criteria sources.
        assert_rejects_key(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { criteria-section = "success_criteria", marker = "[hard]", id-pattern = "CR-[0-9]+", kind = "custom-requirement" } }
"#,
            "'kind'",
        );
    }

    #[test]
    fn test_label_coverage_kind_only_rule_expands() {
        // A kind-ONLY rule (no inline triple key) expands to that kind's triple.
        let sugared = rule_from_config(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { kind = "custom-requirement", child-state = "done" } }
"#,
        );
        let cfg = coverage_config(&sugared);
        assert_eq!(cfg.criteria_section(), "success_criteria");
        assert_eq!(cfg.marker.as_deref(), Some("[hard]"));
    }

    #[test]
    fn test_label_coverage_kind_sugar_unknown_kind_is_error() {
        // The constraint: an undeclared `kind` used as the sole triple source is a
        // typed config error. The registry declares `custom-requirement` completely (so it
        // loads), but `decision` is absent.
        let config = item_kind_config();
        let err = RuleSet::parse(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { kind = "decision" } }
"#,
            Some(&config),
            [],
        )
        .unwrap_err();
        match err {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "coverage");
                assert!(
                    message.contains("decision") && message.contains("[item_kinds]"),
                    "unknown-kind error must name the kind and registry: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_label_coverage_kind_sugar_requires_explicit_config() {
        let err = RuleSet::parse(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { kind = "custom-requirement" } }
"#,
            None,
            [],
        )
        .unwrap_err();
        assert!(matches!(
            err,
            RuleConfigError::MissingConfiguration {
                rule,
                purpose: "label-coverage kind expansion",
            } if rule == "coverage"
        ));
    }

    #[test]
    fn test_label_coverage_kind_only_non_string_is_error() {
        // A non-string `kind` used as the SOLE triple source cannot be resolved.
        let err = RuleSet::parse(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
assert = { label-coverage = { kind = 42 } }
"#,
            None,
            [],
        )
        .unwrap_err();
        assert!(matches!(err, RuleConfigError::InvalidAssertion { .. }));
    }

    #[test]
    fn test_parse_label_coverage_lowers_authored_options_and_compiles_defaults() {
        let parse_coverage = |payload: &str| {
            parse(&format!(
                "[[rules]]\nname = \"coverage\"\nassert = {{ label-coverage = {{ {payload} }} }}\n"
            ))
            .unwrap()
        };
        let defaults = parse_coverage("");
        let defaults = coverage_config(&defaults.rules[0]);
        assert_eq!(defaults.criteria_section(), "success_criteria");
        assert_eq!(defaults.satisfies_namespace(), "satisfies");
        assert_eq!(defaults.child_link(), ChildLink::Dependents);
        assert_eq!(defaults.child_state, None);
        assert!(defaults.id_pattern().regex().is_match("REQ-01"));

        let authored = parse_coverage(
            r#"criteria-section = "requirements", satisfies-namespace = "covers", child-link = "dependencies", child-state = "done", id-pattern = 'C[0-9]+'"#,
        );
        let authored = coverage_config(&authored.rules[0]);
        assert_eq!(authored.criteria_section(), "requirements");
        assert_eq!(authored.satisfies_namespace(), "covers");
        assert_eq!(authored.child_link(), ChildLink::Dependencies);
        assert_eq!(authored.child_state, Some(State::Done));
        assert_eq!(
            authored.id_pattern().regex().find("C12").unwrap().as_str(),
            "C12"
        );
    }

    #[test]
    fn test_parse_label_coverage_rejects_malformed_options_naming_rule() {
        for payload in [
            "child-link = \"bogus\"",
            "child-link = 1",
            "child-state = \"don\"",
            "id-pattern = \"(\"",
            "id-pattern = 1",
            "child-type-exclude = \"breakdown\"",
            "child-type-exclude = [1]",
            "satisfies-namespace = 1",
            "container-from-label = 1",
            "marker = 1",
            "criteria-section = 1",
        ] {
            let err = parse(&format!(
                "[[rules]]\nname = \"broken-coverage\"\nassert = {{ label-coverage = {{ {payload} }} }}\n"
            ))
            .unwrap_err();
            assert!(
                matches!(&err, RuleConfigError::InvalidAssertion { rule, .. } if rule == "broken-coverage"),
                "{payload}: {err}"
            );
        }
    }

    // --- unknown keys and aliases ------------------------------------------

    #[test]
    fn test_parse_unknown_key_at_any_declaration_level_is_rejected_naming_it() {
        let cases = [
            ("top-level", "bogus-top = 1\n[[rules]]\nname = \"r\"\nassert = { require-section = { heading = \"A\" } }\n", "bogus-top"),
            ("rule", "[[rules]]\nname = \"r\"\nbogus-rule = 1\nassert = { require-section = { heading = \"A\" } }\n", "bogus-rule"),
            ("selector", "[[rules]]\nname = \"r\"\nwhen = { bogus-when = \"x\" }\nassert = { require-section = { heading = \"A\" } }\n", "bogus-when"),
            ("label-reference", "[[rules]]\nname = \"r\"\nassert = { label-reference = { from = \"a\", to = \"b\", bogus-ref = 1 } }\n", "bogus-ref"),
            ("dependency-shape", "[[rules]]\nname = \"r\"\nassert = { dependency-shape = { target = { type = \"design\" }, bogus-shape = 1 } }\n", "bogus-shape"),
            ("dependency-shape target", "[[rules]]\nname = \"r\"\nassert = { dependency-shape = { target = { bogus-target = \"x\" } } }\n", "bogus-target"),
        ];
        for (level, rule_toml, key) in cases {
            let err = parse(rule_toml).expect_err(level);
            assert!(
                err.to_string().contains(key),
                "{level}: error must name '{key}': {err}"
            );
        }
    }

    #[test]
    fn test_parse_doc_type_key_aliases_are_rejected() {
        let cases = [
            ("when = { has-doc-type = \"design\" }\nassert = { require-section = { heading = \"A\" } }", "has-doc-type"),
            ("when = { doc_type = \"design\" }\nassert = { require-section = { heading = \"A\" } }", "doc_type"),
            ("assert = { require-doc-type = { doc_type = \"design\" } }", "doc_type"),
        ];
        for (body, key) in cases {
            let err = parse(&format!("[[rules]]\nname = \"r\"\n{body}\n")).unwrap_err();
            assert!(
                err.to_string().contains(key),
                "error must name '{key}': {err}"
            );
        }
    }

    #[test]
    fn test_dependency_shape_target_parses_as_selector_at_load() {
        let err = parse(
            r#"
[[rules]]
name = "shape"
assert = { dependency-shape = { target = { state = "don" } } }
"#,
        )
        .unwrap_err();
        assert!(
            matches!(&err, RuleConfigError::InvalidState { rule, value, .. } if rule == "shape" && value == "don"),
            "target state is validated at load: {err:?}"
        );
    }

    #[test]
    fn test_parse_dependency_shape_lowers_mode_and_transitive_and_rejects_unknown_mode() {
        let parse_shape = |extra: &str| {
            parse(&format!(
                "[[rules]]\nname = \"shape\"\n\
                 assert = {{ dependency-shape = {{ target = {{ type = \"design\" }}{extra} }} }}\n"
            ))
        };
        for (extra, mode, transitive) in [
            ("", DependencyShapeMode::Must, false),
            (", mode = \"should\"", DependencyShapeMode::Should, false),
            (", transitive = true", DependencyShapeMode::Must, true),
        ] {
            let set = parse_shape(extra).unwrap();
            match &set.rules[0].assert {
                Assertion::DependencyShape { config } => {
                    assert_eq!(config.mode(), mode);
                    assert_eq!(config.is_transitive(), transitive);
                }
                other => panic!("expected DependencyShape, got {other:?}"),
            }
        }
        for extra in [", mode = \"maybe\"", ", transitive = \"yes\""] {
            match parse_shape(extra).unwrap_err() {
                RuleConfigError::InvalidAssertion { rule, .. } => assert_eq!(rule, "shape"),
                other => panic!("expected InvalidAssertion, got {other:?}"),
            }
        }
    }

    #[test]
    fn test_parse_label_reference_lowers_scope_and_rejects_unknown_scope() {
        let parse_reference = |extra: &str| {
            parse(&format!(
                "[[rules]]\nname = \"reference\"\n\
                 assert = {{ label-reference = {{ from = \"satisfies\", to = \"req\"{extra} }} }}\n"
            ))
        };
        for (extra, expected) in [
            ("", LabelReferenceScope::Global),
            (", scope = \"linked\"", LabelReferenceScope::Linked),
        ] {
            let set = parse_reference(extra).unwrap();
            match &set.rules[0].assert {
                Assertion::LabelReference { config } => assert_eq!(config.scope(), expected),
                other => panic!("expected LabelReference, got {other:?}"),
            }
        }
        match parse_reference(", scope = \"nearby\"").unwrap_err() {
            RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "reference");
                assert!(message.contains("nearby"), "message was: {message}");
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_graph_payload_missing_required_key_is_rejected_at_load() {
        let cases = [
            ("label-reference = { from = \"implements\" }", "`to`"),
            ("dependency-shape = { mode = \"must\" }", "`target`"),
        ];
        for (assert, key) in cases {
            let err = parse(&format!(
                "[[rules]]\nname = \"r\"\nassert = {{ {assert} }}\n"
            ))
            .unwrap_err();
            assert!(
                err.to_string().contains(key),
                "error must name {key}: {err}"
            );
        }
    }
}
