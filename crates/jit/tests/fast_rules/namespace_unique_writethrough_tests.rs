//! Integration tests (jit:d74a9ed1): the `namespace-unique-*` default-rule
//! family's FILE MEMBERSHIP — not just its in-memory reconciliation
//! (jit:af4c901a) — is write-through synced into `.jit/rules.toml` on the same
//! jit-driven-write triggers that already republish `schemas/default-*.json`
//! (`jit init` re-run, `jit config set`).
//!
//! Exercises the full disk-based path: a real `.jit/` scaffolded by
//! `scaffold_default_rules`, `config.toml` hand-edited (the analogue of a
//! human declaring a namespace unique, or dropping one) with no intervening
//! `jit` write, then a jit-driven write triggers the sync. The deliverable is
//! that `rules.toml` itself — not only the in-memory effective ruleset — ends
//! up carrying exactly the rows the registry implies, with every other byte of
//! the file (custom rules, hand-edited policy fields, comments) untouched.

use jit::commands::CommandExecutor;
use jit::storage::{IssueStore, JsonFileStorage};
use std::fs;
use tempfile::TempDir;

/// A freshly-`jit init`ed repo: `config.toml` with a single unique `type`
/// namespace, plus the scaffolded `rules.toml` + baked `schemas/default-*.json`.
fn setup_initialized_repo() -> (TempDir, std::path::PathBuf) {
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    fs::create_dir(&jit_dir).unwrap();
    fs::write(
        jit_dir.join("config.toml"),
        "\
[namespaces.type]\n\
description = \"Issue type\"\n\
unique = true\n",
    )
    .unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();
    CommandExecutor::new(storage)
        .scaffold_default_rules()
        .unwrap();
    (temp, jit_dir)
}

/// Hand-declare an additional UNIQUE `squad` namespace by rewriting
/// config.toml — the analogue of a human editing the registry directly, with
/// no intervening `jit` write.
fn declare_unique_squad_namespace(jit_dir: &std::path::Path) {
    fs::write(
        jit_dir.join("config.toml"),
        "\
[namespaces.type]\n\
description = \"Issue type\"\n\
unique = true\n\
\n\
[namespaces.squad]\n\
description = \"Owning squad\"\n\
unique = true\n",
    )
    .unwrap();
}

/// Drop the `squad` namespace entirely, leaving only `type`.
fn drop_squad_namespace(jit_dir: &std::path::Path) {
    fs::write(
        jit_dir.join("config.toml"),
        "\
[namespaces.type]\n\
description = \"Issue type\"\n\
unique = true\n",
    )
    .unwrap();
}

fn read_rules(jit_dir: &std::path::Path) -> String {
    fs::read_to_string(jit_dir.join("rules.toml")).unwrap()
}

#[test]
fn test_direct_sync_appends_row_for_hand_declared_unique_namespace() {
    let (_temp, jit_dir) = setup_initialized_repo();
    declare_unique_squad_namespace(&jit_dir);

    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let outcome = executor.sync_default_rule_membership().unwrap();
    assert_eq!(outcome.added, vec!["namespace-unique-squad".to_string()]);
    assert!(outcome.dropped.is_empty());

    let rules = read_rules(&jit_dir);
    assert!(rules.contains("name = \"namespace-unique-squad\""));
    assert!(rules.contains("origin = \"default\""));
}

/// jit:d74a9ed1 review F1 regression: the sync is identity-only, so a custom
/// rule whose FULL load fails (here: an unresolvable schema file reference)
/// cannot strand the membership write-through after config.toml was already
/// edited.
#[test]
fn test_sync_survives_custom_rule_that_fails_full_load() {
    let (_temp, jit_dir) = setup_initialized_repo();

    // Append a custom rule that full RuleSet loading rejects.
    let mut rules = read_rules(&jit_dir);
    rules.push_str(
        "\n[[rules]]\n\
         name = \"broken-custom\"\n\
         origin = \"custom\"\n\
         description = \"references a schema file that does not exist\"\n\
         when = { type = \"task\" }\n\
         severity = \"error\"\n\
         enforce = false\n\
         assert = { schema = { file = \"schemas/does-not-exist.json\" } }\n",
    );
    fs::write(jit_dir.join("rules.toml"), &rules).unwrap();
    assert!(
        jit::storage::ruleset_store::load_ruleset(
            &jit_dir,
            &toml::from_str::<jit::config::JitConfig>("").unwrap(),
        )
        .is_err(),
        "premise: the broken custom rule must fail a full RuleSet load"
    );

    declare_unique_squad_namespace(&jit_dir);
    let outcome = CommandExecutor::new(JsonFileStorage::new(&jit_dir))
        .sync_default_rule_membership()
        .expect("identity-only sync must not depend on full rule validation");
    assert_eq!(outcome.added, vec!["namespace-unique-squad".to_string()]);

    let after = read_rules(&jit_dir);
    assert!(after.contains("name = \"namespace-unique-squad\""));
    assert!(
        after.contains("name = \"broken-custom\""),
        "the failing custom rule survives byte-exact"
    );
}

