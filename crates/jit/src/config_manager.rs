//! Configuration management layer.
//!
//! This module provides a clean separation between configuration loading and
//! the storage layer. ConfigManager is responsible for loading and providing
//! access to configuration data, ensuring the storage layer remains generic
//! and focused on persisting runtime state.

use crate::config::{JitConfig, NamespaceConfig};
use crate::domain::{LabelNamespace, LabelNamespaces};
use crate::type_icons::{resolve_icons_for_hierarchy, IconConfig};
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Manages configuration loading and access.
///
/// ConfigManager provides a single point of access for all configuration data,
/// separating configuration concerns from storage layer implementation.
/// This enables the storage layer to remain generic and support different
/// backends (JSON, SQL, etc.) without coupling to TOML parsing.
///
/// # Examples
///
/// ```no_run
/// use jit::config_manager::ConfigManager;
///
/// let config_mgr = ConfigManager::new(".jit");
/// let namespaces = config_mgr.get_namespaces().unwrap();
/// ```
#[derive(Clone)]
pub struct ConfigManager {
    root: PathBuf,
}

impl ConfigManager {
    /// Create a new ConfigManager for the given JIT repository root.
    ///
    /// # Arguments
    ///
    /// * `root` - Path to the .jit directory
    pub fn new<P: AsRef<Path>>(root: P) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    /// Load the JIT configuration from config.toml.
    ///
    /// Returns an empty config (all fields None) if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(&self) -> Result<JitConfig> {
        JitConfig::load(&self.root)
    }

    /// Get the namespace registry from configuration.
    ///
    /// Builds LabelNamespaces from config.toml, providing default namespaces
    /// if none are configured. This replaces the legacy labels.json file.
    ///
    /// # Errors
    ///
    /// Returns an error if config.toml exists but cannot be parsed.
    pub fn get_namespaces(&self) -> Result<LabelNamespaces> {
        let config = self.load()?;

        // If config has namespaces, build from those
        if let Some(ref namespaces_config) = config.namespaces {
            return Ok(self.build_namespaces_from_config(&config, namespaces_config.clone()));
        }

        // Otherwise return defaults
        Ok(self.default_namespaces())
    }

    /// Get the enforcement mode for lease requirements.
    ///
    /// Returns the configured enforcement mode from `.jit/config.toml`,
    /// defaulting to `EnforcementMode::Off` if not configured.
    ///
    /// **Default Behavior:**
    /// - Single-agent development: Enforcement OFF (no friction)
    /// - Multi-agent coordination: Explicitly enable `strict` or `warn` in config
    ///
    /// # Enforcement Modes
    ///
    /// - `Off`: No lease enforcement (default for single-agent work)
    /// - `Warn`: Log warnings but allow operations without lease
    /// - `Strict`: Block operations without active lease (for multi-agent teams)
    ///
    /// # Errors
    ///
    /// Returns an error if config.toml exists but has an invalid enforcement mode.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::config_manager::ConfigManager;
    /// use jit::config::EnforcementMode;
    ///
    /// let config_mgr = ConfigManager::new(".jit");
    /// let mode = config_mgr.get_enforcement_mode().unwrap();
    /// assert_eq!(mode, EnforcementMode::Off);  // Default
    /// ```
    pub fn get_enforcement_mode(&self) -> Result<crate::config::EnforcementMode> {
        let config = self.load()?;

        if let Some(worktree_config) = config.worktree {
            worktree_config.enforcement_mode()
        } else {
            // No worktree section - default to Off for single-agent development
            Ok(crate::config::EnforcementMode::Off)
        }
    }

    /// Get resolved icons for the current hierarchy.
    ///
    /// Returns a map of type name to icon string. Icons are resolved using the
    /// hierarchy configuration (levels) and icon configuration (preset + custom).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::config_manager::ConfigManager;
    ///
    /// let config_mgr = ConfigManager::new(".jit");
    /// let icons = config_mgr.get_hierarchy_icons().unwrap();
    /// assert!(icons.contains_key("epic"));
    /// ```
    pub fn get_hierarchy_icons(&self) -> Result<HashMap<String, String>> {
        let config = self.load()?;

        // Get type hierarchy (levels)
        let types = config
            .type_hierarchy
            .as_ref()
            .map(|h| h.types.clone())
            .unwrap_or_default();

        // Get icon configuration
        let icon_config = config
            .type_hierarchy
            .as_ref()
            .and_then(|h| h.icons.as_ref())
            .map(|icons_toml| IconConfig::new(icons_toml.preset.clone(), icons_toml.custom.clone()))
            .unwrap_or_default();

        // Resolve icons for all types
        Ok(resolve_icons_for_hierarchy(&types, &icon_config))
    }

    /// Build LabelNamespaces from configuration.
    fn build_namespaces_from_config(
        &self,
        config: &JitConfig,
        namespaces_config: HashMap<String, NamespaceConfig>,
    ) -> LabelNamespaces {
        let mut namespaces = HashMap::new();
        for (name, ns_config) in namespaces_config {
            namespaces.insert(
                name,
                LabelNamespace::new(ns_config.description, ns_config.unique),
            );
        }

        let mut result = LabelNamespaces {
            schema_version: config.version.as_ref().map(|v| v.schema).unwrap_or(2),
            namespaces,
            type_hierarchy: config.type_hierarchy.as_ref().map(|h| h.types.clone()),
            label_associations: config
                .type_hierarchy
                .as_ref()
                .and_then(|h| h.label_associations.clone()),
            strategic_types: config
                .type_hierarchy
                .as_ref()
                .and_then(|h| h.strategic_types.clone()),
        };

        // Sync membership namespaces from label_associations
        result.sync_membership_namespaces();

        result
    }

    /// Get default namespace configuration.
    ///
    /// Provides sensible defaults when no config.toml exists.
    fn default_namespaces(&self) -> LabelNamespaces {
        LabelNamespaces::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_dir() -> TempDir {
        TempDir::new().unwrap()
    }

    #[test]
    fn test_load_missing_config() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let config = config_mgr.load().unwrap();

        // Missing config should return empty config, not error
        assert!(config.version.is_none());
        assert!(config.namespaces.is_none());
    }

    #[test]
    fn test_load_valid_config() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = r#"
