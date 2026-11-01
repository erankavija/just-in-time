//! `effective_rules()` treats `.jit/rules.toml` as the operative single source of
//! truth (DR §8.2/§8.4).
//!
//! Three branches, distinguished by file PRESENCE (not emptiness):
//!
//! 1. File present with rules  -> exactly those rules (no in-code defaults mixed).
//! 2. File present but EMPTY    -> empty rule set (an intentional empty is honored).
//! 3. File ABSENT              -> build the fixed `default_ruleset` IN MEMORY (no
//!    disk write on the read path; MF4).

use jit::test_utils::{declaration_dir, file_executor};

/// A minimal config with a registry, so the in-memory `default_ruleset` is non-trivial.
const REGISTRY_CONFIG: &str = r#"
[validation]
default_type = "task"

[namespaces.type]
description = "Issue type"
unique = true
"#;

#[test]
fn test_effective_rules_file_present_is_sole_source() {
    let dir = declaration_dir(&[(".jit/config.toml", REGISTRY_CONFIG)]).unwrap();
    let jit_root = dir.path().join(".jit");
    // A user file with a single rule; its presence must SUPPRESS the in-code
    // defaults entirely (no `origin = "default"` rules combined in).
    std::fs::write(
        jit_root.join("rules.toml"),
        r#"
[[rules]]
name = "only-rule"
when = { type = "epic" }
assert = { require-section = { heading = "Goals" } }
"#,
    )
    .unwrap();

    let exec = file_executor(dir.path()).unwrap();
    let rules = exec.effective_rules().unwrap();
    let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["only-rule"]);
    // The defaults (e.g. `label-format`, origin = "default") are NOT present.
    assert!(rules.rules.iter().all(|r| r.origin.is_none()));
}

#[test]
fn test_effective_rules_empty_file_yields_empty_set() {
    let dir = declaration_dir(&[(".jit/config.toml", REGISTRY_CONFIG)]).unwrap();
    let jit_root = dir.path().join(".jit");
    // An intentionally-emptied rules file (present, zero rules).
    std::fs::write(jit_root.join("rules.toml"), "# intentionally empty\n").unwrap();

    let exec = file_executor(dir.path()).unwrap();
    let rules = exec.effective_rules().unwrap();
    assert!(
        rules.rules.is_empty(),
        "an empty file must honor the user's intent: {:?}",
        rules.rules
    );
}

#[test]
fn test_effective_rules_absent_file_builds_defaults_in_memory_without_writing() {
    let dir = declaration_dir(&[(".jit/config.toml", REGISTRY_CONFIG)]).unwrap();
    let jit_root = dir.path().join(".jit");
    // No rules.toml at all -> fixed defaults built in memory from config.toml.
    assert!(!jit_root.join("rules.toml").exists());

    let exec = file_executor(dir.path()).unwrap();
    let rules = exec.effective_rules().unwrap();
    // The fixed default ruleset is emitted (canonical format + namespace rules),
    // each carrying `origin = "default"`.
    assert!(rules
        .rules
        .iter()
        .any(|r| r.name == "label-format" && r.origin.as_deref() == Some("default")));
    assert!(rules
        .rules
        .iter()
        .any(|r| r.name == "namespace-unique-type" && r.origin.as_deref() == Some("default")));

    // MF4: building the defaults on the read path must NOT materialize the file.
    assert!(
        !jit_root.join("rules.toml").exists(),
        "effective_rules must not write rules.toml on the read path"
    );
    assert!(
        !jit_root.join("schemas").exists(),
        "effective_rules must not write schema files on the read path"
    );
}
