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

/// Environment variable carrying one invocation's linked-checkout write stance.
///
/// It is the per-invocation counterpart of the repository's declared
/// `[worktree] write_policy`, named after that key the way
/// `JIT_ENFORCE_LEASES` and `JIT_WORKTREE_MODE` are named after theirs.
pub const LINKED_CHECKOUT_WRITE_STANCE_ENV: &str = "JIT_WORKTREE_WRITE_POLICY";

/// Manages configuration loading and access.
///
/// ConfigManager provides a single point of access for all configuration data,
/// separating configuration concerns from storage layer implementation.
/// This enables the storage layer to remain generic and support different
/// backends (JSON, SQL, etc.) without coupling to TOML parsing.
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
    /// Builds [`LabelNamespaces`] from `config.toml`, carrying exactly the
    /// namespaces and type hierarchy it declares.
    ///
    /// # Errors
    ///
    /// Returns an error if config.toml exists but cannot be parsed.
    pub fn get_namespaces(&self) -> Result<LabelNamespaces> {
        let config = self.load()?;
        Ok(self.namespaces_from_config(&config))
    }

    /// Build the namespace registry from an ALREADY-loaded config, without
    /// re-reading `config.toml` from disk.
    ///
    /// Callers that already hold a parsed [`JitConfig`] (e.g. a cached copy)
    /// should use this instead of [`get_namespaces`](Self::get_namespaces) so a
    /// single write command parses `config.toml` at most once.
    pub fn namespaces_from_config(&self, config: &JitConfig) -> LabelNamespaces {
        namespaces_from_config(config)
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
    pub fn get_enforcement_mode(&self) -> Result<crate::config::EnforcementMode> {
        let config = self.load()?;
        self.enforcement_mode_from_config(&config)
    }

    /// Resolve the lease enforcement mode from an ALREADY-loaded config, without
    /// re-reading `config.toml` from disk.
    ///
    /// Callers on a write path that already hold a cached [`JitConfig`] should use
    /// this so a single command parses `config.toml` at most once.
    pub fn enforcement_mode_from_config(
        &self,
        config: &JitConfig,
    ) -> Result<crate::config::EnforcementMode> {
        match config.worktree.as_ref() {
            Some(worktree_config) => Ok(worktree_config.enforcement_mode()),
            // No worktree section - default to Off for single-agent development.
            None => Ok(crate::config::EnforcementMode::Off),
        }
    }

    /// Get the worktree write-policy stance for state-mutating commands run
    /// inside a linked non-primary checkout.
    ///
    /// Returns the configured stance from `.jit/config.toml`, defaulting to
    /// [`crate::domain::LinkedCheckoutWriteStance::DEFAULT`] (refuse) if not
    /// configured.
    ///
    /// Unlike [`Self::get_enforcement_mode`], an absent `write_policy` key in
    /// a present `[worktree]` section and an absent `[worktree]` section
    /// altogether resolve identically — both refuse. This method states that
    /// symmetry explicitly rather than inheriting the section-presence
    /// asymmetry `enforce_leases` has.
    ///
    /// # Errors
    ///
    /// Returns an error if config.toml exists but cannot be parsed.
    pub fn get_write_policy(&self) -> Result<crate::domain::LinkedCheckoutWriteStance> {
        let config = self.load()?;
        self.write_policy_from_config(&config)
    }

    /// Resolve the write-policy stance from an ALREADY-loaded config, without
    /// re-reading `config.toml` from disk.
    pub fn write_policy_from_config(
        &self,
        config: &JitConfig,
    ) -> Result<crate::domain::LinkedCheckoutWriteStance> {
        Ok(config
            .worktree
            .as_ref()
            .map(crate::config::WorktreeConfig::write_policy)
            .unwrap_or(crate::domain::LinkedCheckoutWriteStance::DEFAULT))
    }

    /// Read the per-invocation override of the linked-checkout write stance from
    /// the environment.
    ///
    /// It takes no repository because an invocation's own stance is not repository
    /// state; it sits beside [`Self::get_write_policy`] because the two are the
    /// paired inputs of one resolution, and a caller needing one usually needs both.
    ///
    /// `None` means the invocation supplies no override, so
    /// [`LinkedCheckoutWriteStance::resolve`](crate::domain::LinkedCheckoutWriteStance::resolve)
    /// keeps whatever the repository declares. The token is parsed by the domain
    /// type itself, the same parser the `write_policy` TOML key uses, so the two
    /// sources accept the same tokens with the same case handling.
    ///
    /// # Errors
    ///
    /// Returns an error naming the accepted tokens when the variable is set to a
    /// value outside the stance vocabulary. Reading it is a load-time check: an
    /// unusable override is reported rather than silently ignored.
    pub fn invocation_write_override() -> Result<Option<crate::domain::LinkedCheckoutWriteStance>> {
        std::env::var(LINKED_CHECKOUT_WRITE_STANCE_ENV)
            .ok()
            .map(|token| {
                token
                    .parse::<crate::domain::LinkedCheckoutWriteStance>()
                    .map_err(|error| {
                        crate::errors::InvalidArgumentError::new(format!(
                            "invalid {LINKED_CHECKOUT_WRITE_STANCE_ENV}: {error}"
                        ))
                        .into()
                    })
            })
            .transpose()
    }

    /// Get the configured canonical project name.
    ///
    /// Returns the `[project] name` value from `.jit/config.toml`, or `None`
    /// when the `[project]` table (or its `name` key) is absent.
    ///
    /// # Errors
    ///
    /// Returns an error if `config.toml` exists but is malformed, including an
    /// invalid `[project] name` (rejected at parse time by the typed
    /// [`ProjectName`](crate::config::ProjectName) field).
    pub fn get_project_name(&self) -> Result<Option<String>> {
        let config = self.load()?;
        Ok(self.project_name_from_config(&config))
    }

    /// Resolve the configured project name from an ALREADY-loaded config,
    /// without re-reading `config.toml` from disk.
    ///
    /// Callers that already hold a parsed [`JitConfig`] should use this so a
    /// single command parses `config.toml` at most once. Returns `None` when
    /// the `[project]` table or its `name` key is absent.
    pub fn project_name_from_config(&self, config: &JitConfig) -> Option<String> {
        config
            .project
            .as_ref()?
            .name
            .as_ref()
            .map(|name| name.as_str().to_string())
    }

    /// Get resolved icons for the current hierarchy.
    ///
    /// Returns a map of type name to icon string. Icons are resolved using the
    /// hierarchy configuration (levels) and custom per-type icon configuration.
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
            .map(|icons_toml| IconConfig::new(icons_toml.custom.clone()))
            .unwrap_or_default();

        // Resolve icons for all types
        Ok(resolve_icons_for_hierarchy(&types, &icon_config))
    }
}

