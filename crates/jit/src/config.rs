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
    /// Schema version for migrations (optional).
    pub version: Option<VersionConfig>,
    /// Type hierarchy configuration (optional).
    pub type_hierarchy: Option<HierarchyConfigToml>,
    /// Validation behavior configuration (optional).
    pub validation: Option<ValidationConfig>,
    /// Documentation lifecycle configuration (optional).
    pub documentation: Option<DocumentationConfig>,
    /// Label namespace registry (optional - replaces labels.json).
    pub namespaces: Option<HashMap<String, NamespaceConfig>>,
    /// Worktree and parallel work configuration (optional).
    pub worktree: Option<WorktreeConfig>,
}

/// Schema version configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct VersionConfig {
    /// Schema version number (default: 1).
    pub schema: u32,
}

/// Type hierarchy configuration from TOML.
#[derive(Debug, Clone, Deserialize)]
pub struct HierarchyConfigToml {
    /// Type name to hierarchy level mapping (lower = more strategic).
    pub types: HashMap<String, u8>,
    /// Type name to membership label namespace mapping (optional).
    pub label_associations: Option<HashMap<String, String>>,
    /// List of type names considered strategic (optional).
    pub strategic_types: Option<Vec<String>>,
}

/// Validation behavior configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct ValidationConfig {
    /// Strictness level: "strict", "loose", or "permissive".
    pub strictness: Option<String>,
    /// Default type when none specified (optional).
    pub default_type: Option<String>,
    /// Require exactly one type:* label per issue (default: false).
    pub require_type_label: Option<bool>,
    /// Label format regex (optional).
    pub label_regex: Option<String>,
    /// Reject malformed labels (default: false).
    pub reject_malformed_labels: Option<bool>,
    /// Enforce namespace registry (default: false).
    pub enforce_namespace_registry: Option<bool>,
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

/// Label namespace configuration from TOML.
/// Replaces the namespace definitions in labels.json.
#[derive(Debug, Clone, Deserialize)]
pub struct NamespaceConfig {
    /// Human-readable description.
    pub description: String,
    /// Whether only one label from this namespace can be applied per issue.
    pub unique: bool,
    /// Example labels (optional, for documentation).
    pub examples: Option<Vec<String>>,
}

/// Worktree and parallel work configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct WorktreeConfig {
    /// Lease enforcement mode: "strict", "warn", or "off" (default: "strict").
    pub enforce_leases: Option<String>,
}

/// Enforcement mode for lease requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnforcementMode {
    /// Block operations without active lease (production-safe default).
    Strict,
    /// Warn but allow operations without lease (development-friendly).
    Warn,
    /// No enforcement - bypass lease checks (backward compatible).
    Off,
}

impl WorktreeConfig {
    /// Get the enforcement mode, defaulting to Strict if not specified.
    pub fn enforcement_mode(&self) -> Result<EnforcementMode> {
        match self.enforce_leases.as_deref() {
            None | Some("strict") => Ok(EnforcementMode::Strict),
            Some("warn") => Ok(EnforcementMode::Warn),
            Some("off") => Ok(EnforcementMode::Off),
            Some(invalid) => anyhow::bail!(
                "Invalid enforce_leases mode: '{}'. Valid options: 'strict', 'warn', 'off'",
                invalid
            ),
        }
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
                version: None,
                type_hierarchy: None,
                validation: None,
                documentation: None,
                namespaces: None,
                worktree: None,
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

    #[test]
    fn test_parse_schema_v2_with_version() {
        let config_toml = r#"
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, task = 3 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        assert!(config.version.is_some());
        assert_eq!(config.version.unwrap().schema, 2);

        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(
            hierarchy.strategic_types,
            Some(vec!["milestone".to_string(), "epic".to_string()])
        );
    }

    #[test]
    fn test_parse_validation_with_new_fields() {
        let config_toml = r#"
[validation]
default_type = "task"
require_type_label = true
label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'
reject_malformed_labels = true
enforce_namespace_registry = true
warn_orphaned_leaves = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let validation = config.validation.unwrap();
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.require_type_label, Some(true));
        assert_eq!(
            validation.label_regex,
            Some("^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$".to_string())
        );
        assert_eq!(validation.reject_malformed_labels, Some(true));
        assert_eq!(validation.enforce_namespace_registry, Some(true));
        assert_eq!(validation.warn_orphaned_leaves, Some(false));
    }