#[test]
fn test_direct_sync_drops_row_for_removed_unique_namespace() {
    let (_temp, jit_dir) = setup_initialized_repo();
    declare_unique_squad_namespace(&jit_dir);
    CommandExecutor::new(JsonFileStorage::new(&jit_dir))
        .sync_default_rule_membership()
        .unwrap();
    assert!(read_rules(&jit_dir).contains("namespace-unique-squad"));

    drop_squad_namespace(&jit_dir);
    let outcome = CommandExecutor::new(JsonFileStorage::new(&jit_dir))
        .sync_default_rule_membership()
        .unwrap();
    assert!(outcome.added.is_empty());
    assert_eq!(outcome.dropped, vec!["namespace-unique-squad".to_string()]);
    assert!(!read_rules(&jit_dir).contains("namespace-unique-squad"));
}

#[test]
fn test_direct_sync_is_noop_when_registry_unchanged() {
    let (_temp, jit_dir) = setup_initialized_repo();
    let before = read_rules(&jit_dir);

    let outcome = CommandExecutor::new(JsonFileStorage::new(&jit_dir))
        .sync_default_rule_membership()
        .unwrap();
    assert!(outcome.added.is_empty());
    assert!(outcome.dropped.is_empty());
    assert_eq!(
        read_rules(&jit_dir),
        before,
        "an unchanged registry writes nothing"
    );
}

#[test]
fn test_reinit_write_throughs_membership_alongside_schema_projections() {
    // The re-init path (jit init on an existing rules.toml) triggers the SAME
    // write-through as a direct sync call, alongside its existing
    // schema-projection refresh.
    let (_temp, jit_dir) = setup_initialized_repo();
    declare_unique_squad_namespace(&jit_dir);

    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let scaffolded = executor.scaffold_default_rules().unwrap();
    assert!(
        !scaffolded,
        "re-scaffold must not clobber an existing rules.toml"
    );

    assert!(read_rules(&jit_dir).contains("name = \"namespace-unique-squad\""));
}

#[test]
fn test_config_set_write_throughs_membership() {
    // `jit config set` on ANY key re-derives the full registry (mirroring
    // `refresh_default_schema_projections`'s existing trigger), so it also
    // picks up a namespace hand-declared since the last jit-driven write.
    let (_temp, jit_dir) = setup_initialized_repo();
    declare_unique_squad_namespace(&jit_dir);

    let storage = JsonFileStorage::new(&jit_dir);
    // `jit config set` publishes the repo config edit through the recovered
    // session, so the executor needs its canonical layout.
    let layout = jit::storage::discover_repository_layout(jit_dir.parent().unwrap(), &jit_dir)
        .unwrap();
    let executor = CommandExecutor::new(storage).with_layout(layout);
    executor
        .set_config("project.name", "demo-project", false)
        .unwrap();

    assert!(read_rules(&jit_dir).contains("name = \"namespace-unique-squad\""));
}

#[test]
fn test_sync_preserves_hand_edited_policy_and_custom_rules() {
    // REQ-03: a hand edit to a SURVIVING default rule's policy fields, and a
    // custom rule anywhere in the file, both survive the write-through
    // byte-exact, while the new membership row is appended.
    let (_temp, jit_dir) = setup_initialized_repo();

    // Hand-edit the scaffolded `label-format` rule's severity, and append a
    // custom rule with its own comment.
    let mut rules = read_rules(&jit_dir);
    rules = rules.replacen(
        "name = \"label-format\"\norigin = \"default\"\ndescription",
        "name = \"label-format\"\norigin = \"default\"\n# hand note: keep strict\ndescription",
        1,
    );
    rules.push_str(
        "\n[[rules]]\nname = \"custom-shape\"\n# a hand-authored custom rule\nseverity = \"warn\"\nassert = { require-section = { heading = \"Goals\" } }\n",
    );
    fs::write(jit_dir.join("rules.toml"), &rules).unwrap();

    declare_unique_squad_namespace(&jit_dir);
    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let outcome = executor.sync_default_rule_membership().unwrap();
    assert_eq!(outcome.added, vec!["namespace-unique-squad".to_string()]);

    let updated = read_rules(&jit_dir);
    assert!(
        updated.contains("# hand note: keep strict"),
        "hand-edited comment on a surviving default rule survives: {updated}"
    );
    assert!(
        updated.contains("name = \"custom-shape\"")
            && updated.contains("# a hand-authored custom rule"),
        "custom rule survives byte-exact: {updated}"
    );
    assert!(updated.contains("name = \"namespace-unique-squad\""));
}

#[test]
fn test_sync_is_noop_when_rules_toml_absent() {
    // A repo whose `rules.toml` was never materialized (read path builds
    // defaults in memory) has nothing to sync: no file is created.
    std::env::set_var("JIT_TEST_MODE", "1");
    let temp = TempDir::new().unwrap();
    let jit_dir = temp.path().join(".jit");
    fs::create_dir(&jit_dir).unwrap();
    fs::write(
        jit_dir.join("config.toml"),
        "[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n",
    )
    .unwrap();
    let storage = JsonFileStorage::new(&jit_dir);
    storage.init().unwrap();

    let executor = CommandExecutor::new(JsonFileStorage::new(&jit_dir));
    let outcome = executor.sync_default_rule_membership().unwrap();
    assert!(outcome.added.is_empty());
    assert!(outcome.dropped.is_empty());
    assert!(!jit_dir.join("rules.toml").exists());
}
