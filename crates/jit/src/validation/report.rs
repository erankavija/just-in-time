//! Validation report types: the findings a report holds with their provenance,
//! the wire [`RuleReport`] and [`ExplainReport`] it projects to, and the
//! selections every validation view computes from one
//! [`ValidationReport`].

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::declarations::rules::{RuleScope, RuleSet, Selector, Severity};
use crate::domain::identity::{IssueId, IssueIdPrefix};
use crate::domain::vocabulary::wire_vocabulary;
use crate::domain::Issue;
use crate::templates::TemplateRegistry;
use crate::validation::engine::Finding;
use crate::validation::graph::GraphFinding;
use crate::validation::ValidationReport;

wire_vocabulary! {
    /// Rule names emitted by the built-in repository validation passes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
    pub enum BuiltInPass {
        RulesFile = "rules-file",
        DanglingItemLink = "dangling-item-link",
        EnforcementDrift = "enforcement-drift",
        ReviewPlaceholder = "review-placeholder",
    }
}

/// Where a report finding came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingOrigin {
    /// A local rule.
    Local,
    /// A graph rule.
    Graph {
        /// Whether the rule needs the whole repository at transition time.
        repo_wide_at_transition: bool,
    },
    /// A built-in validation pass.
    BuiltIn(BuiltInPass),
}

/// A report finding with the provenance selections filter on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportFinding {
    /// Where the finding came from.
    pub origin: FindingOrigin,
    /// The issue whose rule application fired the finding.
    pub subject: Option<IssueId>,
    /// Whether the finding reports unusable configuration.
    pub is_config_error: bool,
    /// The wire form; its `issue_id` is the attribution.
    pub reported: ReportedFinding,
}

impl ReportFinding {
    /// A local rule's finding on `issue_id`.
    pub(crate) fn local(issue_id: &IssueId, finding: &Finding) -> Self {
        Self {
            origin: FindingOrigin::Local,
            subject: Some(issue_id.clone()),
            is_config_error: false,
            reported: ReportedFinding::new(Some(issue_id.clone()), finding),
        }
    }

    /// A graph rule's finding.
    pub(crate) fn graph(finding: &GraphFinding, repo_wide_at_transition: bool) -> Self {
        Self {
            origin: FindingOrigin::Graph {
                repo_wide_at_transition,
            },
            subject: finding.subject.clone(),
            is_config_error: finding.is_config_error(),
            reported: ReportedFinding::new(finding.issue_id.clone(), &finding.finding),
        }
    }

    /// A built-in pass's finding, fired by and attributed to `issue_id`.
    pub(crate) fn built_in(
        pass: BuiltInPass,
        issue_id: Option<IssueId>,
        severity: Severity,
        message: String,
    ) -> Self {
        Self {
            origin: FindingOrigin::BuiltIn(pass),
            subject: issue_id.clone(),
            is_config_error: pass == BuiltInPass::RulesFile,
            reported: ReportedFinding {
                issue_id,
                rule: pass.as_str().to_owned(),
                severity,
                message,
            },
        }
    }
}

/// One reported finding, scoped to the issue it concerns (if any).
///
/// `issue_id` is the full id of the issue that produced the finding for a
/// local rule, or the issue named in a graph-rule message; it is `None` for a
/// finding that is not attributable to a single issue. The remaining fields
/// mirror the originating [`Finding`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ReportedFinding {
    /// Full id of the issue this finding concerns, when attributable.
    pub issue_id: Option<IssueId>,
    /// Name of the rule that produced this finding.
    pub rule: String,
    /// Severity of this finding, serialized as its token.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
}

impl ReportedFinding {
    /// Build a [`ReportedFinding`] from a [`Finding`] and an optional issue id.
    pub fn new(issue_id: Option<IssueId>, finding: &Finding) -> Self {
        Self {
            issue_id,
            rule: finding.rule.clone(),
            severity: finding.severity,
            message: finding.message.clone(),
        }
    }

    /// Whether this finding is error-severity (the threshold that fails
    /// `jit validate`).
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// The full result of a `jit validate` rule run (per-issue or whole-repo).
///
/// Carries every reported finding across the local and graph rules that were
/// evaluated. The command layer decides the process exit code from
/// [`RuleReport::has_errors`] and renders [`RuleReport::findings`] in human or
/// `--json` form.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RuleReport {
    /// Every finding produced, local then graph, in evaluation order.
    pub findings: Vec<ReportedFinding>,
}

impl RuleReport {
    /// Whether any finding is error-severity, which fails `jit validate`.
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(ReportedFinding::is_error)
    }

    /// The number of error-severity findings.
    pub fn error_count(&self) -> usize {
        self.findings.iter().filter(|f| f.is_error()).count()
    }
}

