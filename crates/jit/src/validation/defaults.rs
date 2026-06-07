//! Built-in DEFAULT rule set: the migrated former hard-coded checks.
//!
//! Before the declarative engine, label/type validation was split across two
//! hard-coded sites: a per-issue `IssueValidator` (run on write) and the
//! whole-repo `validate_labels`/`validate_type_hierarchy` (run only by
//! `jit validate`). Both consumed `config.toml`'s `[validation]` block and the
//! `[namespaces]` registry. This module re-expresses every one of those checks as
//! declarative [`Rule`]s derived from the SAME inputs, so behavior is preserved
//! (DR §8.3, plan step 8).
//!
//! # Composition with user rules
//!
//! The default rule set is the BASE ruleset. A user `.jit/rules.toml` is appended
//! after it (see [`CommandExecutor::effective_rules`](crate::commands::CommandExecutor)),
//! so user rules extend — and, by being authored later, can add stricter checks
//! alongside — the defaults. The defaults are derived per-repo from that repo's
//! own `config.toml`, NOT a fixed static table, because the namespace registry
//! (allowed values, patterns, uniqueness, required-ness) differs per repo.
//!
//! # Enforcement parity (DR §7.2, §8.3)
//!
//! Each default rule carries an `enforce` flag matching the legacy reject-vs-warn
//! behavior under the repo's effective config:
//!
//! - `require_type_label`, the per-whole-label format regex (`label_regex`), and
//!   the namespace-registry check map to LOCAL rules whose `enforce` follows the
//!   legacy `[validation]` flags (`require_type_label` / `reject_malformed_labels`
//!   respectively). They default to warn (`enforce = false`) — exactly today's
//!   non-blocking default.
//! - The per-namespace `values` / `pattern` / `unique` / `required` constraints
//!   were enforced ONLY by the whole-repo `validate_labels` (never on write), so
//!   their default rules are `severity = error` with `enforce = false`: they
//!   surface as errors in `jit validate` (which fails on any error finding) but
//!   never block a write, preserving the legacy timing exactly.
//!
//! Most rules here are [`Scope::Local`]. The orphan-leaf and
//! strategic-consistency warnings are the exceptions: they need the whole issue
//! set, so they are built-in [`Scope::Graph`] rules
//! ([`Assertion::TypeHierarchy`](crate::validation::rules::Assertion::TypeHierarchy))
//! whose evaluation REUSES the existing
//! [`type_hierarchy::validate_orphans`](crate::type_hierarchy::validate_orphans)
//! / [`validate_strategic_labels`](crate::type_hierarchy::validate_strategic_labels)
//! domain functions. They remain warn-only and config-toggled
//! (`warn_orphaned_leaves` / `warn_strategic_consistency`), exactly as the
//! former hard-coded `check_warnings` path was.

use crate::config::ValidationConfig;
use crate::domain::LabelNamespaces;
use crate::type_hierarchy::HierarchyConfig;
use crate::validation::rules::{
    Assertion, Rule, RuleSet, SchemaSource, Scope, Selector, Severity, TypeHierarchyKind,
};

/// The canonical `namespace:value` label format, mirroring the regex the legacy
/// `validate_labels` enforced unconditionally via `labels::validate_label`.
const CANONICAL_LABEL_REGEX: &str = r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$";

