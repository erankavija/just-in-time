//! Type hierarchy icon configuration and resolution.
//!
//! Provides domain-agnostic icon support for issue types based on hierarchy levels.
//!
//! # Design Principles
//!
//! - Icons assigned by hierarchy LEVEL, not type name (domain-agnostic)
//! - Full flexibility through custom type name → icon mapping
//! - Partial overrides (only specify what changes)
//! - Fallback chain: custom → preset → level default → no icon
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

/// Icon preset definitions.
const PRESETS: &[(&str, &[(u8, &str)])] = &[
    ("simple", &[
        (1, "⭐"),
        (2, "📦"),
        (3, "📝"),
        (4, "☑️"),
    ]),
    ("navigation", &[
        (1, "🏔️"),
        (2, "🗺️"),
        (3, "🧭"),
        (4, "📍"),
    ]),
    ("minimal", &[
        (1, "◆"),
        (2, "▣"),
        (3, "▢"),
        (4, "□"),
    ]),
    ("construction", &[
        (1, "🏁"),
        (2, "🏗️"),
        (3, "🧱"),
        (4, "🔨"),
    ]),
];

/// Default icons by hierarchy level (domain-agnostic).
const DEFAULT_ICONS_BY_LEVEL: &[(u8, &str)] = &[
    (1, "⭐"),  // Level 1: Strategic/goal
    (2, "📦"),  // Level 2: Container/grouping
    (3, "📝"),  // Level 3: Work unit
    (4, "☑️"),  // Level 4+: Atomic action
];

/// Fallback icon for levels >= 4.
const LEAF_ICON: &str = "☑️";

/// Icon configuration.
#[derive(Debug, Clone, Default)]
pub struct IconConfig {
    /// Icon preset name (optional).
    pub preset: Option<String>,
    /// Custom type name to icon mapping (optional).
    pub custom: Option<HashMap<String, String>>,
}

impl IconConfig {
    /// Creates a new icon configuration.
    pub fn new(preset: Option<String>, custom: Option<HashMap<String, String>>) -> Self {
        Self { preset, custom }
    }
}

