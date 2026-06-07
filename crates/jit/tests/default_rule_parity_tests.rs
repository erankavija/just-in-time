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
//! * ALWAYS-ON inline write-path rejects (independent of any config flag):
//!   - the canonical `namespace:value` format (`labels::validate_label`) hard-
//!     rejected a malformed label on EVERY write; and
//!   - a second label from a `unique` namespace was hard-rejected on EVERY write.
//!
//!   Both are reproduced by ALWAYS-enforced default rules (and both also FAIL
//!   `jit validate`).
//! * `validate_labels` (whole-repo `jit validate`), ALWAYS on, hard-reject:
//!   - a value outside a namespace `values` enum, a value not matching a
//!     namespace `pattern`, and a missing `required` namespace each FAIL `jit
//!     validate` but do NOT block a write.
//!
//! The default rules are evaluated directly via `default_ruleset` + the local
//! engine so the assertions are about the rule layer itself.

use jit::config::ValidationConfig;
use jit::domain::{ContentFormat, Issue, LabelNamespace, LabelNamespaces};
use jit::validation::defaults::default_ruleset;
use jit::validation::evaluate_local;
use jit::validation::rules::{RuleSet, Severity};
use std::collections::HashMap;

fn validation() -> ValidationConfig {
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

fn issue(labels: &[&str]) -> Issue {
    let mut i = Issue::new("t".to_string(), String::new());
    i.labels = labels.iter().map(|s| s.to_string()).collect();
    i
}

/// Whether evaluating `labels` against `rules` BLOCKS a write (an enforce rule
/// produced an error finding).
fn blocks(rules: &RuleSet, labels: &[&str]) -> bool {
    evaluate_local(&issue(labels), rules, ContentFormat::Markdown)
        .unwrap()
        .is_blocking()
}

/// Whether evaluating `labels` against `rules` produces any `error`-severity
/// finding (i.e. would FAIL `jit validate`), regardless of whether it blocks.
fn fails_validate(rules: &RuleSet, labels: &[&str]) -> bool {
    evaluate_local(&issue(labels), rules, ContentFormat::Markdown)
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

/// The canonical `namespace:value` format is the ALWAYS-ON format check that the
/// inline `labels::validate_label` enforced on EVERY write regardless of config.
/// (a) A malformed CANONICAL label is hard-rejected on create even with
/// `reject_malformed_labels` off.
#[test]
fn parity_malformed_canonical_label_blocks_even_when_reject_off() {
    // reject_malformed_labels unset/false => canonical rule STILL blocks, exactly
    // as the legacy inline `validate_label(label)?` did on every write path.
    let v = validation();
    let rules = default_ruleset(&v, &registry(vec![]));

    assert!(
        blocks(&rules, &["INVALID:label"]),
        "legacy inline validate_label always rejected a malformed label on write"
    );
    // A well-formed canonical label passes.
    assert!(!blocks(&rules, &["type:task"]));

    // Same outcome with the flag explicitly false.
    let mut v2 = validation();
    v2.reject_malformed_labels = Some(false);
    let rules2 = default_ruleset(&v2, &registry(vec![]));
    assert!(blocks(&rules2, &["INVALID:label"]));
}

/// (e) The canonical/validate path still FAILS `jit validate` on a malformed
/// label (legacy whole-repo `validate_labels` canonical check).
#[test]
fn parity_malformed_canonical_label_fails_validate() {
    let rules = default_ruleset(&validation(), &registry(vec![]));
    assert!(
        fails_validate(&rules, &["INVALID:label"]),
        "malformed label must fail jit validate (canonical, severity=error)"
    );
    assert!(!fails_validate(&rules, &["type:task"]));
}

/// (d) A CUSTOM `label_regex` violation (on a canonically-valid label) is
/// rejected on the WRITE path only when `reject_malformed_labels = true`.
///
/// APPROVED DEVIATION (decision record §8.3a, user-approved 2026-06-07): unlike
/// the legacy `validate_labels` (which used only the fixed canonical regex on the
/// validate path), the migrated custom-regex rule ALSO surfaces on the validate
/// path (`severity=error`), because the rule engine has no write-only local-rule
/// representation. See `parity_custom_label_regex_also_applies_to_validate`.
#[test]
fn parity_custom_label_regex_write_only() {
    // Custom regex stricter than canonical; `type:task` is canonical-valid but
    // violates it.
    let mut warn_cfg = validation();
    warn_cfg.label_regex = Some(r"^team:[a-z]+$".to_string());
    // reject_malformed_labels off => custom rule warns only, does not block.
    let warn_rules = default_ruleset(&warn_cfg, &registry(vec![]));
    assert!(
        !blocks(&warn_rules, &["type:task"]),
        "custom regex must NOT block when reject_malformed_labels off"
    );

    // reject_malformed_labels on => custom rule blocks the WRITE.
    let mut block_cfg = warn_cfg.clone();
    block_cfg.reject_malformed_labels = Some(true);
    let block_rules = default_ruleset(&block_cfg, &registry(vec![]));
    assert!(
        blocks(&block_rules, &["type:task"]),
        "custom regex must block the write when reject_malformed_labels on"
    );
    // A label satisfying the custom regex is accepted.
    assert!(!blocks(&block_rules, &["team:platform"]));
}

/// APPROVED DEVIATION (decision record §8.3a): the migrated custom `label_regex`
/// rule applies to the validate path too, whereas legacy `validate_labels` used
/// only the canonical regex there. A canonically-valid label that violates a
/// custom regex therefore FAILS `jit validate`, regardless of
/// `reject_malformed_labels` (which only governs write-time blocking). When
/// `label_regex` equals the canonical regex the rule is not emitted, so most
/// repos see no change. This test pins the intentional new behavior so it is not
/// mistaken for a regression.
#[test]
fn parity_custom_label_regex_also_applies_to_validate() {
    let mut cfg = validation();
    cfg.label_regex = Some(r"^team:[a-z]+$".to_string());
    // reject_malformed_labels off: does NOT block the write, but DOES fail validate.
    let rules = default_ruleset(&cfg, &registry(vec![]));
    assert!(
        !blocks(&rules, &["type:task"]),
        "custom regex must not block write when reject_malformed_labels off"
    );
    assert!(
        fails_validate(&rules, &["type:task"]),
        "deviation §8.3a: custom regex surfaces as a validate error"
    );
    // A label satisfying the custom regex neither blocks nor fails validate.
    assert!(!fails_validate(&rules, &["team:platform"]));

    // When label_regex equals canonical, the custom rule is not emitted, so a
    // canonical label that would only have violated a *stricter* custom regex is
    // unaffected (no validate failure beyond the canonical check).
    let mut canon = validation();
    canon.label_regex = Some(CANONICAL_LABEL_REGEX_FOR_TEST.to_string());
    let canon_rules = default_ruleset(&canon, &registry(vec![]));
    assert!(!fails_validate(&canon_rules, &["type:task"]));
}

/// Mirror of the production `CANONICAL_LABEL_REGEX` (defaults.rs). Kept in the
/// test so the "label_regex == canonical => custom rule not emitted" branch is
/// exercised without exporting the constant.
const CANONICAL_LABEL_REGEX_FOR_TEST: &str = r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$";

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
fn parity_unknown_namespace_registry_block_requires_both_flags() {
    // Legacy `validate_namespace_registry` returned early unless
    // `enforce_namespace_registry = true`, and then HARD-REJECTED only when
    // `reject_malformed_labels = true` (else it warned). So a write is blocked
    // ONLY when BOTH flags are true; either flag alone never blocks the write
    // (an unknown namespace still fails `jit validate` via severity=error).
    let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);

    // enforce_namespace_registry on, reject_malformed off -> warn only, no block.
    let mut v1 = validation();
    v1.enforce_namespace_registry = Some(true);
    v1.reject_malformed_labels = Some(false);
    let rules1 = default_ruleset(&v1, &reg);
    assert!(
        !blocks(&rules1, &["unknown:x"]),
        "registry must not block a write when reject_malformed_labels is off"
    );
    assert!(
        fails_validate(&rules1, &["unknown:x"]),
        "unknown namespace must still fail jit validate"
    );

    // Both flags on -> blocks the write.
    let mut v2 = validation();
    v2.enforce_namespace_registry = Some(true);
    v2.reject_malformed_labels = Some(true);
    let rules2 = default_ruleset(&v2, &reg);
    assert!(
        blocks(&rules2, &["unknown:x"]),
        "registry must block the write when both flags are on"
    );
    assert!(!blocks(&rules2, &["type:task"]));

    // enforce_namespace_registry off -> never blocks, regardless of reject flag.
    let mut v3 = validation();
    v3.enforce_namespace_registry = Some(false);
    v3.reject_malformed_labels = Some(true);
    let rules3 = default_ruleset(&v3, &reg);
    assert!(
        !blocks(&rules3, &["unknown:x"]),
        "registry must not block when enforce_namespace_registry is off"
    );
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

/// (b) A duplicate UNIQUE-namespace label is hard-rejected on create (always-
/// enforced), reproducing the legacy inline unique-namespace collision check.
/// It ALSO fails `jit validate` (severity=error), matching whole-repo
/// `validate_labels`.
#[test]
fn parity_namespace_unique_blocks_write_and_fails_validate() {
    let reg = registry(vec![("priority", LabelNamespace::new("Priority", true))]);
    let rules = default_ruleset(&validation(), &reg);

    assert!(fails_validate(&rules, &["priority:high", "priority:low"]));
    assert!(
        blocks(&rules, &["priority:high", "priority:low"]),
        "duplicate unique label must block the write (legacy inline reject)"
    );
    // A single label in the unique namespace is fine.
    assert!(!blocks(&rules, &["priority:high"]));
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

/// (c) A WELL-FORMED, non-duplicate label set passes both the write path and
/// `jit validate` — the always-enforced canonical-format and uniqueness rules
/// fire ONLY on genuinely malformed or duplicate labels, never on normal labels.
#[test]
fn parity_wellformed_unique_set_passes() {
    let reg = registry(vec![
        ("type", LabelNamespace::new("Type", true)),
        ("team", LabelNamespace::new("Team", true)),
        ("resolution", LabelNamespace::new("Resolution", true)),
    ]);
    let rules = default_ruleset(&validation(), &reg);

    // One label per unique namespace, all canonical -> nothing blocks, nothing
    // fails validate.
    let labels = &["type:task", "team:platform-eng", "resolution:fixed"];
    assert!(
        !blocks(&rules, labels),
        "well-formed unique set must not block"
    );
    assert!(
        !fails_validate(&rules, labels),
        "well-formed unique set must pass jit validate"
    );
}

/// Live-repo-config safety: this repo's own config is loose
/// (reject_malformed_labels=false, enforce_namespace_registry=false,
/// require_type_label=false) with `type`/`team`/`resolution` unique=true. Under
/// EXACTLY that config, normal well-formed labels used by `jit issue
/// create/update/claim` must not be newly blocked, while genuinely malformed /
/// duplicate labels are still rejected (parity with the removed inline checks).
#[test]
fn parity_live_repo_loose_config_does_not_block_normal_labels() {
    let mut v = validation();
    v.require_type_label = Some(false);
    v.reject_malformed_labels = Some(false);
    v.enforce_namespace_registry = Some(false);
    let reg = registry(vec![
        ("type", LabelNamespace::new("Type", true)),
        ("team", LabelNamespace::new("Team", true)),
        ("resolution", LabelNamespace::new("Resolution", true)),
    ]);
    let rules = default_ruleset(&v, &reg);

    // Normal labels: not blocked.
    assert!(!blocks(&rules, &["type:task"]));
    assert!(!blocks(&rules, &["type:task", "team:platform-eng"]));
    // An UNREGISTERED but well-formed namespace is not blocked (registry off).
    assert!(!blocks(&rules, &["sprint:42"]));

    // Genuinely malformed -> still blocked (canonical always-on).
    assert!(blocks(&rules, &["Bad Label"]));
    // Duplicate unique namespace -> still blocked (uniqueness always-on).
    assert!(blocks(&rules, &["type:task", "type:bug"]));
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

// ---------------------------------------------------------------------------
// A. unknown type label (legacy validate_type_hierarchy unknown-type check):
// a `type:` value not in the configured hierarchy FAILS validate, never blocks.
// ---------------------------------------------------------------------------

#[test]
fn parity_unknown_type_fails_validate_but_does_not_block() {
    // With no `[type_hierarchy]`, the registry falls back to the default 4-level
    // hierarchy (milestone/epic/story/task), exactly as legacy did.
    let rules = default_ruleset(&validation(), &registry(vec![]));

    // Legacy `validate_type_hierarchy` flagged an unknown type as a validate
    // error and did NOT block the write.
    assert!(
        fails_validate(&rules, &["type:widget"]),
        "unknown type must FAIL validate"
    );
    assert!(
        !blocks(&rules, &["type:widget"]),
        "unknown type must NOT block a write"
    );
    // A known hierarchy type passes.
    assert!(!fails_validate(&rules, &["type:task"]));
}

// ---------------------------------------------------------------------------
// B. orphan-leaf + strategic-consistency as DEFAULT GRAPH rules: each produces
// the same warning the legacy validate_orphans / validate_strategic_labels did.
// ---------------------------------------------------------------------------

#[test]
fn parity_orphan_leaf_and_strategic_consistency_as_default_graph_rules() {
    use jit::type_hierarchy::{validate_orphans, validate_strategic_labels, HierarchyConfig};
    use jit::validation::graph::evaluate_graph;
    use jit::validation::rules::Scope;

    // Default registry -> default 4-level hierarchy, both toggles default-on.
    let rules = default_ruleset(&validation(), &registry(vec![]));
    let graph_rules: Vec<_> = rules
        .rules
        .iter()
        .filter(|r| r.scope == Scope::Graph)
        .collect();
    // Exactly the two type-hierarchy graph defaults are present.
    let mut names: Vec<&str> = graph_rules.iter().map(|r| r.name.as_str()).collect();
    names.sort();
    assert_eq!(
        names,
        vec!["default:orphan-leaf", "default:strategic-consistency"]
    );

    let orphan_task = issue(&["type:task"]); // leaf, no parent label
    let bare_epic = issue(&["type:epic"]); // strategic, no epic:* label
    let issues = vec![orphan_task.clone(), bare_epic.clone()];

    let cfg = HierarchyConfig::default();
    let findings = evaluate_graph(&graph_rules, &issues, &cfg, ContentFormat::Markdown);

    // The orphan-leaf finding is attributed to the task and matches the legacy
    // domain function firing for that issue.
    assert_eq!(validate_orphans(&cfg, &orphan_task).len(), 1);
    let orphan_findings: Vec<_> = findings
        .iter()
        .filter(|f| f.finding.rule == "default:orphan-leaf")
        .collect();
    assert_eq!(orphan_findings.len(), 1);
    assert_eq!(
        orphan_findings[0].issue_id.as_deref(),
        Some(orphan_task.id.as_str())
    );
    assert!(orphan_findings[0].finding.message.contains("orphaned leaf"));

    // The strategic-consistency finding is attributed to the epic.
    assert_eq!(validate_strategic_labels(&cfg, &bare_epic).len(), 1);
    let strategic_findings: Vec<_> = findings
        .iter()
        .filter(|f| f.finding.rule == "default:strategic-consistency")
        .collect();
    assert_eq!(strategic_findings.len(), 1);
    assert_eq!(
        strategic_findings[0].issue_id.as_deref(),
        Some(bare_epic.id.as_str())
    );
    assert!(strategic_findings[0].finding.message.contains("epic:*"));

    // Legacy was warn-only: no graph finding is error severity.
    assert!(findings
        .iter()
        .all(|f| f.finding.severity != Severity::Error));
}

#[test]
fn parity_type_hierarchy_graph_rules_gated_by_toggles() {
    use jit::validation::rules::Scope;

    // Both toggles off => neither default graph rule is emitted.
    let mut v = validation();
    v.warn_orphaned_leaves = Some(false);
    v.warn_strategic_consistency = Some(false);
    let rules = default_ruleset(&v, &registry(vec![]));
    assert!(rules.rules.iter().all(|r| r.scope != Scope::Graph));
}

// ---------------------------------------------------------------------------
// C. namespace-registry write-path block requires BOTH enforce_namespace_registry
// AND reject_malformed_labels (legacy parity). The comprehensive case is covered
// by `parity_unknown_namespace_registry_block_requires_both_flags` above; this
// adds the "enforce on, reject off => warn only" corner explicitly.
// ---------------------------------------------------------------------------

#[test]
fn parity_namespace_registry_enforce_false_warns_only() {
    let mut v = validation();
    v.enforce_namespace_registry = Some(false);
    // reject_malformed_labels = true must NOT, on its own, make the registry
    // rule block: the legacy write-path check returned early when
    // enforce_namespace_registry was off, so it never blocked in this config.
    v.reject_malformed_labels = Some(true);
    let reg = registry(vec![("type", LabelNamespace::new("Type", true))]);
    let rules = default_ruleset(&v, &reg);

    // Surfaces as a validate error (severity = error) ...
    assert!(fails_validate(&rules, &["unknown:x"]));
    // ... but does NOT block the write (registry block needs BOTH flags).
    assert!(!blocks(&rules, &["unknown:x"]));
}