/// Build the built-in default [`RuleSet`] from a repo's validation config and
/// namespace registry.
///
/// The returned rules re-express the former hard-coded checks (type-label
/// requirement, whole-label format, namespace registry, and the per-namespace
/// allowed-values / value-pattern / uniqueness / required constraints) as
/// declarative [`Rule`]s, each carrying an `enforce` flag matching the legacy
/// reject-vs-warn behavior (see the module docs). Rule names are stable and
/// prefixed `default:` so they never collide with user rule names.
///
/// This is a pure function of its inputs: no I/O, deterministic, and total — a
/// malformed value (e.g. a regex that cannot compile) is not rejected here; the
/// engine surfaces it as a compile error when the rule is evaluated, exactly as
/// the legacy `validate_labels` surfaced a bad `pattern` as a config error.
///
/// # Examples
///
/// ```
/// use jit::config::ValidationConfig;
/// use jit::domain::{LabelNamespace, LabelNamespaces};
/// use jit::validation::defaults::default_ruleset;
/// use std::collections::HashMap;
///
/// // A registry with a required, unique `type` namespace restricted to an enum.
/// let mut namespaces = HashMap::new();
/// namespaces.insert(
///     "type".to_string(),
///     LabelNamespace::new("Issue type", true)
///         .required(true)
///         .with_values(vec!["task".to_string(), "bug".to_string()]),
/// );
/// let registry = LabelNamespaces {
///     schema_version: 2,
///     namespaces,
///     type_hierarchy: None,
///     label_associations: None,
///     strategic_types: None,
/// };
/// let validation = ValidationConfig {
///     strictness: None,
///     default_type: None,
///     require_type_label: None,
///     label_regex: None,
///     reject_malformed_labels: None,
///     enforce_namespace_registry: None,
///     warn_orphaned_leaves: None,
///     warn_strategic_consistency: None,
/// };
///
/// let rules = default_ruleset(&validation, &registry);
/// // The required + unique + enum constraints each produce a default rule.
/// assert!(rules.rules.iter().all(|r| r.name.starts_with("default:")));
/// assert!(!rules.rules.is_empty());
/// ```
pub fn default_ruleset(validation: &ValidationConfig, namespaces: &LabelNamespaces) -> RuleSet {
    let mut rules: Vec<Rule> = Vec::new();

    // --- Label format & namespace registry ------------------------------------
    //
    // Two legacy sites enforced these:
    //   * `validate_labels` (whole-repo, `jit validate`): ALWAYS checked the
    //     `namespace:value` format and that each namespace was registered, and
    //     hard-rejected on a violation.
    //   * `IssueValidator` (write path): checked the configurable `label_regex`
    //     and the namespace registry only when their `[validation]` flags were on,
    //     and rejected only when `reject_malformed_labels` was true.
    //
    // A single `error` rule per check unifies both: `severity = error` makes it
    // fail `jit validate` (parity with `validate_labels`), while `enforce` is set
    // to `reject_malformed_labels` so it blocks a WRITE only when the legacy flag
    // would have (parity with `IssueValidator`); otherwise the write only warns.
    let reject_malformed = validation.reject_malformed_labels.unwrap_or(false);

    // require_type_label -> require at least one `type:*` label on every issue.
    // This legacy flag always hard-rejected on the write path when enabled.
    if validation.require_type_label.unwrap_or(false) {
        rules.push(local_rule(
            "default:require-type-label",
            Selector::default(),
            Severity::Error,
            true,
            Assertion::RequireLabel {
                label: "type:*".to_string(),
                min: Some(1),
                max: None,
            },
        ));
    }

    // Label format: every WHOLE label must match the regex. Uses the configured
    // `label_regex` if present, else the canonical format. A per-whole-label
    // (`namespace:value`) pattern cannot use the value-only `label-value-pattern`
    // shorthand, so it is a raw schema over the projection's `raw_labels` array.
    //
    // Parity note: this is a deliberate UNIFICATION. Legacy split the format
    // check across two paths — `IssueValidator` (write) used the configurable
    // `label_regex`, while whole-repo `validate_labels` used the FIXED canonical
    // format. The single rule applies `label_regex` to BOTH paths. For repos
    // whose `label_regex` equals the canonical format (the default) this is
    // identical; a repo that customizes `label_regex` now gets that regex in
    // `jit validate` too, which is the more consistent behavior.
    let format_regex = validation
        .label_regex
        .clone()
        .unwrap_or_else(|| CANONICAL_LABEL_REGEX.to_string());
    rules.push(json_schema_rule(
        "default:label-format",
        Selector::default(),
        Severity::Error,
        reject_malformed,
        raw_labels_pattern_schema(&format_regex),
    ));

    // Namespace registry: every label's namespace must be declared. Always
    // generated when the registry is non-empty (the legacy validate-time
    // `validate_labels` check warned regardless of any flag); empty registry =>
    // skip (nothing to check against).
    //
    // Enforcement parity: the legacy WRITE-PATH registry check
    // (`IssueValidator::validate_namespace_registry`) returned early unless
    // `enforce_namespace_registry = true`, and then HARD-REJECTED only when
    // `reject_malformed_labels = true` (otherwise it merely warned). A write was
    // therefore blocked ONLY when BOTH flags were true, so `enforce` must be
    // their conjunction. Severity stays `error` so an unknown namespace still
    // fails `jit validate` (parity with the validate-time registry check).
    if !namespaces.namespaces.is_empty() {
        let registered: Vec<&str> = namespaces.namespaces.keys().map(|s| s.as_str()).collect();
        let registry_blocks =
            validation.enforce_namespace_registry.unwrap_or(false) && reject_malformed;
        rules.push(json_schema_rule(
            "default:namespace-registry",
            Selector::default(),
            Severity::Error,
            registry_blocks,
            registered_namespace_schema(&registered),
        ));
    }

    // Unknown type label: a `type:<value>` label whose value is not one of the
    // configured hierarchy types. Legacy `validate_type_hierarchy` (run only by
    // `jit validate`) flagged this and did NOT block writes. This is exactly a
    // per-namespace allowed-VALUES rule over the `type` namespace, with the
    // allowed set derived from the hierarchy config (`type_hierarchy.types`
    // keys). Severity = error (so `jit validate` fails on it) with enforce =
    // false (so it never blocks a write), matching legacy timing precisely.
    //
    // The hierarchy is always present (a repo with no `[type_hierarchy]` falls
    // back to the default 4-level set via `get_type_hierarchy`), mirroring the
    // legacy `HierarchyConfig::default()` fallback, so this rule is always
    // emitted.
    let mut hierarchy_types: Vec<String> = namespaces.get_type_hierarchy().into_keys().collect();
    hierarchy_types.sort(); // deterministic schema enum order
    rules.push(json_schema_rule(
        "default:type-hierarchy-known",
        Selector::default(),
        Severity::Error,
        false,
        namespace_values_schema("type", &hierarchy_types),
    ));

    // --- [namespaces] per-namespace constraints (legacy validate_labels) -------
    //
    // These ran ONLY in the whole-repo `validate_labels` (never on write) and
    // ALWAYS hard-rejected. Map to `error` + `enforce = false`: an error finding
    // fails `jit validate` (parity) but never blocks a write (parity: writes that
    // violate them succeeded before).
    let mut ns_names: Vec<&String> = namespaces.namespaces.keys().collect();
    ns_names.sort(); // deterministic rule order
    for name in ns_names {
        let ns = &namespaces.namespaces[name];

        // values -> allowed-value enum on `labels.<ns>` items.
        if let Some(values) = &ns.values {
            rules.push(json_schema_rule(
                &format!("default:namespace-values:{name}"),
                Selector::default(),
                Severity::Error,
                false,
                namespace_values_schema(name, values),
            ));
        }

        // pattern -> value-portion regex on `labels.<ns>` items.
        if let Some(pattern) = &ns.pattern {
            rules.push(local_rule(
                &format!("default:namespace-pattern:{name}"),
                Selector::default(),
                Severity::Error,
                false,
                Assertion::LabelValuePattern {
                    namespace: name.clone(),
                    regex: pattern.clone(),
                },
            ));
        }

        // unique -> at most one label in the namespace.
        if ns.unique {
            rules.push(local_rule(
                &format!("default:namespace-unique:{name}"),
                Selector::default(),
                Severity::Error,
                false,
                Assertion::RequireLabel {
                    label: format!("{name}:*"),
                    min: Some(0),
                    max: Some(1),
                },
            ));
        }

        // required -> at least one label in the namespace.
        if ns.is_required() {
            rules.push(local_rule(
                &format!("default:namespace-required:{name}"),
                Selector::default(),
                Severity::Error,
                false,
                Assertion::RequireLabel {
                    label: format!("{name}:*"),
                    min: Some(1),
                    max: None,
                },
            ));
        }
    }

    // --- Type-hierarchy GRAPH warnings (legacy validate_type_hierarchy path) ---
    //
    // The orphan-leaf and strategic-consistency checks were warn-only, run by
    // `jit validate` (via `check_warnings`), and gated by the `[validation]`
    // toggles `warn_orphaned_leaves` / `warn_strategic_consistency` (both default
    // true). They are now built-in GRAPH rules whose evaluation REUSES the
    // existing `type_hierarchy::validate_orphans` / `validate_strategic_labels`
    // domain functions (see `validation::graph::evaluate_type_hierarchy`). Each is
    // severity Warn + enforce = false (legacy was never blocking) and is emitted
    // only when its config toggle is enabled. The repo's `HierarchyConfig` is
    // derived from the same namespace registry the rest of the defaults use, so a
    // repo with no `[type_hierarchy]` falls back to the default 4-level set —
    // exactly as the legacy `HierarchyConfig::default()` fallback did.
    let hierarchy = hierarchy_config(namespaces);
    if validation.warn_orphaned_leaves.unwrap_or(true) {
        rules.push(graph_rule(
            "default:orphan-leaf",
            Severity::Warn,
            Assertion::TypeHierarchy {
                kind: TypeHierarchyKind::OrphanLeaf,
                config: hierarchy.clone(),
            },
        ));
    }
    if validation.warn_strategic_consistency.unwrap_or(true) {
        rules.push(graph_rule(
            "default:strategic-consistency",
            Severity::Warn,
            Assertion::TypeHierarchy {
                kind: TypeHierarchyKind::StrategicConsistency,
                config: hierarchy,
            },
        ));
    }

    RuleSet { rules }
}

