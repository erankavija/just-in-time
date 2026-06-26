//! Aggregate report types for the `jit validate` rule runner (DR §9, plan step 7).
//!
//! These types bundle the [`Finding`](crate::validation::engine::Finding)s a
//! `jit validate` invocation produces, plus the per-rule explanation surfaced by
//! `--explain`. They are pure value types with no I/O: the command layer builds
//! them from local + graph evaluation and renders them in human or `--json`
//! form. Keeping them here keeps `commands/validate.rs` free of presentation
//! state and lets the report types carry their own (de)serialization and
//! convenience predicates.

use serde::Serialize;

use crate::validation::engine::Finding;
use crate::validation::rules::{Scope, Severity};

/// One reported finding, scoped to the issue it concerns (if any).
///
/// `issue_id` is the full id of the issue that produced the finding for a
/// local rule, or the issue named in a graph-rule message; it is `None` for a
/// finding that is not attributable to a single issue. The remaining fields
/// mirror the originating [`Finding`].
///
/// # Examples
///
/// ```
/// use jit::validation::engine::Finding;
/// use jit::validation::report::ReportedFinding;
/// use jit::validation::rules::Severity;
///
/// let finding = Finding {
///     rule: "epic-needs-req".to_string(),
///     severity: Severity::Error,
///     message: "missing req:* label".to_string(),
/// };
/// let reported = ReportedFinding::new(Some("abcd1234".to_string()), &finding);
/// assert_eq!(reported.rule, "epic-needs-req");
/// assert!(reported.is_error());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportedFinding {
    /// Full id of the issue this finding concerns, when attributable.
    pub issue_id: Option<String>,
    /// Name of the rule that produced this finding.
    pub rule: String,
    /// Severity of this finding. Serializes as `"off"`/`"warn"`/`"error"`.
    pub severity: Severity,
    /// Human-readable message.
    pub message: String,
}

impl ReportedFinding {
    /// Build a [`ReportedFinding`] from a [`Finding`] and an optional issue id.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::Finding;
    /// use jit::validation::report::ReportedFinding;
    /// use jit::validation::rules::Severity;
    ///
    /// let finding = Finding {
    ///     rule: "r".to_string(),
    ///     severity: Severity::Warn,
    ///     message: "m".to_string(),
    /// };
    /// let reported = ReportedFinding::new(None, &finding);
    /// assert_eq!(reported.severity, jit::validation::rules::Severity::Warn);
    /// assert!(!reported.is_error());
    /// ```
    pub fn new(issue_id: Option<String>, finding: &Finding) -> Self {
        Self {
            issue_id,
            rule: finding.rule.clone(),
            severity: finding.severity,
            message: finding.message.clone(),
        }
    }

    /// Whether this finding is error-severity (the threshold that fails
    /// `jit validate`).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::Finding;
    /// use jit::validation::report::ReportedFinding;
    /// use jit::validation::rules::Severity;
    ///
    /// let warn = Finding { rule: "r".into(), severity: Severity::Warn, message: "m".into() };
    /// assert!(!ReportedFinding::new(None, &warn).is_error());
    /// ```
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
///
/// # Examples
///
/// ```
/// use jit::validation::report::RuleReport;
///
/// // An empty report has no findings and no errors.
/// let report = RuleReport::default();
/// assert!(report.findings.is_empty());
/// assert!(!report.has_errors());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RuleReport {
    /// Every finding produced, local then graph, in evaluation order.
    pub findings: Vec<ReportedFinding>,
}

impl RuleReport {
    /// Whether any finding is error-severity, which fails `jit validate`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::engine::Finding;
    /// use jit::validation::report::{ReportedFinding, RuleReport};
    /// use jit::validation::rules::Severity;
    ///
    /// let mut report = RuleReport::default();
    /// assert!(!report.has_errors());
    /// let err = Finding { rule: "r".into(), severity: Severity::Error, message: "m".into() };
    /// report.findings.push(ReportedFinding::new(None, &err));
    /// assert!(report.has_errors());
    /// ```
    pub fn has_errors(&self) -> bool {
        self.findings.iter().any(ReportedFinding::is_error)
    }

