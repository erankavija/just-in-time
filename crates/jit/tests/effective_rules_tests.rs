//! Phase C (decisions D2/D3): `effective_rules()` treats `.jit/rules.toml` as the
//! operative single source of truth.
//!
//! Three branches, distinguished by file PRESENCE (not emptiness):
//!
//! 1. File present with rules  -> exactly those rules (no in-code defaults mixed).
//! 2. File present but EMPTY    -> empty rule set (an intentional empty is honored).
//! 3. File ABSENT              -> fall back to the in-code `default_ruleset`.

use jit::storage::JsonFileStorage;
use jit::CommandExecutor;
use tempfile::TempDir;

/// A `.jit` root with a minimal config but NO rules.toml.
fn repo() -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().unwrap();
    let jit_root = dir.path().join(".jit");
    std::fs::create_dir_all(&jit_root).unwrap();
    // A registry-bearing config so the fallback default_ruleset is non-trivial.
    std::fs::write(
        jit_root.join("config.toml"),
        r#"
[validation]
require_type_label = true

[namespaces.type]
description = "Issue type"
unique = true
"#,
    )
    .unwrap();
    (dir, jit_root)
}

fn executor(jit_root: &std::path::Path) -> CommandExecutor<JsonFileStorage> {
    CommandExecutor::new(JsonFileStorage::new(jit_root))
}

#[test]
fn test_effective_rules_file_present_is_sole_source() {
    let (_dir, jit_root) = repo();
    // A user file with a single rule; its presence must SUPPRESS the in-code
    // defaults entirely (no default:* rules combined in).
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

    let exec = executor(&jit_root);
    let rules = exec.effective_rules().unwrap();
    let names: Vec<&str> = rules.rules.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["only-rule"]);
    // The defaults (e.g. default:label-format) are NOT present.
    assert!(!rules.rules.iter().any(|r| r.name.starts_with("default:")));
}

#[test]
fn test_effective_rules_empty_file_yields_empty_set() {
    let (_dir, jit_root) = repo();
    // An intentionally-emptied rules file (present, zero rules).
    std::fs::write(jit_root.join("rules.toml"), "# intentionally empty\n").unwrap();

    let exec = executor(&jit_root);
    let rules = exec.effective_rules().unwrap();
    assert!(
        rules.rules.is_empty(),
        "an empty file must honor the user's intent: {:?}",
        rules.rules
    );
}

#[test]
fn test_effective_rules_absent_file_falls_back_to_defaults() {
    let (_dir, jit_root) = repo();
    // No rules.toml at all -> in-code defaults derived from config.toml.
    assert!(!jit_root.join("rules.toml").exists());

    let exec = executor(&jit_root);
    let rules = exec.effective_rules().unwrap();
    // The default ruleset is emitted (canonical format + require-type-label etc).
    assert!(rules.rules.iter().any(|r| r.name == "default:label-format"));
    assert!(rules
        .rules
        .iter()
        .any(|r| r.name == "default:require-type-label"));
}