/// Build the repo's [`HierarchyConfig`] from its label-namespace registry.
///
/// Mirrors the legacy `check_warnings` path EXACTLY: that path built the config
/// from `config.toml`'s `[type_hierarchy]` when present (taking its `types` and
/// `label_associations.unwrap_or_default()`), and otherwise fell back to the
/// FULL [`HierarchyConfig::default`] (which includes the default membership
/// associations). The discriminator is whether an explicit `type_hierarchy` was
/// configured — carried through to [`LabelNamespaces::type_hierarchy`]. On the
/// impossible case of a malformed hierarchy (empty type name / level 0), it falls
/// back to the default rather than panicking, keeping this total.
fn hierarchy_config(namespaces: &LabelNamespaces) -> HierarchyConfig {
    match &namespaces.type_hierarchy {
        // Explicit hierarchy: use its types + associations (legacy `Some` branch).
        Some(types) => {
            let label_associations = namespaces.label_associations.clone().unwrap_or_default();
            HierarchyConfig::new(types.clone(), label_associations).unwrap_or_default()
        }
        // No explicit hierarchy: the legacy default (with default associations).
        None => HierarchyConfig::default(),
    }
}

/// Construct a local-scope rule with a shorthand or raw assertion already built.
fn local_rule(
    name: &str,
    when: Selector,
    severity: Severity,
    enforce: bool,
    assert: Assertion,
) -> Rule {
    let scope = assert.scope();
    debug_assert_eq!(scope, Scope::Local, "default rules are local-scope only");
    Rule {
        name: name.to_string(),
        when,
        severity,
        enforce,
        assert,
        scope,
    }
}

