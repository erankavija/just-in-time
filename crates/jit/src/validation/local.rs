//! Local rule evaluation for the write path (create / update / batch).
//!
//! This module composes the existing pure pieces into the single capability the
//! write path needs (DR §7.5, plan step 5 "local"):
//!
//! 1. project the [`Issue`](crate::domain::Issue) into the canonical
//!    [`Projection`](crate::domain::Projection) (cheap selector fields only);
//! 2. select the [`Scope::Local`] rules whose selector matches the issue —
//!    graph-scope rules are SKIPPED entirely on write (DR §7.4);
//! 3. lazily add the parsed `sections` view to the projection ONLY when a
//!    matching rule actually needs body content (a `require-section` shorthand,
//!    or a raw schema that references `sections`) — a write with no body rule
//!    never parses the Markdown description (perf, DR §6.1);
//! 4. obtain each rule's JSON Schema (raw [`Assertion::JsonSchema`] uses its
//!    schema directly; the shorthand kinds are lowered via
//!    [`desugar`](crate::validation::desugar::desugar)) and validate the
//!    projection through a locally-constructed
//!    [`SchemaEngine`](crate::validation::engine::SchemaEngine), collecting
//!    [`Finding`]s carrying the rule's severity.
//!
//! The [`SchemaEngine`] is constructed locally per call: it uses interior
//! mutability and is `!Sync`, so it must never be stored on a long-lived
//! executor or server state.
//!
//! # Enforcement helpers
//!
//! [`LocalEvaluation`] bundles the findings with the convenience predicates the
//! write path uses to decide whether to block ([`LocalEvaluation::blocking_rules`])
//! and which warnings to surface ([`LocalEvaluation::warnings`]). Blocking
//! semantics (an `error` finding from an `enforce=true` rule blocks unless
//! `--force`, in which case a bypass event is logged) live in the command layer
//! so this module stays free of I/O.

use crate::domain::{project, Issue, Projection};
use crate::validation::desugar::desugar;
use crate::validation::engine::{Finding, SchemaCompileError, SchemaEngine};
use crate::validation::rules::{Assertion, Rule, RuleSet, Scope, Severity};

/// Error raised while evaluating local rules against an issue.
///
/// Currently this only wraps a schema compilation failure: a rule whose JSON
/// Schema (raw or desugared) cannot be compiled surfaces here rather than being
/// silently ignored, so a misconfigured rule never disables enforcement.
///
/// # Examples
///
/// ```
/// use jit::validation::local::LocalEvalError;
/// use jit::validation::engine::SchemaCompileError;
///
/// let err = LocalEvalError::from(SchemaCompileError {
///     rule: "bad".to_string(),
///     message: "boom".to_string(),
/// });
/// assert!(err.to_string().contains("bad"));
/// ```
#[derive(Debug, thiserror::Error)]
pub enum LocalEvalError {
    /// A rule's JSON Schema failed to compile.
    #[error(transparent)]
    Compile(#[from] SchemaCompileError),
}

/// The outcome of evaluating an issue against the local rules.
///
/// Carries every [`Finding`] produced (across all matching local rules) plus the
/// `enforce` flag of each rule, so the command layer can decide which findings
/// block a write. Construct via [`evaluate_local`].
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::validation::local::evaluate_local;
/// use jit::validation::rules::RuleSet;
/// use std::path::Path;
///
/// // A rule set with one enforce rule that requires a `req:*` label.
/// let toml = r#"
/// [[rules]]
/// name = "epic-needs-req"
/// when = { type = "epic" }
/// severity = "error"
/// enforce = true
/// assert = { require-label = { label = "req:*", min = 1 } }
/// "#;
/// let rules = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
///
/// // An epic with no `req:*` label violates the enforce rule.
/// let mut epic = Issue::new("An epic".to_string(), String::new());
/// epic.labels = vec!["type:epic".to_string()];
/// let evaluation = evaluate_local(&epic, &rules).unwrap();
/// assert!(!evaluation.blocking_rules().is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct LocalEvaluation {
    /// All findings produced by matching local rules, with their severities.
    findings: Vec<EnforcedFinding>,
}

/// A [`Finding`] paired with whether its rule blocks writes (`enforce`).
#[derive(Debug, Clone)]
struct EnforcedFinding {
    finding: Finding,
    enforce: bool,
}

impl LocalEvaluation {
    /// Every finding produced, in rule order.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::local::LocalEvaluation;
    ///
    /// // A default (empty) evaluation has no findings.
    /// let evaluation = LocalEvaluation::default();
    /// assert!(evaluation.findings().is_empty());
    /// ```
    pub fn findings(&self) -> Vec<&Finding> {
        self.findings.iter().map(|f| &f.finding).collect()
    }

