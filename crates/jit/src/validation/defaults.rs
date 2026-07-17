//! Built-in DEFAULT rule set: the fixed default validation rules.
//!
//! `.jit/rules.toml` declares the operative issue/label ruleset (DR §8.2/§8.4).
//! This module produces the FIXED default rule set that `jit init` serializes
//! into that file, that
//! [`CommandExecutor::effective_rules`](crate::commands::CommandExecutor) builds
//! IN MEMORY when no `rules.toml` exists yet (no disk write on the read path),
//! and that [`reconcile_default_rules_with_config`] reconciles a scaffolded
//! file's default rules against the current registry at load — so a default
//! rule's assertion and the `namespace-unique-*` membership follow `config.toml`,
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
//! 3. `type-hierarchy-known` — `severity = error`, `enforce = false`,
//!    ALWAYS. A `type:<value>` outside the configured hierarchy fails
//!    `jit validate` but never blocks a write.
//! 4. `namespace-unique-<ns>` — `severity = error`, `enforce = true`, per
//!    UNIQUE namespace (sorted). At most one label per unique namespace; blocks
//!    the write and fails `jit validate`.
//! 5. `orphan-leaf` + `strategic-consistency` — `severity = warn`,
//!    `enforce = false`, UNCONDITIONAL. Built-in [`RuleScope::Graph`] rules whose
//!    evaluation REUSES the existing
//!    [`type_taxonomy::validate_orphans`](crate::domain::type_taxonomy::validate_orphans)
//!    / [`validate_strategic_labels`](crate::domain::type_taxonomy::validate_strategic_labels)
//!    domain functions.
//!
//! DROPPED (no longer config-derivable): `require-type-label`,
//! `label-format-custom`, and the per-namespace `values`/`pattern`/`required`
//! rules. A repo wanting those authors them directly in `rules.toml`.

use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::LabelNamespaces;
use crate::validation::rules::{
    Assertion, Rule, RuleScope, RuleSet, SchemaSource, Selector, Severity, TypeHierarchyKind,
    DEFAULT_ORIGIN,
};
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
/// assert!(rules.rules.iter().all(|r| r.origin.as_deref() == Some("default")));
/// // The unique `type` namespace yields a uniqueness rule.
/// assert!(rules
///     .rules
///     .iter()
///     .any(|r| r.name == "namespace-unique-type"));
/// ```
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
    // The hierarchy is always present (a repo with no `[type_hierarchy]` falls
    // back to the default 4-level set via `get_type_hierarchy`), so this rule is
    // always emitted.
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

    // (5) Type-hierarchy GRAPH warnings: orphan-leaf + strategic-consistency.
    // Built-in GRAPH rules whose evaluation REUSES the existing
    // `type_taxonomy::validate_orphans` / `validate_strategic_labels` domain
    // functions (see `validation::graph`). Each is `severity = warn` /
    // `enforce = false` and UNCONDITIONAL (the former `warn_*` toggles defaulted
    // true, so unconditional preserves behavior). The repo `HierarchyConfig` is
    // injected by the graph evaluator at evaluation time.
    rules.push(graph_rule(
        "orphan-leaf",
        "Warn when a leaf-level-typed issue (a type at the deepest hierarchy \
         level, e.g. task) carries no parent-membership label (e.g. `epic:*`), \
         leaving it unattached to any strategic container. Advisory: never \
         blocks a write.",
        Severity::Warn,
        Assertion::TypeHierarchy {
            kind: TypeHierarchyKind::OrphanLeaf,
        },
    ));
    rules.push(graph_rule(
        "strategic-consistency",
        "Warn when a strategic-typed issue (a type with a membership namespace, \
         e.g. epic/milestone) lacks its own identifying membership label, such \
         as a `type:epic` issue that has no `epic:*` label. Advisory: never \
         blocks a write.",
        Severity::Warn,
        Assertion::TypeHierarchy {
            kind: TypeHierarchyKind::StrategicConsistency,
        },
    ));

    RuleSet { rules }
}