    /// The number of error-severity findings.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::report::RuleReport;
    ///
    /// assert_eq!(RuleReport::default().error_count(), 0);
    /// ```
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
///
/// # Examples
///
/// ```
/// use jit::validation::report::RuleOutcome;
/// use jit::validation::rules::{Scope, Severity};
///
/// let outcome = RuleOutcome {
///     rule: "epic-needs-req".to_string(),
///     scope: Scope::Local,
///     severity: Severity::Error,
///     selector: "type=epic".to_string(),
///     matched: true,
///     skip_reason: None,
///     passed: false,
///     messages: vec!["missing req:* label".to_string()],
/// };
/// assert!(outcome.matched);
/// assert!(!outcome.passed);
/// assert_eq!(outcome.messages.len(), 1);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleOutcome {
    /// Name of the rule.
    pub rule: String,
    /// Evaluation scope. Serializes as `"local"`/`"graph"`.
    pub scope: Scope,
    /// Severity. Serializes as `"off"`/`"warn"`/`"error"`.
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
///
/// # Examples
///
/// ```
/// use jit::validation::report::ExplainReport;
///
/// let report = ExplainReport {
///     issue_id: "abcd1234".to_string(),
///     outcomes: vec![],
/// };
/// assert!(report.outcomes.is_empty());
/// assert!(!report.has_failures());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplainReport {
    /// Full id of the explained issue.
    pub issue_id: String,
    /// One outcome per rule in the ruleset, in rule order. Matched rules carry a
    /// PASS/FAIL result; non-matched rules carry a skip reason.
    pub outcomes: Vec<RuleOutcome>,
}

impl ExplainReport {
    /// Whether any matched rule failed for the issue.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::report::{ExplainReport, RuleOutcome};
    ///
    /// use jit::validation::rules::{Scope, Severity};
    ///
    /// let report = ExplainReport {
    ///     issue_id: "x".to_string(),
    ///     outcomes: vec![RuleOutcome {
    ///         rule: "r".to_string(),
    ///         scope: Scope::Local,
    ///         severity: Severity::Warn,
    ///         selector: "*".to_string(),
    ///         matched: true,
    ///         skip_reason: None,
    ///         passed: false,
    ///         messages: vec!["m".to_string()],
    ///     }],
    /// };
    /// assert!(report.has_failures());
    /// ```
    pub fn has_failures(&self) -> bool {
        self.outcomes.iter().any(|o| !o.passed)
    }

    /// Whether any matched rule failed with error severity (fails
    /// `jit validate <id> --explain`).
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::report::ExplainReport;
    ///
    /// let report = ExplainReport { issue_id: "x".to_string(), outcomes: vec![] };
    /// assert!(!report.has_errors());
    /// ```
    pub fn has_errors(&self) -> bool {
        self.outcomes
            .iter()
            .any(|o| !o.passed && o.severity == Severity::Error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::engine::Finding;
    use crate::validation::rules::{Scope, Severity};

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
    /// String-field layout. Previously `severity` was stored as
    /// `finding.severity.token().to_string()`; now it is a typed `Severity`
    /// that serializes via `#[serde(rename_all="snake_case")]`. The token
    /// values must be identical.
    #[test]
    fn test_reported_finding_json_round_trip_byte_identical() {
        let finding = Finding {
            rule: "r".into(),
            severity: Severity::Error,
            message: "some message".into(),
        };
        let reported = ReportedFinding::new(Some("abc123".into()), &finding);
        let serialized = serde_json::to_string(&reported).unwrap();
        let back: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        // The JSON must have string values matching the old token strings.
        assert_eq!(back["severity"], "error");
        assert_eq!(back["rule"], "r");
        assert_eq!(back["message"], "some message");
        assert_eq!(back["issue_id"], "abc123");
    }

    // --- RuleOutcome JSON byte-identical round-trip --------------------------

    /// Serialized `scope` and `severity` on `RuleOutcome` must produce the same
    /// snake_case tokens as the old String-field layout.
    #[test]
    fn test_rule_outcome_json_scope_severity_byte_identical() {
        let outcome = RuleOutcome {
            rule: "r".into(),
            scope: Scope::Graph,
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
            scope: Scope::Local,
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
            scope: Scope::Local,
            severity: sev,
            selector: "*".into(),
            matched: true,
            skip_reason: None,
            passed,
            messages: if passed { Vec::new() } else { vec!["m".into()] },
        };
        let warn_fail = ExplainReport {
            issue_id: "x".into(),
            outcomes: vec![outcome_with(Severity::Warn, false)],
        };
        assert!(warn_fail.has_failures());
        assert!(!warn_fail.has_errors());

        let error_fail = ExplainReport {
            issue_id: "x".into(),
            outcomes: vec![outcome_with(Severity::Error, false)],
        };
        assert!(error_fail.has_errors());
    }
}
