//! Graph / aggregate rule evaluation (DR §4.2).
//!
//! These rule kinds need cross-issue context and therefore run in
//! `jit validate`, gate checkers, and at state transitions (where enforcing
//! failures block the transition; see `enforce_transition_graph_rules` in
//! `commands`). They never run on plain field writes:
//!
//! - **`label-coverage`** — every canonical criterion id declared in a source
//!   issue's `## Success Criteria` section is satisfied by at least one related
//!   child issue (via a derived `satisfies:<id>` label) in a configurable state.
//! - **`label-reference`** — a `from`-namespace reference (e.g. `satisfies:REQ-01`)
//!   resolves to a `to`-namespace source (e.g. `req:REQ-01`) that is actually
//!   declared somewhere in a configurable scope.
//! - **`dependency-shape`** — issues matching a rule's selector must (or should)
//!   depend on at least one issue matching a target selector, evaluated over the
//!   dependency DAG via [`DependencyGraph`].
//! - **`gate-recency`** — an issue's recorded gate results must be no older than
//!   a configured age, computed against `GateState.updated_at` and an injected
//!   `now` (the engine never reads wall-clock).
//!
//! # Entry point
//!
//! [`evaluate_graph`] takes the graph rules to run and the full issue set
//! (typically `store.list_issues()`), and returns a [`Finding`] per violation.
//! Reading the store happens at the call boundary (the validate / gate-checker
//! context); the evaluation itself is a pure function of the issue set, so it is
//! trivially testable with constructed [`Issue`] lists.
//!
//! # Config interpretation
//!
//! Each graph [`Assertion`] carries a raw, unvalidated [`toml::value::Table`].
//! This module interprets the keys each kind expects and DOCUMENTS them on the
//! per-kind evaluators below. Malformed or missing required config never panics:
//! it surfaces as a `config-error` [`Finding`] (attributed to the rule) so the
//! problem is visible rather than silently swallowed. Optional keys fall back to
//! documented defaults.
//!
//! # Purity & layering (DR §12)
//!
//! Evaluation is a deterministic function of the issue slice. The caller reads
//! the store; nothing here touches the filesystem.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use crate::document::content_parser_for;
use crate::domain::{project, ContentFormat, Issue};
use crate::graph::DependencyGraph;
use crate::type_hierarchy::{
    validate_orphans, validate_strategic_labels, HierarchyConfig, ValidationWarning,
};
use std::collections::HashMap;

use crate::validation::engine::Finding;
use crate::validation::rules::{
    Assertion, Rule, Scope, Selector, Severity, StatePredicate, TypeHierarchyKind,
};

/// Default label namespace whose values are criterion ids a child claims to
/// satisfy (e.g. `satisfies:REQ-01`).
const DEFAULT_SATISFIES_NAMESPACE: &str = "satisfies";

/// Default projection slug of the section holding success criteria.
const DEFAULT_CRITERIA_SECTION: &str = "success_criteria";

/// Default regex extracting a criterion id from an item's text (e.g. `REQ-01`).
const DEFAULT_ID_PATTERN: &str = "[A-Z][A-Z0-9]*-[0-9]+";

/// Prefix every `config-error` finding message carries, so a [`GraphFinding`]
/// can be recognized as a config error structurally.
const CONFIG_ERROR_PREFIX: &str = "config error: ";

/// How a "child" issue relates to the source issue in `label-coverage`.
///
/// # Examples
///
/// ```
/// use jit::validation::graph::ChildLink;
///
/// assert_eq!(ChildLink::parse("dependents"), Some(ChildLink::Dependents));
/// assert_eq!(ChildLink::parse("dependencies"), Some(ChildLink::Dependencies));
/// assert_eq!(ChildLink::parse("any"), Some(ChildLink::Any));
/// assert_eq!(ChildLink::parse("bogus"), None);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildLink {
    /// Children are issues that depend on the source (the source is in their
    /// `dependencies`). This is the default.
    Dependents,
    /// Children are issues the source depends on (in the source's
    /// `dependencies`).
    Dependencies,
    /// Any issue in the set is a candidate child, regardless of dependency edges.
    Any,
}

/// A graph-rule [`Finding`] paired with the issue it pertains to.
///
/// Graph rules are inherently cross-issue, so per-issue reporting needs to know
/// *which* issue each finding concerns. Rather than re-deriving that from the
/// finding's message text (a lossy substring match), [`evaluate_graph`] returns
/// this struct so attribution is exact and structural:
///
/// - `issue_id` is `Some(full_id)` for a violation attributable to one specific
///   issue (e.g. the source issue whose criterion is uncovered, or whose
///   dependency shape is wrong).
/// - `issue_id` is `None` for a `config-error` finding: a malformed rule config
///   pertains to the rule itself, not a single issue, and must never be silently
///   dropped from a per-issue view.
///
/// # Examples
///
/// ```
/// use jit::domain::{ContentFormat, Issue};
/// use jit::type_hierarchy::HierarchyConfig;
/// use jit::validation::graph::{evaluate_graph, GraphFinding};
/// use jit::validation::rules::RuleSet;
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "task-needs-design-dep"
/// when = { type = "task" }
/// severity = "error"
/// assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
/// let rules: Vec<&_> = set.rules.iter().collect();
///
/// let mut task = Issue::new("a task".into(), String::new());
/// task.labels = vec!["type:task".into()];
/// let task_id = task.id.clone();
/// let findings: Vec<GraphFinding> = evaluate_graph(
///     &rules,
///     &[task],
///     &HierarchyConfig::default(),
///     ContentFormat::Markdown,
///     chrono::Utc::now(),
/// );
/// assert_eq!(findings.len(), 1);
/// // The finding is attributed to the offending issue, not parsed from text.
/// assert_eq!(findings[0].issue_id.as_deref(), Some(task_id.as_str()));
/// assert!(!findings[0].is_config_error());
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphFinding {
    /// Full id of the issue this finding concerns, or `None` for a config-error
    /// (which pertains to the rule, not a single issue).
    pub issue_id: Option<String>,
    /// The underlying engine finding (rule name, severity, message).
    pub finding: Finding,
}

impl GraphFinding {
    /// A finding attributed to a specific issue id.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::Finding;
    /// use jit::validation::graph::GraphFinding;
    /// use jit::validation::rules::Severity;
    ///
    /// let f = Finding { rule: "r".into(), severity: Severity::Error, message: "m".into() };
    /// let gf = GraphFinding::for_issue("abc123", f);
    /// assert_eq!(gf.issue_id.as_deref(), Some("abc123"));
    /// assert!(!gf.is_config_error());
    /// ```
    pub fn for_issue(issue_id: impl Into<String>, finding: Finding) -> Self {
        Self {
            issue_id: Some(issue_id.into()),
            finding,
        }
    }

    /// A finding not attributable to a single issue (e.g. a `config-error`).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::Finding;
    /// use jit::validation::graph::GraphFinding;
    /// use jit::validation::rules::Severity;
    ///
    /// let f = Finding { rule: "r".into(), severity: Severity::Error, message: "config error: x".into() };
    /// let gf = GraphFinding::unattributed(f);
    /// assert!(gf.issue_id.is_none());
    /// assert!(gf.is_config_error());
    /// ```
    pub fn unattributed(finding: Finding) -> Self {
        Self {
            issue_id: None,
            finding,
        }
    }

    /// Whether this finding is a `config-error` (a malformed rule config).
    ///
    /// Config errors carry no issue id and are reported as `config error: …` by
    /// the per-kind evaluators. They must surface for any per-issue view of a
    /// rule that applies, never be silently dropped.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::Finding;
    /// use jit::validation::graph::GraphFinding;
    /// use jit::validation::rules::Severity;
    ///
    /// let bad = Finding { rule: "r".into(), severity: Severity::Error, message: "config error: missing 'to'".into() };
    /// assert!(GraphFinding::unattributed(bad).is_config_error());
    /// let ok = Finding { rule: "r".into(), severity: Severity::Error, message: "criterion uncovered".into() };
    /// assert!(!GraphFinding::for_issue("x", ok).is_config_error());
    /// ```
    pub fn is_config_error(&self) -> bool {
        self.finding.message.starts_with(CONFIG_ERROR_PREFIX)
    }
}

impl ChildLink {
    /// Parse a `child-link` config value into a [`ChildLink`].
    ///
    /// Returns `None` for an unrecognized value so the caller can emit a clear
    /// config-error finding rather than silently defaulting.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::graph::ChildLink;
    ///
    /// assert_eq!(ChildLink::parse("any"), Some(ChildLink::Any));
    /// assert!(ChildLink::parse("nope").is_none());
    /// ```
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "dependents" => Some(Self::Dependents),
            "dependencies" => Some(Self::Dependencies),
            "any" => Some(Self::Any),
            _ => None,
        }
    }
}

/// Evaluate all graph rules over the given issue set, returning one [`Finding`]
/// per violation (and a `config-error` finding per malformed rule config).
///
/// Only rules whose [`Rule::scope`] is [`Scope::Graph`] are evaluated; any
/// non-graph rule passed in is ignored (so callers may hand the full rule set).
/// Rules whose severity is [`Severity::Off`] are skipped entirely. This function
/// is pure: it reads nothing but the supplied slices.
///
/// The issue set is normally `store.list_issues()`, read by the caller at the
/// validate / gate-checker boundary.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::type_hierarchy::HierarchyConfig;
/// use jit::validation::graph::evaluate_graph;
/// use jit::validation::rules::RuleSet;
/// use std::path::Path;
///
/// // A dependency-shape rule: every `type:task` must depend on a `type:design`.
/// let toml = r#"
/// [[rules]]
/// name = "task-needs-design-dep"
/// when = { type = "task" }
/// severity = "error"
/// assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }
/// "#;
/// let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
/// let rules: Vec<&_> = set.rules.iter().collect();
///
/// let mut task = Issue::new("a task".into(), String::new());
/// task.labels = vec!["type:task".into()];
/// // No design dependency -> one finding, attributed to the task.
/// let findings = evaluate_graph(
///     &rules,
///     &[task],
///     &HierarchyConfig::default(),
///     jit::domain::ContentFormat::Markdown,
///     chrono::Utc::now(),
/// );
/// assert_eq!(findings.len(), 1);
/// assert_eq!(findings[0].finding.rule, "task-needs-design-dep");
/// ```
///
/// The repo's [`HierarchyConfig`] is injected here (not stored in the parsed
/// rule) and passed to any `type-hierarchy` rule during evaluation.
///
/// `repo_default_format` is the repo-level default content format used to parse
/// a source issue's criteria section (a `label-coverage` rule) when the issue
/// carries no per-issue `content_format`. The same
/// [`content_parser_for`](crate::document::content_parser_for) selector the write
/// path uses is applied here, so HTML/XML sources are parsed consistently. A
/// selected format whose parser feature is not compiled surfaces as a
/// config-error finding on the source issue (no silent Markdown fallback).
///
/// `now` is the injected clock instant against which `gate-recency` rules compute
/// each gate result's age. CLOCK INJECTION IS MANDATORY: this is the single graph
/// entry point and the only place a wall-clock instant enters graph evaluation,
/// so the engine stays pure and deterministic. Callers pass `Utc::now()` at the
/// boundary; tests pass a fixed instant.
pub fn evaluate_graph(
    rules: &[&Rule],
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    repo_default_format: ContentFormat,
    now: DateTime<Utc>,
) -> Vec<GraphFinding> {
    rules
        .iter()
        .filter(|rule| rule.scope == Scope::Graph && rule.severity != Severity::Off)
        .flat_map(|rule| evaluate_one(rule, issues, hierarchy, repo_default_format, now))
        .collect()
}