/// Construct a built-in graph-scope rule (warn-only, never blocking). Used for
/// the type-hierarchy defaults, whose assertions are [`Scope::Graph`].
fn graph_rule(name: &str, severity: Severity, assert: Assertion) -> Rule {
    let scope = assert.scope();
    debug_assert_eq!(scope, Scope::Graph, "graph default rules are graph-scope");
    Rule {
        name: name.to_string(),
        when: Selector::default(),
        severity,
        enforce: false,
        assert,
        scope,
    }
}

/// Construct a local-scope rule carrying a raw JSON Schema (inline, no file).
fn json_schema_rule(
    name: &str,
    when: Selector,
    severity: Severity,
    enforce: bool,
    schema: serde_json::Value,
) -> Rule {
    Rule {
        name: name.to_string(),
        when,
        severity,
        enforce,
        assert: Assertion::JsonSchema(SchemaSource {
            reference: format!("<default:{name}>"),
            path: std::path::PathBuf::from(format!("<default:{name}>")),
            schema,
        }),
        scope: Scope::Local,
    }
}

/// Schema asserting every entry of the projection's `raw_labels` array matches
/// `regex` (the whole `namespace:value` label, mirroring legacy `label_regex`).
fn raw_labels_pattern_schema(regex: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "raw_labels": {
                "type": "array",
                "items": { "type": "string", "pattern": regex }
            }
        }
    })
}

