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
        strategic.into_iter().map(|(name, _)| name.clone()).collect()
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

[validation]
# Strictness level: "strict" | "loose" | "permissive"
strictness = "loose"

# Auto-assign this type when creating issues without a type:* label.
default_type = "task"

# Require exactly one type:* label per issue.
require_type_label = false

# =============================================================================
# NAMESPACE REGISTRY (optional)
# =============================================================================
# Define label namespaces for documentation and optional enforcement.

[namespaces.type]
description = "Issue type (hierarchical). Exactly one per issue."
unique = true
examples = ["type:task", "type:story", "type:epic", "type:milestone"]

[namespaces.component]
description = "Technical area or subsystem affected."
unique = false
examples = ["component:backend", "component:frontend", "component:cli"]

[namespaces.resolution]
description = "Reason for issue closure (used with rejected state)."
unique = true
examples = ["resolution:wont-fix", "resolution:duplicate", "resolution:obsolete"]
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
}