/// Evaluate a single graph rule, dispatching on its assertion kind.
fn evaluate_one(
    rule: &Rule,
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    repo_default_format: ContentFormat,
    now: DateTime<Utc>,
) -> Vec<GraphFinding> {
    match &rule.assert {
        Assertion::LabelCoverage { config } => {
            evaluate_label_coverage(rule, config, issues, repo_default_format)
        }
        Assertion::LabelReference { config } => evaluate_label_reference(rule, config, issues),
        Assertion::DependencyShape { config } => evaluate_dependency_shape(rule, config, issues),
        Assertion::GateRecency {
            max_age_hours,
            gates,
        } => evaluate_gate_recency(rule, *max_age_hours, gates, issues, now),
        Assertion::TypeHierarchy { kind } => {
            evaluate_type_hierarchy(rule, *kind, hierarchy, issues)
        }
        Assertion::CriteriaLabelMatch {
            namespace,
            criteria_section,
            marker,
            id_pattern,
        } => evaluate_criteria_label_match(
            rule,
            namespace,
            criteria_section,
            marker.as_deref(),
            id_pattern,
            issues,
            repo_default_format,
        ),
        Assertion::CriteriaToCheck {
            criteria_section,
            marker,
            id_pattern,
            gate_prefix,
            check_namespace,
        } => evaluate_criteria_to_check(
            rule,
            criteria_section,
            marker.as_deref(),
            id_pattern,
            gate_prefix.as_deref(),
            check_namespace.as_deref(),
            issues,
            repo_default_format,
        ),
        Assertion::LabelUniqueness { namespace } => {
            evaluate_label_uniqueness(rule, namespace, issues)
        }
        // Non-graph kinds are never dispatched here (filtered by scope), but be
        // exhaustive and total rather than panic.
        _ => Vec::new(),
    }
}

/// Evaluate a `label-uniqueness` rule.
///
/// Every value in the configured namespace must appear on at most one matching
/// issue. If two or more matching issues carry `namespace:<value>`, one finding
/// is produced per colliding value, naming the value and the short-ids of all
/// colliding issues.
///
/// The evaluation is a single pass: one `HashMap<value, Vec<short_id>>` is
/// built over the matching issue slice, and groups with length >= 2 are
/// reported. This is O(n * k) in issues and labels per issue — no N² scan.
///
/// # Rule-wide semantics
///
/// This kind is always `scope = "all"` (validated at load). It is therefore
/// repo-wide and must NOT run at transition time (only in `jit validate`).
/// `Assertion::is_repo_wide_at_transition` returns `true` for this variant so
/// the transition enforcer skips it automatically.
///
/// # Finding format
///
/// `"label 'req:REQ-01' is declared by multiple issues: abc1 def2"` — names
/// the value and the short-ids of all declaring issues (space-separated, in
/// the order they appear in the issue slice). One finding per colliding value.
fn evaluate_label_uniqueness(rule: &Rule, namespace: &str, issues: &[Issue]) -> Vec<GraphFinding> {
    // Single pass: collect the short-ids of every matching issue that carries
    // each value in the namespace. Using HashMap for O(1) insertion; the
    // entries are collected into a BTreeMap at the end for deterministic order.
    let mut value_to_ids: HashMap<String, Vec<String>> = HashMap::new();

    for issue in issues {
        if !rule.when.matches(issue) {
            continue;
        }
        for value in values_in_namespace(issue, namespace) {
            value_to_ids
                .entry(value.to_string())
                .or_default()
                .push(issue.short_id().to_string());
        }
    }

    // Report one finding per value owned by 2 or more issues. Sort by value
    // for deterministic output regardless of HashMap iteration order.
    let mut collisions: Vec<(String, Vec<String>)> = value_to_ids
        .into_iter()
        .filter(|(_, ids)| ids.len() >= 2)
        .collect();
    collisions.sort_by(|(a, _), (b, _)| a.cmp(b));

    collisions
        .into_iter()
        .map(|(value, ids)| {
            GraphFinding::unattributed(finding(
                rule,
                format!(
                    "label '{namespace}:{value}' is declared by multiple issues: {}",
                    ids.join(" ")
                ),
            ))
        })
        .collect()
}

/// Build a [`Finding`] carrying this rule's name and severity.
fn finding(rule: &Rule, message: String) -> Finding {
    Finding {
        rule: rule.name.clone(),
        severity: rule.severity,
        message,
    }
}

/// Build a [`GraphFinding`] attributed to `issue_id` for this rule.
fn issue_finding(rule: &Rule, issue_id: &str, message: String) -> GraphFinding {
    GraphFinding::for_issue(issue_id, finding(rule, message))
}

/// Build a `config-error` [`GraphFinding`]: a malformed/missing config key is
/// always reported (never swallowed, never a panic), attributed to the rule (not
/// a single issue, so `issue_id` is `None`).
fn config_error(rule: &Rule, message: impl Into<String>) -> GraphFinding {
    GraphFinding::unattributed(finding(
        rule,
        format!("{CONFIG_ERROR_PREFIX}{}", message.into()),
    ))
}

/// Read a required string key from a config table, or `Err(GraphFinding)`.
fn require_str<'a>(
    rule: &Rule,
    config: &'a toml::value::Table,
    key: &str,
) -> Result<&'a str, GraphFinding> {
    match config.get(key) {
        Some(toml::Value::String(s)) => Ok(s.as_str()),
        Some(_) => Err(config_error(rule, format!("key '{key}' must be a string"))),
        None => Err(config_error(rule, format!("missing required key '{key}'"))),
    }
}

/// Read an optional string key, defaulting when absent. A present non-string is
/// a config error.
fn optional_str<'a>(
    rule: &Rule,
    config: &'a toml::value::Table,
    key: &str,
    default: &'a str,
) -> Result<&'a str, GraphFinding> {
    match config.get(key) {
        None => Ok(default),
        Some(toml::Value::String(s)) => Ok(s.as_str()),
        Some(_) => Err(config_error(rule, format!("key '{key}' must be a string"))),
    }
}

// ---------------------------------------------------------------------------
// label-coverage
// ---------------------------------------------------------------------------

