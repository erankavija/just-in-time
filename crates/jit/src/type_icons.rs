//! Type hierarchy icon configuration and resolution.
//!
//! Provides domain-agnostic icon support for issue types based on hierarchy levels.
//!
//! # Design Principles
//!
//! - Icons assigned by hierarchy LEVEL, not type name (domain-agnostic)
//! - Full flexibility through custom type name → icon mapping
//! - Partial overrides (only specify what changes)
//! - Fallback chain: custom → level default → no icon
//!
//! # Examples
//!
//! ```
//! use jit::type_icons::{IconConfig, get_icon_for_type};
//!
//! let config = IconConfig::default();
//! assert_eq!(get_icon_for_type("epic", 2, &config), Some("📦".to_string()));
//! assert_eq!(get_icon_for_type("task", 4, &config), Some("☑️".to_string()));
//! ```

use std::collections::HashMap;

/// Default icons by hierarchy level (domain-agnostic).
const DEFAULT_ICONS_BY_LEVEL: &[(u8, &str)] = &[
    (1, "⭐"), // Level 1: Strategic/goal
    (2, "📦"), // Level 2: Container/grouping
    (3, "📝"), // Level 3: Work unit
    (4, "☑️"), // Level 4+: Atomic action
];

/// Fallback icon for levels >= 4.
const LEAF_ICON: &str = "☑️";

/// Icon configuration.
#[derive(Debug, Clone, Default)]
pub struct IconConfig {
    /// Custom type name to icon mapping (optional).
    pub custom: Option<HashMap<String, String>>,
}

impl IconConfig {
    /// Creates a new icon configuration.
    pub fn new(custom: Option<HashMap<String, String>>) -> Self {
        Self { custom }
    }
}

/// Resolves the icon for a given type name and hierarchy level.
///
/// # Resolution Priority
///
/// 1. Custom type mapping (highest priority)
/// 2. Default level mapping
/// 3. Leaf icon for levels >= 4
/// 4. No icon (None)
///
/// # Arguments
///
/// * `type_name` - The issue type name (e.g., "epic", "task", "bug")
/// * `level` - The hierarchy level (1 = highest, higher numbers = lower)
/// * `config` - Icon configuration
pub fn get_icon_for_type(type_name: &str, level: u8, config: &IconConfig) -> Option<String> {
    // 1. Check custom type mapping (highest priority)
    if let Some(custom_icons) = &config.custom {
        if let Some(icon) = custom_icons.get(type_name) {
            return Some(icon.clone());
        }
    }

    // 2. Fall back to default level mapping
    if let Some((_, icon)) = DEFAULT_ICONS_BY_LEVEL.iter().find(|(lvl, _)| *lvl == level) {
        return Some(icon.to_string());
    }

    // 3. Fall back to leaf icon for levels >= 4
    if level >= 4 {
        return Some(LEAF_ICON.to_string());
    }

    // 4. No icon
    None
}

/// Resolves icons for all types in a hierarchy.
///
/// Returns a map of type name to icon string.
///
/// # Arguments
///
/// * `types` - Map of type name to hierarchy level
/// * `config` - Icon configuration
pub fn resolve_icons_for_hierarchy(
    types: &HashMap<String, u8>,
    config: &IconConfig,
) -> HashMap<String, String> {
    types
        .iter()
        .filter_map(|(type_name, level)| {
            get_icon_for_type(type_name, *level, config).map(|icon| (type_name.clone(), icon))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_type(level: u8) -> String {
        crate::test_taxonomy::test_taxonomy()
            .type_at_level(level)
            .to_string()
    }

    #[test]
    fn test_get_icon_default_level_mapping() {
        let config = IconConfig::default();

        assert_eq!(
            get_icon_for_type(&test_type(1), 1, &config),
            Some("⭐".to_string())
        );
        assert_eq!(
            get_icon_for_type(&test_type(2), 2, &config),
            Some("📦".to_string())
        );
        assert_eq!(
            get_icon_for_type(&test_type(3), 3, &config),
            Some("📝".to_string())
        );
        assert_eq!(
            get_icon_for_type(&test_type(4), 4, &config),
            Some("☑️".to_string())
        );
    }

    #[test]
    fn test_get_icon_leaf_fallback() {
        let config = IconConfig::default();

        // Levels >= 4 should get leaf icon
        assert_eq!(
            get_icon_for_type("subtask", 5, &config),
            Some("☑️".to_string())
        );
        assert_eq!(
            get_icon_for_type(&test_type(4), 10, &config),
            Some("☑️".to_string())
        );
    }

    #[test]
    fn test_get_icon_from_custom() {
        let mut custom = HashMap::new();
        custom.insert(test_type(2), "🚀".to_string());
        custom.insert("bug".to_string(), "🐛".to_string());

        let config = IconConfig::new(Some(custom));

        // Custom overrides
        assert_eq!(
            get_icon_for_type(&test_type(2), 2, &config),
            Some("🚀".to_string())
        );
        assert_eq!(get_icon_for_type("bug", 4, &config), Some("🐛".to_string()));

        // Falls back to default for non-custom types
        assert_eq!(
            get_icon_for_type(&test_type(4), 4, &config),
            Some("☑️".to_string())
        );
    }

    #[test]
    fn test_get_icon_partial_override() {
        let mut custom = HashMap::new();
        custom.insert("bug".to_string(), "🐛".to_string());

        let config = IconConfig::new(Some(custom));

        // Custom override wins
        assert_eq!(get_icon_for_type("bug", 4, &config), Some("🐛".to_string()));

        // Level default used for others
        assert_eq!(
            get_icon_for_type(&test_type(2), 2, &config),
            Some("📦".to_string())
        );
        assert_eq!(
            get_icon_for_type(&test_type(4), 4, &config),
            Some("☑️".to_string())
        );
    }

    #[test]
    fn test_resolve_icons_for_hierarchy() {
        let types = crate::test_taxonomy::test_taxonomy().hierarchy;

        let config = IconConfig::default();
        let icons = resolve_icons_for_hierarchy(&types, &config);

        assert_eq!(icons.len(), 4);
        assert_eq!(icons.get(&test_type(1)), Some(&"⭐".to_string()));
        assert_eq!(icons.get(&test_type(2)), Some(&"📦".to_string()));
        assert_eq!(icons.get(&test_type(3)), Some(&"📝".to_string()));
        assert_eq!(icons.get(&test_type(4)), Some(&"☑️".to_string()));
    }

    #[test]
    fn test_resolve_icons_with_custom_names() {
        let types = crate::test_taxonomy::test_taxonomy().hierarchy;

        let config = IconConfig::default();
        let icons = resolve_icons_for_hierarchy(&types, &config);

        // Icons assigned by level, not name
        assert_eq!(icons.get(&test_type(1)), Some(&"⭐".to_string()));
        assert_eq!(icons.get(&test_type(2)), Some(&"📦".to_string()));
        assert_eq!(icons.get(&test_type(3)), Some(&"📝".to_string()));
        assert_eq!(icons.get(&test_type(4)), Some(&"☑️".to_string()));
    }
}
