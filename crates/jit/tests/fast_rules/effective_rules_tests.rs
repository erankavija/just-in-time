//! Effective rules combine authored rules with registry-derived defaults without writes.

use jit::test_utils::{declaration_dir, file_executor};

const REGISTRY_CONFIG: &str = r#"
[namespaces.type]
description = "Issue type"
unique = true
"#;

#[test]
fn test_captured_rules_load_defaults_with_custom_empty_or_absent_file_without_writing() {
    let custom = r#"
[[rules]]
name = "only-rule"
when = { type = "epic" }
assert = { require-section = { heading = "Goals" } }
"#;
    for content in [Some(custom), Some("# empty\n"), None] {
        let dir = declaration_dir(&[(".jit/config.toml", REGISTRY_CONFIG)]).unwrap();
        let jit_root = dir.path().join(".jit");
        let path = jit_root.join("rules.toml");
        if let Some(content) = content {
            std::fs::write(&path, content).unwrap();
        }

        let exec = file_executor(dir.path()).unwrap();
        let declarations = exec.config().captured_declarations().unwrap();
        let rules = declarations.rules();
        assert!(rules
            .rules
            .iter()
            .any(|r| r.name == "label-format" && r.origin.as_deref() == Some("default")));
        assert_eq!(
            rules.rules.iter().any(|r| r.name == "only-rule"),
            content == Some(custom)
        );
        assert_eq!(std::fs::read_to_string(&path).ok().as_deref(), content);
        assert!(!jit_root.join("schemas").exists());
    }
}