/// One rule's outcome for a single issue under `--explain`.
///
/// Records the rule's name, scope, severity, the authored selector (rendered for
/// display), whether the rule's selector matched the issue, whether it passed (for
/// matched rules), the reason it was skipped (for non-matched rules), and any
/// messages it produced. This is the per-issue debugging view that justifies not
/// having a separate `jit rule` subcommand (DR §9.1).
///
/// Every rule in the ruleset becomes a `RuleOutcome`, not just the ones whose
/// selector matched: a rule that does not apply is reported with `matched =
/// false` and a [`RuleOutcome::skip_reason`] naming the selector dimension(s)
/// that excluded the issue, so `--explain` can show "the state predicate did not
/// match". For a matched rule, `matched` is `true`, `skip_reason` is `None`, and
/// `passed`/`messages` carry the PASS/FAIL result as before.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RuleOutcome {
    /// Name of the rule.
    pub rule: String,
    /// Evaluation scope, serialized as its token.
    pub scope: RuleScope,
    /// Severity, serialized as its token.
    pub severity: Severity,
    /// Human-readable rendering of the rule's authored selector.
    pub selector: String,
    /// Whether the rule's selector matched the issue. When `false`, the rule did
    /// not execute and [`RuleOutcome::skip_reason`] explains why.
    pub matched: bool,
    /// Why the rule was skipped, when its selector did not match (`None` for a
    /// matched rule). Names the excluding dimension(s), e.g. `"state predicate
    /// did not match: issue is 'in_progress', wants 'done'"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_reason: Option<String>,
    /// Whether the rule passed for this issue (no findings). Always `true` for a
    /// skipped (non-matched) rule, which produces no findings.
    pub passed: bool,
    /// Messages produced by the rule for this issue (empty when it passed or was
    /// skipped).
    pub messages: Vec<String>,
}

/// The `--explain` report for one issue: every matching rule and its outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct ExplainReport {
    /// Full id of the explained issue.
    pub issue_id: IssueId,
    /// One outcome per rule in the ruleset, in rule order. Matched rules carry a
    /// PASS/FAIL result; non-matched rules carry a skip reason.
    pub outcomes: Vec<RuleOutcome>,
}

impl ExplainReport {
    /// Whether any matched rule failed for the issue.
    pub fn has_failures(&self) -> bool {
        self.outcomes.iter().any(|o| !o.passed)
    }

    /// Whether any matched rule failed with error severity (fails
    /// `jit validate <id> --explain`).
    pub fn has_errors(&self) -> bool {
        self.outcomes
            .iter()
            .any(|o| !o.passed && o.severity == Severity::Error)
    }
}

/// A validation view, computed by [`ValidationReport::select`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Selection {
    /// Every finding, and the structural failure as [`StructuralFailure`].
    Whole,
    /// The findings concerning one issue, reported as that issue's.
    Issue(IssueIdPrefix),
    /// The findings of a container's bracket scope.
    Scope(IssueIdPrefix),
    /// One graph rule's findings fired by one issue.
    Rule {
        /// The firing issue.
        issue: IssueIdPrefix,
        /// The rule's name.
        rule: String,
    },
}

/// What selections other than [`Selection::Whole`] read besides the findings.
#[derive(Debug)]
pub(crate) struct Selectable {
    pub(crate) records: Arc<[Issue]>,
    /// The rule set, or the `rules-file` message when it did not load.
    pub(crate) rules: std::result::Result<RuleSet, String>,
    pub(crate) templates: TemplateRegistry,
}

impl Selectable {
    fn issue(&self, prefix: &IssueIdPrefix) -> Result<&Issue> {
        let id = crate::storage::resolve_issue_id_among(prefix, &self.records)?;
        self.records
            .iter()
            .find(|issue| issue.id == id)
            .ok_or_else(|| crate::storage::IssueNotFoundError::new(id).into())
    }

    /// Names of the graph rules whose selector matches any of `issues`.
    fn applicable<'a>(&'a self, issues: &[&Issue]) -> HashSet<&'a str> {
        self.rules
            .iter()
            .flat_map(|rules| &rules.rules)
            .filter(|rule| rule.scope == RuleScope::Graph)
            .filter(|rule| issues.iter().any(|issue| rule.when.matches(issue)))
            .map(|rule| rule.name.as_str())
            .collect()
    }

    fn issue_membership<'a>(&'a self, issue: &'a Issue) -> Membership<'a> {
        Membership::Issue {
            issue: &issue.id,
            applicable: self.applicable(&[issue]),
        }
    }

    fn scope_membership(&self, container: &Issue) -> Membership<'_> {
        let templates = &self.templates;
        let breakdown_type = crate::labels::type_label_value(&container.labels)
            .and_then(|kind| templates.template_for_container(kind))
            .and_then(|template| template.breakdown_type(&templates.roles));
        let members =
            crate::domain::queries::bracket_scope_ids(&container.id, &self.records, breakdown_type);
        let in_scope: Vec<&Issue> = self
            .records
            .iter()
            .filter(|issue| members.contains(&issue.id))
            .collect();
        Membership::Scope {
            applicable: self.applicable(&in_scope),
            members,
        }
    }
}