    #[test]
    fn test_parse_namespaces_from_toml() {
        let config_toml = r#"
[namespaces.type]
description = "Issue type (hierarchical)"
unique = true
examples = ["type:task", "type:epic"]

[namespaces.epic]
description = "Feature or initiative membership"
unique = false
examples = ["epic:auth", "epic:billing"]

[namespaces.component]
description = "Technical area"
unique = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 3);

        let type_ns = &namespaces["type"];
        assert_eq!(type_ns.description, "Issue type (hierarchical)");
        assert!(type_ns.unique);
        assert_eq!(
            type_ns.examples,
            Some(vec!["type:task".to_string(), "type:epic".to_string()])
        );

        let epic_ns = &namespaces["epic"];
        assert!(!epic_ns.unique);

        let component_ns = &namespaces["component"];
        assert!(component_ns.examples.is_none());
    }

    #[test]
    fn test_parse_full_schema_v2_config() {
        let config_toml = r#"
[version]
schema = 2

[type_hierarchy]
types = { milestone = 1, epic = 2, story = 3, task = 4 }
strategic_types = ["milestone", "epic"]

[type_hierarchy.label_associations]
milestone = "milestone"
epic = "epic"
story = "story"

[validation]
default_type = "task"
require_type_label = true
warn_orphaned_leaves = true
warn_strategic_consistency = true

[namespaces.type]
description = "Issue type"
unique = true

[namespaces.epic]
description = "Epic membership"
unique = false
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();

        // Version
        assert_eq!(config.version.unwrap().schema, 2);

        // Hierarchy
        let hierarchy = config.type_hierarchy.unwrap();
        assert_eq!(hierarchy.types.len(), 4);
        assert_eq!(
            hierarchy.strategic_types,
            Some(vec!["milestone".to_string(), "epic".to_string()])
        );

        // Validation
        let validation = config.validation.unwrap();
        assert_eq!(validation.default_type, Some("task".to_string()));
        assert_eq!(validation.require_type_label, Some(true));

        // Namespaces
        let namespaces = config.namespaces.unwrap();
        assert_eq!(namespaces.len(), 2);
        assert!(namespaces["type"].unique);
        assert!(!namespaces["epic"].unique);
    }

    #[test]
    fn test_enforcement_mode_default_to_strict() {
        let config_toml = r#"
[worktree]
# No enforce_leases specified
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(
            worktree.enforcement_mode().unwrap(),
            EnforcementMode::Strict
        );
    }

    #[test]
    fn test_enforcement_mode_explicit_strict() {
        let config_toml = r#"
[worktree]
enforce_leases = "strict"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(
            worktree.enforcement_mode().unwrap(),
            EnforcementMode::Strict
        );
    }

    #[test]
    fn test_enforcement_mode_warn() {
        let config_toml = r#"
[worktree]
enforce_leases = "warn"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode().unwrap(), EnforcementMode::Warn);
    }

    #[test]
    fn test_enforcement_mode_off() {
        let config_toml = r#"
[worktree]
enforce_leases = "off"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        assert_eq!(worktree.enforcement_mode().unwrap(), EnforcementMode::Off);
    }

    #[test]
    fn test_enforcement_mode_invalid() {
        let config_toml = r#"
[worktree]
enforce_leases = "maybe"
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        let worktree = config.worktree.unwrap();
        let result = worktree.enforcement_mode();
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("Invalid enforce_leases mode"));
        assert!(err_msg.contains("maybe"));
    }

    #[test]
    fn test_config_without_worktree_section() {
        let config_toml = r#"
[type_hierarchy]
types = { task = 1 }
"#;
        let config: JitConfig = toml::from_str(config_toml).unwrap();
        assert!(config.worktree.is_none());
    }
}
