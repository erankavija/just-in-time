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

use crate::config::{ItemKindConfigError, JitConfig, WorktreeConfig};
use crate::declarations::invariants::{InvariantConfigError, InvariantRegistry};
use crate::templates::{TemplateConfigError, TemplateRegistry};

/// One syntax document, with full and consumed-field views sharing its parse.
#[derive(Clone)]
pub(crate) struct ConfigurationDocument {
    syntax: Result<toml::Value, ConfigurationSyntaxError>,
}

#[derive(Clone)]
enum ConfigurationSyntaxError {
    Utf8(std::str::Utf8Error),
    Toml(toml::de::Error),
}

impl ConfigurationDocument {
    pub(crate) fn parse(bytes: Option<&[u8]>) -> Self {
        let syntax = bytes.map_or_else(
            || Ok(toml::Value::Table(toml::Table::new())),
            |bytes| {
                let text = std::str::from_utf8(bytes).map_err(ConfigurationSyntaxError::Utf8)?;
                toml::from_str(text).map_err(ConfigurationSyntaxError::Toml)
            },
        );
        Self { syntax }
    }

    fn value(&self) -> Result<&toml::Value, ConfigurationAssemblyError> {
        self.syntax.as_ref().map_err(|error| match error {
            ConfigurationSyntaxError::Utf8(error) => ConfigurationAssemblyError::ConfigUtf8(*error),
            ConfigurationSyntaxError::Toml(error) => {
                ConfigurationAssemblyError::Config(error.clone())
            }
        })
    }

    /// Deserialize only fields needed for this invocation's root/policy authority.
    pub(crate) fn worktree(
        &self,
        needs_root: bool,
        needs_policy: bool,
    ) -> Result<WorktreeConfig, ConfigurationAssemblyError> {
        let mut selected = toml::Table::new();
        if needs_root || needs_policy {
            if let Some(worktree) = self.value()?.get("worktree") {
                let Some(fields) = worktree.as_table() else {
                    return worktree.clone().try_into().map_err(Into::into);
                };
                for (key, needed) in [("root", needs_root), ("write_policy", needs_policy)] {
                    if let Some(value) = fields.get(key).filter(|_| needed) {
                        selected.insert(key.to_owned(), value.clone());
                    }
                }
            }
        }
        toml::Value::Table(selected).try_into().map_err(Into::into)
    }