/// A whole-repository selection from a report computed without the history
/// verdicts.
#[derive(Debug, thiserror::Error)]
#[error("whole-repository validation needs a report carrying the history verdicts")]
pub struct MissingHistoryError;

/// A whole-repository selection from a report with a structural failure,
/// holding the findings collected beside it.
#[derive(Debug, thiserror::Error)]
#[error("{failure:#}")]
pub struct StructuralFailure {
    /// The report's structural failure.
    pub failure: Arc<anyhow::Error>,
    /// Every finding of the report.
    pub findings: RuleReport,
}

/// A configured rule that cannot be evaluated against the selected issue.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RuleSelectionFault {
    /// The selected issue.
    pub issue_id: IssueId,
    /// The requested rule.
    pub rule: String,
    /// Why the rule cannot be selected.
    pub message: String,
}

impl RuleSelectionFault {
    /// The fault as the one error finding the CLI reports.
    pub fn into_report(self) -> RuleReport {
        RuleReport {
            findings: vec![ReportedFinding {
                issue_id: Some(self.issue_id),
                rule: self.rule,
                severity: Severity::Error,
                message: self.message,
            }],
        }
    }
}

/// The membership predicate of one resolved [`Selection`].
enum Membership<'a> {
    Whole,
    Issue {
        issue: &'a IssueId,
        applicable: HashSet<&'a str>,
    },
    Scope {
        members: HashSet<IssueId>,
        applicable: HashSet<&'a str>,
    },
    Rule {
        issue: &'a IssueId,
        rule: &'a str,
    },
}

impl Membership<'_> {
    /// The wire form of `finding` when the selection admits it.
    fn admit(&self, finding: &ReportFinding) -> Option<ReportedFinding> {
        use BuiltInPass::{EnforcementDrift, ReviewPlaceholder, RulesFile};
        use FindingOrigin::{BuiltIn, Graph};

        let attribution = finding.reported.issue_id.as_ref();
        let subject = finding.subject.as_ref();
        let rule = finding.reported.rule.as_str();
        let config_error = finding.is_config_error;
        let admitted = match (self, finding.origin) {
            (Self::Whole, _) | (_, BuiltIn(RulesFile)) => true,
            (Self::Scope { .. }, BuiltIn(EnforcementDrift)) => true,
            (_, BuiltIn(EnforcementDrift | ReviewPlaceholder)) => false,
            (Self::Issue { issue, applicable }, Graph { .. }) if config_error => {
                applicable.contains(rule) && subject.is_none_or(|subject| subject == *issue)
            }
            (Self::Issue { issue, .. }, _) => attribution == Some(*issue),
            (
                Self::Scope {
                    members,
                    applicable,
                },
                Graph { .. },
            ) if config_error => {
                subject.map_or_else(|| applicable.contains(rule), |id| members.contains(id))
            }
            (
                Self::Scope { .. },
                Graph {
                    repo_wide_at_transition: true,
                },
            ) => false,
            (Self::Scope { members, .. }, _) => attribution.is_some_and(|id| members.contains(id)),
            (Self::Rule { issue, rule: name }, Graph { .. }) => {
                rule == *name && subject.map_or(config_error, |subject| subject == *issue)
            }
            (Self::Rule { .. }, _) => false,
        };
        admitted.then(|| match self {
            Self::Issue { issue, .. } => ReportedFinding {
                issue_id: Some((*issue).clone()),
                ..finding.reported.clone()
            },
            _ => finding.reported.clone(),
        })
    }
}

impl ValidationReport {
    /// The findings `selection` admits.
    ///
    /// # Errors
    ///
    /// A prefix naming no issue or several fails with the lookup error. A
    /// report without parsed declarations or records fails every selection but
    /// `Whole` with its structural failure; `Whole` from a report without
    /// history fails with [`MissingHistoryError`], and from a report with a
    /// structural failure with [`StructuralFailure`]; a rule that cannot be
    /// selected for the issue fails with its [`RuleSelectionFault`].
    pub fn select(&self, selection: &Selection) -> Result<RuleReport> {
        let membership = self.membership(selection)?;
        let findings = RuleReport {
            findings: self
                .findings
                .iter()
                .filter_map(|finding| membership.admit(finding))
                .collect(),
        };
        match (&membership, &self.structural_failure) {
            (Membership::Whole, Some(failure)) => Err(StructuralFailure {
                failure: Arc::clone(failure),
                findings,
            }
            .into()),
            _ => Ok(findings),
        }
    }

