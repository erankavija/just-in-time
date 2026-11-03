//! Built-in DEFAULT rule set: the fixed default validation rules.
//!
//! `.jit/rules.toml` declares the operative issue/label ruleset (DR §8.2/§8.4).
//! This module produces the FIXED default rule set that `jit init` serializes
//! into that file, that
//! [`CommandExecutor::effective_rules`](crate::commands::CommandExecutor) builds
//! IN MEMORY when no `rules.toml` exists yet (no disk write on the read path),
//! and that [`reconcile_default_rules_with_config`] reconciles a scaffolded
//! file's default rules against the current registry at load — so a default
//! rule's assertion and the family's membership follow `config.toml`,
//! never a stale `schemas/default-*.json` projection.
//!
//! After the backward-compat hard removal (issue d4188154), the default set no
//! longer reads any `[validation]` enforcement flags or per-namespace
//! `values`/`pattern`/`required` constraints — those keys were removed. The set
//! is derived purely from the RETAINED taxonomy: the `[namespaces]` registry and
//! `[type_hierarchy]`. Hence the signature is [`default_ruleset(namespaces)`].
//!
//! # The fixed default contract (MF1)
//!
//! [`default_ruleset`] emits EXACTLY, each carrying `origin = "default"` (a rule
//! `name` is a colon-free slug; `:` stays reserved for the label
//! `namespace:value` separator):
//!
//! 1. `label-format` — `severity = error`, `enforce = true`, ALWAYS. The
//!    canonical `namespace:value` whole-label format; blocks the write and fails
//!    `jit validate`.
//! 2. `namespace-registry` — `severity = error`, `enforce = false`, when
//!    the namespace registry is NON-EMPTY. An unknown namespace fails
//!    `jit validate` but never blocks a write.
//! 3. `type-hierarchy-known` — `severity = error`, `enforce = false`, when the
//!    declared type hierarchy is NON-EMPTY. A `type:<value>` outside the
//!    configured hierarchy fails `jit validate` but never blocks a write.
//! 4. `namespace-unique-<ns>` — `severity = error`, `enforce = true`, per
//!    UNIQUE namespace (sorted). At most one label per unique namespace; blocks
//!    the write and fails `jit validate`.
//!
//! Every member is therefore derived from a declaration: a repository whose
//! `config.toml` declares neither `[namespaces]` nor `[type_hierarchy]` receives
//! the label grammar alone.
//!
//! DROPPED (no longer config-derivable): `require-type-label`,
//! `label-format-custom`, and the per-namespace `values`/`pattern`/`required`
//! rules. A repo wanting those authors them directly in `rules.toml`.

use crate::declarations::rules::{
    Assertion, Rule, RuleConfigError, RuleScope, RuleSet, SchemaSource, Selector, Severity,
    DEFAULT_ORIGIN,
};
use crate::domain::LabelNamespaces;
use std::collections::{HashMap, HashSet};

/// The canonical `namespace:value` label format, mirroring the regex the legacy
/// `validate_labels` enforced unconditionally via `labels::validate_label`.
///
/// The value class admits two forms: an unqualified/qualified-link value
/// (`[a-zA-Z0-9][a-zA-Z0-9._/-]*`), whose `/` lets a QUALIFIED link reference
/// value `<issue>/<self-id>` (e.g. `satisfies:56ab0224/REQ-01`) be authored as a
/// label (REQ-05), and an `@`-prefixed address value: `@` optionally followed by
/// a project name, then at least two `/`-delimited path segments (e.g.
/// `@myproject/gate/cargo-ci`).
///
/// Character-for-character identical to [`labels::label_regex`](crate::labels),
/// enforced by `test_canonical_label_regex_matches_labels_module` below rather
/// than left to a manual-sync comment.
const CANONICAL_LABEL_REGEX: &str = r"^[a-z][a-z0-9-]*:(?:[a-zA-Z0-9][a-zA-Z0-9._/-]*|@(?:[a-z][a-z0-9-]*)?(?:/[a-zA-Z0-9._-]+){2,})$";