/// Reconcile the `origin = "default"` rules in `loaded` against the fixed default
/// rule set that [`default_ruleset`] derives from the declared `namespaces`
/// registry, so both the MEMBERSHIP and the ASSERTIONS of the default family
/// follow `config.toml` in memory at load rather than the scaffolded `rules.toml`
/// / `schemas/default-*.json` snapshot.
///
/// When `loaded` carries at least one default-origin rule (the scaffolded state),
/// the default family becomes a config projection:
///
/// - a default-origin rule the registry still generates keeps its editable policy
///   fields (`severity`, `enforce`, `when`, `description`) but takes its assertion
///   (and derived [`scope`](Rule::scope)) from config, in its original position;
/// - a default-origin rule the registry no longer generates — e.g. a
///   `namespace-unique-<ns>` whose namespace was removed — is DROPPED;
/// - a default rule the registry now generates but `loaded` lacks — e.g. a
///   `namespace-unique-<ns>` for a newly-declared unique namespace — is APPENDED.
///
/// So a hand edit of the registry adds or drops the corresponding default rule
/// with no `rules.toml` write, and the baked schema is never the authority.
/// Custom rules (any other `origin`) are returned untouched, in place. When
/// `loaded` carries NO default-origin rule — a curated file that opted out of the
/// defaults — nothing is added: the file is returned unchanged.
///
/// Pure: performs no I/O, deterministic, and total.
///
/// # Examples
///
/// ```
/// use jit::domain::{LabelNamespace, LabelNamespaces};
/// use jit::validation::defaults::{default_ruleset, reconcile_default_rules_with_config};
/// use std::collections::HashMap;
///
/// // A ruleset scaffolded when only `type` was declared (unique).
/// let mut old = HashMap::new();
/// old.insert("type".to_string(), LabelNamespace::new("Type", true));
/// let scaffolded = default_ruleset(&LabelNamespaces {
///     schema_version: 2,
///     namespaces: old,
///     type_hierarchy: None,
///     label_associations: None,
///     strategic_types: None,
/// });
/// assert!(!scaffolded.rules.iter().any(|r| r.name == "namespace-unique-team"));
///
/// // The registry now also declares a unique `team` namespace (a later hand edit).
/// let mut now = HashMap::new();
/// now.insert("type".to_string(), LabelNamespace::new("Type", true));
/// now.insert("team".to_string(), LabelNamespace::new("Team", true));
/// let current = LabelNamespaces {
///     schema_version: 2,
///     namespaces: now,
///     type_hierarchy: None,
///     label_associations: None,
///     strategic_types: None,
/// };
///
/// let reconciled = reconcile_default_rules_with_config(scaffolded, &current);
/// // The uniqueness rule for the newly-declared namespace now enforces.
/// assert!(reconciled.rules.iter().any(|r| r.name == "namespace-unique-team"));
/// ```
pub fn reconcile_default_rules_with_config(
    loaded: RuleSet,
    namespaces: &LabelNamespaces,
) -> RuleSet {
    // A file with no default-origin rule has deliberately opted out of the fixed
    // defaults; leave it exactly as authored.
    if !loaded
        .rules
        .iter()
        .any(|r| r.origin.as_deref() == Some(DEFAULT_ORIGIN))
    {
        return loaded;
    }

    let derived = default_ruleset(namespaces);
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
    // A name already present — including a CUSTOM rule shadowing the derived
    // default name — suppresses the append, so reconciliation never yields two
    // rules with one name (jit:d74a9ed1 review F1, round 10).
    let present: HashSet<String> = rules.iter().map(|r| r.name.clone()).collect();
    for rule in &derived.rules {
        if !consumed.contains(rule.name.as_str()) && !present.contains(&rule.name) {
            rules.push(rule.clone());
        }
    }

    RuleSet { rules }
}

