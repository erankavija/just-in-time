//! Regression tests (jit:af4c901a REQ-04): a namespace hand-declared in
//! `config.toml` — with NO intervening `jit` write to regenerate the baked
//! `schemas/default-namespace-registry.json` — validates clean, because the
//! `namespace-registry` default rule derives its allowed set from the declared
//! registry in memory at load rather than from the stale on-disk projection.
//!
//! The scenario mirrors the observed defect: `[namespaces.enforces]` was declared
//! long after `jit init` baked the registry schema, and the first `enforces:`
//! label failed `namespace-registry` against the frozen file.

use jit::domain::identity::IssueIdPrefix;
use jit::domain::{ContentFormat, Issue, Priority};
use jit::test_utils::{file_executor, setup_test_repo_with_config};
use jit::validation::evaluate_local;
use jit::validation::report::Selection;
use std::fs;

/// A freshly initialised repo whose `config.toml` declares a single `type` namespace.
const TYPE_ONLY_CONFIG: &str = r#"
[namespaces.type]
description = "Issue type"
unique = true
"#;

/// Hand-declare an additional `enforces` namespace by REWRITING config.toml (the
/// analogue of a human editing the registry). No `jit` command runs, so no
/// projection is regenerated.
fn declare_enforces_namespace(jit_dir: &std::path::Path) {
    fs::write(
        jit_dir.join("config.toml"),
        r#"
[namespaces.type]
description = "Issue type"
unique = true

[namespaces.enforces]
description = "What an issue enforces"
unique = false
"#,
    )
    .unwrap();
}

/// Hand-declare an additional UNIQUE `squad` namespace by rewriting config.toml.
fn declare_unique_squad_namespace(jit_dir: &std::path::Path) {
    fs::write(
        jit_dir.join("config.toml"),
        r#"
[namespaces.type]
description = "Issue type"
unique = true

[namespaces.squad]
description = "Owning squad"
unique = true
"#,
    )
    .unwrap();
}

fn issue_with_label(label: &str) -> Issue {
    let mut issue = crate::fixture_issue("t".to_string(), String::new());
    issue.labels = vec![label.to_string()];
    issue
}

fn issue_with_labels(labels: &[&str]) -> Issue {
    let mut issue = crate::fixture_issue("t".to_string(), String::new());
    issue.labels = labels.iter().map(|s| s.to_string()).collect();
    issue
}

#[test]
fn test_new_unique_namespace_enforces_uniqueness_without_regeneration() {
    // REQ-01 membership: a hand-declared UNIQUE namespace materializes its
    // `namespace-unique-*` rule from config at load, so a duplicate label in it
    // blocks — with no `rules.toml`/schema write in between.
    let (temp, _) = setup_test_repo_with_config(TYPE_ONLY_CONFIG).unwrap();
    let jit_dir = temp.path().join(".jit");
    declare_unique_squad_namespace(&jit_dir);

    let exec = file_executor(temp.path()).unwrap();
    let rules = exec.effective_rules().unwrap();
    assert!(
        rules
            .rules
            .iter()
            .any(|r| r.name == "namespace-unique-squad"),
        "the new unique namespace gains a uniqueness rule at load"
    );

    let dup = evaluate_local(
        &issue_with_labels(&["squad:a", "squad:b"]),
        rules,
        ContentFormat::Markdown,
    )
    .unwrap();
    assert!(
        dup.is_blocking(),
        "a duplicate in the newly-enforced namespace must block, got {:?}",
        dup.findings()
    );
}

