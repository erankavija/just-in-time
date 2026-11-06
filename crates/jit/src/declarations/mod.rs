//! Filesystem-free authored repository declarations.
//!
//! This module is the neutral owner of gate/rule semantics and configuration
//! assembly. Persistence and validation consume these declarations; neither owns
//! an alternate representation.

mod gates;
pub mod invariants;
pub mod rules;

pub(crate) use gates::normalize_gate_registry_for_document;
pub use gates::{
    edit_gate_registry_document, parse_gate_registry, serialize_gate_registry, GateChecker,
    GateDeclarationError, GateDefinition, GateMode, GateRegistry, GateStage,
    REVIEW_PLACEHOLDER_WARNING,
};

use crate::config::{ItemKindConfigError, JitConfig};
use crate::declarations::invariants::{InvariantConfigError, InvariantRegistry};
use crate::templates::{TemplateConfigError, TemplateRegistry};

/// Assemble the repository configuration from the bytes of `config.toml`,
/// `templates.toml` and `invariants.toml`, each `None` when absent.
///
/// An absent `config.toml` is the empty configuration. Template node types are
/// checked against the declared `[type_hierarchy].types`, and `[item_kinds]` is
/// validated; absent templates or invariants are empty registries.
pub fn assemble_configuration(
    config: Option<&[u8]>,
    templates: Option<&[u8]>,
    invariants: Option<&[u8]>,
) -> Result<JitConfig, ConfigurationAssemblyError> {
    let mut assembled = match config {
        Some(bytes) => toml::from_str::<JitConfig>(
            std::str::from_utf8(bytes).map_err(ConfigurationAssemblyError::ConfigUtf8)?,
        )?,
        None => JitConfig::default(),
    };
    assembled.validate_item_kinds()?;
    let hierarchy_types: Vec<&str> = assembled
        .type_hierarchy
        .as_ref()
        .map(|hierarchy| hierarchy.types.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let templates = templates
        .map(|bytes| {
            let text =
                std::str::from_utf8(bytes).map_err(ConfigurationAssemblyError::TemplatesUtf8)?;
            Ok::<_, ConfigurationAssemblyError>(TemplateRegistry::from_toml_str(
                text,
                &hierarchy_types,
            )?)
        })
        .transpose()?
        .unwrap_or_default();
    let invariants = invariants
        .map(|bytes| {
            let text =
                std::str::from_utf8(bytes).map_err(ConfigurationAssemblyError::InvariantsUtf8)?;
            Ok::<_, ConfigurationAssemblyError>(InvariantRegistry::from_toml_str(text)?)
        })
        .transpose()?
        .unwrap_or_default();
    assembled.templates = templates;
    assembled.invariants = invariants;
    Ok(assembled)
}

/// A configuration source that failed assembly, named by its file.
#[derive(Debug, thiserror::Error)]
pub enum ConfigurationAssemblyError {
    #[error("config.toml is not UTF-8: {0}")]
    ConfigUtf8(#[source] std::str::Utf8Error),
    /// Displays the one-line TOML message; the source keeps the full report.
    #[error("Failed to parse config.toml: {}", .0.message())]
    Config(#[from] toml::de::Error),
    #[error("invalid [item_kinds] in config.toml: {0}")]
    ItemKinds(#[from] ItemKindConfigError),
    #[error("templates.toml is not UTF-8: {0}")]
    TemplatesUtf8(#[source] std::str::Utf8Error),
    #[error("invalid templates.toml: {0}")]
    Templates(#[from] TemplateConfigError),
    #[error("invariants.toml is not UTF-8: {0}")]
    InvariantsUtf8(#[source] std::str::Utf8Error),
    #[error("invalid invariants.toml: {0}")]
    Invariants(#[from] InvariantConfigError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_assemble_configuration_absent_config_is_empty_configuration() {
        let config = assemble_configuration(
            None,
            Some(b"[[template]]\nname = \"t\"\napplies_to = [\"any\"]\n"),
            Some(b"[[invariants]]\nid = \"i\"\nstatement = \"s\"\nkind = \"advisory\"\n"),
        )
        .unwrap();
        assert!(config.type_hierarchy.is_none());
        assert!(config.item_kinds.is_none());
        assert!(config.templates.get("t").is_some());
        assert_eq!(config.invariants.invariants.len(), 1);
    }

    #[test]
    fn test_assemble_configuration_absent_templates_and_invariants_are_empty_registries() {
        let config = assemble_configuration(
            Some(b"[type_hierarchy]\ntypes = { task = 1 }\n"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(config.templates, TemplateRegistry::empty());
        assert_eq!(config.invariants, InvariantRegistry::empty());
    }

    #[test]
    fn test_assemble_configuration_rejects_template_container_type_outside_declared_hierarchy() {
        let error = assemble_configuration(
            Some(b"[type_hierarchy]\ntypes = { epic = 1, task = 2 }\n"),
            Some(b"[[template]]\nname = \"t\"\napplies_to = [\"undeclared\"]\n"),
            None,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ConfigurationAssemblyError::Templates(TemplateConfigError::UnknownAppliesToType { .. })
        ));
    }

    #[test]
    fn test_assemble_configuration_rejects_malformed_item_kinds() {
        let error = assemble_configuration(
            Some(b"[item_kinds.custom]\nsection = \"Items\"\n"),
            None,
            None,
        )
        .unwrap_err();
        assert!(matches!(
            error,
            ConfigurationAssemblyError::ItemKinds(ItemKindConfigError::MissingFields { .. })
        ));
    }

    #[test]
    fn test_assemble_configuration_rejects_unknown_config_key() {
        let error = assemble_configuration(Some(b"undeclared_key = 1\n"), None, None).unwrap_err();
        assert!(matches!(error, ConfigurationAssemblyError::Config(_)));
    }
}