/// Rule-name prefix marking the per-namespace uniqueness family
/// (`namespace-unique-<ns>`) — the only default-rule family whose FILE
/// MEMBERSHIP (not just its assertion) is write-through synced to
/// `rules.toml` by [`default_rule_membership_diff`], because it is the only
/// family whose existence (not merely its schema content) varies with the
/// registry.
const NAMESPACE_UNIQUE_PREFIX: &str = "namespace-unique-";

/// The `namespace-unique-*` file-membership delta between the rules currently
/// authored on disk (`loaded`, i.e. `.jit/rules.toml` as parsed by
/// [`RuleSet::load`](crate::validation::rules::RuleSet::load) — NOT the
/// in-memory-reconciled set [`reconcile_default_rules_with_config`] produces)
/// and the CURRENT `namespaces` registry.
///
/// Companion to [`reconcile_default_rules_with_config`], which folds this same
/// family into the in-memory effective ruleset at every load with no disk
/// write — the validation authority stays there (out of scope for this diff,
/// jit:d74a9ed1). This diff instead reports what a caller must WRITE to
/// `rules.toml` so the registry-first `rule` item kind — which resolves
/// `@/rule/<name>` straight from the file, not the reconciled ruleset — never
/// dangles behind a registry edit the in-memory path already honors.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DefaultRuleMembershipDiff {
    /// `origin = "default"` rules, in [`default_ruleset`]'s emission order, for
    /// a namespace the registry newly declares unique — absent from `loaded`.
    pub to_add: Vec<Rule>,
    /// Names of `loaded`'s `origin = "default"` `namespace-unique-<ns>` rows
    /// whose namespace is no longer unique, or no longer declared at all.
    pub to_drop: Vec<String>,
}

impl DefaultRuleMembershipDiff {
    /// Whether applying this diff would change anything.
    pub fn is_empty(&self) -> bool {
        self.to_add.is_empty() && self.to_drop.is_empty()
    }
}

/// Compute the `namespace-unique-*` [`DefaultRuleMembershipDiff`] between
/// `loaded` (the rules as currently authored in `rules.toml`) and `namespaces`
/// (the current registry).
///
/// Mirrors [`reconcile_default_rules_with_config`]'s opt-out and matching
/// rules exactly, restricted to this one family:
///
/// - `loaded` carrying NO `origin = "default"` rule at all has deliberately
///   opted out of the defaults; the diff is empty (nothing to add or drop).
/// - Otherwise, a `namespace-unique-<ns>` rule [`default_ruleset`] would emit
///   for `namespaces` that has no `origin = "default"` counterpart already in
///   `loaded` is in `to_add`.
/// - An `origin = "default"` `namespace-unique-<ns>` row in `loaded` that
///   [`default_ruleset`] no longer emits for `namespaces` is in `to_drop`.
///
/// A rule sharing a `namespace-unique-<ns>` NAME but a different `origin`
/// (a custom rule shadowing the default name) is never counted as "existing"
/// here, never targeted for drop, and SUPPRESSES the append of the derived
/// default row — the shadowing rule keeps the name, so the sync never writes a
/// duplicate-name file that `RuleSet::load` would reject. This matches how
/// [`reconcile_default_rules_with_config`] only ever touches `origin =
/// "default"` rows and skips a derived default whose name is already present.
///
/// Pure: no I/O, deterministic, total.
///
/// # Examples
///
/// ```
/// use jit::domain::{LabelNamespace, LabelNamespaces};
/// use jit::validation::defaults::{default_ruleset, default_rule_membership_diff};
/// use std::collections::HashMap;
///
/// let mut ns = HashMap::new();
/// ns.insert("type".to_string(), LabelNamespace::new("Type", true));
/// let scaffolded_registry = LabelNamespaces {
///     schema_version: 2,
///     namespaces: ns,
///     type_hierarchy: None,
///     label_associations: None,
///     strategic_types: None,
/// };
/// let loaded = default_ruleset(&scaffolded_registry);
///
/// // `team` becomes unique later (e.g. a hand edit of config.toml).
/// let mut now = HashMap::new();
/// now.insert("type".to_string(), LabelNamespace::new("Type", true));
/// now.insert("team".to_string(), LabelNamespace::new("Team", true));
/// let current_registry = LabelNamespaces {
///     schema_version: 2,
///     namespaces: now,
///     type_hierarchy: None,
///     label_associations: None,
///     strategic_types: None,
/// };
///
/// let diff = default_rule_membership_diff(&loaded, &current_registry);
/// assert_eq!(diff.to_add.len(), 1);
/// assert_eq!(diff.to_add[0].name, "namespace-unique-team");
/// assert!(diff.to_drop.is_empty());
/// ```
pub fn default_rule_membership_diff(
    loaded: &RuleSet,
    namespaces: &LabelNamespaces,
) -> DefaultRuleMembershipDiff {
    let identities: Vec<(String, Option<String>)> = loaded
        .rules
        .iter()
        .map(|r| (r.name.clone(), r.origin.clone()))
        .collect();
    default_rule_membership_diff_from_identities(&identities, namespaces)
}