/// Schema asserting every entry of `raw_labels` has a namespace prefix that is
/// one of the `registered` namespaces (mirroring legacy namespace-registry).
///
/// Each label is `namespace:value`; the schema requires the whole string to match
/// `^(ns1|ns2|...):` for one of the registered namespaces (anchored, with the
/// alternatives regex-escaped). A label with no registered prefix fails.
fn registered_namespace_schema(registered: &[&str]) -> serde_json::Value {
    // An empty registry means NOTHING is registered: every namespaced label is
    // unknown. Express that as a pattern that no `namespace:` prefix can match.
    let alternation = if registered.is_empty() {
        // `(?!)` is not portable; use an impossible alternative instead.
        "\\b\\B".to_string()
    } else {
        registered
            .iter()
            .map(|ns| regex_escape(ns))
            .collect::<Vec<_>>()
            .join("|")
    };
    let pattern = format!("^({alternation}):");
    serde_json::json!({
        "type": "object",
        "properties": {
            "raw_labels": {
                "type": "array",
                "items": { "type": "string", "pattern": pattern }
            }
        }
    })
}

/// Schema asserting every value in `labels.<namespace>` is one of `values`
/// (mirroring the legacy per-namespace allowed-value enum).
fn namespace_values_schema(namespace: &str, values: &[String]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "labels": {
                "type": "object",
                "properties": {
                    namespace: {
                        "type": "array",
                        "items": { "enum": values }
                    }
                }
            }
        }
    })
}

