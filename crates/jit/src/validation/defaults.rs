//! Built-in DEFAULT rule set: the fixed default validation rules.
//!
//! `.jit/rules.toml` is the SOLE source of truth for issue/label validation (DR
//! §8.2/§8.4). This module produces the FIXED default rule set that `jit init`
//! serializes into that file, and that
//! [`CommandExecutor::effective_rules`](crate::commands::CommandExecutor) builds
//! IN MEMORY when no `rules.toml` exists yet (no disk write on the read path).
//!
//! After the backward-compat hard removal (issue d4188154), the default set no
//! longer reads any `[validation]` enforcement flags or per-namespace
//! `values`/`pattern`/`required` constraints — those keys were removed. The set
//! is derived purely from the RETAINED taxonomy: the `[namespaces]` registry and
//! `[type_hierarchy]`. Hence the signature is [`default_ruleset(namespaces)`].
//!
//! # The fixed default contract (MF1)
//!
//! [`default_ruleset`] emits EXACTLY:
//!
//! 1. `default:label-format` — `severity = error`, `enforce = true`, ALWAYS. The
//!    canonical `namespace:value` whole-label format; blocks the write and fails
//!    `jit validate`.
//! 2. `default:namespace-registry` — `severity = error`, `enforce = false`, when
//!    the namespace registry is NON-EMPTY. An unknown namespace fails
//!    `jit validate` but never blocks a write.
//! 3. `default:type-hierarchy-known` — `severity = error`, `enforce = false`,
//!    ALWAYS. A `type:<value>` outside the configured hierarchy fails
//!    `jit validate` but never blocks a write.
//! 4. `default:namespace-unique:<ns>` — `severity = error`, `enforce = true`, per
//!    UNIQUE namespace (sorted). At most one label per unique namespace; blocks
//!    the write and fails `jit validate`.
//! 5. `default:orphan-leaf` + `default:strategic-consistency` — `severity = warn`,
//!    `enforce = false`, UNCONDITIONAL. Built-in [`Scope::Graph`] rules whose
//!    evaluation REUSES the existing
//!    [`type_hierarchy::validate_orphans`](crate::type_hierarchy::validate_orphans)
//!    / [`validate_strategic_labels`](crate::type_hierarchy::validate_strategic_labels)
//!    domain functions.
//!
//! DROPPED (no longer config-derivable): `require-type-label`,
//! `label-format-custom`, and the per-namespace `values`/`pattern`/`required`
//! rules. A repo wanting those authors them directly in `rules.toml`.

use crate::domain::LabelNamespaces;
use crate::type_hierarchy::HierarchyConfig;
use crate::validation::rules::{
    Assertion, Rule, RuleSet, SchemaSource, Scope, Selector, Severity, TypeHierarchyKind,
};

/// The canonical `namespace:value` label format, mirroring the regex the legacy
/// `validate_labels` enforced unconditionally via `labels::validate_label`.
const CANONICAL_LABEL_REGEX: &str = r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$";

