//! Parity tests for the a0f0f342 migration: the former hard-coded label/type
//! checks (the deleted `IssueValidator` + `validate_labels`/`validate_type_hierarchy`)
//! are now built-in DEFAULT rules. These tests pin the EXPECTED legacy outcomes
//! (captured from the pre-migration behavior) and assert the default rules
//! reproduce them — accept vs reject, and warn vs block — for representative
//! cases.
//!
//! Legacy behavior captured (pre-migration):
//!
//! * `IssueValidator` (write path), gated on `[validation]` flags:
//!   - `require_type_label = true`  => an issue with no `type:*` label is REJECTED
//!     on write.
//!   - `label_regex` + `reject_malformed_labels = true` => a label not matching
//!     the regex is REJECTED on write; with `reject_malformed_labels = false` it
//!     only WARNS.
//!   - `enforce_namespace_registry = true` + `reject_malformed_labels = true` =>
//!     an unregistered namespace is REJECTED on write; otherwise WARNS.
//! * `validate_labels` (whole-repo `jit validate`), ALWAYS on, hard-reject:
//!   - a value outside a namespace `values` enum, a value not matching a
//!     namespace `pattern`, multiple labels in a `unique` namespace, and a
//!     missing `required` namespace each FAIL `jit validate` but do NOT block a
//!     write.
//!
//! The default rules are evaluated directly via `default_ruleset` + the local
//! engine so the assertions are about the rule layer itself.

use jit::config::ValidationConfig;
use jit::domain::{Issue, LabelNamespace, LabelNamespaces};
use jit::validation::defaults::default_ruleset;
use jit::validation::evaluate_local;
use jit::validation::rules::{RuleSet, Severity};
use std::collections::HashMap;

fn validation() -> ValidationConfig {
    ValidationConfig {
        strictness: None,
        default_type: None,
        require_type_label: None,
        label_regex: None,
        reject_malformed_labels: None,
        enforce_namespace_registry: None,
        warn_orphaned_leaves: None,
        warn_strategic_consistency: None,
    }
}

fn registry(entries: Vec<(&str, LabelNamespace)>) -> LabelNamespaces {
    let mut namespaces = HashMap::new();
    for (name, ns) in entries {
        namespaces.insert(name.to_string(), ns);
    }
    LabelNamespaces {
        schema_version: 2,
        namespaces,
        type_hierarchy: None,
        label_associations: None,
        strategic_types: None,
    }
}

fn issue(labels: &[&str]) -> Issue {
    let mut i = Issue::new("t".to_string(), String::new());
    i.labels = labels.iter().map(|s| s.to_string()).collect();
    i
}

/// Whether evaluating `labels` against `rules` BLOCKS a write (an enforce rule
/// produced an error finding).
fn blocks(rules: &RuleSet, labels: &[&str]) -> bool {
    evaluate_local(&issue(labels), rules).unwrap().is_blocking()
}

/// Whether evaluating `labels` against `rules` produces any `error`-severity
/// finding (i.e. would FAIL `jit validate`), regardless of whether it blocks.
fn fails_validate(rules: &RuleSet, labels: &[&str]) -> bool {
    evaluate_local(&issue(labels), rules)
        .unwrap()
        .findings()
        .iter()
        .any(|f| f.severity == Severity::Error)
}

// ---------------------------------------------------------------------------
// require_type_label (legacy IssueValidator, write path)
// ---------------------------------------------------------------------------

#[test]
fn parity_require_type_label_blocks_when_missing() {
    let mut v = validation();
    v.require_type_label = Some(true);
    let rules = default_ruleset(&v, &registry(vec![]));

    // Legacy: REJECTED on write when no type label.
    assert!(blocks(&rules, &["epic:auth"]));
    // Legacy: accepted when present.
    assert!(!blocks(&rules, &["type:task"]));
}

#[test]
fn parity_require_type_label_off_by_default() {
    // Legacy default (`require_type_label` unset) never blocked on a missing type.
    let rules = default_ruleset(&validation(), &registry(vec![]));
    assert!(!blocks(&rules, &["epic:auth"]));
}

// ---------------------------------------------------------------------------
// label format (legacy IssueValidator label_regex / reject_malformed_labels)
// ---------------------------------------------------------------------------

#[test]
fn parity_malformed_label_warns_when_reject_off() {
    let mut v = validation();
    v.label_regex = Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string());
    // reject_malformed_labels unset => warn only (legacy default).
    let rules = default_ruleset(&v, &registry(vec![]));

    let eval = evaluate_local(&issue(&["INVALID:label"]), &rules).unwrap();
    assert!(
        !eval.is_blocking(),
        "legacy: malformed label only WARNS by default"
    );
    assert_eq!(eval.warnings().len(), 1);
}