    /// Every rule's outcome for one issue; a matched rule fails exactly when
    /// the issue's selection holds a finding of it.
    ///
    /// # Errors
    ///
    /// As [`Self::select`] for an issue, and the `rules-file` message when the
    /// rule set did not load.
    pub fn explain(&self, prefix: &IssueIdPrefix) -> Result<ExplainReport> {
        let selectable = self.selectable()?;
        let issue = selectable.issue(prefix)?;
        let rules = selectable
            .rules
            .as_ref()
            .map_err(|message| anyhow!("{message}"))?;
        let membership = selectable.issue_membership(issue);
        let mut messages = group_messages(
            self.findings
                .iter()
                .filter_map(|finding| membership.admit(finding))
                .map(|finding| (finding.rule, finding.message)),
        );
        let outcomes = rules
            .rules
            .iter()
            .map(|rule| {
                let skip_reason = rule.when.match_failure(issue);
                let messages = match skip_reason {
                    Some(_) => Vec::new(),
                    None => messages.remove(&rule.name).unwrap_or_default(),
                };
                RuleOutcome {
                    rule: rule.name.clone(),
                    scope: rule.scope,
                    severity: rule.severity,
                    selector: render_selector(&rule.when),
                    matched: skip_reason.is_none(),
                    skip_reason,
                    passed: messages.is_empty(),
                    messages,
                }
            })
            .collect();
        Ok(ExplainReport {
            issue_id: issue.id.clone(),
            outcomes,
        })
    }

    fn selectable(&self) -> Result<&Selectable> {
        self.selectable
            .as_deref()
            .map_err(|failure| anyhow!("{failure:#}"))
    }

    fn membership<'a>(&'a self, selection: &'a Selection) -> Result<Membership<'a>> {
        Ok(match selection {
            Selection::Whole if self.has_history => Membership::Whole,
            Selection::Whole => return Err(MissingHistoryError.into()),
            Selection::Issue(prefix) => {
                let selectable = self.selectable()?;
                selectable.issue_membership(selectable.issue(prefix)?)
            }
            Selection::Scope(prefix) => {
                let selectable = self.selectable()?;
                selectable.scope_membership(selectable.issue(prefix)?)
            }
            Selection::Rule { issue, rule } => {
                let selectable = self.selectable()?;
                let issue = selectable.issue(issue)?;
                if let Ok(rules) = &selectable.rules {
                    crate::validation::graph::select_graph_rule(&rules.rules, rule, issue)
                        .map_err(|message| RuleSelectionFault {
                            issue_id: issue.id.clone(),
                            rule: rule.clone(),
                            message,
                        })?;
                }
                Membership::Rule {
                    issue: &issue.id,
                    rule,
                }
            }
        })
    }
}