    pub(crate) fn assemble(
        &self,
        templates: Option<&[u8]>,
        invariants: Option<&[u8]>,
    ) -> Result<JitConfig, ConfigurationAssemblyError> {
        let mut assembled: JitConfig = self.value()?.clone().try_into()?;
        assembled.validate_item_kinds()?;
        let hierarchy_types: Vec<&str> = assembled
            .type_hierarchy
            .as_ref()
            .map(|hierarchy| hierarchy.types.keys().map(String::as_str).collect())
            .unwrap_or_default();
        let templates = templates
            .map(|bytes| {
                let text = std::str::from_utf8(bytes)
                    .map_err(ConfigurationAssemblyError::TemplatesUtf8)?;
                Ok::<_, ConfigurationAssemblyError>(TemplateRegistry::from_toml_str(
                    text,
                    &hierarchy_types,
                )?)
            })
            .transpose()?
            .unwrap_or_default();
        let invariants = invariants
            .map(|bytes| {
                let text = std::str::from_utf8(bytes)
                    .map_err(ConfigurationAssemblyError::InvariantsUtf8)?;
                Ok::<_, ConfigurationAssemblyError>(InvariantRegistry::from_toml_str(text)?)
            })
            .transpose()?
            .unwrap_or_default();
        assembled.templates = templates;
        assembled.invariants = invariants;
        Ok(assembled)
    }
}

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
    ConfigurationDocument::parse(config).assemble(templates, invariants)
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

    /// Each sibling registry assembles from its own file whether or not
    /// `config.toml` is present, and an absent file contributes an empty one.
    #[test]
    fn test_assemble_configuration_reads_each_source_independently_of_the_others() {
        let config: &[u8] = b"[type_hierarchy]\ntypes = { task = 1 }\n";
        let templates: &[u8] = b"[[template]]\nname = \"t\"\napplies_to = [\"task\"]\n";
        let invariants: &[u8] =
            b"[[invariants]]\nid = \"i\"\nstatement = \"s\"\nkind = \"advisory\"\n";
        [None, Some(config)]
            .into_iter()
            .flat_map(|config| {
                [None, Some(templates)]
                    .into_iter()
                    .flat_map(move |templates| {
                        [None, Some(invariants)]
                            .into_iter()
                            .map(move |invariants| (config, templates, invariants))
                    })
            })
            .for_each(|(config, templates, invariants)| {
                let assembled = assemble_configuration(config, templates, invariants).unwrap();
                let case = (config.is_some(), templates.is_some(), invariants.is_some());
                assert_eq!(
                    assembled.type_hierarchy.is_some(),
                    config.is_some(),
                    "{case:?}"
                );
                assert!(assembled.item_kinds.is_none(), "{case:?}");
                assert_eq!(
                    assembled.templates.get("t").is_some(),
                    templates.is_some(),
                    "{case:?}"
                );
                if templates.is_none() {
                    assert_eq!(assembled.templates, TemplateRegistry::empty(), "{case:?}");
                }
                assert_eq!(
                    assembled.invariants.invariants.len(),
                    usize::from(invariants.is_some()),
                    "{case:?}"
                );
                if invariants.is_none() {
                    assert_eq!(assembled.invariants, InvariantRegistry::empty(), "{case:?}");
                }
            });
    }

    #[test]
    fn test_assemble_configuration_names_invariants_toml_for_a_malformed_invariant() {
        let error = assemble_configuration(
            None,
            None,
            Some(b"[[invariants]]\nid = \"i\"\nkind = \"advisory\"\n"),
        )
        .unwrap_err();
        assert!(matches!(error, ConfigurationAssemblyError::Invariants(_)));
        assert!(error.to_string().contains("invariants.toml"), "{error}");
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
        assert!(
            error.to_string().contains("unknown field `undeclared_key`"),
            "the top-level message carries the TOML message: {error}"
        );
    }
    #[test]
    fn test_configuration_projection_ignores_unconsumed_fields_but_full_view_rejects_them() {
        let document = ConfigurationDocument::parse(Some(b"[project]\nname = 'INVALID'\n[worktree]\nroot = '../code'\nenforce_leases = 'invalid'\nwrite_policy = 'invalid'\n"));
        let selected = document.worktree(true, false).unwrap();
        assert_eq!(
            selected.root.as_deref(),
            Some(std::path::Path::new("../code"))
        );
        assert!(document.assemble(None, None).is_err());
        assert!(document.worktree(false, true).is_err());
    }

    #[test]
    fn test_configuration_projection_requires_valid_original_policy_even_without_declared_root() {
        let document =
            ConfigurationDocument::parse(Some(b"[worktree]\nroot = 42\nwrite_policy = 'allow'\n"));
        assert_eq!(
            document.worktree(false, true).unwrap().write_policy(),
            crate::domain::LinkedCheckoutWriteStance::Allow
        );
        assert!(document.worktree(true, false).is_err());
        assert!(document.assemble(None, None).is_err());
    }

    #[test]
    fn test_configuration_projection_distinguishes_unneeded_syntax_from_absent_fields() {
        let document = ConfigurationDocument::parse(Some(b"broken = ["));
        assert!(document.worktree(false, false).is_ok());
        assert!(document.worktree(true, false).is_err());
        assert!(document.worktree(false, true).is_err());
        assert!(document.assemble(None, None).is_err());
        assert!(ConfigurationDocument::parse(None)
            .worktree(true, true)
            .unwrap()
            .root
            .is_none());
    }
}
