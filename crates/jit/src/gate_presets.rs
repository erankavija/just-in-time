//! Gate preset management system
//!
//! This module provides functionality for managing gate presets - pre-configured
//! bundles of quality gates that can be applied to issues. A preset captures a
//! repeated gate set (a project's CI bundle, a review workflow) so it attaches in
//! one command instead of gate-by-gate.
//!
//! [`BuiltinPresets`] carries the presets the binary ships — only the
//! planning-bracket trio, which is workflow infrastructure of the plan bracket
//! rather than domain vocabulary. [`PresetManager`] loads those plus a project's
//! own presets from `.jit/config/gate-presets/`, where domain-specific bundles
//! belong (`@/inv/domain-agnostic`, `@/charter/D-2`).
//! The [`reference`] submodule projects the built-in definitions into the
//! committed markdown reference [`REFERENCE_PATH`].

mod builtin;
mod manager;
mod planning;
pub mod reference;

pub use builtin::BuiltinPresets;
pub use manager::PresetManager;
pub use planning::{
    breakdown_review_preset, coverage_preview_preset, plan_review_preset, preview_coverage_rule,
    BREAKDOWN_REVIEW_PRESET, COVERAGE_PREVIEW_GATE, COVERAGE_PREVIEW_PRESET, PLAN_REVIEW_PRESET,
};
pub use reference::{render_reference_markdown, REFERENCE_PATH};

use crate::declarations::GateDefinition;
use crate::declarations::{GateChecker, GateMode, GateStage};
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A gate template within a preset.
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
    pub fn to_gate(&self) -> GateDefinition {
        GateDefinition {
            version: 1,
            key: self.key.clone(),
            title: self.title.clone(),
            description: self.description.clone(),
            stage: self.stage,
            mode: self.mode,
            checker: self.checker.clone(),
            priority: 100,
            reserved: HashMap::new(),
            auto: self.mode == GateMode::Auto,
            example_integration: None,
        }
    }
}

/// A preset definition containing multiple gate templates.
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
        validate_preset_name(&self.name)?;

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

/// Validate the canonical, path-safe spelling of a gate preset name.
///
/// Preset names are lowercase kebab case: they start with a lowercase ASCII
/// letter and contain lowercase letters, digits, and single separating hyphens.
/// This same spelling is used as the custom preset filename stem.
pub fn validate_preset_name(name: &str) -> Result<()> {
    let mut bytes = name.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase()) {
        return Err(anyhow!(
            "Preset name must be lowercase kebab case (for example 'ci-checks')"
        ));
    }
    let mut previous_hyphen = false;
    for byte in bytes {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' => previous_hyphen = false,
            b'-' if !previous_hyphen => previous_hyphen = true,
            _ => {
                return Err(anyhow!(
                    "Preset name must be lowercase kebab case (for example 'ci-checks')"
                ))
            }
        }
    }
    if previous_hyphen {
        return Err(anyhow!(
            "Preset name must be lowercase kebab case (for example 'ci-checks')"
        ));
    }
    Ok(())
}

/// Parse the binary-shipped presets plus a captured set of custom JSON files.
///
/// This is the single pure policy boundary shared by filesystem and captured
/// readers: canonical names, filename/name agreement, builtin collision
/// rejection, and deterministic file-order diagnostics.
pub(crate) fn load_presets_from_custom_files(
    mut files: Vec<(String, Vec<u8>)>,
) -> Result<(
    HashMap<String, GatePresetDefinition>,
    std::collections::HashSet<String>,
)> {
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut presets = BuiltinPresets::load()?;
    let builtin_names = presets
        .keys()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    let mut custom_names = std::collections::HashSet::new();
    for (filename, bytes) in files {
        let preset: GatePresetDefinition = serde_json::from_slice(&bytes)
            .with_context(|| format!("Failed to parse custom gate preset file '{filename}'"))?;
        preset.validate().map_err(|_| {
            crate::errors::InvalidArgumentError::new(format!("Invalid preset in file: {filename}"))
        })?;
        let basename = filename.rsplit('/').next().unwrap_or_default();
        let stem = basename.strip_suffix(".json").ok_or_else(|| {
            crate::errors::InvalidArgumentError::new(format!(
                "Custom preset file '{filename}' must have a .json extension"
            ))
        })?;
        if stem != preset.name {
            return Err(crate::errors::InvalidArgumentError::new(format!(
                "Custom preset filename stem '{}' must match embedded name '{}'",
                stem, preset.name
            ))
            .into());
        }
        if builtin_names.contains(&preset.name) {
            return Err(crate::errors::InvalidArgumentError::new(format!(
                "Custom preset '{}' collides with a builtin preset",
                preset.name
            ))
            .into());
        }
        custom_names.insert(preset.name.clone());
        presets.insert(preset.name.clone(), preset);
    }
    Ok((presets, custom_names))
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
                pass_context: false,
                prompt: None,
                prompt_file: None,
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
    fn test_preset_validation_rejects_path_and_non_kebab_names() {
        for name in ["../escape", "Upper", "two--hyphens", "trailing-"] {
            assert!(validate_preset_name(name).is_err(), "accepted {name}");
        }
        assert!(validate_preset_name("ci-checks-2").is_ok());
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
