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
}

/// Severity of a rule finding.
///
/// `off` disables the rule entirely; `warn` reports without blocking; `error`
/// reports and (when `enforce` is set) can block a write.
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

/// Evaluation scope of a rule, derived from its assertion kind.
///
/// `Local` rules are pure predicates over a single projected issue and run on
/// write. `Graph` rules need the whole store and run only in `jit validate` and
/// gate checkers, never on write.
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
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct Selector {
    /// Match issues of this `type:*` label value (e.g. `"epic"`).
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
    /// Match issues carrying this label, supporting `ns:*` wildcards.
    #[serde(default)]
    pub label: Option<String>,
    /// Match issues in this lifecycle state (serde snake_case, e.g. `"ready"`).
    #[serde(default)]
    pub state: Option<String>,
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
    pub fn matches(&self, issue: &Issue) -> bool {
        self.matches_type(issue)
            && self.matches_label(issue)
            && self.matches_state(issue)
            && self.matches_doc_type(issue)
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
            Some(state) => state_token(issue.state) == state.to_lowercase(),
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
}

impl Assertion {
    /// The evaluation scope implied by this assertion kind.
    pub fn scope(&self) -> Scope {
        match self {
            Assertion::LabelCoverage { .. }
            | Assertion::LabelReference { .. }
            | Assertion::DependencyShape { .. } => Scope::Graph,
            _ => Scope::Local,
        }
    }
}

/// A fully parsed validation rule.
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

    /// Load and parse `.jit/rules.toml` relative to the given `.jit` root.
    ///
    /// Returns an empty [`RuleSet`] when the file does not exist. Referenced
    /// schema files are read relative to `jit_root` and transcoded to JSON.
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
    pub fn from_toml_str(content: &str, jit_root: &Path) -> Result<Self, RuleConfigError> {
        let raw: RawRulesFile = toml::from_str(content)?;
        let rules = raw
            .rules
            .into_iter()
            .map(|r| r.into_rule(jit_root))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { rules })
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
        let assert = self.assert.into_assertion(&self.name, jit_root)?;
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
            || self.dependency_shape.is_some();
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
            return Ok(Assertion::LabelCoverage { config });
        }
        if let Some(config) = self.label_reference {
            return Ok(Assertion::LabelReference { config });
        }
        if let Some(config) = self.dependency_shape {
            return Ok(Assertion::DependencyShape { config });
        }

        // Unreachable: total == 1 guarantees one branch above matched.
        Err(RuleConfigError::InvalidAssertion {
            rule: rule.to_string(),
            message: "internal error: assertion kind not handled".to_string(),
        })
    }
}

/// Load a `.jit/schemas/<name>.json` file referenced by a `json-schema` rule and
/// transcode it into a `serde_json::Value`.
fn load_schema_source(
    rule: &str,
    jit_root: &Path,
    reference: String,
) -> Result<SchemaSource, RuleConfigError> {
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
            state: Some("in_progress".to_string()),
            ..Default::default()
        };
        assert!(sel.matches(&issue_with(&[], State::InProgress)));
        assert!(!sel.matches(&issue_with(&[], State::Ready)));
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
            state: Some("ready".to_string()),
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