#[test]
fn test_hand_declared_namespace_validates_without_regeneration() {
    // REQ-04: declare `enforces` in config.toml, apply an `enforces:` label, and
    // the label-validation path `jit validate` runs produces NO error finding —
    // with no regeneration step in between.
    let (temp, _) = setup_test_repo_with_config(TYPE_ONLY_CONFIG).unwrap();
    let jit_dir = temp.path().join(".jit");
    declare_enforces_namespace(&jit_dir);

    // Precondition: the on-disk projection is STALE — its pattern does not admit
    // `enforces` (proving the file, if it were authority, would reject the label).
    let baked = fs::read_to_string(
        jit_dir
            .join("schemas")
            .join("default-namespace-registry.json"),
    )
    .unwrap();
    assert!(
        !baked.contains("enforces"),
        "precondition: baked registry projection is stale: {baked}"
    );

    // Store a real issue carrying the label and select it as `jit validate <id>`
    // does, on a FRESH executor — REQ-04's literal scenario.
    let issue = issue_with_label("enforces:gate-semantics");
    let exec = file_executor(temp.path()).unwrap();
    let id = exec
        .issues()
        .create_issue(
            issue.title,
            issue.description,
            issue.priority,
            issue.gates_required,
            issue.labels,
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;
    let report = exec
        .validation()
        .selection_report()
        .unwrap()
        .select(&Selection::Issue(IssueIdPrefix::from(&id)))
        .unwrap();
    assert!(
        !report.has_errors(),
        "`jit validate` must pass for a hand-declared namespace with no \
         regeneration step, got {:?}",
        report.findings
    );
}

#[test]
fn test_undeclared_namespace_still_fails_validation() {
    // Boundary: a namespace NOT in the registry still fails `namespace-registry`,
    // so derive-at-load did not blanket-accept every namespace.
    let (temp, _) = setup_test_repo_with_config(TYPE_ONLY_CONFIG).unwrap();

    let exec = file_executor(temp.path()).unwrap();
    let rules = exec.effective_rules().unwrap();
    let eval = evaluate_local(
        &issue_with_label("undeclared:x"),
        rules,
        ContentFormat::Markdown,
    )
    .unwrap();
    assert!(
        eval.findings()
            .iter()
            .any(|f| f.rule == "namespace-registry"),
        "an undeclared namespace must fail namespace-registry, got {:?}",
        eval.findings()
    );
}

#[test]
fn test_create_issue_in_new_namespace_then_validate_label_clean() {
    // End-to-end through the store: create an issue carrying the new-namespace
    // label (never blocked — namespace-registry is enforce=false), then confirm
    // re-loading and evaluating it yields no namespace error.
    let (temp, _) = setup_test_repo_with_config(TYPE_ONLY_CONFIG).unwrap();
    let jit_dir = temp.path().join(".jit");
    declare_enforces_namespace(&jit_dir);

    let exec = file_executor(temp.path()).unwrap();
    let id = exec
        .issues()
        .create_issue(
            "enforcer".to_string(),
            String::new(),
            Priority::Normal,
            vec![],
            vec!["enforces:atomic-writes".to_string()],
            None,
            None,
            false,
        )
        .unwrap()
        .issue
        .id;

    let reloaded = file_executor(temp.path()).unwrap();
    let issue = reloaded.issues().get_issue(&id).unwrap();
    let rules = reloaded.effective_rules().unwrap();
    let eval = evaluate_local(&issue, rules, ContentFormat::Markdown).unwrap();
    assert!(
        !eval
            .findings()
            .iter()
            .any(|f| f.rule == "namespace-registry"),
        "the persisted new-namespace label must validate clean, got {:?}",
        eval.findings()
    );
}

/// The baked namespace-registry projection path in an initialized repo.
fn namespace_registry_projection(jit_dir: &std::path::Path) -> std::path::PathBuf {
    jit_dir
        .join("schemas")
        .join("default-namespace-registry.json")
}

#[test]
fn test_missing_default_schema_still_validates_and_enforces() {
    // F1: a DELETED default projection must not break validation — the default
    // rules derive from config, so `effective_rules` loads and they still enforce.
    let (temp, _) = setup_test_repo_with_config(TYPE_ONLY_CONFIG).unwrap();
    let jit_dir = temp.path().join(".jit");
    let taxonomy = jit::test_taxonomy::test_taxonomy();
    std::fs::remove_file(namespace_registry_projection(&jit_dir)).unwrap();

    let exec = file_executor(temp.path()).unwrap();
    let rules = exec
        .effective_rules()
        .expect("a missing default projection must not fail rule loading");

    // The default rule still ENFORCES from config: a registered namespace is
    // clean, an unregistered one still fails namespace-registry.
    let ok = evaluate_local(
        &issue_with_label(&format!("type:{}", taxonomy.default_type)),
        rules,
        ContentFormat::Markdown,
    )
    .unwrap();
    assert!(
        ok.findings().is_empty(),
        "registered namespace validates clean, got {:?}",
        ok.findings()
    );
    let bad = evaluate_local(
        &issue_with_label("undeclared:x"),
        rules,
        ContentFormat::Markdown,
    )
    .unwrap();
    assert!(
        bad.findings()
            .iter()
            .any(|f| f.rule == "namespace-registry"),
        "namespace-registry must still enforce from config, got {:?}",
        bad.findings()
    );
}

#[test]
fn test_corrupt_default_schema_still_validates_and_enforces() {
    // F1: a MALFORMED default projection is likewise non-fatal.
    let (temp, _) = setup_test_repo_with_config(TYPE_ONLY_CONFIG).unwrap();
    let jit_dir = temp.path().join(".jit");
    std::fs::write(namespace_registry_projection(&jit_dir), "{ not valid json").unwrap();

    let exec = file_executor(temp.path()).unwrap();
    let rules = exec
        .effective_rules()
        .expect("a malformed default projection must not fail rule loading");

    let bad = evaluate_local(
        &issue_with_label("undeclared:x"),
        rules,
        ContentFormat::Markdown,
    )
    .unwrap();
    assert!(
        bad.findings()
            .iter()
            .any(|f| f.rule == "namespace-registry"),
        "namespace-registry must still enforce from config, got {:?}",
        bad.findings()
    );
}
