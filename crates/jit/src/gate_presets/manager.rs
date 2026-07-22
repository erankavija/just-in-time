//! Preset manager for loading and managing gate presets

use super::{GatePresetDefinition, PresetInfo};
use anyhow::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Manages gate presets from builtin and custom sources.
pub struct PresetManager {
    jit_root: PathBuf,
    presets: HashMap<String, GatePresetDefinition>,
    custom_names: std::collections::HashSet<String>,
}

impl PresetManager {
    /// Create a new preset manager
    pub fn new(jit_root: PathBuf) -> Result<Self> {
        let (presets, custom_names) = Self::load_presets(&jit_root)?;

        Ok(Self {
            jit_root,
            presets,
            custom_names,
        })
    }

    /// Load custom presets from .jit/config/gate-presets/
    fn load_presets(
        jit_root: &Path,
    ) -> Result<(HashMap<String, GatePresetDefinition>, HashSet<String>)> {
        let presets_dir = jit_root.join("config").join("gate-presets");

        match fs::symlink_metadata(&presets_dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return super::load_presets_from_custom_files(Vec::new())
            }
            Err(error) => return Err(error.into()),
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err(crate::errors::InvalidArgumentError::new(format!(
                    "Custom preset root '{}' must be an ordinary directory",
                    presets_dir.display()
                ))
                .into())
            }
            Ok(_) => {}
        }

        // Read all JSON files in the directory
        let mut entries = fs::read_dir(&presets_dir)
            .with_context(|| format!("Failed to read presets directory: {:?}", presets_dir))?
            .collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        let files = entries
            .into_iter()
            .filter(|entry| entry.path().extension() == Some(std::ffi::OsStr::new("json")))
            .map(|entry| {
                let path = entry.path();
                if !entry.file_type()?.is_file() {
                    return Err(crate::errors::InvalidArgumentError::new(format!(
                        "Custom preset file '{}' must be an ordinary file",
                        path.display()
                    ))
                    .into());
                }
                let bytes = read_regular_preset_no_follow(&path)?;
                Ok((entry.file_name().to_string_lossy().into_owned(), bytes))
            })
            .collect::<Result<Vec<_>>>()?;
        super::load_presets_from_custom_files(files)
    }

    /// Get a preset by name.
    pub fn get_preset(&self, name: &str) -> Result<&GatePresetDefinition> {
        self.presets
            .get(name)
            .ok_or_else(|| crate::storage::PresetNotFoundError::new(name).into())
    }

    /// List all available presets.
    pub fn list_presets(&self) -> Vec<PresetInfo> {
        let mut presets = self
            .presets
            .values()
            .map(|preset| PresetInfo {
                name: preset.name.clone(),
                description: preset.description.clone(),
                gate_count: preset.gates.len(),
                builtin: !self.custom_names.contains(&preset.name),
            })
            .collect::<Vec<_>>();
        presets.sort_by(|left, right| left.name.cmp(&right.name));
        presets
    }

    /// Check if a preset exists.
    pub fn has_preset(&self, name: &str) -> bool {
        self.presets.contains_key(name)
    }

    /// Get custom presets directory path.
    pub fn custom_presets_dir(&self) -> PathBuf {
        self.jit_root.join("config").join("gate-presets")
    }
}

#[cfg(unix)]
fn read_regular_preset_no_follow(path: &Path) -> Result<Vec<u8>> {
    use nix::fcntl::{open, OFlag};
    use nix::sys::stat::Mode;
    use std::io::Read;

    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Custom preset file '{}' must be an ordinary file",
            path.display()
        ))
        .into());
    }
    let file = open(
        path,
        OFlag::O_RDONLY | OFlag::O_CLOEXEC | OFlag::O_NOFOLLOW,
        Mode::empty(),
    )
    .with_context(|| {
        format!(
            "opening preset file without following links: {}",
            path.display()
        )
    })?;
    let mut file = fs::File::from(file);
    if !file.metadata()?.is_file() {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Custom preset file '{}' must be an ordinary file",
            path.display()
        ))
        .into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(not(unix))]