[version]
schema = 2

[namespaces.type]
description = "Issue type"
unique = true

[namespaces.epic]
description = "Epic membership"
unique = false
"#;
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let config = config_mgr.load().unwrap();

        assert_eq!(config.version.unwrap().schema, 2);
        assert!(config.namespaces.is_some());
        assert_eq!(config.namespaces.unwrap().len(), 2);
    }

    #[test]
    fn test_get_namespaces_from_config() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = r#"
[version]
schema = 2

[namespaces.type]
description = "Issue type (hierarchical)"
unique = true

[namespaces.epic]
description = "Epic membership"
unique = false

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"
"#;
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let namespaces = config_mgr.get_namespaces().unwrap();

        assert_eq!(namespaces.schema_version, 2);
        assert_eq!(namespaces.namespaces.len(), 3); // type, epic, + milestone (auto-synced)
        assert!(namespaces.namespaces.contains_key("type"));
        assert!(namespaces.namespaces.contains_key("epic"));
        assert!(namespaces.namespaces.contains_key("milestone"));

        // Check strategic types
        assert_eq!(
            namespaces.strategic_types.as_ref().unwrap(),
            &vec!["milestone".to_string(), "epic".to_string()]
        );
    }

    #[test]
    fn test_get_namespaces_defaults() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let namespaces = config_mgr.get_namespaces().unwrap();

        // Should return defaults
        assert!(namespaces.namespaces.contains_key("type"));
        assert!(namespaces.namespaces.contains_key("epic"));
    }

    #[test]
    fn test_namespace_unique_property() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = r#"
[namespaces.type]
description = "Issue type"
unique = true

[namespaces.component]
description = "Component"
unique = false
"#;
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let namespaces = config_mgr.get_namespaces().unwrap();

        assert!(namespaces.namespaces.get("type").unwrap().unique);
        assert!(!namespaces.namespaces.get("component").unwrap().unique);
    }

    #[test]
    fn test_malformed_config_returns_error() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let invalid_toml = "this is not valid toml [[[";
        fs::write(jit_dir.join("config.toml"), invalid_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let result = config_mgr.load();

        assert!(result.is_err());
    }

    #[test]
    fn test_get_enforcement_mode_default_when_missing() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let mode = config_mgr.get_enforcement_mode().unwrap();

        // Default to Off when no config exists (single-agent development)
        assert_eq!(mode, crate::config::EnforcementMode::Off);
    }

    #[test]
    fn test_get_enforcement_mode_from_config() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = r#"
[worktree]
enforce_leases = "warn"
"#;
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let mode = config_mgr.get_enforcement_mode().unwrap();

        assert_eq!(mode, crate::config::EnforcementMode::Warn);
    }

    #[test]
    fn test_get_enforcement_mode_invalid() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = r#"
[worktree]
enforce_leases = "invalid"
"#;
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let result = config_mgr.get_enforcement_mode();

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Invalid enforce_leases mode"));
    }
}
