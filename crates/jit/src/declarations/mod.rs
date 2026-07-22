//! Filesystem-free authored repository declarations.
//!
//! This module is the neutral owner of gate/rule semantics and configuration
//! parsing. Persistence and validation consume these declarations; neither owns
//! an alternate representation.

mod gates;
pub mod invariants;
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
/// The plan-enumerated components are exposed directly. The complete parsed
/// configuration is retained privately so materialization consumers that still
/// need several components can derive their view from this one parse instead of
/// reparsing captured bytes into a competing authority.
#[derive(Debug, Clone)]
pub struct ConfigurationDeclarations {
    parsed: JitConfig,
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

impl ConfigurationDeclarations {
    /// Authored project identity from this authoritative parse.
    pub fn project_name(&self) -> Option<&crate::config::ProjectName> {
        self.parsed.project.as_ref()?.name.as_ref()
    }

    /// Build the projection/default-rule configuration view from this parse.
    ///
    /// `invariants.toml` is a sibling declaration, so its registry is supplied by
    /// the captured-image assembler rather than parsed from `config.toml`.
    pub(crate) fn materialization_config(
        &self,
        invariants: invariants::InvariantRegistry,
    ) -> JitConfig {
        let mut config = self.parsed.clone();
        config.invariants = invariants;
        config
    }
}

/// Parse captured `config.toml` bytes without filesystem access or validation.
pub fn parse_configuration(
    bytes: &[u8],
) -> Result<ConfigurationDeclarations, ConfigurationDeclarationError> {
    let text = std::str::from_utf8(bytes)?;
    let config: JitConfig = toml::from_str(text)?;
    Ok(ConfigurationDeclarations {
        parsed: config.clone(),
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

    #[test]
    fn test_materialization_config_comes_from_the_same_authoritative_parse() {
        let parsed = parse_configuration(
            br#"
[version]
schema = 7
[validation]
content_format = "html"
[projection.reference]
kind = "definition"
target = "docs/reference.md"
"#,
        )
        .unwrap();

        let config = parsed.materialization_config(invariants::InvariantRegistry::empty());
        assert_eq!(config.version.unwrap().schema, 7);
        assert_eq!(
            config.validation.unwrap().content_format.as_deref(),
            Some("html")
        );
        assert_eq!(
            config.projection.unwrap()["reference"].target.as_deref(),
            Some("docs/reference.md")
        );
    }
}