/// Resolves the icon for a given type name and hierarchy level.
///
/// # Resolution Priority
///
/// 1. Custom type mapping (highest priority)
/// 2. Preset for that level
/// 3. Default level mapping
/// 4. Leaf icon for levels >= 4
/// 5. No icon (None)
///
/// # Arguments
///
/// * `type_name` - The issue type name (e.g., "epic", "task", "bug")
/// * `level` - The hierarchy level (1 = highest, higher numbers = lower)
/// * `config` - Icon configuration
///
/// # Examples
///
/// ```
/// use jit::type_icons::{IconConfig, get_icon_for_type};
///
/// let config = IconConfig::default();
/// assert_eq!(get_icon_for_type("epic", 2, &config), Some("📦".to_string()));
/// ```
pub fn get_icon_for_type(type_name: &str, level: u8, config: &IconConfig) -> Option<String> {
    // 1. Check custom type mapping (highest priority)
    if let Some(custom_icons) = &config.custom {
        if let Some(icon) = custom_icons.get(type_name) {
            return Some(icon.clone());
        }
    }

    // 2. Check preset for this level
    if let Some(preset_name) = &config.preset {
        if let Some(preset) = PRESETS.iter().find(|(name, _)| name == preset_name) {
            if let Some((_, icon)) = preset.1.iter().find(|(lvl, _)| *lvl == level) {
                return Some(icon.to_string());
            }
        }
    }

    // 3. Fall back to default level mapping
    if let Some((_, icon)) = DEFAULT_ICONS_BY_LEVEL.iter().find(|(lvl, _)| *lvl == level) {
        return Some(icon.to_string());
    }

    // 4. Fall back to leaf icon for levels >= 4
    if level >= 4 {
        return Some(LEAF_ICON.to_string());
    }

    // 5. No icon
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
///
/// # Examples
///
/// ```
/// use jit::type_icons::{IconConfig, resolve_icons_for_hierarchy};
/// use std::collections::HashMap;
///
/// let mut types = HashMap::new();
/// types.insert("epic".to_string(), 2);
/// types.insert("task".to_string(), 4);
///
/// let config = IconConfig::default();
/// let icons = resolve_icons_for_hierarchy(&types, &config);
///
/// assert_eq!(icons.get("epic"), Some(&"📦".to_string()));
/// assert_eq!(icons.get("task"), Some(&"☑️".to_string()));
/// ```
pub fn resolve_icons_for_hierarchy(
    types: &HashMap<String, u8>,
    config: &IconConfig,
) -> HashMap<String, String> {
    types
        .iter()
        .filter_map(|(type_name, level)| {
            get_icon_for_type(type_name, *level, config)
                .map(|icon| (type_name.clone(), icon))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_icon_default_level_mapping() {
        let config = IconConfig::default();

        assert_eq!(get_icon_for_type("milestone", 1, &config), Some("⭐".to_string()));
        assert_eq!(get_icon_for_type("epic", 2, &config), Some("📦".to_string()));
        assert_eq!(get_icon_for_type("story", 3, &config), Some("📝".to_string()));
        assert_eq!(get_icon_for_type("task", 4, &config), Some("☑️".to_string()));
    }

    #[test]
    fn test_get_icon_leaf_fallback() {
        let config = IconConfig::default();

        // Levels >= 4 should get leaf icon
        assert_eq!(get_icon_for_type("subtask", 5, &config), Some("☑️".to_string()));
        assert_eq!(get_icon_for_type("action", 10, &config), Some("☑️".to_string()));
    }

    #[test]
    fn test_get_icon_from_preset() {
        let config = IconConfig::new(Some("navigation".to_string()), None);

        assert_eq!(get_icon_for_type("milestone", 1, &config), Some("🏔️".to_string()));
        assert_eq!(get_icon_for_type("epic", 2, &config), Some("🗺️".to_string()));
        assert_eq!(get_icon_for_type("story", 3, &config), Some("🧭".to_string()));
        assert_eq!(get_icon_for_type("task", 4, &config), Some("📍".to_string()));
    }

    #[test]
    fn test_get_icon_from_custom() {
        let mut custom = HashMap::new();
        custom.insert("epic".to_string(), "🚀".to_string());
        custom.insert("bug".to_string(), "🐛".to_string());

        let config = IconConfig::new(None, Some(custom));

        // Custom overrides
        assert_eq!(get_icon_for_type("epic", 2, &config), Some("🚀".to_string()));
        assert_eq!(get_icon_for_type("bug", 4, &config), Some("🐛".to_string()));

        // Falls back to default for non-custom types
        assert_eq!(get_icon_for_type("task", 4, &config), Some("☑️".to_string()));
    }

    #[test]
    fn test_get_icon_partial_override() {
        let mut custom = HashMap::new();
        custom.insert("bug".to_string(), "🐛".to_string());

        let config = IconConfig::new(Some("navigation".to_string()), Some(custom));

        // Custom override wins
        assert_eq!(get_icon_for_type("bug", 4, &config), Some("🐛".to_string()));

        // Preset used for others
        assert_eq!(get_icon_for_type("epic", 2, &config), Some("🗺️".to_string()));
        assert_eq!(get_icon_for_type("task", 4, &config), Some("📍".to_string()));
    }

    #[test]
    fn test_resolve_icons_for_hierarchy() {
        let mut types = HashMap::new();
        types.insert("milestone".to_string(), 1);
        types.insert("epic".to_string(), 2);
        types.insert("story".to_string(), 3);
        types.insert("task".to_string(), 4);

        let config = IconConfig::default();
        let icons = resolve_icons_for_hierarchy(&types, &config);

        assert_eq!(icons.len(), 4);
        assert_eq!(icons.get("milestone"), Some(&"⭐".to_string()));
        assert_eq!(icons.get("epic"), Some(&"📦".to_string()));
        assert_eq!(icons.get("story"), Some(&"📝".to_string()));
        assert_eq!(icons.get("task"), Some(&"☑️".to_string()));
    }

    #[test]
    fn test_resolve_icons_with_custom_names() {
        let mut types = HashMap::new();
        types.insert("objective".to_string(), 1);
        types.insert("initiative".to_string(), 2);
        types.insert("feature".to_string(), 3);
        types.insert("action".to_string(), 4);

        let config = IconConfig::default();
        let icons = resolve_icons_for_hierarchy(&types, &config);

        // Icons assigned by level, not name
        assert_eq!(icons.get("objective"), Some(&"⭐".to_string()));
        assert_eq!(icons.get("initiative"), Some(&"📦".to_string()));
        assert_eq!(icons.get("feature"), Some(&"📝".to_string()));
        assert_eq!(icons.get("action"), Some(&"☑️".to_string()));
    }

    #[test]
    fn test_all_presets_defined() {
        let presets = vec!["simple", "navigation", "minimal", "construction"];

        for preset_name in presets {
            let config = IconConfig::new(Some(preset_name.to_string()), None);

            // Each preset should have icons for levels 1-4
            assert!(get_icon_for_type("level1", 1, &config).is_some());
            assert!(get_icon_for_type("level2", 2, &config).is_some());
            assert!(get_icon_for_type("level3", 3, &config).is_some());
            assert!(get_icon_for_type("level4", 4, &config).is_some());
        }
    }
}