#[test]
fn parity_malformed_label_blocks_when_reject_on() {
    let mut v = validation();
    v.label_regex = Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string());
    v.reject_malformed_labels = Some(true);
    let rules = default_ruleset(&v, &registry(vec![]));

    // Legacy: REJECTED on write.
    assert!(blocks(&rules, &["INVALID:label"]));
    assert!(!blocks(&rules, &["type:task"]));
}

// ---------------------------------------------------------------------------
// namespace registry (legacy IssueValidator enforce_namespace_registry +
// always-on validate_labels registry check)
// ---------------------------------------------------------------------------

#[test]
fn parity_unknown_namespace_fails_validate_but_does_not_block_when_reject_off() {
    // validate_labels ALWAYS flagged an unknown namespace as a validate error;
    // the write path only warned (reject off).
    let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
    let rules = default_ruleset(&validation(), &reg);

    assert!(fails_validate(&rules, &["unknown:x"]), "validate must fail");
    assert!(!blocks(&rules, &["unknown:x"]), "write must NOT block");
    assert!(!fails_validate(&rules, &["type:task"]));
}

#[test]
fn parity_unknown_namespace_blocks_write_when_reject_on() {
    let mut v = validation();
    v.reject_malformed_labels = Some(true);
    let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
    let rules = default_ruleset(&v, &reg);

    // Legacy: with reject on, the registry check rejected on the write path.
    assert!(blocks(&rules, &["unknown:x"]));
    assert!(!blocks(&rules, &["type:task"]));
}

// ---------------------------------------------------------------------------
// namespace value / pattern / unique / required (legacy validate_labels)
// All: FAIL validate, NEVER block a write.
// ---------------------------------------------------------------------------

#[test]
fn parity_namespace_values_enum() {
    let reg = registry(vec![(
        "type",
        LabelNamespace::new("Type", true).with_values(vec!["task".to_string(), "bug".to_string()]),
    )]);
    let rules = default_ruleset(&validation(), &reg);

    assert!(fails_validate(&rules, &["type:taks"]));
    assert!(
        !blocks(&rules, &["type:taks"]),
        "enum violation never blocks a write"
    );
    assert!(!fails_validate(&rules, &["type:task"]));
}

#[test]
fn parity_namespace_pattern() {
    let reg = registry(vec![(
        "milestone",
        LabelNamespace::new("Release", false).with_pattern(r"^v\d+\.\d+$"),
    )]);
    let rules = default_ruleset(&validation(), &reg);

    assert!(fails_validate(&rules, &["milestone:1.2"]));
    assert!(!blocks(&rules, &["milestone:1.2"]));
    assert!(!fails_validate(&rules, &["milestone:v1.0"]));
}

#[test]
fn parity_namespace_unique() {
    let reg = registry(vec![("priority", LabelNamespace::new("Priority", true))]);
    let rules = default_ruleset(&validation(), &reg);

    assert!(fails_validate(&rules, &["priority:high", "priority:low"]));
    assert!(!blocks(&rules, &["priority:high", "priority:low"]));
    assert!(!fails_validate(&rules, &["priority:high"]));
}

#[test]
fn parity_namespace_required() {
    let reg = registry(vec![(
        "type",
        LabelNamespace::new("Type", true).required(true),
    )]);
    let rules = default_ruleset(&validation(), &reg);

    assert!(fails_validate(&rules, &["component:core"]));
    assert!(!blocks(&rules, &["component:core"]));
    assert!(!fails_validate(&rules, &["type:task"]));
}

// ---------------------------------------------------------------------------
// Composition: user rules extend, never lose, the defaults.
// ---------------------------------------------------------------------------

#[test]
fn parity_user_rules_compose_after_defaults() {
    // A user rule.toml that adds a stricter check must run ALONGSIDE the defaults.
    let reg = registry(vec![(
        "type",
        LabelNamespace::new("Type", true).required(true),
    )]);
    let mut rules = default_ruleset(&validation(), &reg);
    let user = RuleSet::from_toml_str(
        r#"
[[rules]]
name = "epic-needs-req"
when = { type = "epic" }
severity = "error"
enforce = true
assert = { require-label = { label = "req:*", min = 1 } }
"#,
        std::path::Path::new("/nonexistent"),
    )
    .unwrap();
    rules.rules.extend(user.rules);

    // Default required-type rule still fires (a non-type issue fails validate).
    assert!(fails_validate(&rules, &["component:core"]));
    // The user enforce rule blocks an epic with no req label.
    assert!(blocks(&rules, &["type:epic"]));
    // A compliant epic passes both.
    assert!(!blocks(&rules, &["type:epic", "req:REQ-01"]));
}