/// Evaluate a `label-coverage` rule.
///
/// Every canonical criterion id declared in the **container** issue's
/// success-criteria section must be satisfied by at least one related child
/// carrying the derived `satisfies:<id>` label, optionally in a required state.
/// The issues whose criteria are checked are those matching the rule's
/// [`Rule::when`] selector (the *firing* issues); by default the firing issue is
/// its own container, but `container-from-label` can redirect the criteria source
/// to another issue.
///
/// Unlike the per-issue `criteria-to-check` / `criteria-label-match` rules, this
/// rule's child search is **transitive**: it walks the dependency subtree (via
/// [`children_of`]) so a criterion satisfied by a non-sink issue deep in a
/// transitively-reduced DAG is still credited.
///
/// # Config keys
///
/// - `criteria-section` (string, optional, default `"success_criteria"`):
///   projection slug of the section whose list items hold the criteria.
/// - `marker` (string, optional): when set, only items whose text begins with
///   this marker (e.g. `"[hard]"`) are required; others are ignored.
/// - `id-pattern` (string, optional, default `"[A-Z][A-Z0-9]*-[0-9]+"`): regex
///   that extracts the criterion id from an item's text.
/// - `satisfies-namespace` (string, optional, default `"satisfies"`): label
///   namespace on children whose value is the satisfied criterion id.
/// - `child-state` (string, optional): when set, a satisfying child must be in
///   this lifecycle state (snake_case, e.g. `"done"`). **Absent means any
///   state** — there is no `"any"` token.
/// - `child-link` (string, optional, default `"dependents"`): how a child
///   relates to the container — `"dependents"`, `"dependencies"`, or `"any"`.
///   For `"dependents"` / `"dependencies"` the walk is a transitive closure in
///   that direction.
/// - `child-type-exclude` (array of strings, optional): bare `type:` names that
///   are dropped from coverage candidates AND act as a traversal boundary (the
///   walk does not descend through them).
/// - `container-from-label` (string, optional): label namespace whose value on a
///   firing issue names the criteria-bearing container, so a rule keyed on one
///   issue type can evaluate another issue's criteria (container indirection).
///
/// One finding is produced per uncovered criterion id per container. A malformed
/// config — including a firing issue whose `container-from-label` pointer cannot
/// be resolved — yields a single `config-error` finding.
fn evaluate_label_coverage(
    rule: &Rule,
    config: &toml::value::Table,
    issues: &[Issue],
    repo_default_format: ContentFormat,
) -> Vec<GraphFinding> {
    // --- Interpret config (any error short-circuits to one config-error). ---
    let section_slug =
        match optional_str(rule, config, "criteria-section", DEFAULT_CRITERIA_SECTION) {
            Ok(v) => v,
            Err(f) => return vec![f],
        };
    let marker = match config.get("marker") {
        None => None,
        Some(toml::Value::String(s)) => Some(s.as_str()),
        Some(_) => return vec![config_error(rule, "key 'marker' must be a string")],
    };
    let id_pattern_src = match optional_str(rule, config, "id-pattern", DEFAULT_ID_PATTERN) {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    let id_pattern = match regex::Regex::new(id_pattern_src) {
        Ok(re) => re,
        Err(e) => return vec![config_error(rule, format!("invalid 'id-pattern': {e}"))],
    };
    let satisfies_ns = match optional_str(
        rule,
        config,
        "satisfies-namespace",
        DEFAULT_SATISFIES_NAMESPACE,
    ) {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    let child_state = match config.get("child-state") {
        None => None,
        Some(toml::Value::String(s)) => Some(s.as_str()),
        Some(_) => return vec![config_error(rule, "key 'child-state' must be a string")],
    };
    let child_link_src = match optional_str(rule, config, "child-link", "dependents") {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    let child_link = match ChildLink::parse(child_link_src) {
        Some(c) => c,
        None => {
            return vec![config_error(
                rule,
                format!(
                    "key 'child-link' must be one of 'dependents', 'dependencies', 'any', \
                     got '{child_link_src}'"
                ),
            )]
        }
    };
    // `child-type-exclude` (optional, default empty): bare type names dropped
    // from coverage candidates AND used as the transitive-walk boundary.
    let exclude_types = match config.get("child-type-exclude") {
        None => BTreeSet::new(),
        Some(toml::Value::Array(arr)) => {
            let mut set = BTreeSet::new();
            for v in arr {
                match v {
                    toml::Value::String(s) => {
                        set.insert(s.clone());
                    }
                    _ => {
                        return vec![config_error(
                            rule,
                            "key 'child-type-exclude' must be an array of strings",
                        )]
                    }
                }
            }
            set
        }
        Some(_) => {
            return vec![config_error(
                rule,
                "key 'child-type-exclude' must be an array of strings",
            )]
        }
    };
    // `container-from-label` (optional): when set, the criteria-bearing container
    // is not the matched (firing) issue itself but the issue named by the firing
    // issue's `<namespace>:<id>` label. This lets a rule keyed on the breakdown
    // node evaluate its container's criteria (D13 container indirection).
    let container_ns = match config.get("container-from-label") {
        None => None,
        Some(toml::Value::String(s)) => Some(s.as_str()),
        Some(_) => {
            return vec![config_error(
                rule,
                "key 'container-from-label' must be a string",
            )]
        }
    };

    // A child satisfies criterion `id` if it carries `satisfies-ns:id` and (when
    // configured) is in `child-state`.
    let state_matcher = child_state.map(|s| Selector {
        state: Some(StatePredicate::Single(s.to_string())),
        ..Selector::default()
    });
    let satisfied_id = |child: &Issue, id: &str| -> bool {
        let claims = child
            .labels
            .iter()
            .any(|l| l == &format!("{satisfies_ns}:{id}"));
        let state_ok = state_matcher.as_ref().is_none_or(|sel| sel.matches(child));
        claims && state_ok
    };

    let by_id: HashMap<&str, &Issue> = issues.iter().map(|i| (i.id.as_str(), i)).collect();

    issues
        .iter()
        .filter(|source| rule.when.matches(source))
        .flat_map(|source| {
            // The criteria-bearing container is the firing issue itself, unless
            // container indirection redirects to the issue named by the firing
            // issue's `<container_ns>:<id>` label (D13). An unresolvable pointer
            // is a config error (never a silent pass).
            let container: &Issue = match container_ns {
                None => source,
                Some(ns) => {
                    let mut targets = values_in_namespace(source, ns);
                    match targets.next().and_then(|id| by_id.get(id).copied()) {
                        Some(c) => c,
                        None => {
                            return vec![config_error(
                                rule,
                                format!(
                                    "issue {} matched the rule but its '{ns}:<id>' container \
                                     pointer is missing or names no known issue",
                                    source.short_id()
                                ),
                            )]
                        }
                    }
                }
            };

            // Select the parser per container issue (content_format -> repo
            // default -> Markdown). A feature-not-compiled selection surfaces as a
            // single config-error finding rather than parsing wrongly.
            let criteria = match criterion_ids(
                container,
                section_slug,
                marker,
                &id_pattern,
                repo_default_format,
            ) {
                Ok(ids) => ids,
                Err(err) => return vec![config_error(rule, err.to_string())],
            };
            let candidates: Vec<&Issue> =
                children_of(container, issues, child_link, &exclude_types);
            criteria
                .into_iter()
                .filter(move |id| !candidates.iter().any(|child| satisfied_id(child, id)))
                .map(move |id| {
                    issue_finding(
                        rule,
                        &container.id,
                        format!(
                            "criterion '{id}' of issue {} is not satisfied by any {} child{}",
                            container.short_id(),
                            describe_link(child_link),
                            child_state
                                .map(|s| format!(" in state '{s}'"))
                                .unwrap_or_default(),
                        ),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Human-readable description of a child link for a finding message.
fn describe_link(link: ChildLink) -> &'static str {
    match link {
        ChildLink::Dependents => "dependent",
        ChildLink::Dependencies => "dependency",
        ChildLink::Any => "",
    }
}

/// Extract the canonical criterion ids from a source issue's criteria section.
///
/// The section is located by `section_slug` in the (lazily parsed) projection.
/// For each list item, if `marker` is set the item must start with it; the first
/// match of `id_pattern` in the item text is the criterion id. Ids are returned
/// de-duplicated in first-seen order.
///
/// The body is parsed with the [`ContentParser`](crate::document::ContentParser)
/// selected by [`content_parser_for`]: the source's own `content_format` → else
/// `repo_default_format` → else Markdown. A selected format whose parser feature
/// is not compiled returns
/// [`ContentParserError`](crate::document::ContentParserError) rather than a
/// silent Markdown fallback, so the caller can surface it as a finding.
fn criterion_ids(
    source: &Issue,
    section_slug: &str,
    marker: Option<&str>,
    id_pattern: &regex::Regex,
    repo_default_format: ContentFormat,
) -> Result<Vec<String>, crate::document::ContentParserError> {
    let parser = content_parser_for(source.content_format, repo_default_format)?;
    let projection = project(source).with_sections(&source.description, parser.as_ref());
    let Some(sections) = projection.sections else {
        return Ok(Vec::new());
    };
    let Some(section) = sections.get(section_slug) else {
        return Ok(Vec::new());
    };

    let mut seen = BTreeSet::new();
    Ok(section
        .items
        .iter()
        .filter(|item| marker.is_none_or(|m| item.trim_start().starts_with(m)))
        .filter_map(|item| id_pattern.find(item).map(|m| m.as_str().to_string()))
        .filter(|id| seen.insert(id.clone()))
        .collect())
}

/// Whether `issue`'s `type:` label is one of the excluded type names.
///
/// `excluded` holds bare type names (e.g. `"breakdown"`); an issue is excluded
/// when it carries `type:<name>` for any of them.
fn type_is_excluded(issue: &Issue, excluded: &BTreeSet<String>) -> bool {
    !excluded.is_empty() && values_in_namespace(issue, "type").any(|t| excluded.contains(t))
}

/// The candidate child issues for a source under the given link semantics,
/// walked **transitively** through the dependency DAG.
///
/// For the directional links the walk is a breadth-first closure: from `source`
/// it follows dependency edges in the link's direction (`Dependents` = issues
/// that depend on the frontier; `Dependencies` = issues the frontier depends on)
/// and keeps descending. This lets a `label-coverage` rule credit a satisfying
/// issue anywhere in the transitive subtree — necessary because the DAG is kept
/// transitively reduced, so a container links only to its immediate (sink)
/// children and a criterion satisfied deeper would otherwise read as uncovered.
///
/// `exclude_types` bounds the walk: a reached issue whose `type:` label is in the
/// set is **dropped from the candidates** *and* the walk **does not descend
/// through it** (it is a traversal boundary). In a bracket the boundary is the
/// `type:breakdown` node, so coverage collects exactly the impl interior between
/// the container and the breakdown node.
///
/// `ChildLink::Any` has no traversal notion — it returns every other issue as a
/// candidate — but excluded types are still dropped from the candidate set.
fn children_of<'a>(
    source: &Issue,
    issues: &'a [Issue],
    link: ChildLink,
    exclude_types: &BTreeSet<String>,
) -> Vec<&'a Issue> {
    if link == ChildLink::Any {
        return issues
            .iter()
            .filter(|i| i.id != source.id && !type_is_excluded(i, exclude_types))
            .collect();
    }

    // Index issues by id for O(1) neighbour resolution during the walk.
    let by_id: HashMap<&str, &Issue> = issues.iter().map(|i| (i.id.as_str(), i)).collect();

    // Neighbours of `node` in the link's direction.
    let neighbours = |node: &Issue| -> Vec<&'a Issue> {
        match link {
            ChildLink::Dependents => issues
                .iter()
                .filter(|i| i.dependencies.contains(&node.id))
                .collect(),
            ChildLink::Dependencies => node
                .dependencies
                .iter()
                .filter_map(|dep| by_id.get(dep.as_str()).copied())
                .collect(),
            ChildLink::Any => Vec::new(), // handled above
        }
    };

    let mut visited: BTreeSet<&str> = BTreeSet::new();
    visited.insert(source.id.as_str());
    let mut frontier: Vec<&Issue> = neighbours(source);
    let mut candidates: Vec<&Issue> = Vec::new();

    while let Some(node) = frontier.pop() {
        if !visited.insert(node.id.as_str()) {
            continue;
        }
        // An excluded-type node is neither a candidate nor a path to descend
        // through: it is the traversal boundary.
        if type_is_excluded(node, exclude_types) {
            continue;
        }
        candidates.push(node);
        frontier.extend(neighbours(node));
    }

    candidates
}

// ---------------------------------------------------------------------------
// label-reference
// ---------------------------------------------------------------------------

/// Evaluate a `label-reference` rule.
///
/// Every issue matching the rule's selector and carrying a `from`-namespace
/// reference label must have each referenced value declared as a `to`-namespace
/// source within the configured scope.
///
/// # Config keys
///
/// - `from` (string, required): namespace of the reference label whose values
///   must resolve (e.g. `"satisfies"`).
/// - `to` (string, required): namespace that DECLARES valid source ids (e.g.
///   `"req"`).
/// - `scope` (string, optional, default `"global"`): where a declaration may
///   live — `"global"` (any issue in the set) or `"linked"` (only issues this
///   issue depends on, or that depend on it).
///
/// One finding is produced per dangling reference. A malformed config yields a
/// single `config-error` finding.
fn evaluate_label_reference(
    rule: &Rule,
    config: &toml::value::Table,
    issues: &[Issue],
) -> Vec<GraphFinding> {
    let from_ns = match require_str(rule, config, "from") {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    let to_ns = match require_str(rule, config, "to") {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    let scope = match optional_str(rule, config, "scope", "global") {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    let linked = match scope {
        "global" => false,
        "linked" => true,
        other => {
            return vec![config_error(
                rule,
                format!("key 'scope' must be 'global' or 'linked', got '{other}'"),
            )]
        }
    };

    // Globally declared source ids (the `to` namespace values across all issues).
    let global_sources: BTreeSet<&str> = issues
        .iter()
        .flat_map(|i| values_in_namespace(i, to_ns))
        .collect();

    issues
        .iter()
        .filter(|issue| rule.when.matches(issue))
        .flat_map(|issue| {
            let allowed: BTreeSet<&str> = if linked {
                linked_issues(issue, issues)
                    .into_iter()
                    .flat_map(|i| values_in_namespace(i, to_ns))
                    .collect()
            } else {
                global_sources.clone()
            };
            values_in_namespace(issue, from_ns)
                .filter(move |value| !allowed.contains(value))
                .map(move |value| {
                    issue_finding(
                        rule,
                        &issue.id,
                        format!(
                            "issue {} references '{from_ns}:{value}' but no '{to_ns}:{value}' \
                             source is declared in {scope} scope",
                            issue.short_id()
                        ),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Values of an issue's labels in the given namespace.
fn values_in_namespace<'a>(issue: &'a Issue, namespace: &'a str) -> impl Iterator<Item = &'a str> {
    let prefix = format!("{namespace}:");
    issue
        .labels
        .iter()
        .filter_map(move |label| label.strip_prefix(&prefix))
}

/// Issues linked to `issue` by a dependency edge in either direction.
fn linked_issues<'a>(issue: &Issue, issues: &'a [Issue]) -> Vec<&'a Issue> {
    issues
        .iter()
        .filter(|other| {
            other.id != issue.id
                && (issue.dependencies.contains(&other.id)
                    || other.dependencies.contains(&issue.id))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// dependency-shape
// ---------------------------------------------------------------------------

/// Evaluate a `dependency-shape` rule.
///
/// Every issue matching the rule's selector must (or, for `mode = "should"`,
/// ought to) depend — directly or transitively, over the dependency DAG — on at
/// least one issue matching the `target` selector.
///
/// # Config keys
///
/// - `target` (table, required): a [`Selector`]-shaped table (`type`/`label`/
///   `state`/`has-doc-type`) identifying the issues the source must depend on.
/// - `mode` (string, optional, default `"must"`): `"must"` or `"should"`. This
///   only shapes the finding message; the finding's blocking weight comes from
///   the rule's own [`Severity`].
/// - `transitive` (boolean, optional, default `false`): when `true`, a
///   transitive dependency satisfies the rule; otherwise only a direct
///   dependency does.
///
/// One finding is produced per source issue lacking a qualifying dependency. A
/// malformed config yields a single `config-error` finding.
fn evaluate_dependency_shape(
    rule: &Rule,
    config: &toml::value::Table,
    issues: &[Issue],
) -> Vec<GraphFinding> {
    // `target` must be a table we can deserialize into a Selector.
    let target = match config.get("target") {
        Some(value @ toml::Value::Table(_)) => match value.clone().try_into::<Selector>() {
            Ok(sel) => sel,
            Err(e) => {
                return vec![config_error(
                    rule,
                    format!("invalid 'target' selector: {e}"),
                )]
            }
        },
        Some(_) => return vec![config_error(rule, "key 'target' must be a table")],
        None => return vec![config_error(rule, "missing required key 'target'")],
    };
    let mode = match optional_str(rule, config, "mode", "must") {
        Ok(v) => v,
        Err(f) => return vec![f],
    };
    if mode != "must" && mode != "should" {
        return vec![config_error(
            rule,
            format!("key 'mode' must be 'must' or 'should', got '{mode}'"),
        )];
    }
    let transitive = match config.get("transitive") {
        None => false,
        Some(toml::Value::Boolean(b)) => *b,
        Some(_) => return vec![config_error(rule, "key 'transitive' must be a boolean")],
    };

    // Set of issue ids matching the target selector.
    let target_ids: BTreeSet<&str> = issues
        .iter()
        .filter(|i| target.matches(i))
        .map(|i| i.id.as_str())
        .collect();

    let node_refs: Vec<&Issue> = issues.iter().collect();
    let graph = DependencyGraph::new(&node_refs);

    issues
        .iter()
        .filter(|source| rule.when.matches(source))
        .filter(|source| !depends_on_target(source, &target_ids, transitive, &graph))
        .map(|source| {
            issue_finding(
                rule,
                &source.id,
                format!(
                    "issue {} {} depend on an issue matching the target selector but does not",
                    source.short_id(),
                    mode
                ),
            )
        })
        .collect()
}

/// Whether `source` depends on any target id, directly or (when `transitive`)
/// through the dependency DAG.
fn depends_on_target(
    source: &Issue,
    target_ids: &BTreeSet<&str>,
    transitive: bool,
    graph: &DependencyGraph<'_, Issue>,
) -> bool {
    if source
        .dependencies
        .iter()
        .any(|d| target_ids.contains(d.as_str()))
    {
        return true;
    }
    if !transitive {
        return false;
    }
    // A transitive dependency of `source` is, in this graph, an issue for which
    // `source` is a transitive dependent.
    target_ids
        .iter()
        .any(|target| !graph.find_shortest_path(&source.id, target).is_empty())
}

// ---------------------------------------------------------------------------
// gate-recency
// ---------------------------------------------------------------------------

/// Evaluate a `gate-recency` rule.
///
/// Every issue matching the rule's selector must have a recorded result for each
/// checked gate, no older than `max_age_hours`. Age is `now - GateState.updated_at`
/// (the injected clock; never wall-clock). When `gates` is empty, the issue's own
/// `gates_required` set is checked; otherwise only the named gates are.
///
/// One finding is produced per stale or missing gate result, attributed to the
/// issue:
///
/// - missing: `gate '<key>' has no recorded result`
/// - stale: `gate '<key>' result is <N> days old (max <M>)` (rendered in days
///   when the configured max is a whole number of days, else in hours; the day
///   age is rounded UP so a stale finding's displayed age always exceeds the
///   displayed max)
///
/// This kind takes no raw config table (its payload is parsed and validated at
/// load), so it never produces a `config-error` finding.
fn evaluate_gate_recency(
    rule: &Rule,
    max_age_hours: u64,
    gates: &[String],
    issues: &[Issue],
    now: DateTime<Utc>,
) -> Vec<GraphFinding> {
    // Render ages/limits in days when the configured max is a whole number of
    // days, matching the natural `max-age-days` authoring; otherwise in hours.
    let in_days = max_age_hours.is_multiple_of(24);
    let unit = if in_days { "days" } else { "hours" };
    let limit = if in_days {
        max_age_hours / 24
    } else {
        max_age_hours
    };
    // Render the displayed age in the chosen unit. For days, round UP (ceiling):
    // a finding is only produced when `age_hours > max_age_hours`, and truncating
    // (`hours / 24`) could render an age EQUAL to the displayed max (e.g. 180h
    // vs a 7-day/168h max both show "7 days"), making a BLOCKING finding read as
    // if it were within limit. Ceiling keeps the displayed age strictly above the
    // displayed max on every stale finding.
    let age_in_unit = |hours: i64| -> i64 {
        if in_days {
            // Ceiling division. Only called on the stale branch, where
            // `hours > max_age_hours >= 0`, so `hours` is non-negative.
            (hours + 23) / 24
        } else {
            hours
        }
    };

    issues
        .iter()
        .filter(|issue| rule.when.matches(issue))
        .flat_map(|issue| {
            // Empty `gates` means "all of the issue's gates_required".
            let checked: Vec<&String> = if gates.is_empty() {
                issue.gates_required.iter().collect()
            } else {
                gates.iter().collect()
            };
            checked
                .into_iter()
                .filter_map(|gate_key| {
                    match issue.gates_status.get(gate_key) {
                        None => Some(issue_finding(
                            rule,
                            &issue.id,
                            format!("gate '{gate_key}' has no recorded result"),
                        )),
                        Some(state) => {
                            // Whole hours elapsed since the gate was recorded.
                            let age_hours = (now - state.updated_at).num_hours();
                            if age_hours > max_age_hours as i64 {
                                Some(issue_finding(
                                    rule,
                                    &issue.id,
                                    format!(
                                        "gate '{gate_key}' result is {} {unit} old (max {limit})",
                                        age_in_unit(age_hours)
                                    ),
                                ))
                            } else {
                                None
                            }
                        }
                    }
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// criteria-to-check
// ---------------------------------------------------------------------------

/// Evaluate a `criteria-to-check` rule.
///
/// Every criterion id extracted from the configured section of a matching issue
/// must map to a verifiable check. "Checked" means the issue has EITHER:
///
/// - a `gates_required` entry equal to `"<gate_prefix><id>"` (when `gate_prefix`
///   is configured), OR
/// - a label `"<check_namespace>:<id>"` (when `check_namespace` is configured).
///
/// Either mechanism alone satisfies the criterion. At least one mechanism must be
/// configured (enforced at load by the `RawCriteriaToCheck::into_assertion`
/// validator).
///
/// # Config
///
/// - `criteria_section` (default `"success_criteria"`): projection slug of the
///   section whose list items hold the criteria.
/// - `marker` (optional): when set, only items starting with this marker
///   contribute criterion ids.
/// - `id_pattern` (default `"[A-Z][A-Z0-9]*-[0-9]+"`): regex extracting the
///   criterion id from an item's text.
/// - `gate_prefix` (optional): prefix for gate keys. A criterion `id` is
///   gate-checked when `"<gate_prefix><id>"` is in `gates_required`.
/// - `check_namespace` (optional): label namespace. A criterion `id` is
///   label-checked when `"<check_namespace>:<id>"` is a label.
///
/// The finding message names exactly the missing mechanism(s). When only one
/// mechanism is configured, the message names only that one; when both are
/// configured, the message names both.
///
/// One finding per unmapped criterion per matching issue. A malformed config
/// yields a single `config-error` finding.
#[allow(clippy::too_many_arguments)]
fn evaluate_criteria_to_check(
    rule: &Rule,
    criteria_section: &str,
    marker: Option<&str>,
    id_pattern_src: &str,
    gate_prefix: Option<&str>,
    check_namespace: Option<&str>,
    issues: &[Issue],
    repo_default_format: ContentFormat,
) -> Vec<GraphFinding> {
    // Compile the id pattern (already validated at load for user-authored rules;
    // this compile step is a pure guard for programmatic callers).
    let id_pattern = match regex::Regex::new(id_pattern_src) {
        Ok(re) => re,
        Err(e) => return vec![config_error(rule, format!("invalid 'id-pattern': {e}"))],
    };

    issues
        .iter()
        .filter(|issue| rule.when.matches(issue))
        .flat_map(|issue| {
            let ids = match criterion_ids(
                issue,
                criteria_section,
                marker,
                &id_pattern,
                repo_default_format,
            ) {
                Ok(ids) => ids,
                Err(err) => return vec![config_error(rule, err.to_string())],
            };
            ids.into_iter()
                .filter(|id| !criterion_is_checked(issue, id, gate_prefix, check_namespace))
                .map(|id| {
                    let missing = describe_missing(id.as_str(), gate_prefix, check_namespace);
                    issue_finding(
                        rule,
                        &issue.id,
                        format!("criterion '{id}' has no verification: {missing}"),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether criterion `id` is considered "checked" on `issue`.
///
/// A criterion is checked when EITHER the issue has a `gates_required` entry
/// equal to `"<gate_prefix><id>"` (gate-checked) OR the issue carries a label
/// `"<check_namespace>:<id>"` (label-checked). Either mechanism alone satisfies.
fn criterion_is_checked(
    issue: &Issue,
    id: &str,
    gate_prefix: Option<&str>,
    check_namespace: Option<&str>,
) -> bool {
    let gate_checked = gate_prefix
        .map(|prefix| {
            let gate_key = format!("{prefix}{id}");
            issue.gates_required.iter().any(|g| g == &gate_key)
        })
        .unwrap_or(false);

    let label_checked = check_namespace
        .map(|ns| {
            let label = format!("{ns}:{id}");
            issue.labels.iter().any(|l| l == &label)
        })
        .unwrap_or(false);

    gate_checked || label_checked
}

/// Build the "expected …" clause of an unmapped-criterion finding, naming only
/// the configured mechanism(s).
///
/// - Both configured: `expected gate '<gate_prefix><id>' or label '<ns>:<id>'`
/// - Only gate:       `expected gate '<gate_prefix><id>'`
/// - Only label:      `expected label '<ns>:<id>'`
fn describe_missing(id: &str, gate_prefix: Option<&str>, check_namespace: Option<&str>) -> String {
    match (gate_prefix, check_namespace) {
        (Some(prefix), Some(ns)) => format!("expected gate '{prefix}{id}' or label '{ns}:{id}'"),
        (Some(prefix), None) => format!("expected gate '{prefix}{id}'"),
        (None, Some(ns)) => format!("expected label '{ns}:{id}'"),
        // At-least-one is enforced at load; this arm is unreachable in practice.
        (None, None) => "expected a verification mechanism (none configured)".to_string(),
    }
}

// ---------------------------------------------------------------------------
// type-hierarchy (orphan-leaf / strategic-consistency)
// ---------------------------------------------------------------------------

/// Evaluate a built-in `type-hierarchy` rule (orphan-leaf or
/// strategic-consistency) by REUSING the existing
/// [`crate::type_hierarchy`] domain functions over each issue, converting their
/// [`ValidationWarning`]s into [`GraphFinding`]s attributed to the issue.
///
/// This carries no hierarchy logic of its own: `OrphanLeaf` delegates to
/// [`validate_orphans`] and `StrategicConsistency` to [`validate_strategic_labels`],
/// preserving the exact legacy warnings (which were warn-only). Each warning maps
/// to one finding with the rule's severity (Warn for the built-in defaults).
fn evaluate_type_hierarchy(
    rule: &Rule,
    kind: TypeHierarchyKind,
    config: &HierarchyConfig,
    issues: &[Issue],
) -> Vec<GraphFinding> {
    let check = |issue: &Issue| -> Vec<ValidationWarning> {
        match kind {
            TypeHierarchyKind::OrphanLeaf => validate_orphans(config, issue),
            TypeHierarchyKind::StrategicConsistency => validate_strategic_labels(config, issue),
        }
    };

    issues
        .iter()
        .flat_map(|issue| {
            check(issue)
                .into_iter()
                .map(|warning| issue_finding(rule, &issue.id, warning_message(&warning)))
                .collect::<Vec<_>>()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// criteria-label-match
// ---------------------------------------------------------------------------

/// Evaluate a `criteria-label-match` rule (CC-3).
///
/// For each issue matching the rule's selector, extract criterion ids from its
/// configured section (reusing [`criterion_ids`], the same extractor as
/// `label-coverage`) and compare the issue's `namespace:<value>` labels against
/// that set. A `<value>` absent from the extracted id set is a stray or invented
/// label — one that was not derived from the canonical criteria — and yields one
/// finding per unmatched value.
///
/// This is distinct from the declared-but-unsatisfied finding that
/// `label-coverage` emits: coverage asks "is criterion X covered by a child?";
/// this rule asks "does label `req:X` name a real criterion at all?". The two
/// findings use clearly different wording so an agent can tell them apart.
///
/// # Config
///
/// Parsed and validated at load into [`Assertion::CriteriaLabelMatch`] fields;
/// this evaluator receives the resolved values directly, so it never produces a
/// `config-error` finding from missing keys (only from an invalid `id-pattern`
/// regex, caught at load).
///
/// # Finding text
///
/// For a stray label `req:REQ-77` on issue `abc123` with section `Success Criteria`:
/// `"label 'req:REQ-77' on issue abc123 names no criterion in section 'Success Criteria' (stray or invented)"`
fn evaluate_criteria_label_match(
    rule: &Rule,
    namespace: &str,
    criteria_section: &str,
    marker: Option<&str>,
    id_pattern_src: &str,
    issues: &[Issue],
    repo_default_format: ContentFormat,
) -> Vec<GraphFinding> {
    let id_pattern = match regex::Regex::new(id_pattern_src) {
        Ok(re) => re,
        Err(e) => return vec![config_error(rule, format!("invalid 'id-pattern': {e}"))],
    };

    // The section heading displayed in the finding is the human-readable form of
    // the slug: convert underscores back to spaces and title-case each word so
    // `success_criteria` renders as `Success Criteria` in the message. This is
    // an APPROXIMATION (a custom-cased heading like `QA Sign-Off` renders as
    // `Qa Sign Off`): unlike the schema engine, graph rules have no
    // x-jit-section-heading annotation to consult, and the text is cosmetic —
    // the slug uniquely identifies the section either way.
    let section_heading = slug_to_heading(criteria_section);

    issues
        .iter()
        .filter(|issue| rule.when.matches(issue))
        .flat_map(|issue| {
            let criterion_id_set: std::collections::BTreeSet<String> = match criterion_ids(
                issue,
                criteria_section,
                marker,
                &id_pattern,
                repo_default_format,
            ) {
                Ok(ids) => ids.into_iter().collect(),
                Err(err) => return vec![config_error(rule, err.to_string())],
            };

            values_in_namespace(issue, namespace)
                .filter(|value| !criterion_id_set.contains(*value))
                .map(|value| {
                    issue_finding(
                        rule,
                        &issue.id,
                        format!(
                            "label '{namespace}:{value}' on issue {} names no criterion in \
                             section '{section_heading}' (stray or invented)",
                            issue.short_id()
                        ),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Convert a section slug back to a human-readable heading for use in finding
/// messages. Replaces underscores with spaces and title-cases each word.
/// `success_criteria` -> `Success Criteria`.
fn slug_to_heading(slug: &str) -> String {
    slug.split('_')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Render a [`ValidationWarning`] into a finding message, preserving the legacy
/// wording so `jit validate` surfaces the same text through the rule engine.
fn warning_message(warning: &ValidationWarning) -> String {
    match warning {
        ValidationWarning::MissingStrategicLabel {
            issue_id,
            type_name,
            expected_namespace,
        } => format!(
            "issue {issue_id} (type:{type_name}) is missing a {expected_namespace}:* \
             identifying label"
        ),
        ValidationWarning::OrphanedLeaf {
            issue_id,
            type_name,
        } => format!(
            "issue {issue_id} (type:{type_name}) is an orphaned leaf with no parent \
             association label"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::State;
    use crate::validation::rules::RuleSet;
    use std::path::Path;

    fn rule_from(toml: &str) -> Rule {
        let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
        set.rules.into_iter().next().unwrap()
    }

    fn issue(title: &str, labels: &[&str]) -> Issue {
        let mut i = Issue::new(title.to_string(), String::new());
        i.labels = labels.iter().map(|s| s.to_string()).collect();
        i
    }

    /// A fixed clock instant for deterministic graph evaluation. Rules other than
    /// `gate-recency` ignore it; recency tests subtract from it explicitly.
    fn fixed_now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap()
    }

    // --- label-coverage ----------------------------------------------------

    fn coverage_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"coverage\"\nwhen = {{ type = \"epic\" }}\n\
             severity = \"error\"\nassert = {{ label-coverage = {{ {extra} }} }}\n"
        ))
    }

    fn epic_with_criteria(ids: &[&str]) -> Issue {
        let body = format!(
            "## Success Criteria\n\n{}\n",
            ids.iter()
                .map(|id| format!("- [hard] {id}: do the thing"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut epic = Issue::new("epic".to_string(), body);
        epic.labels = vec!["type:epic".to_string()];
        epic
    }

    #[test]
    fn test_label_coverage_satisfied_by_child() {
        let rule = coverage_rule("child-state = \"done\"");
        let epic = epic_with_criteria(&["REQ-01"]);
        let mut child = issue("child", &["satisfies:REQ-01"]);
        child.dependencies = vec![epic.id.clone()];
        child.state = State::Done;

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "covered criterion: {findings:?}");
    }

    #[test]
    fn test_label_coverage_unsatisfied_reports_finding() {
        let rule = coverage_rule("child-state = \"done\"");
        let epic = epic_with_criteria(&["REQ-01", "REQ-02"]);
        let mut child = issue("child", &["satisfies:REQ-01"]);
        child.dependencies = vec![epic.id.clone()];
        child.state = State::Done;

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        // REQ-02 is uncovered.
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("REQ-02"));
        assert_eq!(findings[0].finding.severity, Severity::Error);
        assert_eq!(findings[0].finding.rule, "coverage");
    }

    #[test]
    fn test_label_coverage_wrong_state_is_uncovered() {
        let rule = coverage_rule("child-state = \"done\"");
        let epic = epic_with_criteria(&["REQ-01"]);
        let mut child = issue("child", &["satisfies:REQ-01"]);
        child.dependencies = vec![epic.id.clone()];
        child.state = State::InProgress; // not done

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "wrong-state child does not cover");
    }

    #[test]
    fn test_label_coverage_marker_filters_criteria() {
        // Only [hard] criteria are required; an [aspirational] one is ignored.
        let rule = coverage_rule("marker = \"[hard]\"");
        let body = "## Success Criteria\n\n- [hard] REQ-01: must\n- [aspirational] REQ-99: nice\n";
        let mut epic = Issue::new("epic".to_string(), body.to_string());
        epic.labels = vec!["type:epic".to_string()];
        let mut child = issue("child", &["satisfies:REQ-01"]);
        child.dependencies = vec![epic.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "aspirational criterion must not be required: {findings:?}"
        );
    }

    #[test]
    fn test_label_coverage_any_link_ignores_dependency_edges() {
        let rule = coverage_rule("child-link = \"any\"");
        let epic = epic_with_criteria(&["REQ-01"]);
        // Child has NO dependency edge to the epic, but child-link=any.
        let child = issue("child", &["satisfies:REQ-01"]);

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "any link covers regardless of edges");
    }

    #[test]
    fn test_label_coverage_malformed_config_is_config_error() {
        let rule = coverage_rule("child-link = \"bogus\"");
        let epic = epic_with_criteria(&["REQ-01"]);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("child-link"));
    }

    // --- label-coverage: transitive closure (T3) ---------------------------

    /// Build a `type:epic` issue with `[hard]` criteria but no other labels, so
    /// callers can chain a `dependencies` spine of arbitrary depth beneath it.
    fn epic_chain_head(ids: &[&str]) -> Issue {
        epic_with_criteria(ids)
    }

    #[test]
    fn test_label_coverage_credits_non_sink_via_transitive_walk() {
        // Spine: epic ──dep→ sink ──dep→ deep. Under transitive reduction the
        // epic links only to `sink`; the criterion is satisfied by `deep` (a
        // non-sink, deeper in the subtree). Direct-adjacency coverage would miss
        // it; the transitive walk must credit it.
        let rule = coverage_rule("child-link = \"dependencies\"");
        let mut epic = epic_chain_head(&["REQ-01"]);
        let mut sink = issue("sink", &["type:task"]);
        let deep = issue("deep", &["type:task", "satisfies:REQ-01"]);
        // epic depends on sink; sink depends on deep.
        epic.dependencies = vec![sink.id.clone()];
        sink.dependencies = vec![deep.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, sink, deep],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "deep non-sink child must cover via transitive walk: {findings:?}"
        );
    }

    #[test]
    fn test_label_coverage_transitive_uncovered_still_reports() {
        // Same chain shape, but nobody satisfies REQ-01 — must still report.
        let rule = coverage_rule("child-link = \"dependencies\"");
        let mut epic = epic_chain_head(&["REQ-01"]);
        let mut sink = issue("sink", &["type:task"]);
        let deep = issue("deep", &["type:task"]); // no satisfies label
        epic.dependencies = vec![sink.id.clone()];
        sink.dependencies = vec![deep.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, sink, deep],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "uncovered criterion reports: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_label_coverage_dependents_walk_is_transitive() {
        // The default `dependents` link also becomes transitive: epic <-dep- a
        // <-dep- b, with b (a transitive dependent) satisfying the criterion.
        let rule = coverage_rule(""); // default child-link = dependents
        let epic = epic_chain_head(&["REQ-01"]);
        let mut a = issue("a", &["type:task"]);
        let mut b = issue("b", &["type:task", "satisfies:REQ-01"]);
        a.dependencies = vec![epic.id.clone()]; // a depends on epic
        b.dependencies = vec![a.id.clone()]; // b depends on a (so transitively on epic)

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, a, b],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "transitive dependent must cover: {findings:?}"
        );
    }

    // --- label-coverage: child-type-exclude (T3) ---------------------------

    #[test]
    fn test_child_type_exclude_drops_candidate_and_halts_walk() {
        // Bracket spine: epic ──dep→ impl ──dep→ B(type:breakdown) ──dep→ P.
        // P (beyond the boundary) satisfies REQ-01, but B is excluded, so the
        // walk must halt at B and never reach P -> uncovered.
        let rule =
            coverage_rule("child-link = \"dependencies\", child-type-exclude = [\"breakdown\"]");
        let mut epic = epic_chain_head(&["REQ-01"]);
        let mut impl_node = issue("impl", &["type:task"]);
        let mut breakdown = issue("B", &["type:breakdown"]);
        let plan = issue("P", &["type:planning", "satisfies:REQ-01"]);
        epic.dependencies = vec![impl_node.id.clone()];
        impl_node.dependencies = vec![breakdown.id.clone()];
        breakdown.dependencies = vec![plan.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, impl_node, breakdown, plan],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "walk must halt at excluded breakdown, leaving REQ-01 uncovered: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_child_type_exclude_credits_interior_before_boundary() {
        // Same spine, but the impl interior node (before B) satisfies REQ-01.
        // Excluding `breakdown` must NOT drop the impl node -> covered.
        let rule =
            coverage_rule("child-link = \"dependencies\", child-type-exclude = [\"breakdown\"]");
        let mut epic = epic_chain_head(&["REQ-01"]);
        let mut impl_node = issue("impl", &["type:task", "satisfies:REQ-01"]);
        let breakdown = issue("B", &["type:breakdown"]);
        epic.dependencies = vec![impl_node.id.clone()];
        impl_node.dependencies = vec![breakdown.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, impl_node, breakdown],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "impl interior covers; only the breakdown boundary is excluded: {findings:?}"
        );
    }

    #[test]
    fn test_child_type_exclude_must_be_array_of_strings() {
        let rule = coverage_rule("child-type-exclude = \"breakdown\"");
        let epic = epic_chain_head(&["REQ-01"]);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("child-type-exclude"));
    }

    // --- label-coverage: container indirection via brackets: (T3/D13) ------

    #[test]
    fn test_container_indirection_evaluates_container_criteria() {
        // Preview instance: rule keyed on type:breakdown resolves its container
        // from `brackets:<C-id>` and evaluates C's criteria coverage. C carries
        // the criteria; B carries no criteria of its own.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let mut container = epic_with_criteria(&["REQ-01"]);
        let mut impl_node = issue("impl", &["type:task", "satisfies:REQ-01"]);
        let mut breakdown = issue("B", &["type:breakdown"]);
        // Container depends on impl (its subtree); B brackets the container.
        container.dependencies = vec![impl_node.id.clone()];
        breakdown.labels.push(format!("brackets:{}", container.id));
        // B sits on the spine after impl, but the walk runs from the container.
        impl_node.dependencies = vec![breakdown.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "container criterion satisfied by impl child via indirection: {findings:?}"
        );
    }

    #[test]
    fn test_container_indirection_reports_container_uncovered() {
        // Same indirection, but C's criterion is unsatisfied -> finding fires on
        // the breakdown node's evaluation.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let mut container = epic_with_criteria(&["REQ-01"]);
        let impl_node = issue("impl", &["type:task"]); // does NOT satisfy
        let mut breakdown = issue("B", &["type:breakdown"]);
        container.dependencies = vec![impl_node.id.clone()];
        breakdown.labels.push(format!("brackets:{}", container.id));

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "uncovered container criterion: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_container_indirection_omitted_child_state_means_any_state() {
        // The preview instance omits child-state, so a Backlog (drafted) child
        // counts. Proves the absent-state = any-state semantics under indirection.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let mut container = epic_with_criteria(&["REQ-01"]);
        let mut impl_node = issue("impl", &["type:task", "satisfies:REQ-01"]);
        impl_node.state = State::Backlog; // drafted, not done
        let mut breakdown = issue("B", &["type:breakdown"]);
        container.dependencies = vec![impl_node.id.clone()];
        breakdown.labels.push(format!("brackets:{}", container.id));

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "backlog child counts when child-state is omitted: {findings:?}"
        );
    }

    #[test]
    fn test_container_indirection_unresolvable_label_is_config_error() {
        // The breakdown node carries no brackets: label, so the container cannot
        // be resolved -> a config-error finding (never a silent pass).
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             container-from-label = \"brackets\" } }\n",
        );
        let breakdown = issue("B", &["type:breakdown"]); // no brackets: label
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[breakdown],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("brackets"));
    }

    // --- label-reference ---------------------------------------------------

    fn reference_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"reference\"\nseverity = \"warn\"\n\
             assert = {{ label-reference = {{ {extra} }} }}\n"
        ))
    }

    #[test]
    fn test_label_reference_resolves() {
        let rule = reference_rule("from = \"satisfies\", to = \"req\"");
        let source = issue("epic", &["req:REQ-01"]);
        let child = issue("child", &["satisfies:REQ-01"]);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[source, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "resolved reference: {findings:?}");
    }

    #[test]
    fn test_label_reference_dangles() {
        let rule = reference_rule("from = \"satisfies\", to = \"req\"");
        let source = issue("epic", &["req:REQ-01"]);
        let child = issue("child", &["satisfies:REQ-99"]); // no req:REQ-99 anywhere
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[source, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("REQ-99"));
        assert_eq!(findings[0].finding.severity, Severity::Warn);
    }

    #[test]
    fn test_label_reference_linked_scope_requires_edge() {
        let rule = reference_rule("from = \"satisfies\", to = \"req\", scope = \"linked\"");
        // Declaring issue exists globally but is NOT linked to the child.
        let declarer = issue("epic", &["req:REQ-01"]);
        let child = issue("child", &["satisfies:REQ-01"]); // no dependency edge
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[declarer, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "linked scope: unlinked source dangles");

        // Now add the edge: the reference resolves.
        let declarer = issue("epic", &["req:REQ-01"]);
        let mut child = issue("child", &["satisfies:REQ-01"]);
        child.dependencies = vec![declarer.id.clone()];
        let findings = evaluate_graph(
            &rules,
            &[declarer, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "linked edge resolves: {findings:?}");
    }

    #[test]
    fn test_label_reference_missing_key_is_config_error() {
        let rule = reference_rule("from = \"satisfies\""); // missing `to`
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[issue("x", &["satisfies:REQ-01"])],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("'to'"));
    }

    // --- dependency-shape --------------------------------------------------

    fn shape_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"shape\"\nwhen = {{ type = \"task\" }}\nseverity = \"error\"\n\
             assert = {{ dependency-shape = {{ {extra} }} }}\n"
        ))
    }

    #[test]
    fn test_dependency_shape_satisfied() {
        let rule = shape_rule("target = { type = \"design\" }");
        let design = issue("design", &["type:design"]);
        let mut task = issue("task", &["type:task"]);
        task.dependencies = vec![design.id.clone()];
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[design, task],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "task depends on design: {findings:?}");
    }

    #[test]
    fn test_dependency_shape_violated() {
        let rule = shape_rule("target = { type = \"design\" }");
        let design = issue("design", &["type:design"]);
        let task = issue("task", &["type:task"]); // no dependency
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[design, task],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("depend"));
        assert_eq!(findings[0].finding.rule, "shape");
    }

    #[test]
    fn test_dependency_shape_transitive() {
        // task -> mid -> design; only satisfied when transitive = true.
        let design = issue("design", &["type:design"]);
        let mut mid = issue("mid", &["type:other"]);
        mid.dependencies = vec![design.id.clone()];
        let mut task = issue("task", &["type:task"]);
        task.dependencies = vec![mid.id.clone()];

        let direct = shape_rule("target = { type = \"design\" }");
        let rules = vec![&direct];
        let findings = evaluate_graph(
            &rules,
            &[design.clone(), mid.clone(), task.clone()],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "direct-only must not see transitive dep");

        let trans = shape_rule("target = { type = \"design\" }, transitive = true");
        let rules = vec![&trans];
        let findings = evaluate_graph(
            &rules,
            &[design, mid, task],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "transitive dep satisfies: {findings:?}"
        );
    }

    #[test]
    fn test_dependency_shape_missing_target_is_config_error() {
        let rule = shape_rule("mode = \"must\""); // no target
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[issue("task", &["type:task"])],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("target"));
    }

    // --- gate-recency ------------------------------------------------------

    use crate::domain::{GateState, GateStatus};

    fn recency_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"recency\"\nwhen = {{ state = \"done\" }}\n\
             severity = \"error\"\nassert = {{ gate-recency = {{ {extra} }} }}\n"
        ))
    }

    /// A done issue requiring `gate`, last recorded `hours_ago` before [`fixed_now`].
    fn issue_with_gate(gate: &str, hours_ago: i64) -> Issue {
        let mut i = issue("work", &[]);
        i.state = State::Done;
        i.gates_required = vec![gate.to_string()];
        i.gates_status.insert(
            gate.to_string(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("ci:x".to_string()),
                updated_at: fixed_now() - chrono::Duration::hours(hours_ago),
            },
        );
        i
    }

    #[test]
    fn test_gate_recency_fresh_result_passes() {
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        // Recorded 1 day ago — well within 7 days.
        let i = issue_with_gate("code-review", 24);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "fresh gate must pass: {findings:?}");
    }

    #[test]
    fn test_gate_recency_stale_result_reports_age_in_days() {
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        // Recorded 10 days ago — exceeds 7.
        let i = issue_with_gate("code-review", 10 * 24);
        let id = i.id.clone();
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].issue_id.as_deref(), Some(id.as_str()));
        assert_eq!(
            findings[0].finding.message,
            "gate 'code-review' result is 10 days old (max 7)"
        );
        assert_eq!(findings[0].finding.severity, Severity::Error);
    }

    #[test]
    fn test_gate_recency_missing_result_reports_missing() {
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        // Issue requires the gate but has no recorded status.
        let mut i = issue("work", &[]);
        i.state = State::Done;
        i.gates_required = vec!["code-review".to_string()];
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].finding.message,
            "gate 'code-review' has no recorded result"
        );
    }

    #[test]
    fn test_gate_recency_default_gates_checks_all_required() {
        // No `gates` filter -> every gate in gates_required is checked.
        let rule = recency_rule("max-age-days = 7");
        let mut i = issue_with_gate("code-review", 10 * 24); // stale
                                                             // A second required gate with no recorded result.
        i.gates_required.push("security".to_string());
        let rules = vec![&rule];
        let mut findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        findings.sort_by(|a, b| a.finding.message.cmp(&b.finding.message));
        assert_eq!(
            findings.len(),
            2,
            "both required gates checked: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("code-review"));
        assert!(findings[1].finding.message.contains("security"));
    }

    #[test]
    fn test_gate_recency_named_gate_not_required_is_missing() {
        // A named gate the issue does not even require is reported as missing.
        let rule = recency_rule("max-age-hours = 12, gates = [\"code-review\"]");
        let mut i = issue("work", &[]);
        i.state = State::Done;
        // No gates_required, no gates_status.
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].finding.message,
            "gate 'code-review' has no recorded result"
        );
    }

    #[test]
    fn test_gate_recency_hours_unit_in_message() {
        // A sub-day max renders age and limit in hours.
        let rule = recency_rule("max-age-hours = 12, gates = [\"code-review\"]");
        let i = issue_with_gate("code-review", 30); // 30h old > 12h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].finding.message,
            "gate 'code-review' result is 30 hours old (max 12)"
        );
    }

    #[test]
    fn test_gate_recency_is_deterministic_under_injected_clock() {
        // Same inputs + same `now` -> identical findings (no wall-clock read).
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        let i = issue_with_gate("code-review", 10 * 24);
        let rules = vec![&rule];
        let a = evaluate_graph(
            &rules,
            std::slice::from_ref(&i),
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        let b = evaluate_graph(
            &rules,
            std::slice::from_ref(&i),
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(a, b);
    }

    #[test]
    fn test_gate_recency_stale_age_ceiling_renders_above_max() {
        // M2 regression: 180h with a 7-day (168h) max is stale, but truncating
        // (180 / 24 = 7) would render "7 days old (max 7)" — a blocking finding
        // whose displayed age equals the displayed max. Ceiling division renders
        // the age as 8 so it is strictly greater than the max.
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        let i = issue_with_gate("code-review", 180); // 180h > 168h, < 192h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].finding.message, "gate 'code-review' result is 8 days old (max 7)",
            "stale age must render strictly above the max (ceiling), not equal to it"
        );
    }

    #[test]
    fn test_gate_recency_exact_max_age_days_is_fresh() {
        // Exact boundary: age == max-age (7 days == 168h) is NOT stale (the check
        // is strictly greater-than), so no finding is produced.
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        let i = issue_with_gate("code-review", 7 * 24); // exactly 168h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "a gate at exactly max-age is fresh: {findings:?}"
        );
    }

    #[test]
    fn test_gate_recency_just_over_max_age_days_is_stale_with_age_above_max() {
        // Just over the boundary: 169h > 168h is stale, and the rendered age (8)
        // exceeds the rendered max (7).
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        let i = issue_with_gate("code-review", 7 * 24 + 1); // 169h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(
            findings[0].finding.message,
            "gate 'code-review' result is 8 days old (max 7)"
        );
    }

    #[test]
    fn test_gate_recency_exact_max_age_hours_is_fresh() {
        // N3 boundary (hours unit): age == max-age-hours is fresh, no finding.
        let rule = recency_rule("max-age-hours = 12, gates = [\"code-review\"]");
        let i = issue_with_gate("code-review", 12); // exactly 12h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "a gate at exactly max-age-hours is fresh: {findings:?}"
        );
    }

    #[test]
    fn test_gate_recency_vacuous_pass_no_gates() {
        // N2 vacuous pass: an issue with no `gates_required` and an empty `gates`
        // filter has nothing to check, so it yields zero findings.
        let rule = recency_rule("max-age-days = 7");
        let mut i = issue("work", &[]);
        i.state = State::Done;
        // No gates_required, no gates_status.
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "an issue with no gates produces no recency findings: {findings:?}"
        );
    }

    // --- dispatch / scope filtering ----------------------------------------

    #[test]
    fn test_evaluate_graph_skips_local_and_off_rules() {
        // A local rule must be ignored even if passed in.
        let local = rule_from(
            "[[rules]]\nname = \"local\"\nassert = { require-label = { label = \"type:*\" } }\n",
        );
        // An `off` graph rule must be skipped.
        let off = rule_from(
            "[[rules]]\nname = \"off\"\nseverity = \"off\"\n\
             assert = { dependency-shape = { target = { type = \"design\" } } }\n",
        );
        let rules = vec![&local, &off];
        let findings = evaluate_graph(
            &rules,
            &[issue("task", &["type:task"])],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(findings.is_empty(), "local + off rules produce nothing");
    }

    // --- type-hierarchy (injected HierarchyConfig) -------------------------

    // --- criteria-to-check -------------------------------------------------

    fn ctc_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"ctc\"\nwhen = {{ type = \"epic\" }}\n\
             severity = \"error\"\nassert = {{ criteria-to-check = {{ {extra} }} }}\n"
        ))
    }

    /// Epic with a Success Criteria section carrying the given criterion lines.
    fn epic_with_sc(items: &[&str]) -> Issue {
        let body = format!(
            "## Success Criteria\n\n{}\n",
            items
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut epic = Issue::new("epic".to_string(), body);
        epic.labels = vec!["type:epic".to_string()];
        epic
    }

    #[test]
    fn test_criteria_to_check_gate_mapped_id_passes() {
        // A criterion satisfied via a gates_required entry must produce no finding.
        let rule = ctc_rule("gate-prefix = \"verify:\"");
        let mut epic = epic_with_sc(&["[hard] REQ-01: do the thing"]);
        epic.gates_required = vec!["verify:REQ-01".to_string()];
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "gate-mapped criterion must produce no finding: {findings:?}"
        );
    }

    #[test]
    fn test_criteria_to_check_label_mapped_id_passes() {
        // A criterion satisfied via a label must produce no finding.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let mut epic = epic_with_sc(&["REQ-01: do the thing"]);
        epic.labels.push("checks:REQ-01".to_string());
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "label-mapped criterion must produce no finding: {findings:?}"
        );
    }

    #[test]
    fn test_criteria_to_check_unmapped_id_reports_finding() {
        // An unmapped criterion must be reported with its id in the message.
        let rule = ctc_rule("gate-prefix = \"verify:\", check-namespace = \"checks\"");
        let epic = epic_with_sc(&["REQ-01: do the thing"]);
        let id = epic.id.clone();
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "one unmapped criterion: {findings:?}");
        assert_eq!(findings[0].issue_id.as_deref(), Some(id.as_str()));
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("REQ-01"),
            "finding must name the criterion id: {msg}"
        );
        assert!(
            msg.contains("verify:REQ-01"),
            "finding must name the expected gate: {msg}"
        );
        assert!(
            msg.contains("checks:REQ-01"),
            "finding must name the expected label: {msg}"
        );
    }

    #[test]
    fn test_criteria_to_check_marker_filtering() {
        // Only [hard] items are required when marker = "[hard]".
        let rule = ctc_rule("marker = \"[hard]\", gate-prefix = \"verify:\"");
        let epic = epic_with_sc(&[
            "[hard] REQ-01: must do",
            "[aspirational] REQ-02: nice to have",
        ]);
        // REQ-01 is unmapped; REQ-02 is [aspirational] and must be ignored.
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "only [hard] REQ-01 should be reported: {findings:?}"
        );
        assert!(
            findings[0].finding.message.contains("REQ-01"),
            "finding must name REQ-01: {:?}",
            findings[0].finding.message
        );
        assert!(
            !findings[0].finding.message.contains("REQ-02"),
            "REQ-02 is aspirational and must not be required: {:?}",
            findings[0].finding.message
        );
    }

    #[test]
    fn test_criteria_to_check_only_gate_mechanism_message() {
        // When only gate-prefix is configured the finding names only the gate.
        let rule = ctc_rule("gate-prefix = \"verify:\"");
        let epic = epic_with_sc(&["REQ-01: do the thing"]);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("verify:REQ-01"),
            "gate-only finding must name the gate: {msg}"
        );
        assert!(
            !msg.contains("or label"),
            "gate-only finding must not mention label: {msg}"
        );
    }

    #[test]
    fn test_criteria_to_check_only_label_mechanism_message() {
        // When only check-namespace is configured the finding names only the label.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let epic = epic_with_sc(&["REQ-01: do the thing"]);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("checks:REQ-01"),
            "label-only finding must name the label: {msg}"
        );
        assert!(
            !msg.contains("or gate"),
            "label-only finding must not mention gate: {msg}"
        );
    }

    #[test]
    fn test_criteria_to_check_missing_both_mechanisms_is_config_error() {
        // At least one of gate-prefix / check-namespace is required.
        // The loader enforces this; we verify it surfaces as a load error.
        let toml = "[[rules]]\nname = \"bad\"\n\
                   assert = { criteria-to-check = {} }\n";
        let err = RuleSet::from_toml_str(toml, std::path::Path::new("/x")).unwrap_err();
        match err {
            crate::validation::rules::RuleConfigError::InvalidAssertion { rule, message } => {
                assert_eq!(rule, "bad");
                assert!(
                    message.contains("gate-prefix") && message.contains("check-namespace"),
                    "error must name both missing fields: {message}"
                );
            }
            other => panic!("expected InvalidAssertion, got {other:?}"),
        }
    }

    #[test]
    fn test_criteria_to_check_stays_per_issue_ignores_children() {
        // criteria-to-check maps a criterion to the SAME issue's gate/label and
        // must NOT traverse children. A child carrying the mapping does not
        // satisfy the parent's criterion -> the parent still reports.
        let rule = ctc_rule("gate-prefix = \"verify:\", check-namespace = \"checks\"");
        let epic = epic_with_sc(&["REQ-01: do the thing"]);
        // A dependent child carries the would-be mapping; per-issue semantics
        // mean this is irrelevant to the epic.
        let mut child = issue("child", &["type:task", "checks:REQ-01"]);
        child.dependencies = vec![epic.id.clone()];
        child.gates_required = vec!["verify:REQ-01".to_string()];
        let epic_id = epic.id.clone();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "criteria-to-check is per-issue; a child mapping must not cover: {findings:?}"
        );
        assert_eq!(findings[0].issue_id.as_deref(), Some(epic_id.as_str()));
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_type_hierarchy_orphan_leaf_fires_with_injected_config() {
        // A `type-hierarchy` rule authored in TOML carries only its kind; the
        // HierarchyConfig is injected by `evaluate_graph`. A leaf task with no
        // parent association label is flagged as an orphan.
        let rule = rule_from(
            "[[rules]]\nname = \"default:orphan-leaf\"\nseverity = \"warn\"\n\
             assert = { type-hierarchy = { kind = \"orphan-leaf\" } }\n",
        );
        let rules = vec![&rule];
        let task = issue("task", &["type:task"]);
        let findings = evaluate_graph(
            &rules,
            std::slice::from_ref(&task),
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "orphan leaf must fire: {findings:?}");
        assert_eq!(findings[0].finding.rule, "default:orphan-leaf");
        assert_eq!(findings[0].issue_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(findings[0].finding.severity, Severity::Warn);
    }

    // --- criteria-label-match -----------------------------------------------

    fn clm_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"clm\"\nwhen = {{ type = \"epic\" }}\n\
             severity = \"error\"\nassert = {{ criteria-label-match = {{ {extra} }} }}\n"
        ))
    }

    /// An epic with `## Success Criteria` items shaped `[hard] <id>: text`.
    fn epic_with_clm_criteria(ids: &[&str]) -> Issue {
        let body = format!(
            "## Success Criteria\n\n{}\n",
            ids.iter()
                .map(|id| format!("- [hard] {id}: do the thing"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut epic = Issue::new("epic".to_string(), body);
        epic.labels = vec!["type:epic".to_string()];
        epic
    }

    #[test]
    fn test_criteria_label_match_stray_label_yields_finding() {
        // A `req:REQ-77` label on an epic whose Success Criteria has only REQ-01
        // is stray: no matching criterion id -> finding with the stray message.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut epic = epic_with_clm_criteria(&["REQ-01"]);
        epic.labels.push("req:REQ-77".to_string()); // stray
        epic.labels.push("req:REQ-01".to_string()); // matches criterion

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic.clone()],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        // REQ-77 is stray; REQ-01 matches.
        assert_eq!(
            findings.len(),
            1,
            "only the stray id must yield a finding: {findings:?}"
        );
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("req:REQ-77"),
            "finding must name the stray label: {msg}"
        );
        assert!(
            msg.contains("names no criterion"),
            "finding must say 'names no criterion': {msg}"
        );
        assert!(
            msg.contains("stray or invented"),
            "finding must say 'stray or invented': {msg}"
        );
        assert!(
            msg.contains("Success Criteria"),
            "finding must name the section: {msg}"
        );
        assert_eq!(findings[0].finding.severity, Severity::Error);
        assert_eq!(
            findings[0].issue_id.as_deref(),
            Some(epic.id.as_str()),
            "finding must be attributed to the epic"
        );
    }

    #[test]
    fn test_criteria_label_match_matched_label_produces_no_finding() {
        // A `req:REQ-01` on an epic whose criteria contain REQ-01 is fine.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut epic = epic_with_clm_criteria(&["REQ-01"]);
        epic.labels.push("req:REQ-01".to_string());

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "matched label must produce no finding: {findings:?}"
        );
    }

    #[test]
    fn test_criteria_label_match_id_format_mismatch_is_stray() {
        // Exact string comparison: criterion text `REQ-03` vs label `req:REQ-3`
        // are NOT equal, so `req:REQ-3` is a stray (no normalization).
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut epic = epic_with_clm_criteria(&["REQ-03"]);
        epic.labels.push("req:REQ-3".to_string()); // differs by leading zero

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "REQ-3 vs REQ-03 must be a stray (exact compare): {findings:?}"
        );
        assert!(findings[0].finding.message.contains("req:REQ-3"));
    }

    #[test]
    fn test_criteria_label_match_marker_filters_ids() {
        // With marker = "[hard]", an id only on an unmarked item is NOT in the
        // extracted id set, so a label matching it is a stray.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        // Body: REQ-01 is [hard]; REQ-99 is [aspirational] (no marker match).
        let body = "## Success Criteria\n\n\
                    - [hard] REQ-01: required\n\
                    - [aspirational] REQ-99: nice-to-have\n";
        let mut epic = Issue::new("epic".to_string(), body.to_string());
        epic.labels = vec![
            "type:epic".to_string(),
            "req:REQ-01".to_string(), // matches [hard] criterion -> not stray
            "req:REQ-99".to_string(), // only on [aspirational] item -> stray under marker filter
        ];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "label matching only an unmarked item must be stray when marker is set: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("req:REQ-99"));
    }

    #[test]
    fn test_criteria_label_match_custom_section_name_in_finding() {
        // A non-default criteria-section name is reflected in the finding text.
        let rule = clm_rule(r#"namespace = "req", criteria-section = "hard_requirements""#);
        let body = "## Hard Requirements\n\n- REQ-01: do it\n";
        let mut epic = Issue::new("epic".to_string(), body.to_string());
        epic.labels = vec!["type:epic".to_string(), "req:REQ-77".to_string()]; // stray

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "stray id must be reported: {findings:?}");
        let msg = &findings[0].finding.message;
        // The section slug `hard_requirements` must render as `Hard Requirements`.
        assert!(
            msg.contains("Hard Requirements"),
            "finding must name the section as its human-readable heading: {msg}"
        );
    }

    #[test]
    fn test_criteria_label_match_no_namespace_labels_produces_no_finding() {
        // An issue with no `req:*` labels at all has nothing to check.
        let rule = clm_rule(r#"namespace = "req""#);
        let epic = epic_with_clm_criteria(&["REQ-01"]); // labels: just type:epic

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "no namespace labels -> no findings: {findings:?}"
        );
    }

    #[test]
    fn test_criteria_label_match_stays_per_issue_ignores_children() {
        // criteria-label-match does same-issue stray-label detection. A child's
        // labels must not influence the parent's evaluation: a stray label on the
        // epic still fires, and a child carrying the criterion id does not make
        // the parent's matching label "non-stray" via traversal (there is no
        // traversal). Here the stray is on the epic itself.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut epic = epic_with_clm_criteria(&["REQ-01"]);
        epic.labels.push("req:REQ-77".to_string()); // stray on the epic
        let epic_id = epic.id.clone();
        // A child that happens to carry req:REQ-77 must NOT silence the epic's
        // stray finding (per-issue: the child is never consulted).
        let mut child = issue("child", &["type:task", "req:REQ-77"]);
        child.dependencies = vec![epic.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, child],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(
            findings.len(),
            1,
            "criteria-label-match is per-issue; a child must not affect it: {findings:?}"
        );
        assert_eq!(findings[0].issue_id.as_deref(), Some(epic_id.as_str()));
        assert!(findings[0].finding.message.contains("req:REQ-77"));
    }

    fn uniqueness_rule(namespace: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"uniqueness\"\nseverity = \"error\"\n\
             assert = {{ label-uniqueness = {{ namespace = \"{namespace}\", scope = \"all\" }} }}\n"
        ))
    }

    #[test]
    fn test_label_uniqueness_collision_across_two_unlinked_issues() {
        // Two unlinked epics both declaring req:REQ-01 — one finding naming the
        // value and both short-ids.
        let rule = uniqueness_rule("req");
        let mut epic_a = issue("epic-a", &["type:epic", "req:REQ-01"]);
        let epic_b = issue("epic-b", &["type:epic", "req:REQ-01"]);
        // Deliberately no dependency edge between them.
        let id_a = epic_a.short_id().to_string();
        let id_b = epic_b.short_id().to_string();
        // Set distinct short ids so the message is testable.
        epic_a.id = format!("aaa-{}", epic_a.id);

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic_a, epic_b],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1, "one collision: {findings:?}");
        let msg = &findings[0].finding.message;
        assert!(msg.contains("req:REQ-01"), "message names the value: {msg}");
        assert!(
            msg.contains(&id_a) || msg.contains(&id_b),
            "message names colliding ids: {msg}"
        );
        // Config errors never have issue_id; uniqueness findings have no specific
        // attributed issue either (they are cross-issue).
        assert!(!findings[0].is_config_error());
    }

    #[test]
    fn test_label_uniqueness_no_finding_for_unique_values() {
        // Two epics each declaring a distinct req — no collision.
        let rule = uniqueness_rule("req");
        let epic_a = issue("epic-a", &["type:epic", "req:REQ-01"]);
        let epic_b = issue("epic-b", &["type:epic", "req:REQ-02"]);

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic_a, epic_b],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "distinct req values must not collide: {findings:?}"
        );
    }

    #[test]
    fn test_label_uniqueness_when_selector_filters_issues() {
        // The rule has `when = { type = "epic" }` via `rule_from`.
        // A task also carrying req:REQ-01 is NOT included because it fails the
        // `when` selector — only matching issues are checked for uniqueness.
        let rule = rule_from(
            "[[rules]]\nname = \"uniqueness\"\nwhen = { type = \"epic\" }\n\
             severity = \"error\"\n\
             assert = { label-uniqueness = { namespace = \"req\", scope = \"all\" } }\n",
        );
        let epic = issue("epic", &["type:epic", "req:REQ-01"]);
        let task = issue("task", &["type:task", "req:REQ-01"]); // not matched

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic, task],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert!(
            findings.is_empty(),
            "a task not matching the when-selector must not trigger uniqueness: {findings:?}"
        );
    }

    #[test]
    fn test_label_uniqueness_finding_names_value_and_both_short_ids() {
        // Verify the finding message format precisely: names the value and both
        // short-ids so actionable remediation is possible.
        let rule = uniqueness_rule("req");
        let mut epic_a = issue("epic-a", &["req:REQ-42"]);
        let mut epic_b = issue("epic-b", &["req:REQ-42"]);
        // Use deterministic prefixes so we can assert them in the message.
        epic_a.id = "aaaa0000-0000-0000-0000-000000000000".to_string();
        epic_b.id = "bbbb0000-0000-0000-0000-000000000000".to_string();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[epic_a.clone(), epic_b.clone()],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("req:REQ-42"),
            "message must name the value: {msg}"
        );
        assert!(
            msg.contains(&epic_a.short_id()),
            "message must contain short-id of first issue: {msg}"
        );
        assert!(
            msg.contains(&epic_b.short_id()),
            "message must contain short-id of second issue: {msg}"
        );
    }

    #[test]
    fn test_label_uniqueness_is_repo_wide_at_transition() {
        // label-uniqueness must return true from is_repo_wide_at_transition so
        // the transition enforcer skips it and it only runs in `jit validate`.
        let rule = uniqueness_rule("req");
        assert!(
            rule.assert.is_repo_wide_at_transition(),
            "label-uniqueness must be skipped at transition time"
        );
    }

    #[test]
    fn test_label_uniqueness_large_fixture_correctness() {
        // Performance and correctness: build 400 issues (300 with unique req values,
        // 100 forming 50 collision pairs) and assert the evaluation finds exactly 50
        // collision findings and the correct values.
        //
        // Design note: this is O(n * k) — a single HashMap pass over issues × labels.
        // No N² scan. The assertion on finding count confirms correctness on a
        // realistic repo-scale fixture, and the elapsed time is MEASURED below
        // with a deliberately generous bound (so CI variability cannot flake it)
        // to satisfy the "performant on hundreds of issues, measured not assumed"
        // criterion.
        let rule = uniqueness_rule("req");

        let mut issues: Vec<Issue> = Vec::with_capacity(400);

        // 300 issues with unique values (no collision).
        for i in 0..300u32 {
            let label = format!("req:UNIQ-{i:04}");
            issues.push(issue(&format!("unique-{i}"), &[label.as_str()]));
        }
        // 50 collision pairs: two issues per colliding value.
        for i in 0..50u32 {
            let label = format!("req:COLL-{i:04}");
            issues.push(issue(&format!("coll-a-{i}"), &[label.as_str()]));
            issues.push(issue(&format!("coll-b-{i}"), &[label.as_str()]));
        }
        assert_eq!(issues.len(), 400);

        let rules = vec![&rule];
        let started = std::time::Instant::now();
        let findings = evaluate_graph(
            &rules,
            &issues,
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
            fixed_now(),
        );
        let elapsed = started.elapsed();
        eprintln!(
            "label-uniqueness over {} issues took {elapsed:?}",
            issues.len()
        );
        // Generous bound: single-pass evaluation over 400 issues completes in
        // milliseconds; 5s only catches an accidental quadratic regression.
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "evaluation over 400 issues took {elapsed:?} (expected well under 5s)"
        );

        // Exactly 50 findings — one per colliding value, none for the unique ones.
        assert_eq!(
            findings.len(),
            50,
            "expected 50 collision findings, got {}: {findings:?}",
            findings.len()
        );
        // Every finding names a COLL- value.
        assert!(
            findings.iter().all(|f| f.finding.message.contains("COLL-")),
            "all findings must name a collision value: {findings:?}"
        );
        // No finding names a UNIQ- value.
        assert!(
            !findings.iter().any(|f| f.finding.message.contains("UNIQ-")),
            "unique values must not produce findings: {findings:?}"
        );
    }
}