/// [`default_rule_membership_diff`] over bare `(name, origin)` identities.
///
/// This is the variant the write-through sync uses
/// ([`sync_default_rule_membership`](crate::commands::CommandExecutor::sync_default_rule_membership)):
/// the membership decision needs only each rule's identity, so reading
/// identities directly (see `storage::ruleset_store`) keeps the sync
/// independent of full [`RuleSet`] validation — a custom rule whose assertion
/// fails to load (bad schema reference, malformed assert table) must not
/// strand `rules.toml` out of sync after `config.toml` was already saved
/// (jit:d74a9ed1 review F1).
///
/// Pure: no I/O, deterministic — `to_add` and `to_drop` are sorted by rule
/// name.
pub fn default_rule_membership_diff_from_identities(
    existing_rules: &[(String, Option<String>)],
    namespaces: &LabelNamespaces,
) -> DefaultRuleMembershipDiff {
    // A file with no default-origin rule has deliberately opted out of the
    // fixed defaults, mirroring `reconcile_default_rules_with_config`: nothing
    // to add or drop.
    if !existing_rules
        .iter()
        .any(|(_, origin)| origin.as_deref() == Some(DEFAULT_ORIGIN))
    {
        return DefaultRuleMembershipDiff::default();
    }

    let existing: HashSet<&str> = existing_rules
        .iter()
        .filter(|(name, origin)| {
            origin.as_deref() == Some(DEFAULT_ORIGIN) && name.starts_with(NAMESPACE_UNIQUE_PREFIX)
        })
        .map(|(name, _)| name.as_str())
        .collect();
    // EVERY existing rule name, regardless of origin: a custom rule shadowing a
    // derived default name must suppress the append, or the sync would write a
    // duplicate-name file that `RuleSet::load` rejects (jit:d74a9ed1 review F1,
    // round 10).
    let taken: HashSet<&str> = existing_rules
        .iter()
        .map(|(name, _)| name.as_str())
        .collect();

    let derived = default_ruleset(namespaces);
    let desired: Vec<&Rule> = derived
        .rules
        .iter()
        .filter(|r| r.name.starts_with(NAMESPACE_UNIQUE_PREFIX))
        .collect();
    let desired_names: HashSet<&str> = desired.iter().map(|r| r.name.as_str()).collect();

    let mut to_add: Vec<Rule> = desired
        .into_iter()
        .filter(|r| !taken.contains(r.name.as_str()))
        .cloned()
        .collect();
    to_add.sort_by(|a, b| a.name.cmp(&b.name));
    let mut to_drop: Vec<String> = existing
        .into_iter()
        .filter(|name| !desired_names.contains(name))
        .map(|s| s.to_string())
        .collect();
    to_drop.sort();

    DefaultRuleMembershipDiff { to_add, to_drop }
}