/// Build the FIXED built-in default [`RuleSet`] from a repo's namespace registry
/// + type hierarchy (MF1).
///
/// The returned rules are derived purely from the RETAINED taxonomy (the
/// `[namespaces]` registry and `[type_hierarchy]`); the removed `[validation]`
/// enforcement flags and per-namespace constraint fields no longer influence it.
/// Rule names are stable, colon-free slugs; every rule carries `origin =
/// Some("default")` marking its provenance (`:` stays reserved for the label
/// `namespace:value` separator). They are serialized into `rules.toml` under
/// those names and are user-editable there.
///
/// See the module docs for the EXACT emitted contract. This is a pure function of
/// its input: no I/O, deterministic, and total.
pub fn default_ruleset(namespaces: &LabelNamespaces) -> RuleSet {
    let mut rules: Vec<Rule> = Vec::new();

    // (1) Label format (canonical): every WHOLE label must match the FIXED
    // canonical `namespace:value` format. `severity = error` (fails
    // `jit validate`) AND `enforce = true` ALWAYS (blocks the write). A
    // per-whole-label pattern cannot use the value-only `label-value-pattern`
    // shorthand, so it is a raw schema over the projection's `raw_labels` array.
    rules.push(json_schema_rule(
        "label-format",
        "Every label must match the canonical `namespace:value` format \
         (namespace lowercase-kebab, value non-empty). Blocks the write and \
         fails validation.",
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
            "namespace-registry",
            "Every label's namespace must be declared in the namespace \
             registry. An unknown namespace fails validation but never blocks a \
             write.",
            Selector::default(),
            Severity::Error,
            false,
            registered_namespace_schema(&registered),
        ));
    }

    // (3) Unknown type label: a `type:<value>` outside the configured hierarchy.
    // Modeled as an allowed-VALUES rule over the `type` namespace, the allowed set
    // being the hierarchy `types` keys. `severity = error` / `enforce = false`.
    // Emitted only for a NON-EMPTY declared hierarchy: with none declared the
    // enum would be empty, and a repository that declared no type names receives
    // no rule enumerating them.
    let declares_hierarchy = !namespaces.declared_type_hierarchy().is_empty();
    if declares_hierarchy {
        rules.push(json_schema_rule(
            "type-hierarchy-known",
            "Every `type:<value>` label must name a type declared in the \
             configured type hierarchy. An unknown type fails validation but never \
             blocks a write.",
            Selector::default(),
            Severity::Error,
            false,
            type_hierarchy_known_schema(namespaces),
        ));
    }

    // (4) Per-namespace UNIQUENESS: at most one label per unique namespace.
    // `severity = error` / `enforce = true` (blocks the write and fails
    // `jit validate`). One rule per UNIQUE namespace, in sorted order.
    let mut ns_names: Vec<&String> = namespaces.namespaces.keys().collect();
    ns_names.sort(); // deterministic rule order
    for name in ns_names {
        let ns = &namespaces.namespaces[name];
        if ns.unique {
            rules.push(local_rule(
                &format!("namespace-unique-{name}"),
                &format!(
                    "At most one `{name}:` label per issue: `{name}` is a \
                     unique namespace. Blocks the write and fails validation."
                ),
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

    RuleSet { rules }
}

/// Derive default membership and assertions from the registry, preserving existing
/// default rows' policy fields and custom rules. Missing defaults apply even in an
/// empty or custom-only file; `severity = "off"` disables a default.
///
/// Defaults the registry no longer derives are dropped. Missing defaults are
/// appended in generator order. A custom rule occupying a derived default's name
/// is a configuration error.
pub fn reconcile_default_rules_with_config(
    loaded: RuleSet,
    namespaces: &LabelNamespaces,
) -> Result<RuleSet, RuleConfigError> {
    let derived = checked_default_ruleset(
        loaded
            .rules
            .iter()
            .map(|rule| (rule.name.as_str(), rule.origin.as_deref())),
        namespaces,
    )?;
    let derived_by_name: HashMap<&str, &Rule> =
        derived.rules.iter().map(|r| (r.name.as_str(), r)).collect();

    let mut consumed: HashSet<&str> = HashSet::new();
    let mut rules: Vec<Rule> = Vec::with_capacity(loaded.rules.len());
    for mut rule in loaded.rules {
        if rule.origin.as_deref() == Some(DEFAULT_ORIGIN) {
            // A default-origin rule survives iff the registry still generates it;
            // its assertion is taken from config, its policy fields preserved.
            if let Some(current) = derived_by_name.get(rule.name.as_str()) {
                consumed.insert(current.name.as_str());
                rule.assert = current.assert.clone();
                rule.scope = rule.assert.scope();
                rules.push(rule);
            }
            // Otherwise the registry dropped it (e.g. a removed namespace): omit.
        } else {
            rules.push(rule);
        }
    }

    // Append the default rules the registry now generates that `loaded` lacked
    // (e.g. a uniqueness rule for a newly-declared namespace), in default order.
    for rule in &derived.rules {
        if !consumed.contains(rule.name.as_str()) {
            rules.push(rule.clone());
        }
    }

    Ok(RuleSet { rules })
}

fn checked_default_ruleset<'a>(
    existing_rules: impl IntoIterator<Item = (&'a str, Option<&'a str>)>,
    namespaces: &LabelNamespaces,
) -> Result<RuleSet, RuleConfigError> {
    let derived = default_ruleset(namespaces);
    let names: HashSet<&str> = derived
        .rules
        .iter()
        .map(|rule| rule.name.as_str())
        .collect();
    if let Some((name, _)) = existing_rules
        .into_iter()
        .find(|(name, origin)| *origin != Some(DEFAULT_ORIGIN) && names.contains(name))
    {
        return Err(RuleConfigError::DefaultRuleNameCollision {
            name: name.to_string(),
        });
    }
    Ok(derived)
}

/// The default-family file-membership delta between the rules currently
/// authored on disk (`loaded`, i.e. `.jit/rules.toml` as parsed by
/// the production rules loader — NOT the
/// in-memory-reconciled set [`reconcile_default_rules_with_config`] produces)
/// and the CURRENT `namespaces` registry.
///
/// Every default rule but the label grammar exists only for a declaration the
/// registry carries, so file membership follows the whole family rather than one
/// prefix of it: withdrawing `[namespaces]` drops the registry rule and its
/// uniqueness rows, and withdrawing `[type_hierarchy]` drops the type
/// enumeration and the two hierarchy graph warnings.
///
/// Companion to [`reconcile_default_rules_with_config`], which folds the same
/// family into the in-memory effective ruleset at every load with no disk
/// write — the validation authority stays there (out of scope for this diff,
/// jit:d74a9ed1). This diff instead reports what a caller must WRITE to
/// `rules.toml` so the registry-first `rule` item kind — which resolves
/// `@/rule/<name>` straight from the file, not the reconciled ruleset — never
/// dangles behind a registry edit the in-memory path already honors, and so a
/// row keeps no reference to a schema projection the same edit withdrew.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DefaultRuleMembershipDiff {
    /// `origin = "default"` rules, in [`default_ruleset`]'s emission order, that
    /// the current registry generates and `loaded` lacks.
    pub to_add: Vec<Rule>,
    /// Names of `loaded`'s `origin = "default"` rows the current registry no
    /// longer generates, because the declaration each was derived from is gone.
    pub to_drop: Vec<String>,
}

/// Compute the [`DefaultRuleMembershipDiff`] between the `(name, origin)`
/// identities of the rules authored in `rules.toml` and `namespaces` (the
/// current registry).
///
/// The membership decision needs only each rule's identity, keeping the edit
/// independent of full [`RuleSet`] validation: a custom rule whose assertion
/// fails to load must not strand `rules.toml` out of sync after `config.toml`
/// was already saved (jit:d74a9ed1 review F1).
///
/// Mirrors [`reconcile_default_rules_with_config`]'s matching rules:
///
/// - A rule [`default_ruleset`] emits for `namespaces` with no counterpart is
///   in `to_add`.
/// - An `origin = "default"` row that [`default_ruleset`] no longer emits is in
///   `to_drop`.
/// - A custom rule sharing a derived name is a configuration error.
///
/// Pure: no I/O, deterministic; `to_add` and `to_drop` are sorted by rule name.
pub(crate) fn default_rule_membership_diff_from_identities(
    existing_rules: &[(String, Option<String>)],
    namespaces: &LabelNamespaces,
) -> Result<DefaultRuleMembershipDiff, RuleConfigError> {
    let derived = checked_default_ruleset(
        existing_rules
            .iter()
            .map(|(name, origin)| (name.as_str(), origin.as_deref())),
        namespaces,
    )?;
    let existing: HashSet<&str> = existing_rules
        .iter()
        .filter(|(_, origin)| origin.as_deref() == Some(DEFAULT_ORIGIN))
        .map(|(name, _)| name.as_str())
        .collect();
    let desired_names: HashSet<&str> = derived.rules.iter().map(|r| r.name.as_str()).collect();

    let mut to_add: Vec<Rule> = derived
        .rules
        .iter()
        .filter(|r| !existing.contains(r.name.as_str()))
        .cloned()
        .collect();
    to_add.sort_by(|a, b| a.name.cmp(&b.name));
    let mut to_drop: Vec<String> = existing
        .into_iter()
        .filter(|name| !desired_names.contains(name))
        .map(|s| s.to_string())
        .collect();
    to_drop.sort();

    Ok(DefaultRuleMembershipDiff { to_add, to_drop })
}

/// The stable schema file name the `type-hierarchy-known` rule (`origin =
/// "default"`) references once serialized to `.jit/rules.toml` (the sanitized
/// `<origin>:<name>` identity + `.json`, matching [`serialize`](crate::repository_state::rule_serialize)'s
/// schema-stem derivation). This file is a write-through projection: the rule
/// derives its enum from `[type_hierarchy]` in memory at load
/// ([`with_default_assertions_from_config`]), so the file tracks config for
/// external consumers but never decides validation.
pub const TYPE_HIERARCHY_SCHEMA_FILE: &str = "default-type-hierarchy-known.json";

/// Build the JSON Schema backing the `type-hierarchy-known` rule from a
/// repo's namespace registry: an allowed-VALUES enum over the `type` namespace
/// whose members are the configured hierarchy `types` keys (sorted for
/// deterministic output).
///
/// This is the SINGLE source for that schema's shape, shared by
/// [`default_ruleset`] (used for both in-memory evaluation and the projection
/// [`serialize`](crate::repository_state::rule_serialize) writes), so the baked
/// `.jit/schemas/default-type-hierarchy-known.json` projection can never drift
/// from the schema validation actually uses (one source, not two). Pure: no I/O,
/// deterministic.
pub fn type_hierarchy_known_schema(namespaces: &LabelNamespaces) -> serde_json::Value {
    let mut hierarchy_types: Vec<String> =
        namespaces.declared_type_hierarchy().into_keys().collect();
    hierarchy_types.sort(); // deterministic schema enum order
    namespace_values_schema(crate::labels::TYPE_NAMESPACE, &hierarchy_types)
}

/// Construct a local-scope rule with a shorthand or raw assertion already built.
fn local_rule(
    name: &str,
    description: &str,
    when: Selector,
    severity: Severity,
    enforce: bool,
    assert: Assertion,
) -> Rule {
    let scope = assert.scope();
    debug_assert_eq!(
        scope,
        RuleScope::Local,
        "default rules are local-scope only"
    );
    Rule {
        name: name.to_string(),
        origin: Some(DEFAULT_ORIGIN.to_string()),
        description: Some(description.to_string()),
        when,
        severity,
        enforce,
        assert,
        scope,
    }
}

/// Construct a local-scope rule carrying a raw JSON Schema (inline, no file).
fn json_schema_rule(
    name: &str,
    description: &str,
    when: Selector,
    severity: Severity,
    enforce: bool,
    schema: serde_json::Value,
) -> Rule {
    Rule {
        name: name.to_string(),
        origin: Some(DEFAULT_ORIGIN.to_string()),
        description: Some(description.to_string()),
        when,
        severity,
        enforce,
        assert: Assertion::JsonSchema(SchemaSource {
            reference: format!("<default:{name}>"),
            path: std::path::PathBuf::from(format!("<default:{name}>")),
            schema,
        }),
        scope: RuleScope::Local,
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

    fn diff_of(loaded: &RuleSet, namespaces: &LabelNamespaces) -> DefaultRuleMembershipDiff {
        let identities: Vec<_> = loaded
            .rules
            .iter()
            .map(|r| (r.name.clone(), r.origin.clone()))
            .collect();
        default_rule_membership_diff_from_identities(&identities, namespaces).unwrap()
    }

    fn issue_with(labels: &[&str]) -> Issue {
        let mut issue = crate::test_utils::fixture_issue("t".to_string(), String::new());
        issue.labels = labels.iter().map(|s| s.to_string()).collect();
        issue
    }

    #[test]
    fn test_declaring_nothing_emits_the_label_grammar_alone() {
        // A registry declaring no namespace and no type hierarchy leaves only the
        // rule that needs no declaration: every other default member is keyed on
        // one of the two tables.
        let rules = default_ruleset(&registry(vec![]));
        let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["label-format"]);
        // Every emitted rule carries the FIXED default's origin marker.
        assert!(rules
            .rules
            .iter()
            .all(|r| r.origin.as_deref() == Some("default")));
    }

    #[test]
    fn test_declaring_namespaces_without_a_hierarchy_emits_no_hierarchy_rule() {
        // The namespace family is keyed on the registry and the hierarchy family
        // on `[type_hierarchy]`; declaring one supplies neither the other's
        // assertion nor its membership.
        let rules = default_ruleset(&registry(vec![("type", LabelNamespace::new("Type", true))]));
        let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "label-format",
                "namespace-registry",
                "namespace-unique-type"
            ]
        );
    }

    #[test]
    fn test_fixed_default_contract_mf1() {
        // The EXACT MF1 contract: name -> (severity, enforce), in emission order,
        // for a registry with two unique + one non-unique namespace and a
        // declared type hierarchy.
        let rules = default_ruleset(
            &registry(vec![
                ("type", LabelNamespace::new("Type", true)),
                ("team", LabelNamespace::new("Team", true)),
                ("component", LabelNamespace::new("Component", false)),
            ])
            .declaring_test_hierarchy(),
        );
        let got: Vec<(&str, Severity, bool)> = rules
            .rules
            .iter()
            .map(|r| (r.name.as_str(), r.severity, r.enforce))
            .collect();
        assert_eq!(
            got,
            vec![
                ("label-format", Severity::Error, true),
                ("namespace-registry", Severity::Error, false),
                ("type-hierarchy-known", Severity::Error, false),
                // namespace-unique only for the UNIQUE namespaces, sorted.
                ("namespace-unique-team", Severity::Error, true),
                ("namespace-unique-type", Severity::Error, true),
            ]
        );
        // Every emitted rule carries the FIXED default's origin marker.
        assert!(rules
            .rules
            .iter()
            .all(|r| r.origin.as_deref() == Some("default")));
        // No emitted name contains a colon: `:` stays reserved for the label
        // `namespace:value` separator.
        let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        assert!(names.iter().all(|n| !n.contains(':')));
        // The dropped rules are never emitted.
        assert!(!names.contains(&"require-type-label"));
        assert!(!names.contains(&"label-format-custom"));
        assert!(!names.iter().any(|n| n.starts_with("namespace-values-")));
        assert!(!names.iter().any(|n| n.starts_with("namespace-pattern-")));
        assert!(!names.iter().any(|n| n.starts_with("namespace-required-")));
    }

    #[test]
    fn test_canonical_label_regex_matches_labels_module() {
        // REQ-02: `CANONICAL_LABEL_REGEX` (the write-path duplicate feeding
        // `label-format`) must stay character-for-character identical to
        // `labels::label_regex` (the value-space source of truth). This fails if
        // either pattern changes without the other, replacing the old manual-sync
        // doc comment with an enforced check.
        assert_eq!(CANONICAL_LABEL_REGEX, crate::labels::label_regex().as_str());
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
        assert!(rules.rules.iter().all(|r| r.name != "namespace-registry"));
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
            .all(|r| r.name != "namespace-unique-epic"));
    }

    #[test]
    fn test_type_hierarchy_known_errors_but_does_not_block() {
        // A type value outside the declared hierarchy errors in validate but does
        // not block a write.
        let rules = default_ruleset(
            &registry(vec![("type", LabelNamespace::new("Type", true))]).declaring_test_hierarchy(),
        );
        let bad = evaluate_local(
            &issue_with(&["type:nonsense"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(!bad.is_blocking(), "unknown type must not block writes");
        assert!(bad.findings().iter().any(|f| f.severity == Severity::Error));

        let good = evaluate_local(
            &issue_with(&["type:action"]),
            &rules,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(good.findings().is_empty());
    }

    /// Build a loaded default-origin `namespace-registry` rule whose baked schema
    /// is `stale_registry` (simulating an on-disk projection that predates a
    /// registry edit).
    fn stale_namespace_registry_rule(stale_registry: &[&str]) -> Rule {
        Rule {
            name: "namespace-registry".to_string(),
            origin: Some("default".to_string()),
            description: Some("stale".to_string()),
            when: Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: "schemas/default-namespace-registry.json".to_string(),
                path: std::path::PathBuf::from("schemas/default-namespace-registry.json"),
                schema: registered_namespace_schema(stale_registry),
            }),
            scope: RuleScope::Local,
        }
    }

    /// A custom (non-default) json-schema rule, standing in for a repo-authored or
    /// bracket rule that must survive reconciliation verbatim.
    fn custom_json_rule(name: &str) -> Rule {
        Rule {
            name: name.to_string(),
            origin: Some("bracket".to_string()),
            description: None,
            when: Selector::default(),
            severity: Severity::Warn,
            enforce: false,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: format!("schemas/{name}.json"),
                path: std::path::PathBuf::from(format!("schemas/{name}.json")),
                schema: serde_json::json!({ "type": "object" }),
            }),
            scope: RuleScope::Local,
        }
    }

    #[test]
    fn test_reconcile_adds_rule_for_new_unique_namespace() {
        // Scaffolded when only `type` was unique; the registry now also declares a
        // unique `team`. Its uniqueness rule appears with NO file write, and a
        // duplicate `team:` label blocks (enforce carried from the derived rule).
        let loaded = default_ruleset(&registry(vec![("type", LabelNamespace::new("Type", true))]));
        assert!(!loaded
            .rules
            .iter()
            .any(|r| r.name == "namespace-unique-team"));

        let now = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]);
        let reconciled = reconcile_default_rules_with_config(loaded, &now).unwrap();
        let team = reconciled
            .rules
            .iter()
            .find(|r| r.name == "namespace-unique-team")
            .expect("new unique namespace gains a uniqueness rule");
        assert!(team.enforce, "appended default keeps enforce = true");

        let dup = evaluate_local(
            &issue_with(&["team:a", "team:b"]),
            &reconciled,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            dup.is_blocking(),
            "a duplicate in the newly-enforced namespace must block"
        );
    }

    #[test]
    fn test_reconcile_drops_rule_for_removed_namespace() {
        // Scaffolded with a unique `team`; the registry drops `team`. Its
        // uniqueness rule goes inert (removed), so a duplicate no longer blocks.
        let loaded = default_ruleset(&registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]));
        assert!(loaded
            .rules
            .iter()
            .any(|r| r.name == "namespace-unique-team"));

        let now = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let reconciled = reconcile_default_rules_with_config(loaded, &now).unwrap();
        assert!(
            !reconciled
                .rules
                .iter()
                .any(|r| r.name == "namespace-unique-team"),
            "a removed namespace drops its uniqueness rule"
        );

        let dup = evaluate_local(
            &issue_with(&["team:a", "team:b"]),
            &reconciled,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            !dup.is_blocking(),
            "with no uniqueness rule, a duplicate must not block"
        );
    }

    #[test]
    fn test_reconcile_rederives_namespace_registry_from_config() {
        // A scaffolded namespace-registry rule carrying a STALE schema (its pattern
        // knows only `type`) is rebuilt from the CURRENT registry (which also
        // declares `enforces`), so the on-disk schema no longer decides validation.
        let loaded = RuleSet {
            rules: vec![stale_namespace_registry_rule(&["type"])],
        };
        let now = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("enforces", LabelNamespace::new("Enforces", false)),
        ]);

        let reconciled = reconcile_default_rules_with_config(loaded, &now).unwrap();
        let reg_rule = reconciled
            .rules
            .iter()
            .find(|r| r.name == "namespace-registry")
            .expect("namespace-registry survives");
        match &reg_rule.assert {
            Assertion::JsonSchema(src) => assert_eq!(
                src.schema,
                registered_namespace_schema(&["enforces", "type"]),
                "assertion must derive from the current registry (sorted)"
            ),
            other => panic!("expected JsonSchema, got {other:?}"),
        }
        // Policy fields are preserved on the surviving default rule.
        assert_eq!(reg_rule.severity, Severity::Error);
        assert!(!reg_rule.enforce);

        // A label in the newly-declared namespace now validates clean, WITHOUT
        // touching the on-disk projection.
        let ok = evaluate_local(
            &issue_with(&["enforces:x"]),
            &reconciled,
            crate::domain::ContentFormat::Markdown,
        )
        .unwrap();
        assert!(
            ok.findings().is_empty(),
            "a declared namespace must validate: {:?}",
            ok.findings()
        );
    }

    #[test]
    fn test_reconcile_restores_missing_defaults_and_preserves_custom_rules() {
        let custom = custom_json_rule("custom-shape");
        let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let defaults = default_ruleset(&reg);
        for retained in 0..=defaults.rules.len() {
            let loaded = RuleSet {
                rules: std::iter::once(custom.clone())
                    .chain(defaults.rules.iter().take(retained).cloned())
                    .collect(),
            };
            let reconciled = reconcile_default_rules_with_config(loaded, &reg).unwrap();
            assert_eq!(reconciled.rules[0], custom);
            assert_eq!(reconciled.rules[1..], defaults.rules);
        }
    }

    #[test]
    fn test_reconcile_preserves_default_policy_edits_including_disabled_rules() {
        let reg = registry(vec![]);
        for severity in [Severity::Error, Severity::Warn, Severity::Off] {
            let mut loaded = default_ruleset(&reg);
            let policy = &mut loaded.rules[0];
            policy.severity = severity;
            policy.enforce = false;
            policy.when.label = Some("guarded:*".to_string());
            policy.description = Some("Edited policy".to_string());
            let expected = loaded.clone();
            let reconciled = reconcile_default_rules_with_config(loaded, &reg).unwrap();
            assert_eq!(reconciled.rules, expected.rules);
        }
    }

    #[test]
    fn test_reconcile_rederives_type_hierarchy_schema() {
        // The `type-hierarchy-known` default rule is rebuilt from the current
        // hierarchy, so a newly-declared type validates without regenerating the
        // baked schema.
        let stale = Rule {
            name: "type-hierarchy-known".to_string(),
            origin: Some("default".to_string()),
            description: None,
            when: Selector::default(),
            severity: Severity::Error,
            enforce: false,
            assert: Assertion::JsonSchema(SchemaSource {
                reference: TYPE_HIERARCHY_SCHEMA_FILE.to_string(),
                path: std::path::PathBuf::from(TYPE_HIERARCHY_SCHEMA_FILE),
                // A stale enum that knows only the default hierarchy.
                schema: type_hierarchy_known_schema(&registry(vec![])),
            }),
            scope: RuleScope::Local,
        };
        let mut hierarchy = HashMap::new();
        for (name, level) in [("epic", 2u8), ("planning", 3), ("task", 4)] {
            hierarchy.insert(name.to_string(), level);
        }
        let reg = LabelNamespaces {
            schema_version: 2,
            namespaces: HashMap::new(),
            type_hierarchy: Some(hierarchy),
            label_associations: None,
            strategic_types: None,
        };
        let reconciled =
            reconcile_default_rules_with_config(RuleSet { rules: vec![stale] }, &reg).unwrap();
        let rule = reconciled
            .rules
            .iter()
            .find(|r| r.name == "type-hierarchy-known")
            .expect("type-hierarchy-known survives");
        match &rule.assert {
            Assertion::JsonSchema(src) => {
                assert_eq!(src.schema, type_hierarchy_known_schema(&reg));
            }
            other => panic!("expected JsonSchema, got {other:?}"),
        }
    }

    #[test]
    fn test_default_ruleset_excludes_workflow_graph_rules() {
        let reg = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ])
        .declaring_test_hierarchy();
        let rules = default_ruleset(&reg);
        assert!(rules
            .rules
            .iter()
            .all(|r| !matches!(r.assert, Assertion::TypeHierarchy { .. })));
        assert!(rules.rules.iter().all(|r| r.scope != RuleScope::Graph));
        assert!(rules
            .rules
            .iter()
            .all(|r| r.origin.as_deref() == Some("default") && !r.name.contains(':')));
        // All generated rule names are unique.
        let mut names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
        names.sort();
        let count = names.len();
        names.dedup();
        assert_eq!(names.len(), count, "default rule names must be unique");
    }

    // -- default_rule_membership_diff_from_identities (jit:d74a9ed1) -------------------------

    #[test]
    fn test_default_rule_name_collision_refuses_reconciliation_and_restoration() {
        let reg = registry(vec![("team", LabelNamespace::new("Team", true))]);
        for default in default_ruleset(&reg).rules {
            for origin in [None, Some("bracket".to_string())] {
                let mut custom = custom_json_rule(&default.name);
                custom.origin = origin;
                let identities = vec![(custom.name.clone(), custom.origin.clone())];
                for error in [
                    reconcile_default_rules_with_config(
                        RuleSet {
                            rules: vec![custom],
                        },
                        &reg,
                    )
                    .unwrap_err(),
                    default_rule_membership_diff_from_identities(&identities, &reg).unwrap_err(),
                ] {
                    assert!(matches!(
                        &error,
                        crate::declarations::rules::RuleConfigError::DefaultRuleNameCollision { name }
                            if name == &default.name
                    ));
                    let message = error.to_string();
                    assert!(message.contains(&default.name));
                    assert!(message.contains("severity = \"off\""));
                    assert!(message.contains("default row"));
                }
            }
        }
    }

    #[test]
    fn test_membership_diff_adds_newly_unique_namespace() {
        // `team` was not unique when `loaded` was scaffolded; the registry now
        // declares it unique. The diff must add exactly its uniqueness rule.
        let loaded = default_ruleset(&registry(vec![("type", LabelNamespace::new("Type", true))]));
        let now = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]);
        let diff = diff_of(&loaded, &now);
        assert_eq!(diff.to_add.len(), 1);
        assert_eq!(diff.to_add[0].name, "namespace-unique-team");
        assert_eq!(diff.to_add[0].origin.as_deref(), Some("default"));
        assert!(diff.to_drop.is_empty());
        assert_ne!(diff, DefaultRuleMembershipDiff::default());
    }

    #[test]
    fn test_membership_diff_drops_no_longer_unique_namespace() {
        // `team` was unique when `loaded` was scaffolded; the registry drops it
        // (namespace removed entirely). The diff must drop its uniqueness rule.
        let loaded = default_ruleset(&registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]));
        let now = registry(vec![("type", LabelNamespace::new("Type", true))]);
        let diff = diff_of(&loaded, &now);
        assert!(diff.to_add.is_empty());
        assert_eq!(diff.to_drop, vec!["namespace-unique-team".to_string()]);
    }

    #[test]
    fn test_membership_diff_drops_namespace_demoted_to_non_unique() {
        // `team` stays declared but is no longer `unique = true`: same drop path
        // as outright removal.
        let loaded = default_ruleset(&registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]));
        let now = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", false)),
        ]);
        let diff = diff_of(&loaded, &now);
        assert!(diff.to_add.is_empty());
        assert_eq!(diff.to_drop, vec!["namespace-unique-team".to_string()]);
    }

    #[test]
    fn test_membership_diff_empty_when_registry_unchanged() {
        let reg = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]);
        let loaded = default_ruleset(&reg);
        let diff = diff_of(&loaded, &reg);
        assert!(
            diff == DefaultRuleMembershipDiff::default(),
            "unchanged registry yields no delta: {diff:?}"
        );
    }

    #[test]
    fn test_membership_diff_restores_defaults_in_custom_only_file() {
        let loaded = RuleSet {
            rules: vec![custom_json_rule("only-custom-rule")],
        };
        let reg = registry(vec![("team", LabelNamespace::new("Team", true))]);
        let diff = diff_of(&loaded, &reg);
        let mut expected = default_ruleset(&reg).rules;
        expected.sort_by(|a, b| a.name.cmp(&b.name));
        assert_eq!(diff.to_add, expected);
        assert!(diff.to_drop.is_empty());
    }
}