fn read_regular_preset_no_follow(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.file_type().is_file() {
        return Err(crate::errors::InvalidArgumentError::new(format!(
            "Custom preset file '{}' must be an ordinary file",
            path.display()
        ))
        .into());
    }
    fs::read(path).with_context(|| format!("reading preset file: {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarations::{GateMode, GateStage};
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

        assert!(manager.has_preset("plan-review"));
        assert!(manager.has_preset("coverage-preview"));
        assert_eq!(manager.presets.len(), 3);
    }

    #[test]
    fn test_get_preset() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        let preset = manager.get_preset("plan-review").unwrap();
        assert_eq!(preset.name, "plan-review");
        assert_eq!(preset.gates.len(), 1);
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
    fn test_custom_preset_cannot_override_builtin() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        create_test_preset_file(&presets_dir, "plan-review").unwrap();
        let error = PresetManager::new(temp_dir.path().to_path_buf())
            .err()
            .expect("builtin collision must be rejected");
        assert!(error.to_string().contains("collides with a builtin"));
    }

    #[test]
    fn test_list_presets() {
        let temp_dir = TempDir::new().unwrap();
        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();

        let list = manager.list_presets();
        assert_eq!(list.len(), 3);

        let plan_review = list.iter().find(|p| p.name == "plan-review").unwrap();
        assert_eq!(plan_review.gate_count, 1);
        assert!(plan_review.builtin);

        let coverage = list.iter().find(|p| p.name == "coverage-preview").unwrap();
        assert_eq!(coverage.gate_count, 1);
        assert!(coverage.builtin);
    }

    #[test]
    fn test_list_includes_custom_presets() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        create_test_preset_file(&presets_dir, "my-custom").unwrap();

        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();
        let list = manager.list_presets();

        assert_eq!(list.len(), 4);
        let custom = list.iter().find(|p| p.name == "my-custom").unwrap();
        assert!(!custom.builtin);
    }

    #[test]
    fn test_custom_filename_must_match_embedded_name() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();

        create_test_preset_file(&presets_dir, "embedded-name").unwrap();
        fs::rename(
            presets_dir.join("embedded-name.json"),
            presets_dir.join("different-name.json"),
        )
        .unwrap();
        let error = PresetManager::new(temp_dir.path().to_path_buf())
            .err()
            .expect("filename mismatch must be rejected");
        assert!(error.to_string().contains("must match embedded name"));
    }

    #[test]
    fn test_list_presets_is_sorted_by_name() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();
        create_test_preset_file(&presets_dir, "zeta-checks").unwrap();
        create_test_preset_file(&presets_dir, "alpha-checks").unwrap();

        let names = PresetManager::new(temp_dir.path().to_path_buf())
            .unwrap()
            .list_presets()
            .into_iter()
            .map(|preset| preset.name)
            .collect::<Vec<_>>();
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]));
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
    fn test_non_regular_json_preset_occupant_is_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(presets_dir.join("directory.json")).unwrap();

        let error = PresetManager::new(temp_dir.path().to_path_buf())
            .err()
            .expect("a .json directory must be rejected");
        assert!(error.to_string().contains("must be an ordinary file"));
    }

    #[cfg(unix)]
    #[test]
    fn test_symlink_json_preset_occupant_is_rejected_without_following() {
        use std::os::unix::fs::symlink;

        let temp_dir = TempDir::new().unwrap();
        let presets_dir = temp_dir.path().join("config").join("gate-presets");
        fs::create_dir_all(&presets_dir).unwrap();
        create_test_preset_file(&presets_dir, "regular").unwrap();
        symlink("regular.json", presets_dir.join("linked.json")).unwrap();

        let error = PresetManager::new(temp_dir.path().to_path_buf())
            .err()
            .expect("a .json symlink must be rejected without following");
        assert!(error.to_string().contains("must be an ordinary file"));
    }

    #[test]
    fn test_non_directory_custom_preset_root_is_rejected() {
        let temp_dir = TempDir::new().unwrap();
        let config_dir = temp_dir.path().join("config");
        fs::create_dir_all(&config_dir).unwrap();
        fs::write(config_dir.join("gate-presets"), "not a directory").unwrap();

        let error = PresetManager::new(temp_dir.path().to_path_buf())
            .err()
            .expect("a non-directory preset root must be rejected");
        assert!(error.to_string().contains("must be an ordinary directory"));
    }

    #[test]
    fn test_missing_presets_dir_is_ok() {
        let temp_dir = TempDir::new().unwrap();
        // Don't create the presets directory

        let manager = PresetManager::new(temp_dir.path().to_path_buf()).unwrap();
        assert_eq!(manager.presets.len(), 3); // Only builtins
    }
}
