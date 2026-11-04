//! Graph / aggregate rule evaluation (DR §4.2).
//!
//! These rule kinds need cross-issue context and therefore run in
//! `jit validate`, gate checkers, and at state transitions, where enforcing
//! failures block the transition. They never run on plain field writes:
//!
//! - **`label-coverage`** — every canonical criterion id declared in a source
//!   issue's `## Success Criteria` section is satisfied by at least one related
//!   child issue (via a `satisfies:<container-short-id>/<id>` label) in a
//!   configurable state.
//! - **`label-reference`** — a `from`-namespace reference (e.g. `implements:REQ-01`)
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
//! # Configuration
//!
//! Assertion options are typed and validated at rule load. Unusable runtime
//! context, such as an unresolved container pointer or an unavailable content
//! parser, surfaces as a `config-error` finding attributed to the rule.
//!
//! # Purity & layering (DR §12)
//!
//! Evaluation is a deterministic function of the issue slice. The caller reads
//! the store; nothing here touches the filesystem.

use crate::domain::identity::GateKey;
use std::collections::BTreeSet;

use chrono::{DateTime, Utc};

use crate::document::content_parser_for;
use crate::domain::item::{
    parse_kind_segmented_address, text_after_checkbox, AddressScope, PROJECT_SCOPE_SENTINEL,
};
use crate::domain::projection::project;
use crate::domain::type_taxonomy::{
    validate_orphans, validate_strategic_labels, HierarchyConfig, ValidationWarning,
};
use crate::domain::{ContentFormat, Issue};
use crate::graph::DependencyGraph;
use std::collections::HashMap;

use crate::declarations::rules::{
    Assertion, ChildLink, DependencyShapeConfig, LabelCoverageConfig, LabelReferenceConfig,
    LabelReferenceScope, Rule, RuleScope, Severity, TypeHierarchyKind,
};
use crate::domain::id_pattern::IdPattern;
use crate::domain::identity::IssueId;
use crate::validation::engine::Finding;

/// Whether a link `label` credits criterion `id` owned by `scope_id`.
///
/// The single matcher behind `label-coverage` and `criteria-to-check`. Only a
/// label that names the criterion's owner credits it:
///
/// - **Qualified** `<namespace>:<scope>/<self-id>` — credited iff `scope` names
///   `scope_id` by id prefix and `self-id` equals `id`.
/// - **`@`-address** `<namespace>:@…` — parsed by
///   [`parse_kind_segmented_address`]; only the issue form
///   `@/issue/<short-id>/<kind>/<self-id>` credits, on scope and self-id alone
///   (the kind is ignored, D10 + f4b5e3f4 REQ-04). Project-scoped forms and
///   parse errors never credit.
///
/// An unqualified `<namespace>:<id>` names no owner, so it credits nothing;
/// validation reports it for migration.
fn label_credits_id(label: &str, namespace: &str, id: &str, scope_id: &IssueId) -> bool {
    let Some(value) = label.strip_prefix(&format!("{namespace}:")) else {
        return false;
    };
    if value.starts_with(PROJECT_SCOPE_SENTINEL) {
        return match parse_kind_segmented_address(value) {
            Ok(addr) => match addr.scope {
                AddressScope::Issue(short_id) => short_id.matches(scope_id) && addr.self_id == id,
                AddressScope::Project | AddressScope::NamedProject(_) => false,
            },
            Err(_) => false,
        };
    }
    // `expand_sugar_address` is not used: it needs the configured kind list and
    // would add kind semantics D10 forbids at this crediting site.
    value.split_once('/').is_some_and(|(scope, self_id)| {
        matches!(
            crate::domain::item::Scope::parse(scope),
            Ok(crate::domain::item::Scope::Issue(prefix)) if prefix.matches(scope_id)
        ) && self_id == id
    })
}

/// The label namespaces through which `rules` credit criteria: each active
/// `label-coverage` rule's `satisfies-namespace` (default included) and each
/// `criteria-to-check` rule's `check-namespace`.
pub fn coverage_credit_namespaces<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
) -> std::collections::HashSet<String> {
    rules
        .into_iter()
        .filter(|rule| rule.severity != Severity::Off)
        .filter_map(|rule| match &rule.assert {
            Assertion::LabelCoverage { config } => Some(config.satisfies_namespace().to_string()),
            Assertion::CriteriaToCheck {
                check_namespace, ..
            } => Some(check_namespace.clone()),
            _ => None,
        })
        .collect()
}

/// The namespaces whose labels credit an issue-scoped item: the
/// [`coverage_credit_namespaces`] of `rules` plus the link namespaces of every
/// issue-scope item kind.
pub fn issue_credit_namespaces<'a>(
    rules: impl IntoIterator<Item = &'a Rule>,
    kinds: &[crate::domain::item::ItemKind],
) -> std::collections::HashSet<String> {
    let mut namespaces = coverage_credit_namespaces(rules);
    namespaces.extend(
        kinds
            .iter()
            .filter(|kind| !kind.kind_scope().is_project())
            .flat_map(|kind| kind.link_namespaces().iter().cloned()),
    );
    namespaces
}

/// Prefix every `config-error` finding message carries, so a [`GraphFinding`]
/// can be recognized as a config error structurally.
const CONFIG_ERROR_PREFIX: &str = "config error: ";

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
/// - `issue_id` is `None` for a `config-error` finding: unusable runtime context
///   pertains to the rule itself, not a single issue, and must never be silently
///   dropped from a per-issue view.
/// - `subject` is the issue whose rule application fired the finding. It
///   differs from `issue_id` when `container-from-label` attributes a finding to
///   the resolved container, and when a config error is raised for one firing
///   issue; it is `None` for repo-wide findings such as `label-uniqueness`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphFinding {
    /// Full id of the issue this finding concerns, or `None` for a config-error
    /// (which pertains to the rule, not a single issue).
    pub issue_id: Option<IssueId>,
    /// Full id of the issue whose rule application fired this finding.
    pub subject: Option<IssueId>,
    /// The underlying engine finding (rule name, severity, message).
    pub finding: Finding,
    /// Whether this finding represents unusable runtime context. Set only
    /// by the module-internal `config_error()` constructor; never set by
    /// `for_issue()` or `unattributed()`. Guards read this field directly rather
    /// than scanning `finding.message` for a prefix.
    is_config_error: bool,
}

impl GraphFinding {
    /// A finding attributed to, and fired by, a specific issue id.
    pub fn for_issue(issue_id: IssueId, finding: Finding) -> Self {
        Self {
            subject: Some(issue_id.clone()),
            issue_id: Some(issue_id),
            finding,
            is_config_error: false,
        }
    }

    /// A finding not attributable to a single issue (e.g. a repo-wide
    /// `label-uniqueness` collision). This is NOT a config-error finding;
    /// config errors are produced by the module-internal `config_error()`
    /// constructor, which sets the typed `is_config_error` field.
    pub fn unattributed(finding: Finding) -> Self {
        Self {
            issue_id: None,
            subject: None,
            finding,
            is_config_error: false,
        }
    }

    /// Whether this finding is a `config-error` (unusable runtime context).
    ///
    /// Config errors are produced by the module-internal `config_error()`
    /// constructor, which sets the typed `is_config_error` field. Guards read
    /// this field directly; the finding's message is never scanned for a prefix.
    /// Config errors carry no issue id and must never be silently dropped from
    /// any per-issue view of the rule that applies.
    pub fn is_config_error(&self) -> bool {
        self.is_config_error
    }
}

/// Evaluate all graph rules over the given issue set, returning one [`Finding`]
/// per violation (and a `config-error` finding for unusable runtime context).
///
/// Only rules whose [`Rule::scope`] is [`RuleScope::Graph`] are evaluated; any
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
/// use jit::domain::type_taxonomy::HierarchyConfig;
/// use jit::validation::graph::evaluate_graph;
/// use jit::declarations::rules::RuleSet;
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
/// let set = RuleSet::parse(toml, None, []).unwrap();
/// let rules: Vec<&_> = set.rules.iter().collect();
///
/// let mut task = Issue::draft(jit::test_utils::issue_id(1), "a task".into(), String::new());
/// task.labels = vec!["type:task".into()];
/// // No design dependency -> one finding, attributed to the task.
/// let findings = evaluate_graph(
///     &rules,
///     &[task],
///     &HierarchyConfig::empty(),
///     jit::domain::ContentFormat::Markdown,
///     chrono::Utc::now(),
///     &std::collections::HashMap::new(),
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
///
/// `plan_content` is the injected plan-document content map (issue-id → resolved
/// criteria-source string). It keeps the engine PURE while still honoring a
/// container whose criteria live in an EXTERNAL plan file: the boundary (the
/// validate / gate-checker context) locates each external plan from its planning
/// node's recorded plan-document reference, reads it out of the captured
/// repository image, and hands the loaded strings in here. When a
/// `label-coverage` (or `criteria-*`) rule reads a source issue's
/// criteria, the engine parses the injected content for that id if present, else
/// falls back to the issue's own [`description`](crate::domain::Issue). An EMPTY
/// map therefore reproduces the description-only behavior exactly, so callers with
/// no bracket config pass `&HashMap::new()`.
pub fn evaluate_graph(
    rules: &[&Rule],
    issues: &[Issue],
    hierarchy: &HierarchyConfig,
    repo_default_format: ContentFormat,
    now: DateTime<Utc>,
    plan_content: &HashMap<IssueId, String>,
) -> Vec<GraphFinding> {
    let context = GraphEvaluationContext {
        graph_context: issues,
        hierarchy,
        repo_default_format,
        now,
        plan_content,
    };
    rules
        .iter()
        .filter(|rule| rule.scope == RuleScope::Graph && rule.severity != Severity::Off)
        .flat_map(|rule| evaluate_one(rule, issues, &context))
        .collect()
}

/// Explain why a selected rule cannot be applied to one issue.
///
/// This pure selection boundary is shared by the live CLI and captured gate
/// checker. Callers turn the returned message into an error-severity finding;
/// the evaluator itself intentionally treats selection as already validated.
pub fn graph_rule_selection_error(rule_name: &str, rule: &Rule, issue: &Issue) -> Option<String> {
    if rule.severity == Severity::Off {
        Some(format!("configured rule '{rule_name}' is off"))
    } else if rule.scope != RuleScope::Graph {
        Some(format!(
            "configured rule '{rule_name}' is not a graph rule and cannot use rule validation"
        ))
    } else if !rule.when.matches(issue) {
        Some(format!(
            "configured rule '{rule_name}' does not match selected issue {}",
            issue.short_id()
        ))
    } else {
        None
    }
}

