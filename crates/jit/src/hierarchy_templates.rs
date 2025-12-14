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
/// Reads the type_hierarchy and label_associations from .jit/labels.json
/// or returns the default config.
pub fn get_hierarchy_config<S: crate::storage::IssueStore>(
    storage: &S,
) -> anyhow::Result<crate::type_hierarchy::HierarchyConfig> {
    let namespaces = storage.load_label_namespaces()?;

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