/// Build the namespace registry from an already-parsed [`JitConfig`], without
/// filesystem access.
///
/// The single pure `config -> LabelNamespaces` conversion shared by the config
/// manager's cached-config read path and the mutation derive pipeline, which
/// derives the default rule family from this same registry. The registry carries
/// exactly what the configuration declares: an absent `[namespaces]` table
/// yields no namespace, and an absent `[type_hierarchy]` table yields no
/// hierarchy.
pub fn namespaces_from_config(config: &JitConfig) -> LabelNamespaces {
    build_namespaces_from_config(config, config.namespaces.clone().unwrap_or_default())
}

fn build_namespaces_from_config(
    config: &JitConfig,
    namespaces_config: HashMap<String, NamespaceConfig>,
) -> LabelNamespaces {
    let namespaces = namespaces_config
        .into_iter()
        // Only the taxonomy (description/unique) crosses into the domain
        // registry; per-namespace constraints (values/pattern/required) were
        // removed when validation became rule-driven.
        .map(|(name, ns_config)| {
            (
                name,
                LabelNamespace::new(ns_config.description, ns_config.unique),
            )
        })
        .collect();

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

/// Load the type taxonomy from the repository's `config.toml`.
///
/// Reads `[type_hierarchy]` and its label associations through a
/// [`ConfigManager`] rooted at the store, and returns them as a
/// [`HierarchyConfig`](crate::domain::type_taxonomy::HierarchyConfig). A
/// repository that declares no `[type_hierarchy]` yields a hierarchy with no
/// types, so every query that resolves tiers resolves none.
///
/// # Errors
///
/// Returns an error if `config.toml` cannot be parsed, or if the declared
/// hierarchy is malformed (an empty type name or a level of 0).
pub fn get_hierarchy_config<S: crate::storage::IssueStore>(
    storage: &S,
) -> Result<crate::domain::type_taxonomy::HierarchyConfig> {
    let namespaces = ConfigManager::new(storage.root()).get_namespaces()?;

    crate::domain::type_taxonomy::HierarchyConfig::new(
        namespaces.declared_type_hierarchy(),
        namespaces.label_associations.unwrap_or_default(),
    )
    .map_err(|e| {
        crate::errors::InvalidArgumentError::new(format!("Invalid hierarchy config: {e}")).into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn test_config(extra: &str) -> String {
        format!(
            "{}\n{extra}",
            crate::test_taxonomy::test_taxonomy().config_fragment()
        )
    }

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

[namespaces.custom]
description = "Custom membership"
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

        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config_toml = taxonomy.config_fragment();
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let namespaces = config_mgr.get_namespaces().unwrap();

        assert_eq!(namespaces.schema_version, 2);
        assert_eq!(namespaces.namespaces.len(), taxonomy.namespaces.len());
        assert!(namespaces.namespaces.contains_key("type"));
        assert!(namespaces
            .namespaces
            .contains_key(taxonomy.type_at_level(1)));
        assert!(namespaces
            .namespaces
            .contains_key(taxonomy.type_at_level(2)));

        // Check strategic types
        assert_eq!(
            namespaces.strategic_types.as_ref().unwrap(),
            &taxonomy.strategic_types
        );
    }

    #[test]
    fn test_get_namespaces_without_declarations_yields_an_empty_registry() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let namespaces = config_mgr.get_namespaces().unwrap();

        // A repository declaring nothing receives nothing.
        assert!(namespaces.namespaces.is_empty());
        assert!(namespaces.type_hierarchy.is_none());
        assert!(namespaces.label_associations.is_none());
        assert!(namespaces.strategic_types.is_none());
    }

    #[test]
    fn test_get_hierarchy_config_without_declared_hierarchy_is_empty() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        let storage = crate::storage::JsonFileStorage::new(&jit_dir);

        let hierarchy = get_hierarchy_config(&storage).unwrap();

        assert_eq!(
            hierarchy,
            crate::domain::type_taxonomy::HierarchyConfig::empty(),
        );
        assert_eq!(hierarchy.types().count(), 0);
    }

    #[test]
    fn test_get_hierarchy_icons_uses_level_defaults_without_icons() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        fs::write(jit_dir.join("config.toml"), test_config("")).unwrap();

        let icons = ConfigManager::new(&jit_dir).get_hierarchy_icons().unwrap();

        let taxonomy = crate::test_taxonomy::test_taxonomy();
        assert_eq!(
            icons.get(taxonomy.type_at_level(1)),
            Some(&"⭐".to_string())
        );
        assert_eq!(
            icons.get(taxonomy.type_at_level(2)),
            Some(&"📦".to_string())
        );
        assert_eq!(
            icons.get(taxonomy.type_at_level(3)),
            Some(&"📝".to_string())
        );
        assert_eq!(
            icons.get(taxonomy.type_at_level(4)),
            Some(&"☑️".to_string())
        );
    }

    #[test]
    fn test_get_hierarchy_icons_preserves_custom_values() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let config_toml = test_config(&format!(
            "[type_hierarchy.icons.custom]\n{} = \"🎯\"\n{} = \"🛠️\"\n",
            taxonomy.type_at_level(1),
            taxonomy.type_at_level(4)
        ));
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let icons = ConfigManager::new(&jit_dir).get_hierarchy_icons().unwrap();

        assert_eq!(
            icons.get(taxonomy.type_at_level(1)),
            Some(&"🎯".to_string())
        );
        assert_eq!(
            icons.get(taxonomy.type_at_level(4)),
            Some(&"🛠️".to_string())
        );
    }

    #[test]
    fn test_namespace_constraint_keys_are_ignored() {
        // Stale per-namespace constraint keys (values/pattern/required) in an old
        // config.toml are IGNORED: the domain registry carries only taxonomy
        // (description/unique). Validation is rule-driven.
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = r#"
[namespaces.type]
description = "Issue type"
unique = true
required = true
values = ["workstream", "defect", "request"]

[namespaces.workstream-group]
description = "Workstream group"
unique = false
pattern = '^v\d+\.\d+$'
"#;
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let namespaces = config_mgr.get_namespaces().unwrap();

        let type_ns = namespaces.namespaces.get("type").unwrap();
        assert_eq!(type_ns.description, "Issue type");
        assert!(type_ns.unique);

        let ms_ns = namespaces.namespaces.get("workstream-group").unwrap();
        assert_eq!(ms_ns.description, "Workstream group");
        assert!(!ms_ns.unique);
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

        // Invalid tokens are now rejected at TOML parse time (via the typed
        // EnforcementMode field), so the error propagates through load().
        assert!(result.is_err());
        // Use the full error chain (anyhow's `{:#}`) to see through the
        // "Failed to parse config.toml" context wrapper.
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("invalid"),
            "error chain must mention invalidity: {msg}"
        );
    }

    #[test]
    fn test_get_write_policy_default_when_missing() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        assert_eq!(
            config_mgr.get_write_policy().unwrap(),
            crate::domain::LinkedCheckoutWriteStance::Refuse
        );
    }

    #[test]
    fn test_get_write_policy_default_when_section_absent() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[type_hierarchy]\ntypes = { task = 1 }\n",
        )
        .unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        assert_eq!(
            config_mgr.get_write_policy().unwrap(),
            crate::domain::LinkedCheckoutWriteStance::Refuse
        );
    }

    #[test]
    fn test_get_write_policy_default_when_key_absent_in_present_section() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nenforce_leases = \"strict\"\n",
        )
        .unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        // This is the same default as an absent [worktree] section; write policy
        // has no section-presence asymmetry like enforce_leases does.
        assert_eq!(
            config_mgr.get_write_policy().unwrap(),
            crate::domain::LinkedCheckoutWriteStance::Refuse
        );
    }

    #[test]
    fn test_get_write_policy_allow_from_config() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nwrite_policy = \"allow\"\n",
        )
        .unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        assert_eq!(
            config_mgr.get_write_policy().unwrap(),
            crate::domain::LinkedCheckoutWriteStance::Allow
        );
    }

    #[test]
    fn test_get_write_policy_invalid() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();
        fs::write(
            jit_dir.join("config.toml"),
            "[worktree]\nwrite_policy = \"invalid\"\n",
        )
        .unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let result = config_mgr.get_write_policy();

        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("invalid"),
            "error chain must mention invalidity: {msg}"
        );
    }

    #[test]
    fn test_get_project_name_default_when_missing() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        assert_eq!(config_mgr.get_project_name().unwrap(), None);
    }

    #[test]
    fn test_get_project_name_from_config() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = "[project]\nname = \"just-in-time\"\n";
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        assert_eq!(
            config_mgr.get_project_name().unwrap(),
            Some("just-in-time".to_string())
        );
    }

    #[test]
    fn test_get_project_name_absent_name_key() {
        // A `[project]` table present but without a `name` key still yields
        // `None`, not an error.
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        fs::write(jit_dir.join("config.toml"), "[project]\n").unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        assert_eq!(config_mgr.get_project_name().unwrap(), None);
    }

    #[test]
    fn test_get_project_name_invalid() {
        let temp_dir = setup_test_dir();
        let jit_dir = temp_dir.path().join(".jit");
        fs::create_dir(&jit_dir).unwrap();

        let config_toml = "[project]\nname = \"Bad_Name\"\n";
        fs::write(jit_dir.join("config.toml"), config_toml).unwrap();

        let config_mgr = ConfigManager::new(&jit_dir);
        let result = config_mgr.get_project_name();

        // Invalid tokens are rejected at TOML parse time (via the typed
        // ProjectName field), so the error propagates through load(), mirroring
        // test_get_enforcement_mode_invalid.
        assert!(result.is_err());
        let msg = format!("{:#}", result.unwrap_err());
        assert!(
            msg.contains("Bad_Name"),
            "error chain must name the offending value: {msg}"
        );
    }
}
