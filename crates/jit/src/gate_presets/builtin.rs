//! Built-in gate presets

use super::{GatePresetDefinition, GateTemplate};
use crate::domain::{GateChecker, GateMode, GateStage};
use anyhow::Result;
use std::collections::HashMap;

/// Built-in presets bundled with the binary
pub struct BuiltinPresets;

impl BuiltinPresets {
    /// Load all built-in presets
    pub fn load() -> Result<HashMap<String, GatePresetDefinition>> {
        let mut presets = HashMap::new();

        // rust-tdd preset
        let rust_tdd = GatePresetDefinition {
            name: "rust-tdd".to_string(),
            description: "Test-driven development workflow for Rust projects".to_string(),
            gates: vec![
                GateTemplate {
                    key: "tdd-reminder".to_string(),
                    title: "Write tests first (TDD)".to_string(),
                    description: "Reminder to write failing tests before implementation"
                        .to_string(),
                    stage: GateStage::Precheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
                GateTemplate {
                    key: "tests".to_string(),
                    title: "All tests pass".to_string(),
                    description: "cargo test must pass".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo test".to_string(),
                        timeout_seconds: 300,
                        working_dir: None,
                        env: HashMap::new(),
                    }),
                },
                GateTemplate {
                    key: "clippy".to_string(),
                    title: "Clippy lints pass".to_string(),
                    description: "No clippy warnings allowed".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo clippy --all-targets -- -D warnings".to_string(),
                        timeout_seconds: 120,
                        working_dir: None,
                        env: HashMap::new(),
                    }),
                },
                GateTemplate {
                    key: "fmt".to_string(),
                    title: "Code formatted".to_string(),
                    description: "Code must be formatted with cargo fmt".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Auto,
                    checker: Some(GateChecker::Exec {
                        command: "cargo fmt --check".to_string(),
                        timeout_seconds: 30,
                        working_dir: None,
                        env: HashMap::new(),
                    }),
                },
                GateTemplate {
                    key: "code-review".to_string(),
                    title: "Code review completed".to_string(),
                    description: "Another developer reviewed the code".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
            ],
        };

        // minimal preset
        let minimal = GatePresetDefinition {
            name: "minimal".to_string(),
            description: "Minimal workflow with just code review".to_string(),
            gates: vec![GateTemplate {
                key: "code-review".to_string(),
                title: "Code review completed".to_string(),
                description: "Code has been reviewed".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };

        // Validate presets
        rust_tdd.validate()?;
        minimal.validate()?;

        presets.insert(rust_tdd.name.clone(), rust_tdd);
        presets.insert(minimal.name.clone(), minimal);

        Ok(presets)
    }

    /// Get list of builtin preset names
    pub fn names() -> Vec<String> {
        vec!["rust-tdd".to_string(), "minimal".to_string()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_builtin_presets() {
        let presets = BuiltinPresets::load().unwrap();
        assert_eq!(presets.len(), 2);
        assert!(presets.contains_key("rust-tdd"));
        assert!(presets.contains_key("minimal"));
    }

    #[test]
    fn test_rust_tdd_preset_structure() {
        let presets = BuiltinPresets::load().unwrap();
        let rust_tdd = presets.get("rust-tdd").unwrap();

        assert_eq!(rust_tdd.name, "rust-tdd");
        assert_eq!(rust_tdd.gates.len(), 5);

        // Verify gate keys
        let keys: Vec<_> = rust_tdd.gates.iter().map(|g| g.key.as_str()).collect();
        assert_eq!(
            keys,
            vec!["tdd-reminder", "tests", "clippy", "fmt", "code-review"]
        );

        // Verify tdd-reminder is precheck
        assert_eq!(rust_tdd.gates[0].stage, GateStage::Precheck);

        // Verify auto gates have checkers
        for gate in &rust_tdd.gates {
            if gate.mode == GateMode::Auto {
                assert!(gate.checker.is_some(), "Gate {} missing checker", gate.key);
            }
        }
    }

    #[test]
    fn test_minimal_preset_structure() {
        let presets = BuiltinPresets::load().unwrap();
        let minimal = presets.get("minimal").unwrap();

        assert_eq!(minimal.name, "minimal");
        assert_eq!(minimal.gates.len(), 1);
        assert_eq!(minimal.gates[0].key, "code-review");
        assert_eq!(minimal.gates[0].mode, GateMode::Manual);
    }

    #[test]
    fn test_builtin_presets_are_valid() {
        let presets = BuiltinPresets::load().unwrap();
        for preset in presets.values() {
            assert!(
                preset.validate().is_ok(),
                "Preset {} is invalid",
                preset.name
            );
        }
    }
}
