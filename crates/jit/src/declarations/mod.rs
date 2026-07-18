//! Filesystem-free authored repository declarations.
//!
//! This module is the neutral owner of gate/rule semantics and configuration
//! parsing. Persistence and validation consume these declarations; neither owns
//! an alternate representation.

mod gates;
pub mod rules;

pub use gates::{
    parse_gate_registry, serialize_gate_registry, GateChecker, GateDeclarationError,
    GateDefinition, GateMode, GateModeParseError, GateRegistry, GateStage, GateStageParseError,
    REVIEW_PLACEHOLDER_WARNING,
};

use crate::config::{
    DocumentationConfig, HierarchyConfigToml, ItemKindConfig, JitConfig, NamespaceConfig,
    ProjectionConfig,
};
use std::collections::{BTreeMap, HashMap};

/// Configuration components used by capture and materialization.
///
/// Exactly the plan-enumerated narrowed components — hierarchy, namespaces, item
/// kinds, projections, documentation roots — and no whole-`JitConfig` field:
/// every consumer takes the one component it needs.
#[derive(Debug, Clone)]
pub struct ConfigurationDeclarations {
    /// Type hierarchy declaration.
    pub hierarchy: Option<HierarchyConfigToml>,
    /// Label namespace declarations.
    pub namespaces: HashMap<String, NamespaceConfig>,
    /// Addressable item-kind declarations.
    pub item_kinds: HashMap<String, ItemKindConfig>,
    /// Configured projection declarations.
    pub projections: BTreeMap<String, ProjectionConfig>,
    /// Documentation lifecycle roots.
    pub documentation: Option<DocumentationConfig>,
}

/// Parse captured `config.toml` bytes without filesystem access or validation.
pub fn parse_configuration(
    bytes: &[u8],
) -> Result<ConfigurationDeclarations, ConfigurationDeclarationError> {
    let text = std::str::from_utf8(bytes)?;
    let config: JitConfig = toml::from_str(text)?;
    Ok(ConfigurationDeclarations {
        hierarchy: config.type_hierarchy.clone(),
        namespaces: config.namespaces.clone().unwrap_or_default(),
        item_kinds: config.item_kinds.clone().unwrap_or_default(),
        projections: config.projection.clone().unwrap_or_default(),
        documentation: config.documentation.clone(),
    })
}

/// Invalid captured configuration declaration.
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationDeclarationError {
    #[error("config.toml is not UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("config.toml is invalid: {0}")]
    Toml(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_configuration_is_filesystem_free_and_extracts_materialization_components() {
        let parsed = parse_configuration(
            br#"
[type_hierarchy]
types = { epic = 2, task = 4 }
[namespaces.component]
description = "Component"
unique = false
[documentation]
development_root = "development"
[projection.reference]
kind = "definition"
target = "docs/reference.md"
"#,
        )
        .unwrap();
        assert_eq!(parsed.hierarchy.unwrap().types["epic"], 2);
        assert!(parsed.namespaces.contains_key("component"));
        assert_eq!(
            parsed.documentation.unwrap().development_root(),
            "development"
        );
        assert_eq!(
            parsed.projections["reference"].target.as_deref(),
            Some("docs/reference.md")
        );
    }
}