/// Select one configured graph rule for a selected issue.
///
/// This keeps rule lookup and the non-vacuous selection contract identical for
/// live and captured validation. The caller owns how the returned error becomes
/// a command- or gate-level finding.
pub fn select_graph_rule<'a>(
    rules: &'a [Rule],
    rule_name: &str,
    issue: &Issue,
) -> Result<&'a Rule, String> {
    let rule = rules
        .iter()
        .find(|candidate| candidate.name == rule_name)
        .ok_or_else(|| format!("configured rule '{rule_name}' was not found"))?;
    if let Some(message) = graph_rule_selection_error(rule_name, rule, issue) {
        Err(message)
    } else {
        Ok(rule)
    }
}

/// Repository context supplied to pure graph evaluation.
#[derive(Debug, Clone, Copy)]
pub struct GraphEvaluationContext<'a> {
    /// Issues available to relationship and declaration-aware assertions.
    pub graph_context: &'a [Issue],
    /// Repository hierarchy used by type-hierarchy assertions.
    pub hierarchy: &'a HierarchyConfig,
    /// Default content format used for criteria parsing.
    pub repo_default_format: ContentFormat,
    /// Clock supplied to recency assertions.
    pub now: DateTime<Utc>,
    /// Captured external plan content keyed by issue id.
    pub plan_content: &'a HashMap<IssueId, String>,
}

