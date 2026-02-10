//! Preset manager for loading and managing gate presets

use super::{BuiltinPresets, GatePresetDefinition, PresetInfo};
use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Manages gate presets from builtin and custom sources
pub struct PresetManager {
    jit_root: PathBuf,
    presets: HashMap<String, GatePresetDefinition>,
}

impl PresetManager {
    /// Create a new preset manager
    pub fn new(jit_root: PathBuf) -> Result<Self> {
        let mut presets = BuiltinPresets::load()?;

        // Load custom presets and override builtin with same name
        let custom_presets = Self::load_custom_presets(&jit_root)?;
        for (name, preset) in custom_presets {
            presets.insert(name, preset);
        }

        Ok(Self { jit_root, presets })
    }

    /// Load custom presets from .jit/config/gate-presets/
    fn load_custom_presets(jit_root: &Path) -> Result<HashMap<String, GatePresetDefinition>> {
        let presets_dir = jit_root.join("config").join("gate-presets");
        let mut presets = HashMap::new();

        // If directory doesn't exist, return empty map (not an error)
        if !presets_dir.exists() {
            return Ok(presets);
        }

        // Read all JSON files in the directory
        let entries = fs::read_dir(&presets_dir)
            .with_context(|| format!("Failed to read presets directory: {:?}", presets_dir))?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            // Skip non-JSON files
            if path.extension() != Some(std::ffi::OsStr::new("json")) {
                continue;
            }

            // Load and parse preset
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read preset file: {:?}", path))?;

            let preset: GatePresetDefinition = serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse preset file: {:?}", path))?;

            // Validate preset
            preset
                .validate()
                .with_context(|| format!("Invalid preset in file: {:?}", path))?;

            presets.insert(preset.name.clone(), preset);
        }

        Ok(presets)
    }

    /// Get a preset by name
    pub fn get_preset(&self, name: &str) -> Result<&GatePresetDefinition> {
        self.presets
            .get(name)
            .ok_or_else(|| anyhow!("Preset not found: {}", name))
    }

    /// List all available presets
    pub fn list_presets(&self) -> Vec<PresetInfo> {
        let builtin_names = BuiltinPresets::names();

        self.presets
            .values()
            .map(|preset| PresetInfo {
                name: preset.name.clone(),
                description: preset.description.clone(),
                gate_count: preset.gates.len(),
                builtin: builtin_names.contains(&preset.name),
            })
            .collect()
    }

    /// Check if a preset exists
    pub fn has_preset(&self, name: &str) -> bool {
        self.presets.contains_key(name)
    }

    /// Get custom presets directory path
    pub fn custom_presets_dir(&self) -> PathBuf {
        self.jit_root.join("config").join("gate-presets")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{GateMode, GateStage};
    use crate::gate_presets::GateTemplate;
    use tempfile::TempDir;

    fn create_test_preset_file(dir: &Path, name: &str) -> Result<()> {
        let preset = GatePresetDefinition {
            name: name.to_string(),
            description: format!("Custom preset {}", name),
            gates: vec![GateTemplate {
                key: "custom-gate".to_string(),
                title: "Custom Gate".to_string(),
                description: "A custom gate".to_string(),
                stage: GateStage::Postcheck,
                mode: GateMode::Manual,
                checker: None,
            }],
        };

        let json = serde_json::to_string_pretty(&preset)?;
        let file_path = dir.join(format!("{}.json", name));
        fs::write(file_path, json)?;
        Ok(())
    }

    #[test]
    fn test_load_builtin_only() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        assert!(manager.has_preset("rust-tdd"));
        assert!(manager.has_preset("minimal"));
        assert_eq!(manager.presets.len(), 2);
    }

    #[test]
    fn test_get_preset() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        let preset = manager.get_preset("rust-tdd").unwrap();
        assert_eq!(preset.name, "rust-tdd");
        assert_eq!(preset.gates.len(), 5);
    }

    #[test]
    fn test_get_nonexistent_preset() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        let result = manager.get_preset("nonexistent");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_load_custom_preset() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        create_test_preset_file(&presets_dir, "my-preset").unwrap();

        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        assert!(manager.has_preset("my-preset"));
        let preset = manager.get_preset("my-preset").unwrap();
        assert_eq!(preset.name, "my-preset");
        assert_eq!(preset.gates.len(), 1);
    }

    #[test]
    fn test_custom_preset_overrides_builtin() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        // Create custom "minimal" preset that overrides builtin
        create_test_preset_file(&presets_dir, "minimal").unwrap();

        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        let preset = manager.get_preset("minimal").unwrap();
        assert_eq!(preset.description, "Custom preset minimal");
    }

    #[test]
    fn test_list_presets() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        let list = manager.list_presets();
        assert_eq!(list.len(), 2);

        let rust_tdd = list.iter().find(|p| p.name == "rust-tdd").unwrap();
        assert_eq!(rust_tdd.gate_count, 5);
        assert!(rust_tdd.builtin);

        let minimal = list.iter().find(|p| p.name == "minimal").unwrap();
        assert_eq!(minimal.gate_count, 1);
        assert!(minimal.builtin);
    }

    #[test]
    fn test_list_includes_custom_presets() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        create_test_preset_file(&presets_dir, "my-custom").unwrap();

        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();
        let list = manager.list_presets();

        assert_eq!(list.len(), 3);
        let custom = list.iter().find(|p| p.name == "my-custom").unwrap();
        assert!(!custom.builtin);
    }

    #[test]
    fn test_invalid_preset_file_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        // Create invalid JSON file
        fs::write(presets_dir.join("bad.json"), "{ invalid json }").unwrap();

        let result = PresetManager::new(temp_dir.path().to_path_buf());
        assert!(result.is_err());
    }

    #[test]
    fn test_missing_presets_dir_is_ok() {
        let temp_dir = TempDir::new().unwrap();
        // Don't create the presets directory

        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();
        assert_eq!(manager.presets.len(), 2); // Only builtins
    }
}
