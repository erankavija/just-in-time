//! Type hierarchy templates for different workflows

use std::collections::HashMap;

/// Available hierarchy templates
#[derive(Debug, Clone)]
pub struct HierarchyTemplate {
    pub name: String,
    pub description: String,
    pub hierarchy: HashMap<String, u8>,
    pub label_associations: HashMap<String, String>,
}

impl HierarchyTemplate {
    /// Get all available templates
    pub fn all() -> Vec<HierarchyTemplate> {
        vec![
            Self::default(),
            Self::extended(),
            Self::agile(),
            Self::minimal(),
        ]
    }

    /// Get template by name
    pub fn get(name: &str) -> Option<HierarchyTemplate> {
        Self::all().into_iter().find(|t| t.name == name)
    }

    /// Get strategic types (levels 1-2) from hierarchy
    pub fn get_strategic_types(&self) -> Vec<String> {
        let mut strategic: Vec<_> = self
            .hierarchy
            .iter()
            .filter(|(_, &level)| level <= 2)
            .collect();
        strategic.sort_by_key(|(_, &level)| level);
        strategic
            .into_iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Generate a well-commented config.toml for this template.
    ///
    /// Produces valid TOML with inline types, strategic_types array, and
    /// label_associations — ready to write directly to `.jit/config.toml`.
    pub fn generate_config_toml(&self) -> String {
        // Sort types by level, then alphabetically within the same level
        let mut sorted_types: Vec<_> = self.hierarchy.iter().collect();
        sorted_types.sort_by(|a, b| a.1.cmp(b.1).then(a.0.cmp(b.0)));
        let types_inline = sorted_types
            .iter()
            .map(|(k, v)| format!("{} = {}", k, v))
            .collect::<Vec<_>>()
            .join(", ");

        let strategic = self.get_strategic_types();
        let strategic_array = strategic
            .iter()
            .map(|s| format!("\"{}\"", s))
            .collect::<Vec<_>>()
            .join(", ");

        // Sort label_associations by the level of the key in the hierarchy
        let mut sorted_assoc: Vec<_> = self.label_associations.iter().collect();
        sorted_assoc.sort_by(|a, b| {
            let level_a = self.hierarchy.get(a.0).copied().unwrap_or(u8::MAX);
            let level_b = self.hierarchy.get(b.0).copied().unwrap_or(u8::MAX);
            level_a.cmp(&level_b).then(a.0.cmp(b.0))
        });
        let label_assoc_lines = sorted_assoc
            .iter()
            .map(|(k, v)| format!("{} = \"{}\"", k, v))
            .collect::<Vec<_>>()
            .join("\n");

        format!(
            r#"# JIT Configuration File
#
# Created by `jit init`. Customize as needed.
# All settings are optional — JIT uses sensible defaults without this file.

[version]
schema = 2

[type_hierarchy]
# Issue types and their hierarchy level (lower number = more strategic).
types = {{ {types_inline} }}

# Types shown by `jit query strategic`.
strategic_types = [{strategic_array}]

# Maps each parent type to its membership label namespace.
# E.g. issues belonging to an epic carry an "epic:<name>" label.
[type_hierarchy.label_associations]
{label_assoc_lines}

# Icon preset for the web UI: "simple" | "navigation" | "minimal" | "construction"
# [type_hierarchy.icons]
# preset = "simple"
# custom = {{ bug = "🐛" }}   # per-type overrides (merged with preset)

[validation]
# How strictly JIT enforces rules:
#   strict      — fail on any violation; suitable for CI and automated pipelines.
#   loose       — warn but allow operations; good for everyday development (default).
#   permissive  — minimal checks; useful during imports or migrations.
strictness = "loose"

# Auto-assign this type when creating issues without a type:* label.
default_type = "task"

# Warning toggles (both enabled by default; set to false to silence):
# warn_orphaned_leaves = true       # tasks that carry no parent epic/story label
# warn_strategic_consistency = true # hierarchy inconsistencies across issues

# Label semantics — new repos start strict so convention drift is caught early.
# Set either flag to false if you want looser enforcement during onboarding.
reject_malformed_labels = true      # block labels that violate namespace:value format
enforce_namespace_registry = true   # reject labels whose namespace isn't declared below

# Legacy flag; prefer `required = true` on individual namespaces (see [namespaces.type]).
# require_type_label = false
# label_regex = '^[a-z][a-z0-9-]*:[a-zA-Z0-9][a-zA-Z0-9._-]*$'

# =============================================================================
# NAMESPACE REGISTRY
# =============================================================================
# Declare label namespaces here. Each namespace supports:
#   description — human-readable purpose (required)
#   unique      — at most one label from this namespace per issue (required)
#   examples    — documentation only, not enforced
#   values      — optional allowed-value enum; free-form if omitted
#   pattern     — optional regex over the value portion (checked by `jit validate`)
#   required    — optional; when true every issue must carry a label from this namespace

[namespaces.type]
description = "Issue type (hierarchical). Exactly one per issue."
unique = true
required = true
values = ["epic", "story", "task", "bug", "spike", "chore", "milestone"]
examples = ["type:task", "type:story", "type:epic"]

[namespaces.component]
description = "Technical area or subsystem affected."
unique = false
examples = ["component:backend", "component:frontend", "component:cli"]

[namespaces.priority]
description = "Work priority. Orthogonal to issue priority field; used for filtering."
unique = true
values = ["critical", "high", "normal", "low"]
examples = ["priority:high", "priority:low"]

[namespaces.team]
description = "Owning team."
unique = true
examples = ["team:backend", "team:platform"]

[namespaces.milestone]
description = "Release milestone membership (version tag)."
unique = false
pattern = '^v\d+\.\d+(\.\d+)?(-[a-zA-Z0-9.-]+)?$'
examples = ["milestone:v1.0", "milestone:v1.2.3", "milestone:v2.0-rc1"]

[namespaces.resolution]
description = "Reason for issue closure (used with rejected state)."
unique = true
values = ["wont-fix", "duplicate", "obsolete", "invalid"]
examples = ["resolution:wont-fix", "resolution:duplicate"]

# =============================================================================
# ADVANCED (uncomment to enable)
# =============================================================================

# Worktree isolation and lease enforcement for parallel/agent work.
# [worktree]
# mode = "auto"              # "auto" | "on" | "off"
# enforce_leases = "strict"  # "strict" | "warn" | "off"

# Multi-agent lease coordination.
# [coordination]
# default_ttl_secs = 600              # claim TTL before expiry
# heartbeat_interval_secs = 30        # renewal interval for indefinite leases
# stale_threshold_secs = 3600         # age at which a TTL=0 lease is considered stale
# max_indefinite_leases_per_agent = 2
# max_indefinite_leases_per_repo = 10
# auto_renew_leases = false

# Development document lifecycle (design docs, session notes, etc.).
# [documentation]
# development_root = "dev"
# managed_paths = ["dev/active", "dev/sessions"]
# permanent_paths = ["docs/"]         # never archived
# archive_root = "dev/archive"
# [documentation.categories]          # doc-type → archive subdirectory
# design = "features"
# session = "sessions"

# Branches permitted to modify global config (default: ["main"]).
# [global_operations]
# require_main_history = true
# allowed_branches = ["main"]

# Low-level tuning (rarely needed):
# [locks]
# max_age_secs = 3600       # stale lock file threshold
# [events]
# enable_sequences = true   # sequence numbers in event log
"#,
            types_inline = types_inline,
            strategic_array = strategic_array,
            label_assoc_lines = label_assoc_lines,
        )
    }

    /// Default 4-level hierarchy: milestone → epic → story → task
    ///
    /// Note: This is a factory method for the "default template", not the Default trait.
    /// We intentionally don't implement Default trait because:
    /// - This returns one of several template options (default, extended, agile, minimal)
    /// - Users should consciously choose which template, not rely on Default::default()
    /// - The method name clearly indicates it returns the "default template" option
    #[allow(clippy::should_implement_trait)]
    pub fn default() -> Self {
        let mut hierarchy = HashMap::new();
        hierarchy.insert("milestone".to_string(), 1);
        hierarchy.insert("epic".to_string(), 2);
        hierarchy.insert("story".to_string(), 3);
        hierarchy.insert("task".to_string(), 4);

        let mut label_associations = HashMap::new();
        label_associations.insert("milestone".to_string(), "milestone".to_string());
        label_associations.insert("epic".to_string(), "epic".to_string());
        label_associations.insert("story".to_string(), "story".to_string());

        Self {
            name: "default".to_string(),
            description: "4-level hierarchy: milestone → epic → story → task".to_string(),
            hierarchy,
            label_associations,
        }
    }

    /// Extended 5-level hierarchy: program → milestone → epic → story → task
    pub fn extended() -> Self {
        let mut hierarchy = HashMap::new();
        hierarchy.insert("program".to_string(), 1);
        hierarchy.insert("milestone".to_string(), 2);
        hierarchy.insert("epic".to_string(), 3);
        hierarchy.insert("story".to_string(), 4);
        hierarchy.insert("task".to_string(), 5);

        let mut label_associations = HashMap::new();
        label_associations.insert("program".to_string(), "program".to_string());
        label_associations.insert("milestone".to_string(), "milestone".to_string());
        label_associations.insert("epic".to_string(), "epic".to_string());
        label_associations.insert("story".to_string(), "story".to_string());

        Self {
            name: "extended".to_string(),
            description: "5-level hierarchy: program → milestone → epic → story → task".to_string(),
            hierarchy,
            label_associations,
        }
    }

    /// Agile-focused 4-level hierarchy: release → epic → story → task
    pub fn agile() -> Self {
        let mut hierarchy = HashMap::new();
        hierarchy.insert("release".to_string(), 1);
        hierarchy.insert("epic".to_string(), 2);
        hierarchy.insert("story".to_string(), 3);
        hierarchy.insert("task".to_string(), 4);

        let mut label_associations = HashMap::new();
        label_associations.insert("release".to_string(), "release".to_string());
        label_associations.insert("epic".to_string(), "epic".to_string());
        label_associations.insert("story".to_string(), "story".to_string());

        Self {
            name: "agile".to_string(),
            description: "4-level hierarchy: release → epic → story → task".to_string(),
            hierarchy,
            label_associations,
        }
    }

    /// Minimal 2-level hierarchy: milestone → task
    pub fn minimal() -> Self {
        let mut hierarchy = HashMap::new();
        hierarchy.insert("milestone".to_string(), 1);
        hierarchy.insert("task".to_string(), 2);

        let mut label_associations = HashMap::new();
        label_associations.insert("milestone".to_string(), "milestone".to_string());

        Self {
            name: "minimal".to_string(),
            description: "2-level hierarchy: milestone → task".to_string(),
            hierarchy,
            label_associations,
        }
    }
}

/// Load hierarchy configuration from storage.
///
/// Reads the type_hierarchy and label_associations from config.toml
/// or returns the default config.
pub fn get_hierarchy_config<S: crate::storage::IssueStore>(
    storage: &S,
) -> anyhow::Result<crate::type_hierarchy::HierarchyConfig> {
    use crate::config_manager::ConfigManager;
    let config_mgr = ConfigManager::new(storage.root());
    let namespaces = config_mgr.get_namespaces()?;

    if let Some(type_hierarchy) = namespaces.type_hierarchy {
        // Load label_associations or use empty map
        let label_associations = namespaces.label_associations.unwrap_or_default();

        // Convert to HierarchyConfig
        crate::type_hierarchy::HierarchyConfig::new(type_hierarchy, label_associations)
            .map_err(|e| anyhow::anyhow!("Invalid hierarchy config: {}", e))
    } else {
        // Return default config
        Ok(crate::type_hierarchy::HierarchyConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_all_templates() {
        let templates = HierarchyTemplate::all();
        assert_eq!(templates.len(), 4);

        let names: Vec<_> = templates.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"default"));
        assert!(names.contains(&"extended"));
        assert!(names.contains(&"agile"));
        assert!(names.contains(&"minimal"));
    }

    #[test]
    fn test_get_template() {
        let default = HierarchyTemplate::get("default").unwrap();
        assert_eq!(default.hierarchy.len(), 4);
        assert_eq!(default.hierarchy.get("milestone"), Some(&1));
        assert_eq!(default.hierarchy.get("task"), Some(&4));
    }

    #[test]
    fn test_extended_template() {
        let extended = HierarchyTemplate::extended();
        assert_eq!(extended.hierarchy.len(), 5);
        assert_eq!(extended.hierarchy.get("program"), Some(&1));
        assert_eq!(extended.hierarchy.get("task"), Some(&5));
    }

    #[test]
    fn test_minimal_template() {
        let minimal = HierarchyTemplate::minimal();
        assert_eq!(minimal.hierarchy.len(), 2);
        assert_eq!(minimal.hierarchy.get("milestone"), Some(&1));
        assert_eq!(minimal.hierarchy.get("task"), Some(&2));
    }

    #[test]
    fn test_get_nonexistent_template() {
        assert!(HierarchyTemplate::get("nonexistent").is_none());
    }

    #[test]
    fn test_generated_config_has_strict_label_defaults() {
        let toml = HierarchyTemplate::default().generate_config_toml();
        assert!(toml.contains("reject_malformed_labels = true"));
        assert!(toml.contains("enforce_namespace_registry = true"));
    }

    #[test]
    fn test_generated_config_seeds_namespace_constraints() {
        let toml = HierarchyTemplate::default().generate_config_toml();
        // type namespace declares enum + required
        assert!(toml.contains("[namespaces.type]"));
        assert!(toml.contains("required = true"));
        assert!(toml.contains(
            r#"values = ["epic", "story", "task", "bug", "spike", "chore", "milestone"]"#
        ));
        // milestone declares a version pattern
        assert!(toml.contains("[namespaces.milestone]"));
        assert!(toml.contains(r"pattern = '^v\d+\.\d+(\.\d+)?"));
    }

    #[test]
    fn test_generated_config_parses_as_valid_toml() {
        let toml = HierarchyTemplate::default().generate_config_toml();
        let cfg: crate::config::JitConfig =
            ::toml::from_str(&toml).expect("generated template must parse");
        let ns = cfg.namespaces.expect("namespaces block present");
        let type_ns = ns.get("type").expect("type namespace present");
        assert_eq!(type_ns.required, Some(true));
        assert!(type_ns.values.is_some());
    }
}