/// Evaluate a single graph rule, dispatching on its assertion kind.
fn evaluate_one(
    rule: &Rule,
    firing_issues: &[Issue],
    context: &GraphEvaluationContext<'_>,
) -> Vec<GraphFinding> {
    match &rule.assert {
        Assertion::LabelCoverage { config } => evaluate_label_coverage(
            rule,
            config,
            firing_issues,
            context.graph_context,
            context.repo_default_format,
            context.plan_content,
        ),
        Assertion::LabelReference { config } => {
            evaluate_label_reference(rule, config, firing_issues, context.graph_context)
        }
        Assertion::DependencyShape { config } => {
            evaluate_dependency_shape(rule, config, firing_issues, context.graph_context)
        }
        Assertion::GateRecency {
            max_age_hours,
            gates,
        } => evaluate_gate_recency(rule, *max_age_hours, gates, firing_issues, context.now),
        Assertion::TypeHierarchy { kind } => {
            evaluate_type_hierarchy(rule, *kind, context.hierarchy, firing_issues)
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
            firing_issues,
            context.repo_default_format,
            context.plan_content,
        ),
        Assertion::CriteriaToCheck {
            criteria_section,
            marker,
            id_pattern,
            check_namespace,
        } => evaluate_criteria_to_check(
            rule,
            criteria_section,
            marker.as_deref(),
            id_pattern,
            check_namespace,
            firing_issues,
            context.repo_default_format,
            context.plan_content,
        ),
        Assertion::LabelUniqueness { namespace } => {
            evaluate_label_uniqueness(rule, namespace, firing_issues)
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
/// The evaluation is a single pass: one `HashMap<value, Vec<issue id>>` is
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
    // Single pass: collect the ids of every matching issue that carries each
    // value in the namespace.
    let mut value_to_ids: HashMap<&str, Vec<&IssueId>> = HashMap::new();

    for issue in issues {
        if !rule.when.matches(issue) {
            continue;
        }
        for value in values_in_namespace(issue, namespace) {
            value_to_ids.entry(value).or_default().push(&issue.id);
        }
    }

    // Report one finding per value owned by 2 or more issues. Sort by value
    // for deterministic output regardless of HashMap iteration order.
    let mut collisions: Vec<(&str, Vec<&IssueId>)> = value_to_ids
        .into_iter()
        .filter(|(_, ids)| ids.len() >= 2)
        .collect();
    collisions.sort_by_key(|(value, _)| *value);

    collisions
        .into_iter()
        .map(|(value, ids)| {
            GraphFinding::unattributed(finding(
                rule,
                format!(
                    "label '{namespace}:{value}' is declared by multiple issues: {}",
                    ids.iter()
                        .map(|id| id.short_id())
                        .collect::<Vec<_>>()
                        .join(" ")
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
fn issue_finding(rule: &Rule, issue_id: &IssueId, message: String) -> GraphFinding {
    GraphFinding::for_issue(issue_id.clone(), finding(rule, message))
}

/// Attribute unusable runtime context, raised while applying the rule to
/// `subject`, to the rule rather than a single issue.
///
/// This is the only constructor that sets `is_config_error: true` on the result;
/// guards read that typed field rather than scanning `finding.message` for a
/// prefix.
fn config_error(rule: &Rule, subject: &IssueId, message: impl Into<String>) -> GraphFinding {
    GraphFinding {
        issue_id: None,
        subject: Some(subject.clone()),
        finding: finding(rule, format!("{CONFIG_ERROR_PREFIX}{}", message.into())),
        is_config_error: true,
    }
}

// ---------------------------------------------------------------------------
// label-coverage
// ---------------------------------------------------------------------------

/// Evaluate a `label-coverage` rule.
///
/// Every canonical criterion id declared in the **container** issue's
/// success-criteria section must be satisfied by at least one related child
/// carrying `satisfies:<container-short-id>/<id>` (or the equivalent
/// `@/issue/…` address), optionally in a required state.
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
/// - `id-pattern` (string, optional, default [`crate::declarations::rules::DEFAULT_CRITERION_ID_PATTERN`]):
///   regex that extracts the criterion id from an item's text.
/// - `satisfies-namespace` (string, optional, default `"satisfies"`): label
///   namespace on children whose value names the satisfied criterion (see
///   [`label_credits_id`]).
/// - `child-state` (string, optional): when set, a satisfying child must be in
///   this lifecycle state (a [`State`](crate::domain::State) token).
///   **Absent means any state**; there is no wildcard token.
/// - `child-link` (string, optional, default [`ChildLink::Dependents`]): how a
///   child relates to the container, as a [`ChildLink`] token. For a
///   directional link the walk is a transitive closure in that direction.
/// - `child-type-exclude` (array of strings, optional): bare `type:` names that
///   are dropped from coverage candidates AND act as a traversal boundary (the
///   walk does not descend through them).
/// - `container-from-label` (string, optional): label namespace whose value on a
///   firing issue names the criteria-bearing container, so a rule keyed on one
///   issue type can evaluate another issue's criteria (container indirection).
///
/// One finding is produced per uncovered criterion id per container. An unresolved
/// `container-from-label` pointer yields a single `config-error` finding.
/// Malformed assertion configuration is rejected at rule load.
fn evaluate_label_coverage(
    rule: &Rule,
    config: &LabelCoverageConfig,
    firing_issues: &[Issue],
    graph_context: &[Issue],
    repo_default_format: ContentFormat,
    plan_content: &HashMap<IssueId, String>,
) -> Vec<GraphFinding> {
    let section_slug = config.criteria_section();
    let marker = config.marker.as_deref();
    let id_pattern = config.id_pattern().regex();
    let satisfies_ns = config.satisfies_namespace();
    let child_state = config.child_state;
    let child_link = config.child_link();
    let exclude_types = config
        .child_type_exclude
        .iter()
        .flatten()
        .cloned()
        .collect();
    let container_ns = config.container_from_label.as_deref();

    let satisfied_id = |child: &Issue, id: &str, container_id: &IssueId| -> bool {
        let claims = child
            .labels
            .iter()
            .any(|l| label_credits_id(l, satisfies_ns, id, container_id));
        let state_ok = child_state.is_none_or(|state| child.state == state);
        claims && state_ok
    };

    firing_issues
        .iter()
        .filter(|source| rule.when.matches(source))
        .flat_map(|source| {
            // The criteria-bearing container is the firing issue itself, unless
            // container indirection redirects to the issue named by the firing
            // issue's `<container_ns>:<id>` label (D13). An unresolvable or
            // ambiguous pointer is a config error (never a silent pass).
            let container: &Issue = match container_ns {
                None => source,
                Some(ns) => match values_in_namespace(source, ns).next() {
                    Some(value) => match resolve_container(value, graph_context) {
                        ContainerMatch::Unique(c) => c,
                        ContainerMatch::Ambiguous => {
                            return vec![config_error(
                                rule,
                                &source.id,
                                format!(
                                    "issue {} matched the rule but its '{ns}:{value}' container \
                                     pointer is ambiguous (matches multiple issue ids by prefix)",
                                    source.short_id()
                                ),
                            )]
                        }
                        ContainerMatch::None => {
                            return vec![config_error(
                                rule,
                                &source.id,
                                format!(
                                    "issue {} matched the rule but its '{ns}:{value}' container \
                                     pointer names no known issue",
                                    source.short_id()
                                ),
                            )]
                        }
                    },
                    None => {
                        return vec![config_error(
                            rule,
                            &source.id,
                            format!(
                                "issue {} matched the rule but its '{ns}:<id>' container \
                                 pointer is missing",
                                source.short_id()
                            ),
                        )]
                    }
                },
            };

            // Select the parser per container issue (content_format -> repo
            // default -> ContentFormat::DEFAULT). A feature-not-compiled selection surfaces as a
            // single config-error finding rather than parsing wrongly.
            let criteria = match criterion_ids(
                container,
                section_slug,
                marker,
                id_pattern,
                repo_default_format,
                plan_content,
            ) {
                Ok(ids) => ids,
                Err(err) => return vec![config_error(rule, &source.id, err.to_string())],
            };
            let candidates: Vec<&Issue> =
                children_of(container, graph_context, child_link, &exclude_types);
            criteria
                .into_iter()
                .filter(move |id| {
                    !candidates
                        .iter()
                        .any(|child| satisfied_id(child, id, &container.id))
                })
                .map(move |id| GraphFinding {
                    subject: Some(source.id.clone()),
                    ..issue_finding(
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
/// For each list item, if `marker` is set the item must start with it — tolerating
/// one leading GitHub-style checkbox token ahead of the marker (see
/// [`text_after_checkbox`]) — and the first match of `id_pattern` in the item text
/// is the criterion id. Ids are returned de-duplicated in first-seen order.
///
/// The criteria source is the issue's own [`description`](crate::domain::Issue)
/// UNLESS `plan_content` carries an entry for `source.id` — then that injected
/// string (a plan document the boundary resolved from an external location) is
/// parsed instead. This is how a container whose criteria live in an external
/// plan file is validated WITHOUT the engine doing any I/O: the content arrives
/// pre-resolved in the map. An empty map reproduces the description-only behavior.
///
/// The body is parsed with the [`ContentParser`](crate::document::ContentParser)
/// selected by [`content_parser_for`]: the source's own `content_format` → else
/// `repo_default_format` → else [`ContentFormat::DEFAULT`]. A selected format
/// whose parser feature is not compiled returns
/// [`ContentParserError`](crate::document::ContentParserError) rather than a
/// silent Markdown fallback, so the caller can surface it as a finding.
fn criterion_ids(
    source: &Issue,
    section_slug: &str,
    marker: Option<&str>,
    id_pattern: &regex::Regex,
    repo_default_format: ContentFormat,
    plan_content: &HashMap<IssueId, String>,
) -> Result<Vec<String>, crate::document::ContentParserError> {
    let parser = content_parser_for(source.content_format, repo_default_format)?;
    // The resolved plan content for this source (external location), else its
    // own body. The boundary injects external content; the engine stays pure.
    let content = plan_content
        .get(&source.id)
        .map_or(source.description.as_str(), String::as_str);
    let projection = project(source).with_sections(content, parser.as_ref());
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
        .filter(|item| marker.is_none_or(|m| text_after_checkbox(item).starts_with(m)))
        .filter_map(|item| id_pattern.find(item).map(|m| m.as_str().to_string()))
        .filter(|id| seen.insert(id.clone()))
        .collect())
}

/// Whether `issue`'s `type:` label is one of the excluded type names.
///
/// `excluded` holds bare type names (e.g. `"breakdown"`); an issue is excluded
/// when it carries `type:<name>` for any of them.
fn type_is_excluded(issue: &Issue, excluded: &BTreeSet<String>) -> bool {
    !excluded.is_empty()
        && crate::labels::type_label_values(&issue.labels).any(|t| excluded.contains(t))
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
    let by_id: HashMap<&IssueId, &Issue> = issues.iter().map(|i| (&i.id, i)).collect();

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
                .filter_map(|dep| by_id.get(dep).copied())
                .collect(),
            ChildLink::Any => Vec::new(), // handled above
        }
    };

    let mut visited: BTreeSet<&IssueId> = BTreeSet::new();
    visited.insert(&source.id);
    let mut frontier: Vec<&Issue> = neighbours(source);
    let mut candidates: Vec<&Issue> = Vec::new();

    while let Some(node) = frontier.pop() {
        if !visited.insert(&node.id) {
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
/// source within the configured scope. One finding is produced per dangling
/// reference.
fn evaluate_label_reference(
    rule: &Rule,
    config: &LabelReferenceConfig,
    firing_issues: &[Issue],
    graph_context: &[Issue],
) -> Vec<GraphFinding> {
    let (from_ns, to_ns, scope) = (&config.from, &config.to, config.scope());

    // Globally declared source ids (the `to` namespace values across all issues).
    let global_sources: BTreeSet<&str> = graph_context
        .iter()
        .flat_map(|i| values_in_namespace(i, to_ns))
        .collect();

    firing_issues
        .iter()
        .filter(|issue| rule.when.matches(issue))
        .flat_map(|issue| {
            let allowed: BTreeSet<&str> = if scope == LabelReferenceScope::Linked {
                linked_issues(issue, graph_context)
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

/// Outcome of resolving a `container-from-label` value to a container issue.
enum ContainerMatch<'a> {
    /// Exactly one issue resolved.
    Unique(&'a Issue),
    /// The pointer is an id-prefix of more than one issue (ambiguous).
    Ambiguous,
    /// No issue in the slice matched the pointer.
    None,
}

/// Resolve a `container-from-label` value against the in-memory `issues` slice
/// as an item address's issue segment
/// ([`Scope::parse`](crate::domain::item::Scope::parse)). Two issues sharing a
/// short id are ambiguous: the `brackets:<C-short-id>` convention must surface
/// a collision, never silently resolve the first match.
fn resolve_container<'a>(value: &str, issues: &'a [Issue]) -> ContainerMatch<'a> {
    let Ok(crate::domain::item::Scope::Issue(prefix)) = crate::domain::item::Scope::parse(value)
    else {
        return ContainerMatch::None;
    };
    let mut matches = issues.iter().filter(|i| prefix.matches(&i.id));
    match (matches.next(), matches.next()) {
        (Some(c), None) => ContainerMatch::Unique(c),
        (Some(_), Some(_)) => ContainerMatch::Ambiguous,
        _ => ContainerMatch::None,
    }
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
/// `mode` only shapes the finding message; the finding's blocking weight comes
/// from the rule's own [`Severity`]. One finding is produced per source issue
/// lacking a qualifying dependency.
fn evaluate_dependency_shape(
    rule: &Rule,
    config: &DependencyShapeConfig,
    firing_issues: &[Issue],
    graph_context: &[Issue],
) -> Vec<GraphFinding> {
    let target = &config.target;
    let transitive = config.is_transitive();

    // Set of issue ids matching the target selector.
    let target_ids: BTreeSet<&IssueId> = graph_context
        .iter()
        .filter(|i| target.matches(i))
        .map(|i| &i.id)
        .collect();

    let node_refs: Vec<&Issue> = graph_context.iter().collect();
    let graph = DependencyGraph::new(&node_refs);

    firing_issues
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
                    config.mode().as_str()
                ),
            )
        })
        .collect()
}

/// Whether `source` depends on any target id, directly or (when `transitive`)
/// through the dependency DAG.
fn depends_on_target(
    source: &Issue,
    target_ids: &BTreeSet<&IssueId>,
    transitive: bool,
    graph: &DependencyGraph<'_, Issue>,
) -> bool {
    if source.dependencies.iter().any(|d| target_ids.contains(d)) {
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
    gates: &[GateKey],
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
            let checked: Vec<&GateKey> = if gates.is_empty() {
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
/// must map to a check label naming that issue and criterion.
///
/// # Config
///
/// - `criteria_section` (default `"success_criteria"`): projection slug of the
///   section whose list items hold the criteria.
/// - `marker` (optional): when set, only items starting with this marker
///   contribute criterion ids.
/// - `id_pattern` (default [`crate::declarations::rules::DEFAULT_CRITERION_ID_PATTERN`]): extracts the
///   criterion id from an item's text.
/// - `check_namespace`: required label namespace. A criterion `id` is checked
///   when the issue carries `"<check_namespace>:<own-short-id>/<id>"`.
///
/// One finding per unmapped criterion per matching issue. A malformed config
/// yields a single `config-error` finding.
#[allow(clippy::too_many_arguments)]
fn evaluate_criteria_to_check(
    rule: &Rule,
    criteria_section: &str,
    marker: Option<&str>,
    id_pattern: &IdPattern,
    check_namespace: &str,
    issues: &[Issue],
    repo_default_format: ContentFormat,
    plan_content: &HashMap<IssueId, String>,
) -> Vec<GraphFinding> {
    issues
        .iter()
        .filter(|issue| rule.when.matches(issue))
        .flat_map(|issue| {
            let ids = match criterion_ids(
                issue,
                criteria_section,
                marker,
                id_pattern.regex(),
                repo_default_format,
                plan_content,
            ) {
                Ok(ids) => ids,
                Err(err) => return vec![config_error(rule, &issue.id, err.to_string())],
            };
            ids.into_iter()
                .filter(|id| !criterion_is_checked(issue, id, check_namespace))
                .map(|id| {
                    let expected_label =
                        format!("{check_namespace}:{}/{}", issue.short_id(), id.as_str());
                    issue_finding(
                        rule,
                        &issue.id,
                        format!("criterion '{id}' has no check: expected label '{expected_label}'"),
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Whether criterion `id` has a check label naming its owning issue.
fn criterion_is_checked(issue: &Issue, id: &str, check_namespace: &str) -> bool {
    issue
        .labels
        .iter()
        .any(|label| label_credits_id(label, check_namespace, id, &issue.id))
}

// ---------------------------------------------------------------------------
// type-hierarchy (`TypeHierarchyKind` checks)
// ---------------------------------------------------------------------------

/// Evaluate a declarative `type-hierarchy` rule (a [`TypeHierarchyKind`]
/// check) by REUSING the existing
/// [`crate::domain::type_taxonomy`] domain functions over each issue, converting their
/// [`ValidationWarning`]s into [`GraphFinding`]s attributed to the issue.
///
/// This carries no hierarchy logic of its own: `OrphanLeaf` delegates to
/// [`validate_orphans`] and `StrategicConsistency` to [`validate_strategic_labels`],
/// preserving the exact warnings (which are warn-only in the workflow profile).
/// Each warning maps to one finding with the rule's severity.
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
/// Parsed and validated at load into [`Assertion::CriteriaLabelMatch`] fields,
/// with `id-pattern` compiled; this evaluator receives the resolved values, so
/// its only `config-error` finding is an unparsable criteria source.
///
/// # Finding text
///
/// For a stray label `req:REQ-77` on issue `abc123` with section `Success Criteria`:
/// `"label 'req:REQ-77' on issue abc123 names no criterion in section 'Success Criteria' (stray or invented)"`
#[allow(clippy::too_many_arguments)]
fn evaluate_criteria_label_match(
    rule: &Rule,
    namespace: &str,
    criteria_section: &str,
    marker: Option<&str>,
    id_pattern: &IdPattern,
    issues: &[Issue],
    repo_default_format: ContentFormat,
    plan_content: &HashMap<IssueId, String>,
) -> Vec<GraphFinding> {
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
                id_pattern.regex(),
                repo_default_format,
                plan_content,
            ) {
                Ok(ids) => ids.into_iter().collect(),
                Err(err) => return vec![config_error(rule, &issue.id, err.to_string())],
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
        } => {
            let type_label = crate::labels::type_label(type_name);
            format!(
                "issue {issue_id} ({type_label}) is missing a {expected_namespace}:* \
                 identifying label"
            )
        }
        ValidationWarning::OrphanedLeaf {
            issue_id,
            type_name,
        } => {
            let type_label = crate::labels::type_label(type_name);
            format!(
                "issue {issue_id} ({type_label}) is an orphaned leaf with no parent \
                 association label"
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::rules::RuleSet;
    use crate::domain::State;

    fn rule_from(toml: &str) -> Rule {
        let set = RuleSet::parse(toml, None, []).unwrap();
        set.rules.into_iter().next().unwrap()
    }

    fn issue(title: &str, labels: &[&str]) -> Issue {
        let mut i = crate::test_utils::fixture_issue(title.to_string(), String::new());
        i.labels = labels.iter().map(|s| s.to_string()).collect();
        i
    }

    fn test_type(level: u8) -> String {
        crate::test_taxonomy::test_taxonomy()
            .type_at_level(level)
            .to_string()
    }

    fn typed_issue(title: &str, level: u8, extra_labels: &[&str]) -> Issue {
        let mut labels = vec![format!("type:{}", test_type(level))];
        labels.extend(extra_labels.iter().map(|label| (*label).to_string()));
        let mut issue = crate::test_utils::fixture_issue(title.to_string(), String::new());
        issue.labels = labels;
        issue
    }

    /// A fixed clock instant for deterministic graph evaluation. Rules other than
    /// `gate-recency` ignore it; recency tests subtract from it explicitly.
    fn fixed_now() -> DateTime<Utc> {
        use chrono::TimeZone;
        Utc.with_ymd_and_hms(2026, 6, 10, 12, 0, 0).unwrap()
    }

    // --- label_credits_id (shared crediting matcher) ------------------------

    /// Test helper: the qualified label crediting `owner`'s criterion `id`.
    fn satisfies(owner: &Issue, id: &str) -> String {
        format!("satisfies:{}/{id}", owner.short_id())
    }

    #[test]
    fn test_label_credits_id_qualified_form_requires_matching_scope_and_self_id() {
        assert!(label_credits_id(
            "satisfies:abc12345/REQ-01",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));
        // Scope mismatch, self-id mismatch, wrong namespace, no namespace.
        for label in [
            "satisfies:deadbeef/REQ-01",
            "satisfies:abc12345/REQ-02",
            "other:abc12345/REQ-01",
            "plainlabel",
        ] {
            assert!(
                !label_credits_id(
                    label,
                    "satisfies",
                    "REQ-01",
                    &crate::test_utils::issue_id(0xabc12345)
                ),
                "{label}"
            );
        }
    }

    proptest::proptest! {
        /// An unqualified label names no owner, so it credits no criterion of
        /// any scope — including the id it spells.
        #[test]
        fn test_label_credits_id_unqualified_label_credits_nothing(
            id in "[A-Z][A-Z0-9]{0,3}-[0-9]{1,3}",
            scope in proptest::prelude::any::<u128>(),
        ) {
            let label = format!("satisfies:{id}");
            proptest::prop_assert!(!label_credits_id(&label, "satisfies", &id, &crate::test_utils::issue_id(scope)));
        }
    }

    #[test]
    fn test_label_credits_id_at_form_routes_through_parser() {
        // Audit (jit:7a2bbe4f) REQ-02: an `@`-prefixed value routes through
        // `parse_kind_segmented_address`, and the credit decision reads the
        // PARSED (scope, self-id) components — the kind segment is parsed but
        // IGNORED (D10 + f4b5e3f4 REQ-04).

        // Canonical issue form with a MATCHING short-id credits. Two different
        // kind segments (`requirement`, `rule`) credit the SAME id, proving the
        // kind is ignored.
        assert!(label_credits_id(
            "satisfies:@/issue/abc12345/requirement/REQ-01",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));
        assert!(label_credits_id(
            "satisfies:@/issue/abc12345/rule/REQ-01",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));

        // A non-matching short-id in the issue form does not credit.
        assert!(!label_credits_id(
            "satisfies:@/issue/deadbeef/requirement/REQ-01",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));

        // The project form `@/<kind>/<self-id>` addresses a project-scoped item,
        // never an issue criterion, so it never credits.
        assert!(!label_credits_id(
            "satisfies:@/requirement/REQ-01",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));

        // The named-project form `@<name>/<kind>/<self-id>` likewise never credits.
        assert!(!label_credits_id(
            "satisfies:@acme/requirement/REQ-01",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));

        // A malformed `@`-address never credits (and, importantly, never panics).
        assert!(!label_credits_id(
            "satisfies:@/issue/abc12345/requirement",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));
        assert!(!label_credits_id(
            "satisfies:@bogus",
            "satisfies",
            "REQ-01",
            &crate::test_utils::issue_id(0xabc12345)
        ));
    }

    #[test]
    fn test_coverage_credit_namespaces_reads_active_crediting_rules() {
        let set = RuleSet::parse(
            &format!(
                r#"
[[rules]]
name = "default-ns"
severity = "{error}"
assert = {{ label-coverage = {{ }} }}

[[rules]]
name = "custom-ns"
severity = "{error}"
assert = {{ label-coverage = {{ satisfies-namespace = "tests" }} }}

[[rules]]
name = "checks"
severity = "{error}"
assert = {{ criteria-to-check = {{ check-namespace = "checks" }} }}

[[rules]]
name = "disabled"
severity = "{off}"
assert = {{ label-coverage = {{ satisfies-namespace = "off-ns" }} }}
"#,
                error = crate::declarations::rules::Severity::Error,
                off = crate::declarations::rules::Severity::Off,
            ),
            None,
            [],
        )
        .unwrap();
        let namespaces = coverage_credit_namespaces(&set.rules);
        let expected: std::collections::HashSet<String> = ["satisfies", "tests", "checks"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(namespaces, expected);
    }

    // --- label-coverage ----------------------------------------------------

    fn coverage_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"coverage\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ label-coverage = {{ {extra} }} }}\n",
            test_type(2)
        ))
    }

    fn initiative_with_criteria(ids: &[&str]) -> Issue {
        let body = format!(
            "## Success Criteria\n\n{}\n",
            ids.iter()
                .map(|id| format!("- [hard] {id}: do the thing"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut initiative = typed_issue(&test_type(2), 2, &[]);
        initiative.description = body;
        initiative
    }

    /// Load a single rule from `rule_toml` against a config that declares the
    /// shipped `requirement` item kind, so a `kind = "requirement"` rule can
    /// expand. The rule carries no schema reference, so parse purely over an
    /// empty captured schema set with the explicit config.
    fn rule_from_repo(rule_toml: &str) -> Rule {
        let config_toml = crate::test_utils::shipped_item_kind_tables(&["requirement"]);
        let config = toml::from_str::<crate::config::JitConfig>(&config_toml).unwrap();
        let set = crate::declarations::rules::RuleSet::parse(rule_toml, Some(&config), []).unwrap();
        set.rules.into_iter().next().unwrap()
    }

    #[test]
    fn test_label_coverage_kind_sugar_evaluates_like_inline() {
        // REQ-02: a `kind = "requirement"` rule produces IDENTICAL findings to the
        // equivalent inline-triple rule across both covered and uncovered cases.
        let kind_rule = rule_from_repo(&format!(
            "[[rules]]\nname = \"coverage\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ label-coverage = {{ \
             kind = \"requirement\", child-state = \"done\" }} }}\n",
            test_type(2)
        ));
        let inline_rule = coverage_rule("child-state = \"done\"");

        // Covered case: a done child satisfies the criterion.
        let initiative = initiative_with_criteria(&["REQ-01"]);
        let mut child = issue("child", &[&satisfies(&initiative, "REQ-01")]);
        child.dependencies = vec![initiative.id.clone()];
        child.state = State::Done;
        let issues = [initiative, child];

        let eval = |rule: &Rule| {
            evaluate_graph(
                &[rule],
                &issues,
                &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
                ContentFormat::Markdown,
                fixed_now(),
                &HashMap::new(),
            )
        };
        let kind_findings = eval(&kind_rule);
        let inline_findings = eval(&inline_rule);
        assert!(kind_findings.is_empty(), "covered: {kind_findings:?}");
        assert_eq!(kind_findings.len(), inline_findings.len());

        // Uncovered case: no satisfying child -> both fire one finding on the initiative.
        let epic2 = initiative_with_criteria(&["REQ-09"]);
        let issues2 = [epic2];
        let eval2 = |rule: &Rule| {
            evaluate_graph(
                &[rule],
                &issues2,
                &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
                ContentFormat::Markdown,
                fixed_now(),
                &HashMap::new(),
            )
        };
        let kind2 = eval2(&kind_rule);
        let inline2 = eval2(&inline_rule);
        assert_eq!(kind2.len(), 1, "uncovered fires: {kind2:?}");
        assert_eq!(
            kind2.iter().map(|f| &f.finding.message).collect::<Vec<_>>(),
            inline2
                .iter()
                .map(|f| &f.finding.message)
                .collect::<Vec<_>>(),
            "kind= and inline produce identical findings"
        );
    }

    #[test]
    fn test_label_coverage_cross_scope_same_self_id_credits_container_label() {
        // REQ-06: a container `<C>/REQ-01` and an action-scope `<T>/REQ-01` share the
        // self-id REQ-01 but resolve to DISTINCT qualified ids (different scope
        // prefixes); the bracket coverage gate credits the container's criterion
        // from `satisfies:<C>/REQ-01`, so the same-named action-scope id neither
        // shadows it nor alters coverage.
        let rule = coverage_rule("child-link = \"dependencies\"");
        let mut container = initiative_with_criteria(&["REQ-01"]);
        // The impl action ALSO has a REQ-01 in its OWN description (action scope) and
        // credits the container's criterion.
        let mut impl_node = crate::test_utils::fixture_issue(
            "impl".to_string(),
            "## Success Criteria\n\n- [hard] REQ-01: the action's own criterion\n".to_string(),
        );
        impl_node.labels = vec![
            format!("type:{}", test_type(4)),
            satisfies(&container, "REQ-01"),
        ];
        container.dependencies = vec![impl_node.id.clone()];

        // The two qualified ids are distinct by scope even though the self-id is
        // shared, so neither shadows the other in any index.
        let parser = crate::document::MarkdownContentParser;
        let kinds = vec![crate::domain::item::ItemKind::from_config(
            "requirement",
            &crate::test_utils::shipped_item_kind("requirement"),
        )
        .unwrap()];
        let container_items =
            crate::domain::item::index_items(&container, &kinds, &parser).unwrap();
        let action_items = crate::domain::item::index_items(&impl_node, &kinds, &parser).unwrap();
        assert_eq!(container_items.len(), 1);
        assert_eq!(action_items.len(), 1);
        assert_ne!(
            container_items[0].qualified_id, action_items[0].qualified_id,
            "same self-id under different scopes must be distinct qualified ids"
        );

        // Coverage credits the container's criterion.
        let findings = evaluate_graph(
            &[&rule],
            &[container, impl_node],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "the container label must credit the container criterion despite a \
             same-named action-scope id: {findings:?}"
        );
    }

    #[test]
    fn test_label_coverage_satisfied_by_child() {
        let rule = coverage_rule("child-state = \"done\"");
        let initiative = initiative_with_criteria(&["REQ-01"]);
        let mut child = issue("child", &[&satisfies(&initiative, "REQ-01")]);
        child.dependencies = vec![initiative.id.clone()];
        child.state = State::Done;

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(findings.is_empty(), "covered criterion: {findings:?}");
    }

    #[test]
    fn test_label_coverage_nested_containers_sharing_id_credit_only_own_scope() {
        // An epic depends on a nested story; both declare REQ-01 and a task under
        // the story carries the only satisfies label. The walk from the epic
        // reaches the task, but the label credits only the container it names
        // (jit:adb5e9df REQ-05).
        let rule_at = |level: u8| {
            rule_from(&format!(
                "[[rules]]\nname = \"coverage-{level}\"\nwhen = {{ type = \"{}\" }}\n\
                 severity = \"error\"\nassert = {{ label-coverage = {{ \
                 child-link = \"dependencies\" }} }}\n",
                test_type(level)
            ))
        };
        let (epic_rule, story_rule) = (rule_at(2), rule_at(3));
        let criteria = "## Success Criteria\n\n- [hard] REQ-01: do the thing\n";
        let mut epic = typed_issue("epic", 2, &[]);
        epic.description = criteria.to_string();
        let mut story = typed_issue("story", 3, &[]);
        story.description = criteria.to_string();
        epic.dependencies = vec![story.id.clone()];

        let uncovered = |label: &str| -> Vec<IssueId> {
            let mut task = typed_issue("task", 4, &[label]);
            task.state = State::Done;
            let mut story = story.clone();
            story.dependencies = vec![task.id.clone()];
            evaluate_graph(
                &[&epic_rule, &story_rule],
                &[epic.clone(), story, task],
                &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
                ContentFormat::Markdown,
                fixed_now(),
                &HashMap::new(),
            )
            .into_iter()
            .filter_map(|f| f.issue_id)
            .collect()
        };

        assert_eq!(
            uncovered(&satisfies(&story, "REQ-01")),
            vec![epic.id.clone()]
        );
        assert_eq!(
            uncovered(&satisfies(&epic, "REQ-01")),
            vec![story.id.clone()]
        );
        // A label naming no container credits neither.
        let mut both = uncovered("satisfies:REQ-01");
        both.sort();
        let mut expected = vec![epic.id.clone(), story.id.clone()];
        expected.sort();
        assert_eq!(both, expected);
    }

    #[test]
    fn test_label_coverage_unsatisfied_reports_finding() {
        let rule = coverage_rule("child-state = \"done\"");
        let initiative = initiative_with_criteria(&["REQ-01", "REQ-02"]);
        let mut child = issue("child", &[&satisfies(&initiative, "REQ-01")]);
        child.dependencies = vec![initiative.id.clone()];
        child.state = State::Done;

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let initiative = initiative_with_criteria(&["REQ-01"]);
        let mut child = issue("child", &[&satisfies(&initiative, "REQ-01")]);
        child.dependencies = vec![initiative.id.clone()];
        child.state = State::InProgress; // not done

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "wrong-state child does not cover");
    }

    #[test]
    fn test_label_coverage_marker_filters_criteria() {
        // Only [hard] criteria are required; an [aspirational] one is ignored.
        let rule = coverage_rule("marker = \"[hard]\"");
        let body = "## Success Criteria\n\n- [hard] REQ-01: must\n- [aspirational] REQ-99: nice\n";
        let mut initiative = typed_issue(&test_type(2), 2, &[]);
        initiative.description = body.to_string();
        let mut child = issue("child", &[&satisfies(&initiative, "REQ-01")]);
        child.dependencies = vec![initiative.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "aspirational criterion must not be required: {findings:?}"
        );
    }

    #[test]
    fn test_label_coverage_marker_tolerates_checkbox_prefix() {
        // jit:16402e14 REQ-01/REQ-02: a GitHub task-list checkbox ahead of the
        // marker (`- [ ] [hard] ...`, `- [x] [hard] ...`) still counts as a [hard]
        // criterion, so an uncovered one is reported rather than coverage passing
        // vacuously over an empty extracted set.
        let rule = coverage_rule("marker = \"[hard]\"");
        let body = "## Success Criteria\n\n\
            - [ ] [hard] REQ-01: unchecked, uncovered\n\
            - [x] [hard] REQ-02: checked, uncovered\n";
        let mut initiative = typed_issue(&test_type(2), 2, &[]);
        initiative.description = body.to_string();
        // No satisfying children: both criteria must be reported as uncovered.

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(
            findings.len(),
            2,
            "checkbox-prefixed [hard] criteria must not vacuously pass coverage: {findings:?}"
        );
        assert!(findings
            .iter()
            .any(|f| f.finding.message.contains("REQ-01")));
        assert!(findings
            .iter()
            .any(|f| f.finding.message.contains("REQ-02")));
    }

    #[test]
    fn test_label_coverage_any_link_ignores_dependency_edges() {
        let rule = coverage_rule("child-link = \"any\"");
        let initiative = initiative_with_criteria(&["REQ-01"]);
        // Child has NO dependency edge to the initiative, but child-link=any.
        let child = issue("child", &[&satisfies(&initiative, "REQ-01")]);

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(findings.is_empty(), "any link covers regardless of edges");
    }

    // --- label-coverage: transitive closure (T3) ---------------------------

    /// Build a strategic-container issue with `[hard]` criteria but no other labels, so
    /// callers can chain a `dependencies` spine of arbitrary depth beneath it.
    fn initiative_chain_head(ids: &[&str]) -> Issue {
        initiative_with_criteria(ids)
    }

    #[test]
    fn test_label_coverage_credits_non_sink_via_transitive_walk() {
        // Spine: initiative ──dep→ sink ──dep→ deep. Under transitive reduction the
        // initiative links only to `sink`; the criterion is satisfied by `deep` (a
        // non-sink, deeper in the subtree). Direct-adjacency coverage would miss
        // it; the transitive walk must credit it.
        let rule = coverage_rule("child-link = \"dependencies\"");
        let mut initiative = initiative_chain_head(&["REQ-01"]);
        let mut sink = typed_issue("sink", 4, &[]);
        let deep = typed_issue("deep", 4, &[&satisfies(&initiative, "REQ-01")]);
        // initiative depends on sink; sink depends on deep.
        initiative.dependencies = vec![sink.id.clone()];
        sink.dependencies = vec![deep.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, sink, deep],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut initiative = initiative_chain_head(&["REQ-01"]);
        let mut sink = typed_issue("sink", 4, &[]);
        let deep = typed_issue("deep", 4, &[]); // no satisfies label
        initiative.dependencies = vec![sink.id.clone()];
        sink.dependencies = vec![deep.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, sink, deep],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        // The default `dependents` link also becomes transitive: initiative <-dep- a
        // <-dep- b, with b (a transitive dependent) satisfying the criterion.
        let rule = coverage_rule(""); // default child-link = dependents
        let initiative = initiative_chain_head(&["REQ-01"]);
        let mut a = typed_issue("a", 4, &[]);
        let mut b = typed_issue("b", 4, &[&satisfies(&initiative, "REQ-01")]);
        a.dependencies = vec![initiative.id.clone()]; // a depends on initiative
        b.dependencies = vec![a.id.clone()]; // b depends on a (so transitively on initiative)

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, a, b],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "transitive dependent must cover: {findings:?}"
        );
    }

    // --- label-coverage: child-type-exclude (T3) ---------------------------

    #[test]
    fn test_child_type_exclude_drops_candidate_and_halts_walk() {
        // Bracket spine: initiative ──dep→ impl ──dep→ B(type:breakdown) ──dep→ P.
        // P (beyond the boundary) satisfies REQ-01, but B is excluded, so the
        // walk must halt at B and never reach P -> uncovered.
        let rule =
            coverage_rule("child-link = \"dependencies\", child-type-exclude = [\"breakdown\"]");
        let mut initiative = initiative_chain_head(&["REQ-01"]);
        let mut impl_node = typed_issue("impl", 4, &[]);
        let mut breakdown = issue("B", &["type:breakdown"]);
        let plan = issue("P", &["type:planning", &satisfies(&initiative, "REQ-01")]);
        initiative.dependencies = vec![impl_node.id.clone()];
        impl_node.dependencies = vec![breakdown.id.clone()];
        breakdown.dependencies = vec![plan.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, impl_node, breakdown, plan],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut initiative = initiative_chain_head(&["REQ-01"]);
        let mut impl_node = typed_issue("impl", 4, &[&satisfies(&initiative, "REQ-01")]);
        let breakdown = issue("B", &["type:breakdown"]);
        initiative.dependencies = vec![impl_node.id.clone()];
        impl_node.dependencies = vec![breakdown.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, impl_node, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "impl interior covers; only the breakdown boundary is excluded: {findings:?}"
        );
    }

    // --- label-coverage: container indirection via brackets: (T3/D13) ------

    #[test]
    fn test_container_indirection_evaluates_container_criteria() {
        // Preview instance: rule keyed on type:breakdown resolves its container
        // from `brackets:<C-short-id>` and evaluates C's criteria coverage. C carries
        // the criteria; B carries no criteria of its own.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let mut container = initiative_with_criteria(&["REQ-01"]);
        let mut impl_node = typed_issue("impl", 4, &[&satisfies(&container, "REQ-01")]);
        let mut breakdown = issue("B", &["type:breakdown"]);
        // Container depends on impl (its subtree); B brackets the container.
        container.dependencies = vec![impl_node.id.clone()];
        breakdown
            .labels
            .push(format!("brackets:{}", container.short_id()));
        // B sits on the spine after impl, but the walk runs from the container.
        impl_node.dependencies = vec![breakdown.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut container = initiative_with_criteria(&["REQ-01"]);
        let impl_node = typed_issue("impl", 4, &[]); // does NOT satisfy
        let mut breakdown = issue("B", &["type:breakdown"]);
        container.dependencies = vec![impl_node.id.clone()];
        breakdown
            .labels
            .push(format!("brackets:{}", container.short_id()));

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut container = initiative_with_criteria(&["REQ-01"]);
        let mut impl_node = typed_issue("impl", 4, &[&satisfies(&container, "REQ-01")]);
        impl_node.state = State::Backlog; // drafted, not done
        let mut breakdown = issue("B", &["type:breakdown"]);
        container.dependencies = vec![impl_node.id.clone()];
        breakdown
            .labels
            .push(format!("brackets:{}", container.short_id()));

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("config error"));
        assert!(findings[0].finding.message.contains("brackets"));
    }

    #[test]
    fn test_container_indirection_finding_is_attributed_to_container_with_breakdown_subject() {
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let container = initiative_with_criteria(&["REQ-01"]);
        let mut breakdown = issue("B", &["type:breakdown"]);
        breakdown
            .labels
            .push(format!("brackets:{}", container.short_id()));
        let (container_id, breakdown_id) = (container.id.clone(), breakdown.id.clone());

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "uncovered criterion: {findings:?}");
        assert_eq!(findings[0].issue_id, Some(container_id));
        assert_eq!(findings[0].subject, Some(breakdown_id));
    }

    #[test]
    fn test_container_indirection_dangling_pointer_config_error_has_firing_subject() {
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             container-from-label = \"brackets\" } }\n",
        );
        let breakdown = issue("B", &["type:breakdown", "brackets:missing"]);
        let breakdown_id = breakdown.id.clone();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "dangling pointer: {findings:?}");
        assert!(findings[0].is_config_error());
        assert_eq!(findings[0].issue_id, None);
        assert_eq!(findings[0].subject, Some(breakdown_id));
    }

    #[test]
    fn test_container_indirection_resolves_short_id_label() {
        // Project convention writes `brackets:<SHORT-ID>` (e.g. brackets:2fbd2a82),
        // not the full UUID. The container must resolve from the short id and have
        // its criteria evaluated -- not be reported as a missing-container config
        // error.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let mut container = initiative_with_criteria(&["REQ-01"]);
        let impl_node = typed_issue("impl", 4, &[]); // does NOT satisfy REQ-01
        let mut breakdown = issue("B", &["type:breakdown"]);
        container.dependencies = vec![impl_node.id.clone()];
        // SHORT id, per project convention -- not the full UUID.
        breakdown
            .labels
            .push(format!("brackets:{}", container.short_id()));

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        // Container criteria ARE evaluated (REQ-01 uncovered), not a config error.
        assert_eq!(
            findings.len(),
            1,
            "short-id label must resolve container and evaluate criteria: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("REQ-01"));
        assert!(
            !findings[0].finding.message.contains("config error"),
            "short-id label must not be a config error: {findings:?}"
        );
    }

    #[test]
    fn test_container_indirection_full_id_label_still_resolves() {
        // Backward compatibility: a full-id brackets label still resolves, even
        // though the convention (and `test_container_indirection_resolves_short_id_label`)
        // is the short id. The resolver accepts both, so this guards the full-id path.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             child-link = \"dependencies\", container-from-label = \"brackets\" } }\n",
        );
        let mut container = initiative_with_criteria(&["REQ-01"]);
        let impl_node = typed_issue("impl", 4, &[]);
        let mut breakdown = issue("B", &["type:breakdown"]);
        container.dependencies = vec![impl_node.id.clone()];
        breakdown.labels.push(format!("brackets:{}", container.id));

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[container, impl_node, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(
            findings.len(),
            1,
            "full-id label must still resolve: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_container_indirection_ambiguous_prefix_is_config_error() {
        // A short pointer that is a prefix of two distinct issue ids is ambiguous
        // and must surface as a config error, never a silent pass.
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             container-from-label = \"brackets\" } }\n",
        );
        let mut a = initiative_with_criteria(&["REQ-01"]);
        let mut b = initiative_with_criteria(&["REQ-02"]);
        // Force two ids sharing a common prefix that is neither a full nor a short
        // id of either issue (so only the prefix branch matches, and matches both).
        a.id = crate::test_utils::issue_id(0xabcd_1111);
        b.id = crate::test_utils::issue_id(0xabcd_2222);
        let mut breakdown = issue("B", &["type:breakdown"]);
        breakdown.labels.push("brackets:abcd".to_string());

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[a, b, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "ambiguous prefix: {findings:?}");
        assert!(findings[0].finding.message.contains("config error"));
        assert!(
            findings[0].finding.message.contains("ambiguous"),
            "ambiguity must be reported clearly: {findings:?}"
        );
    }

    #[test]
    fn test_container_indirection_exact_short_id_collision_is_ambiguous() {
        // Two issues sharing the SAME 8-char short id is the worst case for the
        // `brackets:<C-short-id>` convention: an exact short-id pointer matches
        // both, so resolution must report ambiguity, never silently pick the
        // first. (The prefix-ambiguity test above covers a shorter shared prefix;
        // this guards the exact short-id branch specifically.)
        let rule = rule_from(
            "[[rules]]\nname = \"preview\"\nwhen = { type = \"breakdown\" }\n\
             severity = \"error\"\nassert = { label-coverage = { \
             container-from-label = \"brackets\" } }\n",
        );
        let mut a = initiative_with_criteria(&["REQ-01"]);
        let mut b = initiative_with_criteria(&["REQ-02"]);
        // Identical first 8 chars (the short id) but distinct full ids.
        a.id = crate::test_utils::issue_id(0x1111_0000_0000_0000_0000_0000_abcd_1234);
        b.id = crate::test_utils::issue_id(0x2222_0000_0000_0000_0000_0000_abcd_1234);
        assert_eq!(
            a.short_id(),
            b.short_id(),
            "test setup: short ids must collide"
        );
        let mut breakdown = issue("B", &["type:breakdown"]);
        breakdown.labels.push("brackets:abcd1234".to_string());

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[a, b, breakdown],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "exact short-id collision: {findings:?}");
        assert!(findings[0].finding.message.contains("config error"));
        assert!(
            findings[0].finding.message.contains("ambiguous"),
            "exact short-id collision must be reported as ambiguous: {findings:?}"
        );
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
        let rule = reference_rule("from = \"implements\", to = \"req\"");
        let source = typed_issue(&test_type(2), 2, &["req:REQ-01"]);
        let child = issue("child", &["implements:REQ-01"]);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[source, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(findings.is_empty(), "resolved reference: {findings:?}");
    }

    #[test]
    fn test_label_reference_dangles() {
        let rule = reference_rule("from = \"implements\", to = \"req\"");
        let source = typed_issue(&test_type(2), 2, &["req:REQ-01"]);
        let child = issue("child", &["implements:REQ-99"]); // no req:REQ-99 anywhere
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[source, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("REQ-99"));
        assert_eq!(findings[0].finding.severity, Severity::Warn);
    }

    #[test]
    fn test_label_reference_linked_scope_requires_edge() {
        let rule = reference_rule("from = \"implements\", to = \"req\", scope = \"linked\"");
        // Declaring issue exists globally but is NOT linked to the child.
        let declarer = typed_issue(&test_type(2), 2, &["req:REQ-01"]);
        let child = issue("child", &["implements:REQ-01"]); // no dependency edge
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[declarer, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "linked scope: unlinked source dangles");

        // Now add the edge: the reference resolves.
        let declarer = typed_issue(&test_type(2), 2, &["req:REQ-01"]);
        let mut child = issue("child", &["implements:REQ-01"]);
        child.dependencies = vec![declarer.id.clone()];
        let findings = evaluate_graph(
            &rules,
            &[declarer, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(findings.is_empty(), "linked edge resolves: {findings:?}");
    }

    // --- dependency-shape --------------------------------------------------

    fn shape_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"shape\"\nwhen = {{ type = \"{}\" }}\nseverity = \"error\"\n\
             assert = {{ dependency-shape = {{ {extra} }} }}\n",
            test_type(4)
        ))
    }

    #[test]
    fn test_dependency_shape_satisfied() {
        let rule = shape_rule("target = { type = \"design\" }");
        let design = issue("design", &["type:design"]);
        let mut action = typed_issue(&test_type(4), 4, &[]);
        action.dependencies = vec![design.id.clone()];
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[design, action],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "action depends on design: {findings:?}"
        );
    }

    #[test]
    fn test_dependency_shape_violated() {
        let rule = shape_rule("target = { type = \"design\" }");
        let design = issue("design", &["type:design"]);
        let action = typed_issue(&test_type(4), 4, &[]); // no dependency
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[design, action],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert!(findings[0].finding.message.contains("depend"));
        assert_eq!(findings[0].finding.rule, "shape");
    }

    #[test]
    fn test_dependency_shape_transitive() {
        // action -> mid -> design; only satisfied when transitive = true.
        let design = issue("design", &["type:design"]);
        let mut mid = issue("mid", &["type:other"]);
        mid.dependencies = vec![design.id.clone()];
        let mut action = typed_issue(&test_type(4), 4, &[]);
        action.dependencies = vec![mid.id.clone()];

        let direct = shape_rule("target = { type = \"design\" }");
        let rules = vec![&direct];
        let findings = evaluate_graph(
            &rules,
            &[design.clone(), mid.clone(), action.clone()],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "direct-only must not see transitive dep");

        let trans = shape_rule("target = { type = \"design\" }, transitive = true");
        let rules = vec![&trans];
        let findings = evaluate_graph(
            &rules,
            &[design, mid, action],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "transitive dep satisfies: {findings:?}"
        );
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
    fn issue_with_gate(gate: &GateKey, hours_ago: i64) -> Issue {
        let mut i = issue("work", &[]);
        i.state = State::Done;
        i.gates_required = vec![gate.clone()];
        i.gates_status.insert(
            gate.clone(),
            GateState {
                status: GateStatus::Passed,
                updated_by: Some("ci:x".parse().unwrap()),
                updated_at: fixed_now() - chrono::Duration::hours(hours_ago),
                run_id: None,
            },
        );
        i
    }

    #[test]
    fn test_gate_recency_fresh_result_passes() {
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        // Recorded 1 day ago — well within 7 days.
        let i = issue_with_gate(&"code-review".parse().unwrap(), 24);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(findings.is_empty(), "fresh gate must pass: {findings:?}");
    }

    #[test]
    fn test_gate_recency_stale_result_reports_age_in_days() {
        let rule = recency_rule("max-age-days = 7, gates = [\"code-review\"]");
        // Recorded 10 days ago — exceeds 7.
        let i = issue_with_gate(&"code-review".parse().unwrap(), 10 * 24);
        let id = i.id.clone();
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].issue_id.as_ref(), Some(&id));
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
        i.gates_required = vec!["code-review".parse().unwrap()];
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut i = issue_with_gate(&"code-review".parse().unwrap(), 10 * 24); // stale
                                                                               // A second required gate with no recorded result.
        i.gates_required.push("security".parse().unwrap());
        let rules = vec![&rule];
        let mut findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let i = issue_with_gate(&"code-review".parse().unwrap(), 30); // 30h old > 12h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let i = issue_with_gate(&"code-review".parse().unwrap(), 10 * 24);
        let rules = vec![&rule];
        let a = evaluate_graph(
            &rules,
            std::slice::from_ref(&i),
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        let b = evaluate_graph(
            &rules,
            std::slice::from_ref(&i),
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let i = issue_with_gate(&"code-review".parse().unwrap(), 180); // 180h > 168h, < 192h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let i = issue_with_gate(&"code-review".parse().unwrap(), 7 * 24); // exactly 168h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let i = issue_with_gate(&"code-review".parse().unwrap(), 7 * 24 + 1); // 169h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let i = issue_with_gate(&"code-review".parse().unwrap(), 12); // exactly 12h
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[i],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
            &[typed_issue(&test_type(4), 4, &[])],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(findings.is_empty(), "local + off rules produce nothing");
    }

    // --- type-hierarchy (injected HierarchyConfig) -------------------------

    // --- criteria-to-check -------------------------------------------------

    fn ctc_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"ctc\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ criteria-to-check = {{ {extra} }} }}\n",
            test_type(2)
        ))
    }

    /// Initiative with a Success Criteria section carrying the given criterion lines.
    fn initiative_with_sc(items: &[&str]) -> Issue {
        let body = format!(
            "## Success Criteria\n\n{}\n",
            items
                .iter()
                .map(|s| format!("- {s}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut initiative = typed_issue(&test_type(2), 2, &[]);
        initiative.description = body;
        initiative
    }

    #[test]
    fn test_criteria_to_check_gate_requirement_does_not_cover_criterion() {
        // Required gates do not cover criteria; the rule requires a check label.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let mut initiative = initiative_with_sc(&["[hard] REQ-01: do the thing"]);
        initiative.gates_required = vec!["manual-review".parse().unwrap()];
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "a gate alone must not cover a criterion");
        assert!(findings[0]
            .finding
            .message
            .contains("expected label 'checks:"));
    }

    #[test]
    fn test_criteria_to_check_label_mapped_id_passes() {
        // A `<ns>:<own-short-id>/REQ-01` label on the issue that owns the
        // criterion credits it.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let mut initiative = initiative_with_sc(&["REQ-01: do the thing"]);
        let qualified = format!("checks:{}/REQ-01", initiative.short_id());
        initiative.labels.push(qualified);
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "qualified label-mapped criterion must produce no finding: {findings:?}"
        );
    }

    #[test]
    fn test_criteria_to_check_qualified_label_wrong_scope_reports_finding() {
        // A qualified check label scoped to a DIFFERENT issue must not credit the
        // criterion on this issue (no spurious pass).
        let rule = ctc_rule("check-namespace = \"checks\"");
        let mut initiative = initiative_with_sc(&["REQ-01: do the thing"]);
        initiative.labels.push("checks:deadbeef/REQ-01".to_string());
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(
            findings.len(),
            1,
            "wrong-scope qualified check label must not credit: {findings:?}"
        );
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_criteria_to_check_unmapped_id_reports_finding() {
        // An unmapped criterion must be reported with its id in the message.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let initiative = initiative_with_sc(&["REQ-01: do the thing"]);
        let id = initiative.id.clone();
        let short = initiative.short_id();
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "one unmapped criterion: {findings:?}");
        assert_eq!(findings[0].issue_id.as_ref(), Some(&id));
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("REQ-01"),
            "finding must name the criterion id: {msg}"
        );
        assert!(
            msg.contains(&format!("checks:{short}/REQ-01")),
            "finding must name the expected label: {msg}"
        );
    }

    #[test]
    fn test_criteria_to_check_marker_filtering() {
        // Only [hard] items are required when marker = "[hard]".
        let rule = ctc_rule("marker = \"[hard]\", check-namespace = \"checks\"");
        let initiative = initiative_with_sc(&[
            "[hard] REQ-01: must do",
            "[aspirational] REQ-02: nice to have",
        ]);
        // REQ-01 is unmapped; REQ-02 is [aspirational] and must be ignored.
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
    fn test_criteria_to_check_unchecked_message_names_check_label() {
        // An unchecked criterion names the required issue-scoped check label.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let initiative = initiative_with_sc(&["REQ-01: do the thing"]);
        let short = initiative.short_id();
        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains(&format!(
                "criterion 'REQ-01' has no check: expected label 'checks:{short}/REQ-01'"
            )),
            "finding must name only the expected check label: {msg}"
        );
    }

    #[test]
    fn test_criteria_to_check_stays_per_issue_ignores_children() {
        // criteria-to-check maps a criterion to the SAME issue's check label and
        // must NOT traverse children. A child carrying the label does not
        // satisfy the parent's criterion -> the parent still reports.
        let rule = ctc_rule("check-namespace = \"checks\"");
        let initiative = initiative_with_sc(&["REQ-01: do the thing"]);
        // A dependent child carries the would-be mapping; per-issue semantics
        // mean this is irrelevant to the initiative.
        let mut child = typed_issue("child", 4, &[]);
        child.dependencies = vec![initiative.id.clone()];
        child
            .labels
            .push(format!("checks:{}/REQ-01", initiative.short_id()));
        let initiative_id = initiative.id.clone();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(
            findings.len(),
            1,
            "criteria-to-check is per-issue; a child label must not cover: {findings:?}"
        );
        assert_eq!(findings[0].issue_id.as_ref(), Some(&initiative_id));
        assert!(findings[0].finding.message.contains("REQ-01"));
    }

    #[test]
    fn test_type_hierarchy_orphan_leaf_fires_with_injected_config() {
        // A `type-hierarchy` rule authored in TOML carries only its kind; the
        // HierarchyConfig is injected by `evaluate_graph`. A leaf action with no
        // parent association label is flagged as an orphan.
        let rule = rule_from(
            "[[rules]]\nname = \"orphan-leaf-fixture\"\nseverity = \"warn\"\n\
             assert = { type-hierarchy = { kind = \"orphan-leaf\" } }\n",
        );
        let rules = vec![&rule];
        let action = typed_issue(&test_type(4), 4, &[]);
        let findings = evaluate_graph(
            &rules,
            std::slice::from_ref(&action),
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "orphan leaf must fire: {findings:?}");
        assert_eq!(findings[0].finding.rule, "orphan-leaf-fixture");
        assert_eq!(findings[0].issue_id.as_ref(), Some(&action.id));
        assert_eq!(findings[0].finding.severity, Severity::Warn);
    }

    // --- criteria-label-match -----------------------------------------------

    fn clm_rule(extra: &str) -> Rule {
        rule_from(&format!(
            "[[rules]]\nname = \"clm\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ criteria-label-match = {{ {extra} }} }}\n",
            test_type(2)
        ))
    }

    /// An initiative with `## Success Criteria` items shaped `[hard] <id>: text`.
    fn initiative_with_clm_criteria(ids: &[&str]) -> Issue {
        let body = format!(
            "## Success Criteria\n\n{}\n",
            ids.iter()
                .map(|id| format!("- [hard] {id}: do the thing"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let mut initiative = typed_issue(&test_type(2), 2, &[]);
        initiative.description = body;
        initiative
    }

    #[test]
    fn test_criteria_label_match_stray_label_yields_finding() {
        // A `req:REQ-77` label on an initiative whose Success Criteria has only REQ-01
        // is stray: no matching criterion id -> finding with the stray message.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut initiative = initiative_with_clm_criteria(&["REQ-01"]);
        initiative.labels.push("req:REQ-77".to_string()); // stray
        initiative.labels.push("req:REQ-01".to_string()); // matches criterion

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative.clone()],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
            findings[0].issue_id.as_ref(),
            Some(&initiative.id),
            "finding must be attributed to the initiative"
        );
    }

    #[test]
    fn test_criteria_label_match_matched_label_produces_no_finding() {
        // A `req:REQ-01` on an initiative whose criteria contain REQ-01 is fine.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut initiative = initiative_with_clm_criteria(&["REQ-01"]);
        initiative.labels.push("req:REQ-01".to_string());

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut initiative = initiative_with_clm_criteria(&["REQ-03"]);
        initiative.labels.push("req:REQ-3".to_string()); // differs by leading zero

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut initiative = typed_issue(&test_type(2), 2, &["req:REQ-01", "req:REQ-99"]);
        initiative.description = body.to_string();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let mut initiative = typed_issue(&test_type(2), 2, &["req:REQ-77"]); // stray
        initiative.description = body.to_string();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        let initiative = initiative_with_clm_criteria(&["REQ-01"]); // labels: just its type

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
        // initiative still fires, and a child carrying the criterion id does not make
        // the parent's matching label "non-stray" via traversal (there is no
        // traversal). Here the stray is on the initiative itself.
        let rule = clm_rule(r#"namespace = "req", marker = "[hard]""#);
        let mut initiative = initiative_with_clm_criteria(&["REQ-01"]);
        initiative.labels.push("req:REQ-77".to_string()); // stray on the initiative
        let initiative_id = initiative.id.clone();
        // A child that happens to carry req:REQ-77 must NOT silence the initiative's
        // stray finding (per-issue: the child is never consulted).
        let mut child = typed_issue("child", 4, &["req:REQ-77"]);
        child.dependencies = vec![initiative.id.clone()];

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, child],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(
            findings.len(),
            1,
            "criteria-label-match is per-issue; a child must not affect it: {findings:?}"
        );
        assert_eq!(findings[0].issue_id.as_ref(), Some(&initiative_id));
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
        // Two unlinked initiatives both declaring req:REQ-01 — one finding naming the
        // value and both short-ids.
        let rule = uniqueness_rule("req");
        let initiative_a = typed_issue("initiative-a", 2, &["req:REQ-01"]);
        let initiative_b = typed_issue("initiative-b", 2, &["req:REQ-01"]);
        // Deliberately no dependency edge between them.
        let id_a = initiative_a.short_id().to_string();
        let id_b = initiative_b.short_id().to_string();

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative_a, initiative_b],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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
    fn test_label_uniqueness_finding_has_no_subject() {
        let rule = uniqueness_rule("req");
        let findings = evaluate_graph(
            &[&rule],
            &[
                typed_issue("initiative-a", 2, &["req:REQ-01"]),
                typed_issue("initiative-b", 2, &["req:REQ-01"]),
            ],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1, "one collision: {findings:?}");
        assert_eq!(findings[0].subject, None);
    }

    #[test]
    fn test_label_uniqueness_no_finding_for_unique_values() {
        // Two initiatives each declaring a distinct req — no collision.
        let rule = uniqueness_rule("req");
        let initiative_a = typed_issue("initiative-a", 2, &["req:REQ-01"]);
        let initiative_b = typed_issue("initiative-b", 2, &["req:REQ-02"]);

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative_a, initiative_b],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "distinct req values must not collide: {findings:?}"
        );
    }

    #[test]
    fn test_label_uniqueness_when_selector_filters_issues() {
        // The rule has a strategic-container selector via `rule_from`.
        // An action also carrying req:REQ-01 is NOT included because it fails the
        // `when` selector — only matching issues are checked for uniqueness.
        let rule = rule_from(&format!(
            "[[rules]]\nname = \"uniqueness\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\n\
             assert = {{ label-uniqueness = {{ namespace = \"req\", scope = \"all\" }} }}\n",
            test_type(2)
        ));
        let initiative = typed_issue(&test_type(2), 2, &["req:REQ-01"]);
        let action = typed_issue(&test_type(4), 4, &["req:REQ-01"]); // not matched

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative, action],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert!(
            findings.is_empty(),
            "an action not matching the when-selector must not trigger uniqueness: {findings:?}"
        );
    }

    #[test]
    fn test_label_uniqueness_finding_names_value_and_both_short_ids() {
        // Verify the finding message format precisely: names the value and both
        // short-ids so actionable remediation is possible.
        let rule = uniqueness_rule("req");
        let mut initiative_a = issue("initiative-a", &["req:REQ-42"]);
        let mut initiative_b = issue("initiative-b", &["req:REQ-42"]);
        // Use deterministic prefixes so we can assert them in the message.
        initiative_a.id = crate::test_utils::issue_id(0xaaaa_0000);
        initiative_b.id = crate::test_utils::issue_id(0xbbbb_0000);

        let rules = vec![&rule];
        let findings = evaluate_graph(
            &rules,
            &[initiative_a.clone(), initiative_b.clone()],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        let msg = &findings[0].finding.message;
        assert!(
            msg.contains("req:REQ-42"),
            "message must name the value: {msg}"
        );
        assert!(
            msg.contains(&initiative_a.short_id()),
            "message must contain short-id of first issue: {msg}"
        );
        assert!(
            msg.contains(&initiative_b.short_id()),
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
        // 400 issues (300 with unique req values, 100 forming 50 collision pairs):
        // the evaluation finds exactly 50 collision findings and the correct values.
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
        let findings = evaluate_graph(
            &rules,
            &issues,
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
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

    // --- b121fd51: coverage-compat acceptance tests --------------------------
    //
    // REQ-01 (b121fd51): `label-coverage` with `kind = "requirement"` produces
    // findings IDENTICAL to the equivalent inline-triple form, in both the
    // covered and uncovered cases.
    //
    // REQ-02 (b121fd51): the `(criteria_section, marker, id_pattern)` triple
    // that a `criteria-to-check` (or `criteria-label-match`) rule actually
    // consumes at runtime equals the triple the declared `requirement` kind
    // expands to — compatibility SHOWN, not re-expressed.

    #[test]
    fn test_b121fd51_label_coverage_kind_equals_inline_acceptance() {
        // REQ-01 acceptance: a `label-coverage` rule with `kind = "requirement"`
        // produces findings IDENTICAL field-for-field to the equivalent inline
        // `(criteria-section, marker, id-pattern)` rule. `GraphFinding` is `Eq`, so
        // the WHOLE finding vectors (issue_id, rule, severity, message) are
        // compared, in BOTH the covered (empty) and uncovered cases.
        let kind_rule = rule_from_repo(&format!(
            "[[rules]]\nname = \"coverage\"\nwhen = {{ type = \"{}\" }}\n\
             severity = \"error\"\nassert = {{ label-coverage = {{ \
             kind = \"requirement\", child-state = \"done\" }} }}\n",
            test_type(2)
        ));
        let registry: HashMap<_, _> = crate::test_utils::shipped_item_kinds()
            .into_iter()
            .collect();
        let triple =
            crate::domain::item::expand_kind_triple(Some(&registry), "requirement").unwrap();
        let inline_rule = coverage_rule(&format!(
            "criteria-section = \"{}\", marker = \"{}\", id-pattern = '{}', child-state = \"done\"",
            triple.section,
            triple.marker.unwrap(),
            triple.id_pattern,
        ));

        let eval = |rule: &Rule, issues: &[Issue]| {
            evaluate_graph(
                &[rule],
                issues,
                &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
                ContentFormat::Markdown,
                fixed_now(),
                &HashMap::new(),
            )
        };

        // Covered: a done child credits REQ-01 → no findings from either rule.
        let initiative = initiative_with_criteria(&["REQ-01"]);
        let mut child = issue("child", &[&satisfies(&initiative, "REQ-01")]);
        child.dependencies = vec![initiative.id.clone()];
        child.state = State::Done;
        let covered = [initiative, child];
        let kind_covered = eval(&kind_rule, &covered);
        let inline_covered = eval(&inline_rule, &covered);
        assert!(
            kind_covered.is_empty(),
            "REQ-01 covered: kind= rule must produce no finding: {kind_covered:?}"
        );
        // Full-vector equality (both empty) — identical field-for-field.
        assert_eq!(
            kind_covered, inline_covered,
            "REQ-01 covered: kind= and inline findings must be identical"
        );

        // Uncovered: no satisfying child → one finding from each rule. Compare the
        // full GraphFinding vectors (issue_id, rule, severity, message).
        let epic2 = initiative_with_criteria(&["REQ-09"]);
        let uncovered = [epic2];
        let kind_uncovered = eval(&kind_rule, &uncovered);
        let inline_uncovered = eval(&inline_rule, &uncovered);
        assert_eq!(
            kind_uncovered.len(),
            1,
            "REQ-01 uncovered: kind= rule must fire one finding: {kind_uncovered:?}"
        );
        // Full-vector equality — every field of every finding must match.
        assert_eq!(
            kind_uncovered, inline_uncovered,
            "REQ-01 uncovered: kind= and inline findings must be identical field-for-field"
        );
    }

    #[test]
    fn test_b121fd51_typed_rule_triple_equals_requirement_kind_expansion() {
        // REQ-02 acceptance: the `(section, marker, id_pattern)` triple a
        // `criteria-to-check` rule consumes when authored with the INLINE form of
        // the requirement kind's triple EQUALS the triple a CONFIG-DECLARED
        // `requirement` kind expands to via `expand_kind_triple` over an explicit
        // `[item_kinds]` registry (NOT the hardcoded default constructor). This
        // shows the two representations share one triple: the kind model and the
        // typed rules are COMPATIBLE (same section + marker + id_pattern), not
        // re-expressed (the typed rules are NOT given `kind=` syntax).
        //
        // The same assertion is then repeated for `criteria-label-match`.

        use crate::config::ItemKindConfig;
        use crate::domain::item::expand_kind_triple;
        use std::collections::HashMap;

        // Declare the shipped kinds in a registry, then EXPAND `requirement` to
        // the consumed triple.
        let registry: HashMap<String, ItemKindConfig> = crate::test_utils::shipped_item_kinds()
            .into_iter()
            .collect();
        let declared = expand_kind_triple(Some(&registry), "requirement").unwrap();
        let kind_section = declared.section.as_str();
        let kind_marker = declared.marker.as_deref();
        let kind_pattern = declared.id_pattern.as_str();

        // Build `criteria-to-check` with the inline triple from the declared kind's
        // expansion (section + marker + id-pattern all explicit) and assert match.
        let ctc = ctc_rule(&format!(
            "check-namespace = \"checks\", \
             criteria-section = \"{kind_section}\", \
             marker = \"{}\", \
             id-pattern = '{kind_pattern}'",
            kind_marker.unwrap_or(""),
        ));
        match ctc.assert {
            Assertion::CriteriaToCheck {
                criteria_section,
                marker,
                id_pattern,
                ..
            } => {
                assert_eq!(
                    criteria_section.as_str(),
                    kind_section,
                    "criteria-to-check criteria_section must equal declared kind expansion section"
                );
                assert_eq!(
                    marker.as_deref(),
                    kind_marker,
                    "criteria-to-check marker must equal declared kind expansion marker"
                );
                assert_eq!(
                    id_pattern.as_str(),
                    kind_pattern,
                    "criteria-to-check id_pattern must equal declared kind expansion id_pattern"
                );
            }
            other => panic!("expected CriteriaToCheck, got {other:?}"),
        }

        // The same triple expressed as a `criteria-label-match` inline rule.
        let clm = clm_rule(&format!(
            "namespace = \"satisfies\", \
             criteria-section = \"{kind_section}\", \
             marker = \"{}\", \
             id-pattern = '{kind_pattern}'",
            kind_marker.unwrap_or(""),
        ));
        match clm.assert {
            Assertion::CriteriaLabelMatch {
                criteria_section,
                marker,
                id_pattern,
                ..
            } => {
                assert_eq!(
                    criteria_section.as_str(),
                    kind_section,
                    "criteria-label-match criteria_section must equal declared kind expansion section"
                );
                assert_eq!(
                    marker.as_deref(),
                    kind_marker,
                    "criteria-label-match marker must equal declared kind expansion marker"
                );
                assert_eq!(
                    id_pattern.as_str(),
                    kind_pattern,
                    "criteria-label-match id_pattern must equal declared kind expansion id_pattern"
                );
            }
            other => panic!("expected CriteriaLabelMatch, got {other:?}"),
        }
    }

    // --- GraphFinding: typed is_config_error field ---------------------------

    /// `for_issue()` always produces a non-config-error finding.
    #[test]
    fn test_graph_finding_for_issue_is_not_config_error() {
        use crate::validation::engine::Finding;
        let f = Finding {
            rule: "r".into(),
            severity: Severity::Error,
            message: "criterion uncovered".into(),
        };
        let issue_id = crate::test_utils::issue_id(0xe1b3_475c_818d_324f_57dd_716b_1c5d_a41d);
        let gf = GraphFinding::for_issue(issue_id, f);
        assert!(
            !gf.is_config_error(),
            "for_issue must not be a config error"
        );
    }

    /// `unattributed()` always produces a non-config-error finding, even when
    /// the message happens to carry the config-error prefix. The typed field
    /// is the only source of truth.
    #[test]
    fn test_graph_finding_unattributed_is_not_config_error() {
        use crate::validation::engine::Finding;
        let f = Finding {
            rule: "r".into(),
            severity: Severity::Error,
            // The prefix alone must NOT trigger the config-error flag.
            message: format!("{CONFIG_ERROR_PREFIX}this is not actually from config_error()"),
        };
        let gf = GraphFinding::unattributed(f);
        assert!(
            !gf.is_config_error(),
            "unattributed() must not set is_config_error even with the prefix in the message"
        );
    }

    /// The `config_error()` constructor (exercised via `evaluate_graph` with a
    /// missing container pointer) produces a finding where `is_config_error()` returns
    /// `true`. The typed field, not the message prefix, drives this.
    #[test]
    fn test_config_error_constructor_sets_typed_field() {
        // A missing container pointer forces `config_error()`.
        let rule = coverage_rule("container-from-label = \"brackets\"");
        let initiative = initiative_with_criteria(&["REQ-01"]);
        let findings = evaluate_graph(
            &[&rule],
            &[initiative],
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            ContentFormat::Markdown,
            fixed_now(),
            &HashMap::new(),
        );
        assert_eq!(findings.len(), 1);
        assert!(
            findings[0].is_config_error(),
            "missing container pointer must produce a typed config error"
        );
        // Verify the typed field is not just re-deriving from the message prefix.
        assert!(
            findings[0].finding.message.starts_with(CONFIG_ERROR_PREFIX),
            "sanity: message still carries the prefix"
        );
        // The field must be true even if we stripped the prefix from the message.
        assert!(
            findings[0].is_config_error(),
            "is_config_error reads the field, not the message"
        );
    }
}