/// Build the FIXED built-in default [`RuleSet`] from a repo's namespace registry
/// + type hierarchy (MF1).
///
/// The returned rules are derived purely from the RETAINED taxonomy (the
/// `[namespaces]` registry and `[type_hierarchy]`); the removed `[validation]`
/// enforcement flags and per-namespace constraint fields no longer influence it.
/// Rule names are stable and prefixed `default:`; they are serialized into
/// `rules.toml` under those names and are user-editable there.
///
/// See the module docs for the EXACT emitted contract. This is a pure function of
/// its input: no I/O, deterministic, and total.
///
/// # Examples
///
/// ```
/// use jit::domain::{LabelNamespace, LabelNamespaces};
/// use jit::validation::defaults::default_ruleset;
/// use std::collections::HashMap;
///
/// // A registry with a unique `type` namespace.
/// let mut namespaces = HashMap::new();
/// namespaces.insert("type".to_string(), LabelNamespace::new("Issue type", true));
/// let registry = LabelNamespaces {
///     schema_version: 2,
///     namespaces,
///     type_hierarchy: None,
///     label_associations: None,
///     strategic_types: None,
/// };
///
/// let rules = default_ruleset(&registry);
/// assert!(rules.rules.iter().all(|r| r.name.starts_with("default:")));
/// // The unique `type` namespace yields a uniqueness rule.
/// assert!(rules
///     .rules
///     .iter()
///     .any(|r| r.name == "default:namespace-unique:type"));
/// ```
pub fn default_ruleset(namespaces: &LabelNamespaces) -> RuleSet {
    let mut rules: Vec<Rule> = Vec::new();

    // (1) Label format (canonical): every WHOLE label must match the FIXED
    // canonical `namespace:value` format. `severity = error` (fails
    // `jit validate`) AND `enforce = true` ALWAYS (blocks the write). A
    // per-whole-label pattern cannot use the value-only `label-value-pattern`
    // shorthand, so it is a raw schema over the projection's `raw_labels` array.
    rules.push(json_schema_rule(
        "default:label-format",
        Selector::default(),
        Severity::Error,
        true,
        raw_labels_pattern_schema(CANONICAL_LABEL_REGEX),
    ));

    // (2) Namespace registry: every label's namespace must be declared. Emitted
    // only when the registry is NON-EMPTY. `severity = error` (an unknown
    // namespace fails `jit validate`) with `enforce = false` (never blocks a
    // write).
    if !namespaces.namespaces.is_empty() {
        let mut registered: Vec<&str> = namespaces.namespaces.keys().map(|s| s.as_str()).collect();
        registered.sort(); // deterministic alternation order (namespaces is a HashMap)
        rules.push(json_schema_rule(
            "default:namespace-registry",
            Selector::default(),
            Severity::Error,
            false,
            registered_namespace_schema(&registered),
        ));
    }

    // (3) Unknown type label: a `type:<value>` outside the configured hierarchy.
    // Modeled as an allowed-VALUES rule over the `type` namespace, the allowed set
    // being the hierarchy `types` keys. `severity = error` / `enforce = false`.
    // The hierarchy is always present (a repo with no `[type_hierarchy]` falls
    // back to the default 4-level set via `get_type_hierarchy`), so this rule is
    // always emitted.
    let mut hierarchy_types: Vec<String> = namespaces.get_type_hierarchy().into_keys().collect();
    hierarchy_types.sort(); // deterministic schema enum order
    rules.push(json_schema_rule(
        "default:type-hierarchy-known",
        Selector::default(),
        Severity::Error,
        false,
        namespace_values_schema("type", &hierarchy_types),
    ));

    // (4) Per-namespace UNIQUENESS: at most one label per unique namespace.
    // `severity = error` / `enforce = true` (blocks the write and fails
    // `jit validate`). One rule per UNIQUE namespace, in sorted order.
    let mut ns_names: Vec<&String> = namespaces.namespaces.keys().collect();
    ns_names.sort(); // deterministic rule order
    for name in ns_names {
        let ns = &namespaces.namespaces[name];
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
    }

    // (5) Type-hierarchy GRAPH warnings: orphan-leaf + strategic-consistency.
    // Built-in GRAPH rules whose evaluation REUSES the existing
    // `type_hierarchy::validate_orphans` / `validate_strategic_labels` domain
    // functions (see `validation::graph`). Each is `severity = warn` /
    // `enforce = false` and UNCONDITIONAL (the former `warn_*` toggles defaulted
    // true, so unconditional preserves behavior). The repo `HierarchyConfig` is
    // injected by the graph evaluator at evaluation time.
    rules.push(graph_rule(
        "default:orphan-leaf",
        Severity::Warn,
        Assertion::TypeHierarchy {
            kind: TypeHierarchyKind::OrphanLeaf,
        },
    ));
    rules.push(graph_rule(
        "default:strategic-consistency",
        Severity::Warn,
        Assertion::TypeHierarchy {
            kind: TypeHierarchyKind::StrategicConsistency,
        },
    ));

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
    fn test_empty_registry_emits_exactly_the_unconditional_rules() {
        // With NO namespace registry, the fixed default emits exactly:
        // label-format, type-hierarchy-known, orphan-leaf, strategic-consistency.
        // No registry rule (registry empty), no uniqueness rules (no namespaces).
        let rules = default_ruleset(&registry(vec![]));
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
    fn test_fixed_default_contract_mf1() {
        // The EXACT MF1 contract: name -> (severity, enforce), in emission order,
        // for a registry with two unique + one non-unique namespace.
        let rules = default_ruleset(&registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
            ("component", LabelNamespace::new("Component", false)),
        ]));
        let got: Vec<(&str, Severity, bool)> = rules
            .rules
            .iter()
            .map(|r| (r.name.as_str(), r.severity, r.enforce))
            .collect();
        assert_eq!(
            got,
            vec![
                ("default:label-format", Severity::Error, true),
                ("default:namespace-registry", Severity::Error, false),
                ("default:type-hierarchy-known", Severity::Error, false),
                // namespace-unique only for the UNIQUE namespaces, sorted.
                ("default:namespace-unique:team", Severity::Error, true),
                ("default:namespace-unique:type", Severity::Error, true),
                ("default:orphan-leaf", Severity::Warn, false),
                ("default:strategic-consistency", Severity::Warn, false),
            ]
        );
        // The dropped rules are never emitted.
        let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        assert!(!names.contains(&"default:require-type-label"));
        assert!(!names.contains(&"default:label-format-custom"));
        assert!(!names
            .iter()
            .any(|n| n.starts_with("default:namespace-values:")));
        assert!(!names
            .iter()
            .any(|n| n.starts_with("default:namespace-pattern:")));
        assert!(!names
            .iter()
            .any(|n| n.starts_with("default:namespace-required:")));
    }

    #[test]
    fn test_canonical_label_format_always_blocks_malformed() {
        // The canonical format rule is ALWAYS enforced (enforce=true), so a
        // malformed label blocks the write.
        let rules = default_ruleset(&registry(vec![]));
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
    fn test_namespace_registry_warns_but_does_not_block() {
        // With a registry configured, an unknown namespace surfaces as an error
        // finding (fails `jit validate`) that does NOT block a write.
        let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let rules = default_ruleset(&reg);

        let eval = evaluate_local(
            &issue_with(&["unknown:x"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            !eval.is_blocking(),
            "registry must never block a write (enforce=false)"
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
    fn test_no_registry_rule_when_registry_empty() {
        let rules = default_ruleset(&registry(vec![]));
        assert!(rules
            .rules
            .iter()
            .all(|r| r.name != "default:namespace-registry"));
    }

    #[test]
    fn test_namespace_unique_blocks_on_duplicate() {
        // Uniqueness is ALWAYS enforced (enforce=true): a duplicate unique-
        // namespace label blocks the write.
        let reg = registry(vec![("priority", LabelNamespace::new("Priority", true))]);
        let rules = default_ruleset(&reg);

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
    fn test_non_unique_namespace_has_no_uniqueness_rule() {
        let reg = registry(vec![("epic", LabelNamespace::new("Epic", false))]);
        let rules = default_ruleset(&reg);
        assert!(rules
            .rules
            .iter()
            .all(|r| r.name != "default:namespace-unique:epic"));
    }

    #[test]
    fn test_type_hierarchy_known_errors_but_does_not_block() {
        // A type value outside the (default) hierarchy errors in validate but does
        // not block a write.
        let rules = default_ruleset(&registry(vec![("type", LabelNamespace::new("Type", true))]));
        let bad = evaluate_local(
            &issue_with(&["type:nonsense"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!bad.is_blocking(), "unknown type must not block writes");
        assert!(bad.findings().iter().any(|f| f.severity == Severity::Error));

        let good = evaluate_local(
            &issue_with(&["type:task"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(good.findings().is_empty());
    }

    #[test]
    fn test_all_rules_are_named_default_and_unique() {
        let reg = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]);
        let rules = default_ruleset(&reg);
        // The only graph rules are the two type-hierarchy warnings.
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