    /// Returns whether any finding blocks a non-forced write: an `error`-severity
    /// finding from a rule with `enforce = true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::local::LocalEvaluation;
    ///
    /// // An empty evaluation never blocks.
    /// assert!(!LocalEvaluation::default().is_blocking());
    /// ```
    pub fn is_blocking(&self) -> bool {
        !self.blocking_rules().is_empty()
    }

    /// The distinct names of `enforce` rules whose `error` findings block a write,
    /// in first-seen order. Empty when nothing blocks.
    ///
    /// This drives both the rejection message (which rules failed) and, on a
    /// `--force` write, the per-rule bypass events to log.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::local::LocalEvaluation;
    ///
    /// assert!(LocalEvaluation::default().blocking_rules().is_empty());
    /// ```
    pub fn blocking_rules(&self) -> Vec<String> {
        self.findings
            .iter()
            .filter(|f| f.enforce && f.finding.severity == Severity::Error)
            .fold(Vec::new(), |mut acc, f| {
                if !acc.contains(&f.finding.rule) {
                    acc.push(f.finding.rule.clone());
                }
                acc
            })
    }

    /// Human-readable messages for findings that do NOT block the write
    /// (warnings, and `error` findings from non-`enforce` rules), so callers can
    /// surface them without rejecting.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::local::LocalEvaluation;
    ///
    /// assert!(LocalEvaluation::default().warnings().is_empty());
    /// ```
    pub fn warnings(&self) -> Vec<String> {
        self.findings
            .iter()
            .filter(|f| !(f.enforce && f.finding.severity == Severity::Error))
            .filter(|f| f.finding.severity != Severity::Off)
            .map(|f| format!("[{}] {}", f.finding.rule, f.finding.message))
            .collect()
    }

    /// A single rejection message listing the blocking rules and their findings.
    ///
    /// Returns `None` when nothing blocks. Used by the write path to build the
    /// error surfaced when an `enforce` rule fails and `--force` was not passed.
    ///
    /// # Examples
    ///
    /// ```
    /// use jit::validation::local::LocalEvaluation;
    ///
    /// // Nothing blocking => no rejection message.
    /// assert!(LocalEvaluation::default().rejection_message().is_none());
    /// ```
    pub fn rejection_message(&self) -> Option<String> {
        let blocking: Vec<&EnforcedFinding> = self
            .findings
            .iter()
            .filter(|f| f.enforce && f.finding.severity == Severity::Error)
            .collect();
        if blocking.is_empty() {
            return None;
        }
        let details = blocking
            .iter()
            .map(|f| format!("  - [{}] {}", f.finding.rule, f.finding.message))
            .collect::<Vec<_>>()
            .join("\n");
        Some(format!(
            "Blocked by validation rule(s); pass --force to override:\n{details}"
        ))
    }
}

/// Evaluate an issue against the local rules in `rules`, returning the findings.
///
/// Builds the projection, selects [`Scope::Local`] rules whose selector matches
/// the issue (graph rules are skipped), lazily parses the description into
/// `sections` only if a matching rule needs body content, then validates the
/// projection against each rule's JSON Schema via a locally-constructed
/// [`SchemaEngine`]. Shorthand kinds are desugared; the
/// [`Assertion::CheckerCommand`] escape hatch is not yet evaluated here and
/// produces no findings (deferred, not a regression).
///
/// # Errors
///
/// Returns [`LocalEvalError::Compile`] if any matching rule's schema fails to
/// compile, so a misconfigured rule cannot silently disable enforcement.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::validation::local::evaluate_local;
/// use jit::validation::rules::RuleSet;
/// use std::path::Path;
///
/// let toml = r#"
/// [[rules]]
/// name = "task-warns-without-design"
/// when = { type = "task" }
/// severity = "warn"
/// assert = { require-doc-type = { doc-type = "design" } }
/// "#;
/// let rules = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
///
/// let mut task = Issue::new("A task".to_string(), String::new());
/// task.labels = vec!["type:task".to_string()];
/// let evaluation = evaluate_local(&task, &rules).unwrap();
/// // A warn rule produces a (non-blocking) finding.
/// assert!(!evaluation.is_blocking());
/// assert_eq!(evaluation.warnings().len(), 1);
/// ```
pub fn evaluate_local(issue: &Issue, rules: &RuleSet) -> Result<LocalEvaluation, LocalEvalError> {
    // Select matching LOCAL rules; graph-scope rules never run on write.
    let local_rules: Vec<&Rule> = rules
        .matching_rules(issue)
        .into_iter()
        .filter(|rule| rule.scope == Scope::Local && rule.severity != Severity::Off)
        .collect();

    if local_rules.is_empty() {
        return Ok(LocalEvaluation::default());
    }

    // Resolve each rule's schema up front (desugar shorthand). This also lets us
    // detect whether ANY matching rule needs the parsed body before we decide to
    // parse the description (laziness, DR §6.1).
    let resolved: Vec<(&Rule, serde_json::Value)> = local_rules
        .into_iter()
        .filter_map(|rule| rule_schema(rule).map(|schema| (rule, schema)))
        .collect();

    let needs_body = resolved
        .iter()
        .any(|(_, schema)| schema_needs_sections(schema));

    // Project cheaply, then add `sections` ONLY if a matching rule needs it.
    let projection = build_projection(issue, needs_body);
    let value = serde_json::to_value(&projection).unwrap_or_default();

    // One engine per call (the engine is !Sync; never store it long-lived).
    let engine = SchemaEngine::new();
    let mut findings = Vec::new();
    for (rule, schema) in &resolved {
        let key = crate::validation::engine::schema_key(schema);
        let validator = engine.validator_for(&key, &rule.name, schema)?;
        for error in validator.iter_errors(&value) {
            findings.push(EnforcedFinding {
                finding: Finding {
                    rule: rule.name.clone(),
                    severity: rule.severity,
                    message: error.to_string(),
                },
                enforce: rule.enforce,
            });
        }
    }

    Ok(LocalEvaluation { findings })
}

