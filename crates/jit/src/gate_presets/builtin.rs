//! Built-in gate presets.
//!
//! The binary ships exactly the three [planning-bracket](planning) presets —
//! `plan-review`, `coverage-preview`, and `breakdown-review`. They are workflow
//! infrastructure of the plan-before-fan-out bracket (`@/charter/D-3`): the plan
//! template resolves them by name to gate the planning and breakdown nodes, so
//! the bracket is self-contained without any per-project gate authoring.
//!
//! No language- or content-flavored bundles are built in. Gate keys, titles,
//! and checkers (a test runner, a linter, a formatter, a security audit) are
//! domain vocabulary; per `@/inv/domain-agnostic` and `@/charter/D-2` they come
//! from repository configuration (`.jit/gates.toml` and project-defined presets
//! under `.jit/config/gate-presets/`), never from hardcoded assumptions in the
//! engine. [`super::PresetManager`] loads a project's own presets alongside
//! these built-ins.

use super::{
    breakdown_review_preset, coverage_preview_preset, plan_review_preset, GatePresetDefinition,
    BREAKDOWN_REVIEW_PRESET, COVERAGE_PREVIEW_PRESET, PLAN_REVIEW_PRESET,
};
use anyhow::Result;
use std::collections::HashMap;

/// Built-in presets bundled with the binary.
pub struct BuiltinPresets;

impl BuiltinPresets {
    /// Load all built-in presets.
    ///
    /// The set is the planning-bracket trio and nothing else: the agent
    /// plan-quality gate on the planning node, and the deterministic
    /// coverage-preview plus agent breakdown-review gates on the breakdown node.
    /// They are defined in [`super::planning`] so the gate shapes live next to
    /// the preview-rule constructor.
    pub fn load() -> Result<HashMap<String, GatePresetDefinition>> {
        let mut presets = HashMap::new();

        let plan_review = plan_review_preset();
        let coverage_preview = coverage_preview_preset();
        let breakdown_review = breakdown_review_preset();

        // Validate all presets
        let all_presets = [&plan_review, &coverage_preview, &breakdown_review];
        for preset in &all_presets {
            preset.validate()?;
        }

        presets.insert(plan_review.name.clone(), plan_review);
        presets.insert(coverage_preview.name.clone(), coverage_preview);
        presets.insert(breakdown_review.name.clone(), breakdown_review);

        Ok(presets)
    }

    /// Get list of builtin preset names.
    pub fn names() -> Vec<String> {
        vec![
            PLAN_REVIEW_PRESET.to_string(),
            COVERAGE_PREVIEW_PRESET.to_string(),
            BREAKDOWN_REVIEW_PRESET.to_string(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::GateMode;

    #[test]
    fn test_load_builtin_presets() {
        let presets = BuiltinPresets::load().unwrap();
        assert_eq!(presets.len(), 3);
        assert!(presets.contains_key("plan-review"));
        assert!(presets.contains_key("coverage-preview"));
        assert!(presets.contains_key("breakdown-review"));
    }

    /// The removed language- and content-flavored bundles are not built in
    /// (REQ-01): only the planning-bracket trio ships.
    #[test]
    fn test_language_and_content_bundles_not_builtin() {
        let presets = BuiltinPresets::load().unwrap();
        for removed in [
            "rust-tdd",
            "python-tdd",
            "js-tdd",
            "minimal",
            "security-audit",
        ] {
            assert!(
                !presets.contains_key(removed),
                "{removed} must not be a built-in preset"
            );
            assert!(
                !BuiltinPresets::names().contains(&removed.to_string()),
                "{removed} must not be in the built-in name list"
            );
        }
    }

    #[test]
    fn test_planning_bracket_presets_registered() {
        let presets = BuiltinPresets::load().unwrap();

        let plan_review = presets.get("plan-review").unwrap();
        assert_eq!(plan_review.gates.len(), 1);
        assert_eq!(plan_review.gates[0].key, "plan-review");
        assert_eq!(plan_review.gates[0].mode, GateMode::Auto);

        let coverage = presets.get("coverage-preview").unwrap();
        assert_eq!(coverage.gates.len(), 1);
        assert_eq!(coverage.gates[0].key, "coverage-preview");
        assert_eq!(coverage.gates[0].mode, GateMode::Auto);

        let breakdown_review = presets.get("breakdown-review").unwrap();
        assert_eq!(breakdown_review.gates.len(), 1);
        assert_eq!(breakdown_review.gates[0].key, "breakdown-review");
        assert_eq!(breakdown_review.gates[0].mode, GateMode::Auto);

        assert!(BuiltinPresets::names().contains(&"plan-review".to_string()));
        assert!(BuiltinPresets::names().contains(&"coverage-preview".to_string()));
        assert!(BuiltinPresets::names().contains(&"breakdown-review".to_string()));
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