/// Escape regex metacharacters in a literal namespace name so it can be embedded
/// safely in an alternation. Namespace names are validated lowercase identifiers,
/// but escaping keeps the schema correct even if that ever loosens.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if matches!(
            ch,
            '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Issue, LabelNamespace};
    use crate::validation::evaluate_local;
    use std::collections::HashMap;

    fn empty_validation() -> ValidationConfig {
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

    fn issue_with(labels: &[&str]) -> Issue {
        let mut issue = Issue::new("t".to_string(), String::new());
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue
    }

    #[test]
    fn test_empty_config_still_emits_format_rule() {
        // Even with no config, the canonical label-format rule is always present
        // (the legacy `validate_labels` checked format unconditionally), as is the
        // type-hierarchy-known rule (legacy `validate_type_hierarchy` ran
        // unconditionally over the default hierarchy) and the two type-hierarchy
        // graph warnings (toggles default-on). With no namespace registry, no
        // registry rule is emitted.
        let rules = default_ruleset(&empty_validation(), &registry(vec![]));
        let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "default:label-format",
                "default:type-hierarchy-known",
                "default:orphan-leaf",
                "default:strategic-consistency",
            ]
        );
    }

    #[test]
    fn test_label_format_always_present_and_warns_by_default() {
        // A malformed label fails the always-on canonical format rule, but only
        // warns on the write path when reject_malformed_labels is unset.
        let rules = default_ruleset(&empty_validation(), &registry(vec![]));
        let eval = evaluate_local(&issue_with(&["INVALID:label"]), &rules).unwrap();
        assert!(!eval.is_blocking());
        assert!(eval
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        let ok = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(ok.findings().is_empty());
    }

    #[test]
    fn test_require_type_label_rule_blocks_when_missing() {
        let mut validation = empty_validation();
        validation.require_type_label = Some(true);
        let rules = default_ruleset(&validation, &registry(vec![]));

        // Missing type label -> blocks (enforce=true).
        let eval = evaluate_local(&issue_with(&["epic:auth"]), &rules).unwrap();
        assert!(eval.is_blocking());

        // Present -> passes.
        let eval = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(!eval.is_blocking());
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_label_format_warns_by_default() {
        let mut validation = empty_validation();
        validation.label_regex = Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string());
        // reject_malformed_labels unset -> warn only.
        let rules = default_ruleset(&validation, &registry(vec![]));

        let eval = evaluate_local(&issue_with(&["INVALID:label"]), &rules).unwrap();
        assert!(
            !eval.is_blocking(),
            "malformed label must only warn by default"
        );
        assert_eq!(eval.warnings().len(), 1);
    }

    #[test]
    fn test_label_format_blocks_when_reject_enabled() {
        let mut validation = empty_validation();
        validation.label_regex = Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string());
        validation.reject_malformed_labels = Some(true);
        let rules = default_ruleset(&validation, &registry(vec![]));

        let eval = evaluate_local(&issue_with(&["INVALID:label"]), &rules).unwrap();
        assert!(eval.is_blocking());

        let ok = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(!ok.is_blocking());
    }

    #[test]
    fn test_namespace_registry_warns_when_enforce_off() {
        // With the registry configured but `enforce_namespace_registry` off, an
        // unknown namespace surfaces as an error finding (fails `jit validate`)
        // that does NOT block a write.
        let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let eval = evaluate_local(&issue_with(&["unknown:x"]), &rules).unwrap();
        assert!(
            !eval.is_blocking(),
            "registry must not block when enforce off"
        );
        assert!(eval
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        // Registered namespace -> clean.
        let eval = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_namespace_registry_blocks_when_enforce_on() {
        // `enforce_namespace_registry = true` -> the registry rule blocks a write
        // for an unknown namespace.
        let mut validation = empty_validation();
        validation.enforce_namespace_registry = Some(true);
        let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let rules = default_ruleset(&validation, &reg);

        let eval = evaluate_local(&issue_with(&["unknown:x"]), &rules).unwrap();
        assert!(eval.is_blocking());

        let eval = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_namespace_values_enum_errors_but_does_not_block() {
        let reg = registry(vec![(
            "type",
            LabelNamespace::new("Type", true)
                .with_values(vec!["task".to_string(), "bug".to_string()]),
        )]);
        let rules = default_ruleset(&empty_validation(), &reg);

        // Value outside the enum -> an error finding that does NOT block writes.
        let eval = evaluate_local(&issue_with(&["type:taks"]), &rules).unwrap();
        assert!(!eval.is_blocking(), "enum violation must not block writes");
        assert!(eval
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        // Value inside the enum -> clean.
        let eval = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_namespace_pattern_errors_but_does_not_block() {
        let reg = registry(vec![(
            "milestone",
            LabelNamespace::new("Release", false).with_pattern(r"^v\d+\.\d+$"),
        )]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let bad = evaluate_local(&issue_with(&["milestone:1.2"]), &rules).unwrap();
        assert!(!bad.is_blocking());
        assert!(bad.findings().iter().any(|f| f.severity == Severity::Error));

        let good = evaluate_local(&issue_with(&["milestone:v1.0"]), &rules).unwrap();
        assert!(good.findings().is_empty());
    }

    #[test]
    fn test_namespace_required_errors_but_does_not_block() {
        let reg = registry(vec![(
            "type",
            LabelNamespace::new("Type", true).required(true),
        )]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let missing = evaluate_local(&issue_with(&["component:core"]), &rules).unwrap();
        assert!(!missing.is_blocking());
        assert!(missing
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        let present = evaluate_local(&issue_with(&["type:task"]), &rules).unwrap();
        assert!(present.findings().is_empty());
    }

    #[test]
    fn test_namespace_unique_errors_on_duplicate() {
        let reg = registry(vec![("priority", LabelNamespace::new("Priority", true))]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let dup = evaluate_local(&issue_with(&["priority:high", "priority:low"]), &rules).unwrap();
        assert!(dup.findings().iter().any(|f| f.severity == Severity::Error));

        let single = evaluate_local(&issue_with(&["priority:high"]), &rules).unwrap();
        assert!(single.findings().is_empty());
    }

    #[test]
    fn test_all_rules_are_named_default_and_unique() {
        let mut validation = empty_validation();
        validation.require_type_label = Some(true);
        validation.label_regex = Some("^x".to_string());
        validation.enforce_namespace_registry = Some(true);
        let reg = registry(vec![(
            "type",
            LabelNamespace::new("Type", true)
                .required(true)
                .with_pattern("^t")
                .with_values(vec!["task".to_string()]),
        )]);
        let rules = default_ruleset(&validation, &reg);
        // Local rules dominate; the only graph rules are the two type-hierarchy
        // warnings (gated by their toggles, both default-on here).
        assert!(rules
            .rules
            .iter()
            .filter(|r| r.scope == Scope::Graph)
            .all(|r| r.name == "default:orphan-leaf" || r.name == "default:strategic-consistency"));
        assert!(rules.rules.iter().all(|r| r.name.starts_with("default:")));
        // All generated rule names are unique.
        let mut names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        names.sort();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "default rule names must be unique");
    }
}
