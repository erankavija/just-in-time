//! Configuration file loading and parsing.
//!
//! JIT supports repository-level configuration through `.jit/config.toml`.
//! If no config file exists, the system falls back to sensible defaults.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

/// Root configuration structure loaded from `.jit/config.toml`.
#[derive(Debug, Clone, Deserialize)]
pub struct JitConfig {
    /// Type hierarchy configuration (optional).
    pub type_hierarchy: Option<HierarchyConfigToml>,
    /// Validation behavior configuration (optional).
    pub validation: Option<ValidationConfig>,
    /// Documentation lifecycle configuration (optional).
    pub documentation: Option<DocumentationConfig>,
}

/// Type hierarchy configuration from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct HierarchyConfigToml {
    /// Type name to hierarchy level mapping (lower = more strategic).
    pub types: HashMap<String, u8>,
    /// Type name to membership label namespace mapping (optional).
    pub label_associations: Option<HashMap<String, String>>,
}

/// Validation behavior configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ValidationConfig {
    /// Strictness level: "strict", "loose", or "permissive".
    pub strictness: Option<String>,
    /// Warn on orphaned leaf-level issues (default: true).
    pub warn_orphaned_leaves: Option<bool>,
    /// Warn on strategic issues without matching labels (default: true).
    pub warn_strategic_consistency: Option<bool>,
}

/// Documentation lifecycle management configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct DocumentationConfig {
    /// Root directory for development documentation (default: "dev").
    pub development_root: Option<String>,
    /// Paths subject to archival (default: ["dev/active", "dev/studies", "dev/sessions"]).
    pub managed_paths: Option<Vec<String>>,
    /// Where archived docs are stored (default: "dev/archive").
    pub archive_root: Option<String>,
    /// Paths that never archive (default: ["docs/"]).
    pub permanent_paths: Option<Vec<String>>,
    /// Archive category mappings (e.g., design -> features).
    pub categories: Option<HashMap<String, String>>,
}

impl DocumentationConfig {
    /// Get development root with default fallback.
    pub fn development_root(&self) -> String {
        self.development_root
            .clone()
            .unwrap_or_else(|| "dev".to_string())
    }

    /// Get managed paths with default fallback.
    pub fn managed_paths(&self) -> Vec<String> {
        self.managed_paths.clone().unwrap_or_else(|| {
            vec![
                "dev/active".to_string(),
                "dev/studies".to_string(),
                "dev/sessions".to_string(),
            ]
        })
    }

    /// Get archive root with default fallback.
    pub fn archive_root(&self) -> String {
        self.archive_root
            .clone()
            .unwrap_or_else(|| "dev/archive".to_string())
    }

    /// Get permanent paths with default fallback.
    pub fn permanent_paths(&self) -> Vec<String> {
        self.permanent_paths
            .clone()
            .unwrap_or_else(|| vec!["docs/".to_string()])
    }

    /// Get category mapping (key: category ID, value: archive subdirectory).
    pub fn categories(&self) -> HashMap<String, String> {
        self.categories.clone().unwrap_or_default()
    }
}

impl JitConfig {
    /// Load configuration from `.jit/config.toml` if it exists.
    ///
    /// Returns an empty config (all fields None) if the file doesn't exist.
    /// Returns an error if the file exists but is malformed.
    pub fn load(jit_root: &Path) -> Result<Self> {
        let config_path = jit_root.join("config.toml");

        if !config_path.exists() {
            // No config file - return empty config (will use defaults)
            return Ok(JitConfig {
                type_hierarchy: None,
                validation: None,
                documentation: None,
            });
        }

        let content =
            std::fs::read_to_string(&config_path).context("Failed to read config.toml")?;

        let config: JitConfig = toml::from_str(&content).context("Failed to parse config.toml")?;

        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_parse_minimal_config() {
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert!(config.validation.is_none());
    }

    #[test]
    fn test_parse_full_config() {
        let config_toml = r#"
[type_hierarchy]
types = { milestone = 1, epic = 2, task = 3 }

[type_hierarchy.label_associations]
epic = "epic"
milestone = "milestone"

[validation]
strictness = "loose"
warn_orphaned_leaves = true
warn_strategic_consistency = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(hierarchy.types.len(), 3);
        assert_eq!(hierarchy.label_associations.as_ref().unwrap().len(), 2);

        let validation = config.validation.unwrap();
        assert_eq!(validation.strictness, Some("loose".to_string()));
        assert_eq!(validation.warn_orphaned_leaves, Some(true));
        assert_eq!(validation.warn_strategic_consistency, Some(false));
    }

    #[test]
    fn test_load_missing_config() {
        let temp_dir = TempDir::new().unwrap();
        let config = JitConfig::load(temp_dir.path()).unwrap();

        // Empty config when file doesn't exist
        assert!(config.type_hierarchy.is_none());
        assert!(config.validation.is_none());
    }

    #[test]
    fn test_load_existing_config() {
        let temp_dir = TempDir::new().unwrap();

        let config_toml = r#"
[type_hierarchy]
types = { epic = 1, task = 2 }
"#;
        std::fs::write(temp_dir.path().join("config.toml"), config_toml).unwrap();

        let config = JitConfig::load(temp_dir.path()).unwrap();
        assert!(config.type_hierarchy.is_some());
        assert_eq!(config.type_hierarchy.unwrap().types.len(), 2);
    }

    #[test]
    fn test_malformed_toml_returns_error() {
        let temp_dir = TempDir::new().unwrap();

        let bad_toml = "[broken syntax";
        std::fs::write(temp_dir.path().join("config.toml"), bad_toml).unwrap();

        let result = JitConfig::load(temp_dir.path());
        assert!(result.is_err());
    }
}
