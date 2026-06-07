//! Phase E (decision D5, plan §5): the EXHAUSTIVE before/after parity battery for
//! the full single-source-of-truth migration (0abaddc0).
//!
//! "Before" = the in-code [`default_ruleset`] computed from a rich legacy config.
//! "After"  = that same ruleset serialized to `.jit/rules.toml` (+ schemas) and
//! reloaded from the file (the operative source). For an issue battery that
//! actually TRIGGERS each rule (positive AND negative cases), we snapshot
//! `(is_blocking, error_count, warning_count)` per issue and assert the outcomes
//! are identical before and after — plus equal rule count, no duplicate names,
//! and no dropped rule. Graph warnings (orphan-leaf / strategic-consistency) are
//! asserted to actually FIRE before and after (not count-equality of empty sets).

use std::collections::HashSet;
use std::path::Path;

use jit::config::ValidationConfig;
use jit::domain::{ContentFormat, Issue, LabelNamespace, LabelNamespaces};
use jit::type_hierarchy::HierarchyConfig;
use jit::validation::defaults::default_ruleset;
use jit::validation::evaluate_local;
use jit::validation::graph::evaluate_graph;
use jit::validation::migration::serialize_complete_ruleset;
use jit::validation::rules::{RuleSet, Scope, Severity};
use jit::validation::serialize::serialize_ruleset;
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A rich legacy validation config exercising every enforcement dimension.
fn rich_validation() -> ValidationConfig {
    ValidationConfig {
        strictness: Some("loose".to_string()),
        default_type: Some("task".to_string()),
        content_format: None,
        require_type_label: Some(true),
        label_regex: Some(r"^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string()),
        reject_malformed_labels: Some(true),
        enforce_namespace_registry: Some(true),
        warn_orphaned_leaves: Some(true),
        warn_strategic_consistency: Some(true),
    }
}

/// A registry with values / pattern / required / unique constraints across
/// several namespaces.
fn rich_registry() -> LabelNamespaces {
    let mut namespaces = HashMap::new();
    namespaces.insert(
        "type".to_string(),
        LabelNamespace::new("Issue type", true)
            .required(true)
            .with_values(vec![
                "epic".to_string(),
                "story".to_string(),
                "task".to_string(),
            ]),
    );
    namespaces.insert(
        "priority".to_string(),
        LabelNamespace::new("Priority", true)
            .with_values(vec!["high".to_string(), "low".to_string()]),
    );
    namespaces.insert(
        "milestone".to_string(),
        LabelNamespace::new("Release", false).with_pattern(r"^v\d+\.\d+$"),
    );
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

/// The local-rule outcome snapshot for one issue: blocking, error count, warn
/// count. (Warn count uses warning-severity findings.)
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    blocking: bool,
    errors: usize,
    warnings: usize,
}

fn local_outcome(rules: &RuleSet, labels: &[&str]) -> Outcome {
    let eval = evaluate_local(&issue(labels), rules, ContentFormat::Markdown)
        .expect("local evaluation must not error");
    let errors = eval
        .findings()
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = eval
        .findings()
        .iter()
        .filter(|f| f.severity == Severity::Warn)
        .count();
    Outcome {
        blocking: eval.is_blocking(),
        errors,
        warnings,
    }
}

/// Serialize a ruleset to a temp `.jit` and reload it (the "after" state).
fn serialize_and_reload(set: &RuleSet) -> (tempfile::TempDir, RuleSet) {
    let out = serialize_ruleset(set);
    let dir = tempfile::tempdir().unwrap();
    let schemas = dir.path().join("schemas");
    std::fs::create_dir_all(&schemas).unwrap();
    for f in &out.schema_files {
        std::fs::write(schemas.join(&f.name), &f.content).unwrap();
    }
    std::fs::write(dir.path().join("rules.toml"), &out.rules_toml).unwrap();
    let reloaded = RuleSet::load(dir.path()).expect("serialized rules.toml must reload");
    (dir, reloaded)
}

// ---------------------------------------------------------------------------
// The local-rule parity battery
// ---------------------------------------------------------------------------

/// Issue battery covering positive AND negative cases for every local dimension.
const LOCAL_BATTERY: &[&[&str]] = &[
    // require-type-label: missing (negative) vs present (positive).
    &["priority:high"],
    &["type:task"],
    // canonical format: malformed (negative) vs canonical (positive).
    &["BadLabel"],
    &["type:task", "priority:high"],
    // namespace values enum: out-of-enum (negative) vs in-enum (positive).
    &["type:bug"], // bug not in [epic,story,task]
    &["type:epic"],
    // priority values: out-of-enum vs in-enum.
    &["type:task", "priority:urgent"],
    &["type:task", "priority:low"],
    // milestone pattern: bad vs good.
    &["type:task", "milestone:1.0"],
    &["type:task", "milestone:v2.5"],
    // required (type) missing -> already covered above.
    // unique: duplicate priority (negative) vs single (positive).
    &["type:task", "priority:high", "priority:low"],
    &["type:task", "priority:high"],
    // unknown namespace (registry): unregistered ns vs registered.
    &["type:task", "bogus:x"],
    &["type:task"],
    // unknown type value vs known.
    &["type:gremlin"],
    &["type:story"],
];

#[test]
fn parity_local_rules_identical_before_and_after_migration() {
    let validation = rich_validation();
    let namespaces = rich_registry();
    let before = default_ruleset(&validation, &namespaces);
    let (_dir, after) = serialize_and_reload(&before);

    // Rule count preserved, no duplicates, no dropped rule (set equality on names).
    assert_eq!(
        before.rules.len(),
        after.rules.len(),
        "rule count must be preserved"
    );
    let before_names: HashSet<&str> = before.rules.iter().map(|r| r.name.as_str()).collect();
    let after_names: HashSet<&str> = after.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(before_names, after_names, "rule name set must be identical");
    assert_eq!(
        after_names.len(),
        after.rules.len(),
        "no duplicate rule names after reload"
    );

    // Per-issue outcome parity across the whole battery.
    for labels in LOCAL_BATTERY {
        let b = local_outcome(&before, labels);
        let a = local_outcome(&after, labels);
        assert_eq!(b, a, "outcome mismatch for labels {labels:?}");
    }
}

/// Spot-check that the battery actually triggers BOTH accept and reject (so the
/// parity assertion above is not vacuously over no-op outcomes).
#[test]
fn parity_local_battery_triggers_both_accept_and_reject() {
    let before = default_ruleset(&rich_validation(), &rich_registry());

    // A malformed label blocks (canonical + reject_malformed both error+enforce).
    assert!(local_outcome(&before, &["BadLabel"]).blocking);
    // A missing required type label blocks (require-type-label enforce=true).
    assert!(local_outcome(&before, &["priority:high"]).blocking);
    // A duplicate unique priority blocks.
    assert!(local_outcome(&before, &["type:task", "priority:high", "priority:low"]).blocking);
    // An out-of-enum value errors (fails validate) but does NOT block a write.
    let enum_violation = local_outcome(&before, &["type:bug"]);
    assert!(enum_violation.errors >= 1);
    // A clean canonical issue passes entirely.
    let clean = local_outcome(&before, &["type:task", "priority:high"]);
    assert_eq!(clean.errors, 0);
    assert!(!clean.blocking);
}

// ---------------------------------------------------------------------------
// Graph-rule parity (orphan-leaf / strategic-consistency must actually FIRE)
// ---------------------------------------------------------------------------

#[test]
fn parity_graph_warnings_fire_before_and_after_migration() {
    let validation = rich_validation();
    let namespaces = rich_registry();
    let before = default_ruleset(&validation, &namespaces);
    let (_dir, after) = serialize_and_reload(&before);

    let hierarchy = HierarchyConfig::default();

    // A leaf task (orphan) and a bare epic (strategic) trigger both warnings.
    let orphan_task = issue(&["type:task"]);
    let bare_epic = issue(&["type:epic"]);
    let issues = vec![orphan_task.clone(), bare_epic.clone()];

    let graph_rules_of = |set: &RuleSet| -> Vec<jit::validation::rules::Rule> {
        set.rules
            .iter()
            .filter(|r| r.scope == Scope::Graph)
            .cloned()
            .collect()
    };

    let before_rules = graph_rules_of(&before);
    let after_rules = graph_rules_of(&after);
    let before_refs: Vec<&_> = before_rules.iter().collect();
    let after_refs: Vec<&_> = after_rules.iter().collect();

    let before_findings =
        evaluate_graph(&before_refs, &issues, &hierarchy, ContentFormat::Markdown);
    let after_findings = evaluate_graph(&after_refs, &issues, &hierarchy, ContentFormat::Markdown);

    // The warnings ACTUALLY FIRE (not empty-set equality).
    let count_rule = |fs: &[jit::validation::graph::GraphFinding], rule: &str| {
        fs.iter().filter(|f| f.finding.rule == rule).count()
    };
    assert_eq!(count_rule(&before_findings, "default:orphan-leaf"), 1);
    assert_eq!(count_rule(&after_findings, "default:orphan-leaf"), 1);
    assert_eq!(
        count_rule(&before_findings, "default:strategic-consistency"),
        1
    );
    assert_eq!(
        count_rule(&after_findings, "default:strategic-consistency"),
        1
    );

    // Same total findings before and after, all warn-severity.
    assert_eq!(before_findings.len(), after_findings.len());
    assert!(after_findings
        .iter()
        .all(|f| f.finding.severity == Severity::Warn));
}

// ---------------------------------------------------------------------------
// Edge cases (plan §5 additional)
// ---------------------------------------------------------------------------

#[test]
fn parity_empty_namespace_registry_skips_registry_rule() {
    // With no namespaces, the registry rule and per-namespace rules are not
    // emitted; the canonical format + type-hierarchy + graph warnings remain.
    let mut validation = rich_validation();
    validation.enforce_namespace_registry = Some(true);
    let empty = LabelNamespaces {
        schema_version: 2,
        namespaces: HashMap::new(),
        type_hierarchy: None,
        label_associations: None,
        strategic_types: None,
    };
    let before = default_ruleset(&validation, &empty);
    assert!(!before
        .rules
        .iter()
        .any(|r| r.name == "default:namespace-registry"));
    let (_dir, after) = serialize_and_reload(&before);
    let before_names: HashSet<&str> = before.rules.iter().map(|r| r.name.as_str()).collect();
    let after_names: HashSet<&str> = after.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(before_names, after_names);
}

#[test]
fn parity_severity_off_rule_serializes_and_is_skipped() {
    // A severity=off rule is emitted to the file and reloads as off (skipped at
    // evaluation). Build a set with one off rule + the defaults.
    let toml = r#"
[[rules]]
name = "muted"
severity = "off"
when = { type = "epic" }
assert = { require-section = { heading = "Goals" } }
"#;
    let set = RuleSet::from_toml_str(toml, Path::new("/nonexistent")).unwrap();
    let (_dir, reloaded) = serialize_and_reload(&set);
    assert_eq!(reloaded.rules.len(), 1);
    assert_eq!(reloaded.rules[0].severity, Severity::Off);
    // An off rule produces no findings even on a non-matching/violating issue.
    let eval = evaluate_local(&issue(&["type:epic"]), &reloaded, ContentFormat::Markdown).unwrap();
    assert!(eval.findings().is_empty(), "off rule must not fire");
}

#[test]
fn parity_canonical_label_format_round_trip_specifically() {
    // The always-on canonical rule is the highest-traffic JsonSchema rule.
    let before = default_ruleset(&rich_validation(), &rich_registry());
    let (_dir, after) = serialize_and_reload(&before);
    // Same accept/reject on a malformed vs canonical label.
    assert_eq!(
        local_outcome(&before, &["NOPE"]).blocking,
        local_outcome(&after, &["NOPE"]).blocking
    );
    assert!(local_outcome(&after, &["NOPE"]).blocking);
    assert!(!local_outcome(&after, &["type:task", "priority:high"]).blocking);
}

#[test]
fn parity_full_ruleset_serialize_reload_round_trip_field_wise() {
    // A complete serialize/reload round-trip comparing every field except the
    // SchemaSource reference/path (which necessarily change file<->placeholder).
    let before = default_ruleset(&rich_validation(), &rich_registry());
    let (_dir, after) = serialize_and_reload(&before);
    assert_eq!(before.rules.len(), after.rules.len());
    for (b, a) in before.rules.iter().zip(after.rules.iter()) {
        assert_eq!(b.name, a.name);
        assert_eq!(b.when, a.when, "selector {}", b.name);
        assert_eq!(b.severity, a.severity, "severity {}", b.name);
        assert_eq!(b.enforce, a.enforce, "enforce {}", b.name);
        assert_eq!(b.scope, a.scope, "scope {}", b.name);
        use jit::validation::rules::Assertion;
        match (&b.assert, &a.assert) {
            (Assertion::JsonSchema(x), Assertion::JsonSchema(y)) => {
                assert_eq!(x.schema, y.schema, "schema value {}", b.name);
            }
            (x, y) => assert_eq!(x, y, "assertion {}", b.name),
        }
    }
}

#[test]
fn parity_serialize_complete_ruleset_matches_default_ruleset() {
    // The migration's `serialize_complete_ruleset` helper must serialize exactly
    // the default ruleset (no subset / no extra rules).
    let validation = rich_validation();
    let namespaces = rich_registry();
    let config = jit::config::JitConfig {
        version: None,
        type_hierarchy: None,
        validation: Some(validation.clone()),
        documentation: None,
        namespaces: None,
        worktree: None,
        coordination: None,
        global_operations: None,
        locks: None,
        events: None,
    };
    let serialized = serialize_complete_ruleset(&config, &namespaces);
    let direct = serialize_ruleset(&default_ruleset(&validation, &namespaces));
    assert_eq!(serialized.rules_toml, direct.rules_toml);
    assert_eq!(serialized.schema_files, direct.schema_files);
}
