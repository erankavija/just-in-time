//! Gate preset management system
//!
//! This module provides functionality for managing gate presets - pre-configured
//! bundles of quality gates that can be applied to issues. Presets encode best
//! practices for common workflows (e.g., rust-tdd, minimal) and reduce setup time.

mod builtin;
mod manager;

pub use builtin::BuiltinPresets;
pub use manager::PresetManager;

use crate::domain::{Gate, GateChecker, GateMode, GateStage};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A gate template within a preset
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GateTemplate {
    /// Gate key (unique identifier)
    pub key: String,
    /// Human-readable title
    pub title: String,
    /// Description of what this gate checks
    pub description: String,
    /// Gate execution stage
    pub stage: GateStage,
    /// Gate mode (manual or automated)
    pub mode: GateMode,
    /// Checker configuration for automated gates
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checker: Option<GateChecker>,
}

impl GateTemplate {
    /// Convert template to full Gate definition
    pub fn to_gate(&self) -> Gate {
        Gate {
            version: 1,
            key: self.key.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            stage: self.stage,
            mode: self.mode,
            checker: self.checker.clone(),
            reserved: HashMap::new(),
            auto: self.mode == GateMode::Auto,
            example_integration: None,
        }
    }
}

/// A preset definition containing multiple gate templates
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GatePresetDefinition {
    /// Unique preset name
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Gates included in this preset
    pub gates: Vec<GateTemplate>,
}

impl GatePresetDefinition {
    /// Validate preset structure
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(anyhow!("Preset name cannot be empty"));
        }

        if self.description.is_empty() {
            return Err(anyhow!("Preset description cannot be empty"));
        }

        if self.gates.is_empty() {
            return Err(anyhow!("Preset must contain at least one gate"));
        }

        // Validate each gate
        for gate in &self.gates {
            if gate.key.is_empty() {
                return Err(anyhow!("Gate key cannot be empty"));
            }
            if gate.title.is_empty() {
                return Err(anyhow!("Gate title cannot be empty"));
            }

            // Auto gates must have checker
            if gate.mode == GateMode::Auto && gate.checker.is_none() {
                return Err(anyhow!(
                    "Automated gate '{}' must have checker configuration",
                    gate.key
                ));
            }
        }

        // Check for duplicate gate keys
        let mut keys = std::collections::HashSet::new();
        for gate in &self.gates {
            if !keys.insert(&gate.key) {
                return Err(anyhow!("Duplicate gate key: {}", gate.key));
            }
        }

        Ok(())
    }
}

/// Preset metadata for listing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PresetInfo {
    /// Preset name
    pub name: String,
    /// Description
    pub description: String,
    /// Number of gates
    pub gate_count: usize,
    /// Whether this is a builtin preset
    pub builtin: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gate_template_to_gate() {
        let template = GateTemplate {
            key: "tests".to_string(),
            title: "All tests pass".to_string(),
            description: "Run test suite".to_string(),
            stage: GateStage::Postcheck,
            mode: GateMode::Auto,
            checker: Some(GateChecker::Exec {
                command: "cargo test".to_string(),
                timeout_seconds: 300,
                working_dir: None,
                env: HashMap::new(),
            }),
        };

        let gate = template.to_gate();
        assert_eq!(gate.key, "tests");
        assert_eq!(gate.title, "All tests pass");
        assert_eq!(gate.stage, GateStage::Postcheck);
        assert_eq!(gate.mode, GateMode::Auto);
        assert!(gate.checker.is_some());
    }

    #[test]
    fn test_preset_validation_success() {
        let preset = GatePresetDefinition {
            name: "test-preset".to_string(),
            description: "A test preset".to_string(),
            gates: vec![GateTemplate {
                key: "test-gate".to_string(),
                title: "Test Gate".to_string(),
                description: "A test gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };

        assert!(preset.validate().is_ok());
    }

    #[test]
    fn test_preset_validation_empty_name() {
        let preset = GatePresetDefinition {
            name: "".to_string(),
            description: "A test preset".to_string(),
            gates: vec![],
        };

        assert!(preset.validate().is_err());
    }

    #[test]
    fn test_preset_validation_empty_description() {
        let preset = GatePresetDefinition {
            name: "test".to_string(),
            description: "".to_string(),
            gates: vec![],
        };

        assert!(preset.validate().is_err());
    }

    #[test]
    fn test_preset_validation_no_gates() {
        let preset = GatePresetDefinition {
            name: "test".to_string(),
            description: "A test preset".to_string(),
            gates: vec![],
        };

        assert!(preset.validate().is_err());
    }

    #[test]
    fn test_preset_validation_auto_gate_requires_checker() {
        let preset = GatePresetDefinition {
            name: "test".to_string(),
            description: "A test preset".to_string(),
            gates: vec![GateTemplate {
                key: "auto-gate".to_string(),
                title: "Automated Gate".to_string(),
                description: "Should have checker".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Auto,
                checker: None, // Missing checker
            }],
        };

        let result = preset.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must have checker configuration"));
    }

    #[test]
    fn test_preset_validation_duplicate_keys() {
        let preset = GatePresetDefinition {
            name: "test".to_string(),
            description: "A test preset".to_string(),
            gates: vec![
                GateTemplate {
                    key: "gate1".to_string(),
                    title: "Gate 1".to_string(),
                    description: "First".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
                GateTemplate {
                    key: "gate1".to_string(), // Duplicate
                    title: "Gate 1 Again".to_string(),
                    description: "Second".to_string(),
                    stage: GateStage::Postcheck,
                    mode: GateMode::Manual,
                    checker: None,
                },
            ],
        };

        let result = preset.validate();
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Duplicate gate key"));
    }

    #[test]
    fn test_preset_deserialization() {
        let json = r#"{
            "name": "test-preset",
            "description": "A test preset",
            "gates": [
                {
                    "key": "tests",
                    "title": "Tests Pass",
                    "description": "Run tests",
                    "stage": "postcheck",
                    "mode": "auto",
                    "checker": {
                        "type": "exec",
                        "command": "cargo test",
                        "timeout_seconds": 300
                    }
                }
            ]
        }"#;

        let preset: GatePresetDefinition = serde_json::from_str(json).unwrap();
        assert_eq!(preset.name, "test-preset");
        assert_eq!(preset.gates.len(), 1);
        assert_eq!(preset.gates[0].key, "tests");
    }
}