/// The stable schema file name the `type-hierarchy-known` rule (`origin =
/// "default"`) references once serialized to `.jit/rules.toml` (the sanitized
/// `<origin>:<name>` identity + `.json`, matching [`serialize`](crate::validation::serialize)'s
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
/// [`serialize`](crate::validation::serialize) writes), so the baked
/// `.jit/schemas/default-type-hierarchy-known.json` projection can never drift
/// from the schema validation actually uses (one source, not two). Pure: no I/O,
/// deterministic.
pub fn type_hierarchy_known_schema(namespaces: &LabelNamespaces) -> serde_json::Value {
    let mut hierarchy_types: Vec<String> = namespaces.get_type_hierarchy().into_keys().collect();
    hierarchy_types.sort(); // deterministic schema enum order
    namespace_values_schema(crate::labels::TYPE_NAMESPACE, &hierarchy_types)
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

/// Construct a built-in graph-scope rule (warn-only, never blocking). Used for
/// the type-hierarchy defaults, whose assertions are [`RuleScope::Graph`].
fn graph_rule(name: &str, description: &str, severity: Severity, assert: Assertion) -> Rule {
    let scope = assert.scope();
    debug_assert_eq!(
        scope,
        RuleScope::Graph,
        "graph default rules are graph-scope"
    );
    Rule {
        name: name.to_string(),
        origin: Some(DEFAULT_ORIGIN.to_string()),
        description: Some(description.to_string()),
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
                "label-format",
                "type-hierarchy-known",
                "orphan-leaf",
                "strategic-consistency",
            ]
        );
        // Every emitted rule carries the FIXED default's origin marker.
        assert!(rules
            .rules
            .iter()
            .all(|r| r.origin.as_deref() == Some("default")));
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
                ("label-format", Severity::Error, true),
                ("namespace-registry", Severity::Error, false),
                ("type-hierarchy-known", Severity::Error, false),
                // namespace-unique only for the UNIQUE namespaces, sorted.
                ("namespace-unique-team", Severity::Error, true),
                ("namespace-unique-type", Severity::Error, true),
                ("orphan-leaf", Severity::Warn, false),
                ("strategic-consistency", Severity::Warn, false),
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
        let reconciled = reconcile_default_rules_with_config(loaded, &now);
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
        let reconciled = reconcile_default_rules_with_config(loaded, &now);
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

        let reconciled = reconcile_default_rules_with_config(loaded, &now);
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
    fn test_reconcile_leaves_custom_only_file_untouched() {
        // A file with NO default-origin rule has opted out of the defaults: nothing
        // is added and the custom rule is returned verbatim (REQ-03).
        let custom = custom_json_rule("custom-shape");
        let loaded = RuleSet {
            rules: vec![custom.clone()],
        };
        let reconciled = reconcile_default_rules_with_config(
            loaded,
            &registry(vec![("type", LabelNamespace::new("Type", true))]),
        );
        assert_eq!(
            reconciled.rules,
            vec![custom],
            "a custom-only file is untouched"
        );
    }

    #[test]
    fn test_reconcile_preserves_custom_rule_alongside_defaults() {
        // A custom rule interleaved with the scaffolded defaults survives verbatim
        // while the default family reconciles to config.
        let mut loaded =
            default_ruleset(&registry(vec![("type", LabelNamespace::new("Type", true))]));
        let custom = custom_json_rule("coverage-preview");
        loaded.rules.push(custom.clone());

        let reconciled = reconcile_default_rules_with_config(
            loaded,
            &registry(vec![("type", LabelNamespace::new("Type", true))]),
        );
        assert!(
            reconciled.rules.iter().any(|r| r == &custom),
            "the custom rule is preserved verbatim"
        );
        assert!(
            reconciled.rules.iter().any(|r| r.name == "label-format"),
            "the default family is present"
        );
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
        let reconciled = reconcile_default_rules_with_config(RuleSet { rules: vec![stale] }, &reg);
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
    fn test_all_rules_carry_default_origin_and_are_unique() {
        let reg = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]);
        let rules = default_ruleset(&reg);
        // The only graph rules are the two type-hierarchy warnings.
        assert!(rules
            .rules
            .iter()
            .filter(|r| r.scope == RuleScope::Graph)
            .all(|r| r.name == "orphan-leaf" || r.name == "strategic-consistency"));
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

    // -- default_rule_membership_diff (jit:d74a9ed1) -------------------------

    #[test]
    fn test_reconcile_custom_rule_shadowing_default_name_is_not_duplicated() {
        // In-memory reconciliation must agree with the write-path diff: the
        // derived default whose name a custom rule occupies is not appended, so
        // the reconciled set never carries two rules with one name.
        let mut loaded =
            default_ruleset(&registry(vec![("type", LabelNamespace::new("Type", true))]));
        let mut shadow = custom_json_rule("namespace-unique-team");
        shadow.origin = None;
        loaded.rules.push(shadow.clone());
        let now = registry(vec![
            ("type", LabelNamespace::new("Type", true)),
            ("team", LabelNamespace::new("Team", true)),
        ]);
        let reconciled = reconcile_default_rules_with_config(loaded, &now);
        let with_name: Vec<_> = reconciled
            .rules
            .iter()
            .filter(|r| r.name == "namespace-unique-team")
            .collect();
        assert_eq!(with_name.len(), 1, "exactly one rule owns the name");
        assert_eq!(with_name[0], &shadow, "the custom rule wins verbatim");
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
        let diff = default_rule_membership_diff(&loaded, &now);
        assert_eq!(diff.to_add.len(), 1);
        assert_eq!(diff.to_add[0].name, "namespace-unique-team");
        assert_eq!(diff.to_add[0].origin.as_deref(), Some("default"));
        assert!(diff.to_drop.is_empty());
        assert!(!diff.is_empty());
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
        let diff = default_rule_membership_diff(&loaded, &now);
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
        let diff = default_rule_membership_diff(&loaded, &now);
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
        let diff = default_rule_membership_diff(&loaded, &reg);
        assert!(
            diff.is_empty(),
            "unchanged registry yields no delta: {diff:?}"
        );
    }

    #[test]
    fn test_membership_diff_ignores_custom_rule_with_colliding_name() {
        // A CUSTOM rule (non-default origin) happens to be named
        // `namespace-unique-team`. It is never counted as "existing" for this
        // family, never a drop target, and OWNS the name: the derived default
        // row is suppressed rather than appended beside it, so the sync can
        // never write a duplicate-name file that `RuleSet::load` rejects
        // (jit:d74a9ed1 review F1, round 10).
        let loaded = RuleSet {
            rules: vec![custom_json_rule("namespace-unique-team")],
        };
        let reg = registry(vec![("team", LabelNamespace::new("Team", true))]);
        // `loaded` carries NO default-origin rule at all here, so the opt-out
        // path applies and the diff is empty regardless of the name collision.
        let diff = default_rule_membership_diff(&loaded, &reg);
        assert!(
            diff.is_empty(),
            "an opted-out file yields no delta: {diff:?}"
        );

        // With at least one default-origin rule present (not opted out), the
        // custom-origin `namespace-unique-team` row suppresses the default
        // row's append and is itself never dropped.
        let mut loaded_with_default = default_ruleset(&registry(vec![]));
        loaded_with_default
            .rules
            .push(custom_json_rule("namespace-unique-team"));
        let diff = default_rule_membership_diff(&loaded_with_default, &reg);
        assert!(
            diff.to_add.is_empty(),
            "shadowed name must not be re-added: {:?}",
            diff.to_add.iter().map(|r| &r.name).collect::<Vec<_>>()
        );
        assert!(diff.to_drop.is_empty(), "shadowing rule is never dropped");
    }

    #[test]
    fn test_membership_diff_opted_out_file_is_untouched() {
        // A `rules.toml` with zero default-origin rules has opted out; the diff
        // stays empty even when the registry declares a unique namespace the
        // defaults would otherwise emit a rule for.
        let loaded = RuleSet {
            rules: vec![custom_json_rule("only-custom-rule")],
        };
        let reg = registry(vec![("team", LabelNamespace::new("Team", true))]);
        let diff = default_rule_membership_diff(&loaded, &reg);
        assert!(diff.is_empty());
    }
}
