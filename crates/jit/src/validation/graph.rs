//! Graph / aggregate rule evaluation (DR §4.2).
//!
//! Three rule kinds need cross-issue context and therefore run ONLY in
//! `jit validate` and gate checkers, NEVER on the write path:
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

use crate::document::content_parser_for;
use crate::domain::{project, ContentFormat, Issue};
use crate::graph::DependencyGraph;
use crate::type_hierarchy::{
    validate_orphans, validate_strategic_labels, HierarchyConfig, ValidationWarning,
};
use crate::validation::engine::Finding;
use crate::validation::rules::{Assertion, Rule, Scope, Selector, Severity, TypeHierarchyKind};

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
/// let findings: Vec<GraphFinding> =
///     evaluate_graph(&rules, &[task], &HierarchyConfig::default(), ContentFormat::Markdown);
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
pub fn evaluate_graph(
    rules: &[&Rule],
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    repo_default_format: ContentFormat,
) -> Vec<GraphFinding> {
    rules
        .iter()
        .filter(|rule| rule.scope == Scope::Graph && rule.severity != Severity::Off)
        .flat_map(|rule| evaluate_one(rule, issues, hierarchy, repo_default_format))
        .collect()
}

/// Evaluate a single graph rule, dispatching on its assertion kind.
fn evaluate_one(
    rule: &Rule,
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    repo_default_format: ContentFormat,
) -> Vec<GraphFinding> {
    match &rule.assert {
        Assertion::LabelCoverage { config } => {
            evaluate_label_coverage(rule, config, issues, repo_default_format)
        }
        Assertion::LabelReference { config } => evaluate_label_reference(rule, config, issues),
        Assertion::DependencyShape { config } => evaluate_dependency_shape(rule, config, issues),
        Assertion::TypeHierarchy { kind } => {
            evaluate_type_hierarchy(rule, *kind, hierarchy, issues)
        }
        // Non-graph kinds are never dispatched here (filtered by scope), but be
        // exhaustive and total rather than panic.
        _ => Vec::new(),
    }
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
/// Every canonical criterion id declared in a source issue's success-criteria
/// section must be satisfied by at least one related child carrying the derived
/// `satisfies:<id>` label, optionally in a required state. Source issues are
/// those matching the rule's [`Rule::when`] selector.
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
///   this lifecycle state (snake_case, e.g. `"done"`).
/// - `child-link` (string, optional, default `"dependents"`): how a child
///   relates to the source — `"dependents"`, `"dependencies"`, or `"any"`.
///
/// One finding is produced per uncovered criterion id per source issue. A
/// malformed config yields a single `config-error` finding.
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

    // A child satisfies criterion `id` if it carries `satisfies-ns:id` and (when
    // configured) is in `child-state`.
    let state_matcher = child_state.map(|s| Selector {
        state: Some(s.to_string()),
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

    issues
        .iter()
        .filter(|source| rule.when.matches(source))
        .flat_map(|source| {
            // Select the parser per source issue (content_format -> repo default
            // -> Markdown). A feature-not-compiled selection surfaces as a single
            // config-error finding on the source rather than parsing wrongly.
            let criteria = match criterion_ids(
                source,
                section_slug,
                marker,
                &id_pattern,
                repo_default_format,
            ) {
                Ok(ids) => ids,
                Err(err) => return vec![config_error(rule, err.to_string())],
            };
            let candidates: Vec<&Issue> = children_of(source, issues, child_link);
            criteria
                .into_iter()
                .filter(move |id| !candidates.iter().any(|child| satisfied_id(child, id)))
                .map(move |id| {
                    issue_finding(
                        rule,
                        &source.id,
                        format!(
                            "criterion '{id}' of issue {} is not satisfied by any {} child{}",
                            source.short_id(),
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

/// The candidate child issues for a source under the given link semantics.
fn children_of<'a>(source: &Issue, issues: &'a [Issue], link: ChildLink) -> Vec<&'a Issue> {
    match link {
        ChildLink::Any => issues.iter().filter(|i| i.id != source.id).collect(),
        ChildLink::Dependents => issues
            .iter()
            .filter(|i| i.dependencies.contains(&source.id))
            .collect(),
        ChildLink::Dependencies => issues
            .iter()
            .filter(|i| source.dependencies.contains(&i.id))
            .collect(),
    }
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
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("child-link"));
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
        );
        assert_eq!(findings.len(), 1, "direct-only must not see transitive dep");

        let trans = shape_rule("target = { type = \"design\" }, transitive = true");
        let rules = vec![&trans];
        let findings = evaluate_graph(
            &rules,
            &[design, mid, task],
            &HierarchyConfig::default(),
            ContentFormat::Markdown,
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
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("target"));
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
        );
        assert!(findings.is_empty(), "local + off rules produce nothing");
    }

    // --- type-hierarchy (injected HierarchyConfig) -------------------------

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
        );
        assert_eq!(findings.len(), 1, "orphan leaf must fire: {findings:?}");
        assert_eq!(findings[0].finding.rule, "default:orphan-leaf");
        assert_eq!(findings[0].issue_id.as_deref(), Some(task.id.as_str()));
        assert_eq!(findings[0].finding.severity, Severity::Warn);
    }
}
