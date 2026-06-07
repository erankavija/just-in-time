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
//! # Role under the file-as-source model (DR §8.2, issue 0abaddc0)
//!
//! `.jit/rules.toml` is the OPERATIVE single source of truth: `jit init`
//! SERIALIZES this default rule set into that file, after which
//! [`CommandExecutor::effective_rules`](crate::commands::CommandExecutor) reads the
//! file alone (the `default:*` rules then live in the file and are user-editable).
//! This function is therefore used for (a) generating the scaffolded/migrated
//! `rules.toml`, and (b) a TRANSIENT bootstrap fallback when no `rules.toml` exists
//! yet — it is NOT concatenated with the user file. The defaults are derived
//! per-repo from that repo's `config.toml`, NOT a fixed static table, because the
//! namespace registry (allowed values, patterns, uniqueness, required-ness) differs
//! per repo.
//!
//! # Enforcement parity (DR §7.2, §8.3)
//!
//! Each default rule carries an `enforce` flag matching the legacy reject-vs-warn
//! behavior under the repo's effective config:
//!
//! - The canonical `namespace:value` label format (`default:label-format`) is
//!   ALWAYS enforced (`severity = error`, `enforce = true`), reproducing the
//!   legacy ALWAYS-ON inline `labels::validate_label` write-path reject AND the
//!   canonical check the whole-repo `validate_labels` ran. An optional
//!   `default:label-format-custom` rule (emitted only when `label_regex` is set
//!   and differs from canonical) reproduces the legacy custom-regex write-time
//!   block with `enforce = reject_malformed_labels`. Per the approved deviation
//!   (decision record §8.3a) it ALSO fails `jit validate`, which legacy did not —
//!   the engine has no write-only local rule. Unaffected when `label_regex` is
//!   unset or equals canonical.
//! - Per-namespace UNIQUENESS (`default:namespace-unique:<ns>`) is ALSO always
//!   enforced (`severity = error`, `enforce = true`), reproducing the legacy
//!   inline unique-namespace collision hard-reject on the write path.
//! - `require_type_label` and the namespace-registry check map to LOCAL rules
//!   whose `enforce` follows the legacy `[validation]` flags (`require_type_label`
//!   / `reject_malformed_labels`).
//! - The per-namespace `values` / `pattern` / `required` constraints were enforced
//!   ONLY by the whole-repo `validate_labels` (never on write), so their default
//!   rules are `severity = error` with `enforce = false`: they surface as errors
//!   in `jit validate` (which fails on any error finding) but never block a write,
//!   preserving the legacy timing exactly.
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
/// prefixed `default:`; they are serialized into `rules.toml` under those names
/// and are user-editable there (the former `default:` name reservation was removed
/// when the file became the operative source).
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
///     content_format: None,
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

    // Label format (canonical): every WHOLE label must match the FIXED canonical
    // `namespace:value` format. This is the single source of truth for the two
    // legacy ALWAYS-ON format checks:
    //   * the inline `labels::validate_label(label)?` that EVERY write path
    //     (create/update/bulk/add_label) ran unconditionally and hard-rejected on,
    //     regardless of any config flag; and
    //   * the whole-repo `validate_labels` canonical-format check that always
    //     failed `jit validate` on a malformed label.
    // Hence `severity = error` (fails `jit validate`) AND `enforce = true` ALWAYS
    // (blocks the write), independent of `reject_malformed_labels`. A per-whole-
    // label (`namespace:value`) pattern cannot use the value-only
    // `label-value-pattern` shorthand, so it is a raw schema over the projection's
    // `raw_labels` array.
    rules.push(json_schema_rule(
        "default:label-format",
        Selector::default(),
        Severity::Error,
        true,
        raw_labels_pattern_schema(CANONICAL_LABEL_REGEX),
    ));

    // Custom label format: when a repo configures a `validation.label_regex` that
    // DIFFERS from the canonical format, the legacy `IssueValidator` applied that
    // custom regex on the WRITE path only, gated by `reject_malformed_labels`, and
    // the whole-repo `validate_labels` never used it (it used canonical). We
    // reproduce the write-time blocking exactly via `enforce = reject_malformed_labels`
    // (a write is blocked only when an `enforce` rule emits an `error`).
    //
    // APPROVED DEVIATION (decision record §8.3a, user-approved 2026-06-07): the
    // rule engine has no write-only representation for a LOCAL rule — `jit validate`
    // fails on any `severity = error` finding regardless of `enforce` — so emitting
    // this rule as `severity = error` necessarily makes a custom-regex violation
    // ALSO fail `jit validate`, which legacy did not do. This is the single
    // documented parity deviation; the new behavior is stricter and consistent
    // across the write and validate paths. When `label_regex` is unset or equals
    // canonical, the rule is redundant and is not emitted, so most repos (including
    // this one) are unaffected. See parity test
    // `parity_custom_label_regex_also_applies_to_validate`.
    if let Some(custom_regex) = validation
        .label_regex
        .as_deref()
        .filter(|r| *r != CANONICAL_LABEL_REGEX)
    {
        rules.push(json_schema_rule(
            "default:label-format-custom",
            Selector::default(),
            Severity::Error,
            reject_malformed,
            raw_labels_pattern_schema(custom_regex),
        ));
    }

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
    // `values` / `pattern` / `required` ran ONLY in the whole-repo
    // `validate_labels` (never on write) and ALWAYS hard-rejected: map to `error` +
    // `enforce = false` (fails `jit validate`, never blocks a write). UNIQUENESS is
    // the exception — it ALSO had an inline write-path hard-reject — so its rule is
    // `error` + `enforce = true` (see below).
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

        // unique -> at most one label in the namespace. ALWAYS-enforced: the
        // legacy inline unique-namespace collision check in
        // `create_issue`/`update_issue`/`bulk_update` (and `add_label`)
        // hard-rejected a second label from a `unique` namespace on the WRITE
        // path, regardless of config. So `enforce = true` (blocks the write) with
        // `severity = error` (also fails `jit validate`, matching the whole-repo
        // `validate_labels` uniqueness check).
        if ns.unique {
            rules.push(local_rule(
                &format!("default:namespace-unique:{name}"),
                Selector::default(),
                Severity::Error,
                true,
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
    // The repo `HierarchyConfig` is no longer stored in the assertion; the graph
    // evaluator injects it at evaluation time (see `evaluate_graph`).
    if validation.warn_orphaned_leaves.unwrap_or(true) {
        rules.push(graph_rule(
            "default:orphan-leaf",
            Severity::Warn,
            Assertion::TypeHierarchy {
                kind: TypeHierarchyKind::OrphanLeaf,
            },
        ));
    }
    if validation.warn_strategic_consistency.unwrap_or(true) {
        rules.push(graph_rule(
            "default:strategic-consistency",
            Severity::Warn,
            Assertion::TypeHierarchy {
                kind: TypeHierarchyKind::StrategicConsistency,
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
///
/// Exposed `pub(crate)` so the graph-rule evaluation call site
/// (`CommandExecutor::evaluate_graph_rules`) can build the same repo
/// [`HierarchyConfig`] to inject into `type-hierarchy` rules (D1).
pub(crate) fn hierarchy_config(namespaces: &LabelNamespaces) -> HierarchyConfig {
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
            content_format: None,
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
    fn test_canonical_label_format_always_blocks_malformed() {
        // The canonical format rule is ALWAYS enforced (enforce=true), so a
        // malformed label blocks the write even when reject_malformed_labels is
        // unset, reproducing the legacy inline `validate_label` hard-reject.
        let rules = default_ruleset(&empty_validation(), &registry(vec![]));
        let eval = evaluate_local(
            &issue_with(&["INVALID:label"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(eval.is_blocking(), "malformed label must block (canonical)");
        assert!(eval
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        let ok = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(ok.findings().is_empty());
    }

    #[test]
    fn test_require_type_label_rule_blocks_when_missing() {
        let mut validation = empty_validation();
        validation.require_type_label = Some(true);
        let rules = default_ruleset(&validation, &registry(vec![]));

        // Missing type label -> blocks (enforce=true).
        let eval = evaluate_local(
            &issue_with(&["epic:auth"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(eval.is_blocking());

        // Present -> passes.
        let eval = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!eval.is_blocking());
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_custom_label_regex_only_emitted_when_differs_from_canonical() {
        // A `label_regex` equal to the canonical format is redundant: only the
        // always-on canonical rule is emitted, never `default:label-format-custom`.
        let mut validation = empty_validation();
        validation.label_regex = Some(CANONICAL_LABEL_REGEX.to_string());
        let rules = default_ruleset(&validation, &registry(vec![]));
        assert!(rules
            .rules
            .iter()
            .all(|r| r.name != "default:label-format-custom"));
    }

    #[test]
    fn test_custom_label_regex_write_only_gated_by_reject_flag() {
        // A custom regex stricter than canonical. With reject_malformed_labels
        // unset the custom rule is warn-only on the write path (canonical still
        // blocks genuinely malformed labels, but a canonically-valid label that
        // merely violates the custom regex only warns).
        let mut validation = empty_validation();
        validation.label_regex = Some(r"^team:[a-z]+$".to_string());
        let rules = default_ruleset(&validation, &registry(vec![]));
        assert!(rules
            .rules
            .iter()
            .any(|r| r.name == "default:label-format-custom"));

        // `type:task` is canonical-valid but violates the custom `^team:[a-z]+$`.
        let warn = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            !warn.is_blocking(),
            "custom regex must only warn when reject_malformed off"
        );
        assert!(warn.warnings().iter().any(|w| w.contains("custom")));
        // Approved deviation (§8.3a): although it does not BLOCK the write, the
        // custom-regex finding is `severity=error`, so it DOES fail `jit validate`
        // (legacy applied the custom regex write-time only). See the integration
        // test `parity_custom_label_regex_also_applies_to_validate`.
        assert!(warn
            .findings()
            .iter()
            .any(|f| f.rule == "default:label-format-custom"
                && f.severity == crate::validation::rules::Severity::Error));

        // With reject_malformed_labels on, the custom regex blocks the write.
        validation.reject_malformed_labels = Some(true);
        let rules = default_ruleset(&validation, &registry(vec![]));
        let block = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            block.is_blocking(),
            "custom regex must block when reject on"
        );

        // A label satisfying the custom regex passes.
        let ok = evaluate_local(
            &issue_with(&["team:platform"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!ok.is_blocking());
    }

    #[test]
    fn test_namespace_registry_warns_when_enforce_off() {
        // With the registry configured but `enforce_namespace_registry` off, an
        // unknown namespace surfaces as an error finding (fails `jit validate`)
        // that does NOT block a write.
        let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let eval = evaluate_local(
            &issue_with(&["unknown:x"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            !eval.is_blocking(),
            "registry must not block when enforce off"
        );
        assert!(eval
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        // Registered namespace -> clean.
        let eval = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_namespace_registry_blocks_only_when_both_flags_on() {
        // Legacy parity: the registry write-path block fired only when BOTH
        // `enforce_namespace_registry` AND `reject_malformed_labels` were true
        // (the legacy check returned early without the former and only warned
        // without the latter).
        let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);

        // enforce on, reject off -> warns (does not block).
        let mut one_flag = empty_validation();
        one_flag.enforce_namespace_registry = Some(true);
        one_flag.reject_malformed_labels = Some(false);
        let rules = default_ruleset(&one_flag, &reg);
        let eval = evaluate_local(
            &issue_with(&["unknown:x"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            !eval.is_blocking(),
            "registry must not block a write without reject_malformed_labels"
        );

        // both on -> blocks.
        let mut both = empty_validation();
        both.enforce_namespace_registry = Some(true);
        both.reject_malformed_labels = Some(true);
        let rules = default_ruleset(&both, &reg);
        let eval = evaluate_local(
            &issue_with(&["unknown:x"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(eval.is_blocking());

        let eval = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
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
        let eval = evaluate_local(
            &issue_with(&["type:taks"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!eval.is_blocking(), "enum violation must not block writes");
        assert!(eval
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        // Value inside the enum -> clean.
        let eval = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(eval.findings().is_empty());
    }

    #[test]
    fn test_namespace_pattern_errors_but_does_not_block() {
        let reg = registry(vec![(
            "milestone",
            LabelNamespace::new("Release", false).with_pattern(r"^v\d+\.\d+$"),
        )]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let bad = evaluate_local(
            &issue_with(&["milestone:1.2"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!bad.is_blocking());
        assert!(bad.findings().iter().any(|f| f.severity == Severity::Error));

        let good = evaluate_local(
            &issue_with(&["milestone:v1.0"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(good.findings().is_empty());
    }

    #[test]
    fn test_namespace_required_errors_but_does_not_block() {
        let reg = registry(vec![(
            "type",
            LabelNamespace::new("Type", true).required(true),
        )]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let missing = evaluate_local(
            &issue_with(&["component:core"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!missing.is_blocking());
        assert!(missing
            .findings()
            .iter()
            .any(|f| f.severity == Severity::Error));

        let present = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(present.findings().is_empty());
    }

    #[test]
    fn test_namespace_unique_blocks_on_duplicate() {
        // Uniqueness is ALWAYS enforced (enforce=true): a duplicate unique-
        // namespace label blocks the write, reproducing the legacy inline reject.
        let reg = registry(vec![("priority", LabelNamespace::new("Priority", true))]);
        let rules = default_ruleset(&empty_validation(), &reg);

        let dup = evaluate_local(
            &issue_with(&["priority:high", "priority:low"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(dup.is_blocking(), "duplicate unique label must block");
        assert!(dup.findings().iter().any(|f| f.severity == Severity::Error));

        let single = evaluate_local(
            &issue_with(&["priority:high"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
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