/// Group `(rule_name, message)` pairs by rule name, preserving first-seen
/// order within each rule.
fn group_messages(
    pairs: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, Vec<String>> {
    pairs
        .into_iter()
        .fold(HashMap::new(), |mut acc, (rule, message)| {
            acc.entry(rule).or_default().push(message);
            acc
        })
}

/// Render a rule selector for `--explain`: `"*"` when empty, otherwise the
/// present dimensions joined with `", "` (e.g. `"type=epic, state=ready"`).
fn render_selector(selector: &Selector) -> String {
    let parts: Vec<String> = [
        selector.type_.as_ref().map(|t| format!("type={t}")),
        selector.label.as_ref().map(|l| format!("label={l}")),
        selector
            .state
            .as_ref()
            .map(|s| format!("state={}", s.tokens().join("|"))),
        selector
            .has_doc_type
            .as_ref()
            .map(|d| format!("has_doc_type={d}")),
    ]
    .into_iter()
    .flatten()
    .collect();
    if parts.is_empty() {
        "*".to_string()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::rules::{RuleScope, Severity};
    use crate::validation::engine::Finding;

    // --- ReportedFinding JSON byte-identical round-trip ----------------------

    /// Serialized `severity` MUST be the snake_case token, not the enum variant
    /// name. This test locks the JSON output so the refactoring cannot silently
    /// change the wire format.
    #[test]
    fn test_reported_finding_json_severity_is_snake_case_token() {
        let cases: &[(Severity, &str)] = &[
            (Severity::Off, "\"off\""),
            (Severity::Warn, "\"warn\""),
            (Severity::Error, "\"error\""),
        ];
        for (sev, expected_token) in cases {
            let f = Finding {
                rule: "r".into(),
                severity: *sev,
                message: "m".into(),
            };
            let reported = ReportedFinding::new(None, &f);
            let json = serde_json::to_string(&reported).unwrap();
            assert!(
                json.contains(&format!("\"severity\":{expected_token}")),
                "severity token mismatch for {sev:?}: got {json}"
            );
        }
    }

    /// The serialized `ReportedFinding` shape is byte-identical to the old
    /// String-field layout. `severity` is now a typed `Severity` that serializes
    /// through the vocabulary's explicit serde spellings. The token values must
    /// be identical.
    #[test]
    fn test_reported_finding_json_round_trip_byte_identical() {
        let finding = Finding {
            rule: "r".into(),
            severity: Severity::Error,
            message: "some message".into(),
        };
        let issue_id = crate::test_utils::issue_id(0xe1b3_475c_818d_324f_57dd_716b_1c5d_a41d);
        let reported = ReportedFinding::new(Some(issue_id.clone()), &finding);
        let serialized = serde_json::to_string(&reported).unwrap();
        let back: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        // The JSON must have string values matching the old token strings.
        assert_eq!(back["severity"], "error");
        assert_eq!(back["rule"], "r");
        assert_eq!(back["message"], "some message");
        assert_eq!(back["issue_id"], issue_id.as_str());
    }

    // --- RuleOutcome JSON byte-identical round-trip --------------------------

    /// Serialized `scope` and `severity` on `RuleOutcome` must produce the same
    /// snake_case tokens as the old String-field layout.
    #[test]
    fn test_rule_outcome_json_scope_severity_byte_identical() {
        let outcome = RuleOutcome {
            rule: "r".into(),
            scope: RuleScope::Graph,
            severity: Severity::Warn,
            selector: "*".into(),
            matched: true,
            skip_reason: None,
            passed: true,
            messages: Vec::new(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let back: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(back["scope"], "graph", "scope token changed: {json}");
        assert_eq!(back["severity"], "warn", "severity token changed: {json}");
        // Local scope too.
        let local = RuleOutcome {
            scope: RuleScope::Local,
            severity: Severity::Error,
            ..outcome.clone()
        };
        let local_json = serde_json::to_string(&local).unwrap();
        let lv: serde_json::Value = serde_json::from_str(&local_json).unwrap();
        assert_eq!(lv["scope"], "local", "local scope token: {local_json}");
        assert_eq!(
            lv["severity"], "error",
            "error severity token: {local_json}"
        );
    }

    // --- is_error / has_errors enum comparisons ------------------------------

    /// `is_error()` must use enum comparison, not a string round-trip.
    #[test]
    fn test_reported_finding_is_error_enum_comparison() {
        let make = |sev: Severity| {
            ReportedFinding::new(
                None,
                &Finding {
                    rule: "r".into(),
                    severity: sev,
                    message: "m".into(),
                },
            )
        };
        assert!(make(Severity::Error).is_error());
        assert!(!make(Severity::Warn).is_error());
        assert!(!make(Severity::Off).is_error());
    }

    /// `RuleReport::has_errors()` delegates to `ReportedFinding::is_error()`.
    #[test]
    fn test_rule_report_has_errors_via_enum() {
        let mut report = RuleReport::default();
        assert!(!report.has_errors());
        report.findings.push(ReportedFinding::new(
            None,
            &Finding {
                rule: "r".into(),
                severity: Severity::Error,
                message: "m".into(),
            },
        ));
        assert!(report.has_errors());
    }

    /// `ExplainReport::has_errors()` uses enum comparison on `severity`.
    #[test]
    fn test_explain_report_has_errors_enum_comparison() {
        let outcome_with = |sev: Severity, passed: bool| RuleOutcome {
            rule: "r".into(),
            scope: RuleScope::Local,
            severity: sev,
            selector: "*".into(),
            matched: true,
            skip_reason: None,
            passed,
            messages: if passed { Vec::new() } else { vec!["m".into()] },
        };
        let issue_id = crate::test_utils::issue_id(0xe1b3_475c_818d_324f_57dd_716b_1c5d_a41d);
        let warn_fail = ExplainReport {
            issue_id: issue_id.clone(),
            outcomes: vec![outcome_with(Severity::Warn, false)],
        };
        assert!(warn_fail.has_failures());
        assert!(!warn_fail.has_errors());

        let error_fail = ExplainReport {
            issue_id,
            outcomes: vec![outcome_with(Severity::Error, false)],
        };
        assert!(error_fail.has_errors());
    }
    #[test]
    fn test_render_selector_state_single() {
        use crate::declarations::rules::{Selector, StatePredicate};
        let sel = Selector {
            type_: Some("epic".to_string()),
            state: Some(StatePredicate::single("in_progress")),
            ..Default::default()
        };
        assert_eq!(render_selector(&sel), "type=epic, state=in_progress");
    }

    #[test]
    fn test_render_selector_state_list_joins_with_pipe() {
        use crate::declarations::rules::{Selector, StatePredicate};
        use crate::domain::State;
        let (first, second) = (State::Ready, State::InProgress);
        let sel = Selector {
            state: Some(StatePredicate::list([first, second].map(State::as_str))),
            ..Default::default()
        };
        assert_eq!(render_selector(&sel), format!("state={first}|{second}"));
    }

    // --- Selection membership ---------------------------------------------------

    const RULES: &str = r#"
[[rules]]
name = "cov"
when = { type = "epic" }
assert = { label-coverage = { } }

[[rules]]
name = "shape"
when = { type = "task" }
assert = { dependency-shape = { target = { type = "design" }, mode = "must" } }

[[rules]]
name = "uniq"
assert = { label-uniqueness = { namespace = "req", scope = "all" } }
"#;

    /// Container `c` depends on `a`; `x` is a coverage child of `c` outside
    /// its dependency closure and shares the prefix `c000` with it; `o` is
    /// unrelated.
    struct Fixture {
        c: IssueId,
        a: IssueId,
        x: IssueId,
        o: IssueId,
    }

    impl Fixture {
        fn new() -> Self {
            let id = crate::test_utils::issue_id;
            Self {
                c: id(0xc000_0001),
                a: id(0xa000_0001),
                x: id(0xc000_0002),
                o: id(0xd000_0001),
            }
        }

        fn records(&self) -> Arc<[Issue]> {
            let issue = |id: &IssueId, kind: &str, dependencies: &[&IssueId]| {
                let mut issue = Issue::draft(id.clone(), String::new(), String::new());
                issue.labels = vec![format!("type:{kind}")];
                issue.dependencies = dependencies.iter().map(|id| (*id).clone()).collect();
                issue
            };
            Arc::from(vec![
                issue(&self.c, "epic", &[&self.a]),
                issue(&self.a, "task", &[]),
                issue(&self.x, "task", &[]),
                issue(&self.o, "epic", &[]),
            ])
        }

        /// Each finding's message names it.
        fn findings(&self) -> Vec<ReportFinding> {
            let graph = FindingOrigin::Graph {
                repo_wide_at_transition: false,
            };
            vec![
                finding(
                    FindingOrigin::Local,
                    Some(&self.a),
                    Some(&self.a),
                    "local-r",
                    false,
                    "local-a",
                ),
                finding(
                    graph,
                    Some(&self.x),
                    Some(&self.c),
                    "cov",
                    false,
                    "cov-child",
                ),
                finding(graph, Some(&self.o), None, "cov", true, "cov-config-o"),
                finding(graph, None, None, "cov", true, "cov-config-rule"),
                finding(
                    FindingOrigin::Graph {
                        repo_wide_at_transition: true,
                    },
                    None,
                    Some(&self.a),
                    "uniq",
                    false,
                    "uniq-a",
                ),
                built_in(BuiltInPass::DanglingItemLink, Some(&self.o), "link-o"),
                built_in(BuiltInPass::EnforcementDrift, None, "drift"),
                built_in(BuiltInPass::ReviewPlaceholder, None, "placeholder"),
            ]
        }

        fn report(&self) -> ValidationReport {
            ValidationReport {
                passes: Vec::new(),
                structural_failure: None,
                has_history: true,
                findings: self.findings(),
                selectable: Ok(Box::new(Selectable {
                    records: self.records(),
                    rules: Ok(RuleSet::parse(RULES, None, []).unwrap()),
                    templates: TemplateRegistry::default(),
                })),
            }
        }

        /// A report whose rule set did not load.
        fn unloadable_rules_report(&self) -> ValidationReport {
            ValidationReport {
                findings: vec![
                    ReportFinding::built_in(
                        BuiltInPass::RulesFile,
                        None,
                        Severity::Error,
                        "config error: rules-file".into(),
                    ),
                    built_in(BuiltInPass::EnforcementDrift, None, "drift"),
                ],
                selectable: Ok(Box::new(Selectable {
                    records: self.records(),
                    rules: Err("config error: rules-file".into()),
                    templates: TemplateRegistry::default(),
                })),
                ..self.report()
            }
        }
    }

    fn finding(
        origin: FindingOrigin,
        subject: Option<&IssueId>,
        issue_id: Option<&IssueId>,
        rule: &str,
        is_config_error: bool,
        message: &str,
    ) -> ReportFinding {
        ReportFinding {
            origin,
            subject: subject.cloned(),
            is_config_error,
            reported: ReportedFinding {
                issue_id: issue_id.cloned(),
                rule: rule.into(),
                severity: Severity::Error,
                message: message.into(),
            },
        }
    }

    fn built_in(pass: BuiltInPass, issue_id: Option<&IssueId>, message: &str) -> ReportFinding {
        ReportFinding::built_in(pass, issue_id.cloned(), Severity::Error, message.into())
    }

    fn prefix(id: &IssueId) -> IssueIdPrefix {
        IssueIdPrefix::from(id)
    }

    fn messages(report: &ValidationReport, selection: Selection) -> Vec<String> {
        report
            .select(&selection)
            .unwrap()
            .findings
            .into_iter()
            .map(|finding| finding.message)
            .collect()
    }

    #[test]
    fn test_select_whole_admits_every_finding() {
        let fixture = Fixture::new();
        let report = fixture.report();
        assert_eq!(
            report.select(&Selection::Whole).unwrap(),
            report.rule_report()
        );
    }

    #[test]
    fn test_select_whole_from_a_report_without_history_fails_typed() {
        let report = ValidationReport {
            has_history: false,
            ..Fixture::new().report()
        };
        let error = report.select(&Selection::Whole).unwrap_err();
        assert!(error.downcast_ref::<MissingHistoryError>().is_some());
    }

    #[test]
    fn test_select_issue_reports_attributed_findings_as_the_issues() {
        let fixture = Fixture::new();
        let report = fixture.report();
        assert_eq!(
            messages(&report, Selection::Issue(prefix(&fixture.a))),
            ["local-a", "uniq-a"]
        );
        let selected = report
            .select(&Selection::Issue(prefix(&fixture.c)))
            .unwrap();
        assert!(selected
            .findings
            .iter()
            .all(|finding| finding.issue_id.as_ref() == Some(&fixture.c)));
    }

    #[test]
    fn test_select_issue_omits_a_config_error_fired_by_another_issue() {
        let fixture = Fixture::new();
        assert_eq!(
            messages(&fixture.report(), Selection::Issue(prefix(&fixture.c))),
            ["cov-child", "cov-config-rule"]
        );
    }

    #[test]
    fn test_select_issue_keeps_rules_file_and_omits_enforcement_drift() {
        let fixture = Fixture::new();
        let selected = fixture
            .unloadable_rules_report()
            .select(&Selection::Issue(prefix(&fixture.a)))
            .unwrap();
        let rules: Vec<_> = selected.findings.iter().map(|f| f.rule.as_str()).collect();
        assert_eq!(rules, [BuiltInPass::RulesFile.as_str()]);
    }

    #[test]
    fn test_select_scope_admits_its_members_findings_drift_and_applicable_config_errors() {
        let fixture = Fixture::new();
        assert_eq!(
            messages(&fixture.report(), Selection::Scope(prefix(&fixture.c))),
            ["local-a", "cov-child", "cov-config-rule", "drift"]
        );
    }

    #[test]
    fn test_select_scope_counts_a_coverage_child_outside_the_dependency_closure() {
        let fixture = Fixture::new();
        let report = fixture.report();
        let Ok(selectable) = &report.selectable else {
            unreachable!("the fixture parses its records")
        };
        let container = selectable.issue(&prefix(&fixture.c)).unwrap();
        let Membership::Scope { members, .. } = selectable.scope_membership(container) else {
            unreachable!("a scope membership")
        };
        assert!(!members.contains(&fixture.x));
        assert!(messages(&report, Selection::Scope(prefix(&fixture.c)))
            .contains(&"cov-child".to_string()));
    }

    #[test]
    fn test_select_rule_keeps_the_rules_findings_fired_by_the_issue() {
        let fixture = Fixture::new();
        let report = fixture.report();
        let rule = |issue: &IssueId| Selection::Rule {
            issue: prefix(issue),
            rule: "cov".into(),
        };
        assert_eq!(
            messages(&report, rule(&fixture.o)),
            ["cov-config-o", "cov-config-rule"]
        );
        assert_eq!(messages(&report, rule(&fixture.c)), ["cov-config-rule"]);
    }

    #[test]
    fn test_select_rule_returns_the_fault_of_a_rule_the_issue_cannot_select() {
        let fixture = Fixture::new();
        let report = fixture.report();
        for (issue, rule) in [
            (&fixture.x, "cov"),
            (&fixture.c, "shape"),
            (&fixture.c, "absent"),
        ] {
            let error = report
                .select(&Selection::Rule {
                    issue: prefix(issue),
                    rule: rule.into(),
                })
                .unwrap_err();
            let fault = error.downcast_ref::<RuleSelectionFault>().unwrap();
            assert_eq!((&fault.issue_id, fault.rule.as_str()), (issue, rule));
        }
    }

    #[test]
    fn test_select_rule_reports_rules_file_for_an_unloadable_rule_set() {
        let fixture = Fixture::new();
        let selected = fixture
            .unloadable_rules_report()
            .select(&Selection::Rule {
                issue: prefix(&fixture.a),
                rule: "cov".into(),
            })
            .unwrap();
        let rules: Vec<_> = selected.findings.iter().map(|f| f.rule.as_str()).collect();
        assert_eq!(rules, [BuiltInPass::RulesFile.as_str()]);
    }

    #[test]
    fn test_select_prefix_fails_with_not_found_and_ambiguity_errors() {
        let report = Fixture::new().report();
        let select = |prefix: &str| report.select(&Selection::Issue(prefix.parse().unwrap()));
        assert!(select("ffff")
            .unwrap_err()
            .downcast_ref::<crate::storage::IssueNotFoundError>()
            .is_some());
        assert!(select("c000")
            .unwrap_err()
            .downcast_ref::<crate::storage::AmbiguousIdError>()
            .is_some());
    }

    #[test]
    fn test_select_from_a_report_without_records_fails_every_selection_but_whole() {
        let fixture = Fixture::new();
        let failure = Arc::new(anyhow!("records validation pass: unreadable"));
        let report = ValidationReport {
            structural_failure: Some(Arc::clone(&failure)),
            selectable: Err(failure),
            ..fixture.report()
        };
        let whole = report.select(&Selection::Whole).unwrap_err();
        assert!(whole.downcast_ref::<StructuralFailure>().is_some());
        for selection in [
            Selection::Issue(prefix(&fixture.a)),
            Selection::Scope(prefix(&fixture.c)),
            Selection::Rule {
                issue: prefix(&fixture.c),
                rule: "cov".into(),
            },
        ] {
            let error = report.select(&selection).unwrap_err();
            assert_eq!(error.to_string(), "records validation pass: unreadable");
        }
        assert!(report.explain(&prefix(&fixture.a)).is_err());
    }

    #[test]
    fn test_select_reports_a_failed_history_verdict_only_in_whole() {
        let fixture = Fixture::new();
        let healthy = fixture.report();
        let report = ValidationReport {
            structural_failure: Some(Arc::new(anyhow!(
                "record-fidelity validation pass: stored record does not reproduce"
            ))),
            ..fixture.report()
        };
        let whole = report.select(&Selection::Whole).unwrap_err();
        let whole = whole.downcast_ref::<StructuralFailure>().unwrap();
        assert!(whole
            .to_string()
            .starts_with("record-fidelity validation pass"));
        assert_eq!(whole.findings, healthy.select(&Selection::Whole).unwrap());
        for selection in [
            Selection::Issue(prefix(&fixture.a)),
            Selection::Scope(prefix(&fixture.c)),
            Selection::Rule {
                issue: prefix(&fixture.o),
                rule: "cov".into(),
            },
        ] {
            assert_eq!(
                report.select(&selection).unwrap(),
                healthy.select(&selection).unwrap()
            );
        }
    }

    #[test]
    fn test_explain_fails_a_matched_rule_exactly_when_the_issue_selection_holds_it() {
        let fixture = Fixture::new();
        let report = fixture.report();
        let selected = report
            .select(&Selection::Issue(prefix(&fixture.c)))
            .unwrap();
        let explained = report.explain(&prefix(&fixture.c)).unwrap();
        assert_eq!(explained.outcomes.len(), 3);
        for outcome in &explained.outcomes {
            let held = selected.findings.iter().any(|f| f.rule == outcome.rule);
            assert_eq!(outcome.skip_reason.is_none(), outcome.matched);
            assert_eq!(!outcome.passed, outcome.matched && held, "{outcome:?}");
        }
    }

    #[test]
    fn test_explain_fails_with_the_rules_file_message_for_an_unloadable_rule_set() {
        let fixture = Fixture::new();
        let error = fixture
            .unloadable_rules_report()
            .explain(&prefix(&fixture.a))
            .unwrap_err();
        assert_eq!(error.to_string(), "config error: rules-file");
    }
}