/// Project an issue, populating `sections` only when `with_body` is set.
fn build_projection(issue: &Issue, with_body: bool) -> Projection {
    let projection = project(issue);
    if with_body {
        projection.with_sections(&issue.description, &crate::document::MarkdownContentParser)
    } else {
        projection
    }
}

/// Extract the JSON Schema a local rule validates against.
///
/// A raw [`Assertion::JsonSchema`] contributes its own schema; the four
/// shorthand kinds are lowered with [`desugar`]. [`Assertion::CheckerCommand`]
/// and graph kinds contribute no schema (the latter never reach here as they are
/// filtered out by scope).
fn rule_schema(rule: &Rule) -> Option<serde_json::Value> {
    match &rule.assert {
        Assertion::JsonSchema(source) => Some(source.schema.clone()),
        other => desugar(other),
    }
}

/// Whether a resolved schema references the projection's `sections` view and so
/// requires the description to be parsed before validation.
///
/// Checks for the literal `sections` property name anywhere in the schema. This
/// is conservative: a schema that merely mentions the word would parse the body
/// needlessly, but it never SKIPS a needed parse (correctness over a marginal
/// perf win). The desugared `require-section` schema always carries `sections`,
/// so section rules always trigger the parse.
fn schema_needs_sections(schema: &serde_json::Value) -> bool {
    fn walk(value: &serde_json::Value) -> bool {
        match value {
            serde_json::Value::Object(map) => map
                .iter()
                .any(|(key, child)| key == "sections" || walk(child)),
            serde_json::Value::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    walk(schema)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{DocumentReference, Issue};
    use std::path::Path;

    fn rules_from(toml: &str) -> RuleSet {
        RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap()
    }

    fn epic_without_req() -> Issue {
        let mut issue = Issue::new("An epic".to_string(), String::new());
        issue.labels = vec!["type:epic".to_string()];
        issue
    }

    #[test]
    fn test_enforce_error_blocks() {
        let rules = rules_from(
            r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        assert!(evaluation.is_blocking());
        assert_eq!(evaluation.blocking_rules(), vec!["epic-needs-req"]);
        assert!(evaluation.rejection_message().is_some());
    }

    #[test]
    fn test_enforce_error_passes_when_satisfied() {
        let rules = rules_from(
            r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        let mut issue = epic_without_req();
        issue.labels.push("req:REQ-01".to_string());
        let evaluation = evaluate_local(&issue, &rules).unwrap();
        assert!(!evaluation.is_blocking());
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn test_warn_does_not_block() {
        let rules = rules_from(
            r#"
[[rules]]
name = "epic-warns-req"
when = { type = "epic" }
severity = "warn"
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        assert!(!evaluation.is_blocking());
        assert_eq!(evaluation.warnings().len(), 1);
        assert!(evaluation.blocking_rules().is_empty());
    }

    #[test]
    fn test_error_without_enforce_does_not_block() {
        // An `error` severity rule with enforce=false (the default) reports but
        // never blocks the write.
        let rules = rules_from(
            r#"
[[rules]]
name = "epic-error-soft"
when = { type = "epic" }
severity = "error"
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        assert!(!evaluation.is_blocking());
        assert_eq!(evaluation.warnings().len(), 1);
    }

    #[test]
    fn test_graph_rule_not_evaluated_on_write() {
        // A graph-scope rule must be skipped entirely; even though it matches the
        // issue's selector it produces no findings on the write path.
        let rules = rules_from(
            r#"
[[rules]]
name = "coverage"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { label-coverage = { source = "req", child-state = "done" } }
"#,
        );
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        assert!(!evaluation.is_blocking());
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn test_off_rule_is_skipped() {
        let rules = rules_from(
            r#"
[[rules]]
name = "disabled"
when = { type = "epic" }
severity = "off"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn test_non_matching_rule_is_skipped() {
        let rules = rules_from(
            r#"
[[rules]]
name = "task-rule"
when = { type = "task" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        // The issue is an epic, not a task, so the rule does not match.
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        assert!(evaluation.findings().is_empty());
    }

    #[test]
    fn test_require_section_blocks_when_section_missing() {
        // A require-section rule needs the parsed body; it fails when the heading
        // is absent and passes when present.
        let rules = rules_from(
            r#"
[[rules]]
name = "epic-needs-criteria"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-section = { heading = "Success Criteria" } }
"#,
        );

        let mut missing = epic_without_req();
        missing.description = "## Goals\n\n- ship it\n".to_string();
        assert!(evaluate_local(&missing, &rules).unwrap().is_blocking());

        let mut present = epic_without_req();
        present.description = "## Success Criteria\n\n- [hard] REQ-01\n".to_string();
        assert!(!evaluate_local(&present, &rules).unwrap().is_blocking());
    }

    #[test]
    fn test_schema_needs_sections_detection() {
        // The desugared require-section schema references `sections`...
        let section_schema = desugar(&Assertion::RequireSection {
            heading: "Success Criteria".to_string(),
        })
        .unwrap();
        assert!(schema_needs_sections(&section_schema));

        // ...while a label rule's schema does not.
        let label_schema = desugar(&Assertion::RequireLabel {
            label: "req:*".to_string(),
            min: Some(1),
            max: None,
        })
        .unwrap();
        assert!(!schema_needs_sections(&label_schema));
    }

    #[test]
    fn test_no_body_rule_does_not_parse_description() {
        // A label-only rule must not need the body; the projection built for it
        // leaves `sections` unset (the laziness guarantee). We assert at the
        // projection-building boundary that `with_body` is false for such rules.
        let rules = rules_from(
            r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        );
        let mut issue = epic_without_req();
        // A description that WOULD parse into sections if touched.
        issue.description = "## Success Criteria\n\n- [hard] REQ-01\n".to_string();

        // Reproduce the selection+resolution the evaluator performs and assert no
        // matching rule needs the body, so the description is never parsed.
        let resolved: Vec<_> = rules
            .matching_rules(&issue)
            .into_iter()
            .filter(|r| r.scope == Scope::Local && r.severity != Severity::Off)
            .filter_map(rule_schema)
            .collect();
        assert!(
            !resolved.iter().any(schema_needs_sections),
            "a label-only rule must not require body parsing"
        );

        // And the projection built for this case has no sections populated.
        let projection = build_projection(&issue, false);
        assert!(projection.sections.is_none());
    }

    #[test]
    fn test_doc_type_rule_uses_doc_types_not_body() {
        let rules = rules_from(
            r#"
[[rules]]
name = "task-needs-design"
when = { type = "task" }
severity = "error"
enforce = true
assert = { require-doc-type = { doc-type = "design" } }
"#,
        );
        let mut task = Issue::new("A task".to_string(), String::new());
        task.labels = vec!["type:task".to_string()];
        // No design doc -> blocks.
        assert!(evaluate_local(&task, &rules).unwrap().is_blocking());

        task.documents
            .push(DocumentReference::new("d.md".to_string()).with_type("design".to_string()));
        assert!(!evaluate_local(&task, &rules).unwrap().is_blocking());
    }

    #[test]
    fn test_multiple_blocking_rules_listed_once_each() {
        let rules = rules_from(
            r#"
[[rules]]
name = "needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }

[[rules]]
name = "needs-owner"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "owner:*", min = 1 } }
"#,
        );
        let evaluation = evaluate_local(&epic_without_req(), &rules).unwrap();
        let mut blocking = evaluation.blocking_rules();
        blocking.sort();
        assert_eq!(blocking, vec!["needs-owner", "needs-req"]);
    }

    #[test]
    fn test_invalid_schema_surfaces_error() {
        // A raw schema file that is not a valid JSON Schema must surface as an
        // evaluation error rather than silently passing.
        let dir = tempfile::tempdir().unwrap();
        let schemas = dir.path().join("schemas");
        std::fs::create_dir_all(&schemas).unwrap();
        std::fs::write(schemas.join("bad.json"), r#"{ "type": "not-a-type" }"#).unwrap();
        let toml = r#"
[[rules]]
name = "bad"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { json-schema = "schemas/bad.json" }
"#;
        let rules = RuleSet::from_toml_str(toml, dir.path()).unwrap();
        let err = evaluate_local(&epic_without_req(), &rules).unwrap_err();
        assert!(matches!(err, LocalEvalError::Compile(_)));
    }
}
