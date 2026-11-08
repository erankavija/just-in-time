//! Pure conversions from parsed configuration to domain settings.

use crate::config::JitConfig;
use crate::domain::{LabelNamespace, LabelNamespaces, LinkedCheckoutWriteStance};
use anyhow::Result;

/// Environment variable carrying one invocation's linked-checkout write stance.
///
/// It is the per-invocation counterpart of the repository's declared
/// `[worktree] write_policy`, named after that key.
pub const LINKED_CHECKOUT_WRITE_STANCE_ENV: &str = "JIT_WORKTREE_WRITE_POLICY";

/// The declared linked-checkout write stance, defaulting to
/// [`LinkedCheckoutWriteStance::DEFAULT`].
///
/// Unlike [`JitConfig::enforcement_mode`], an absent `write_policy` key in a
/// present `[worktree]` section and an absent `[worktree]` section resolve
/// identically, to the default.
pub fn write_policy_from_config(config: &JitConfig) -> LinkedCheckoutWriteStance {
    config
        .worktree
        .as_ref()
        .map_or(LinkedCheckoutWriteStance::DEFAULT, |worktree| {
            worktree.write_policy()
        })
}

/// Parse one invocation's override of the linked-checkout write stance, the
/// value of [`LINKED_CHECKOUT_WRITE_STANCE_ENV`].
///
/// `None` supplies no override, so
/// [`LinkedCheckoutWriteStance::resolve`] keeps the declared stance. The token
/// uses the `write_policy` key's parser, so both sources accept the same tokens.
///
/// # Errors
///
/// A token outside the stance vocabulary, named with the accepted tokens.
pub fn invocation_write_override(token: Option<&str>) -> Result<Option<LinkedCheckoutWriteStance>> {
    token
        .map(|token| {
            token.parse::<LinkedCheckoutWriteStance>().map_err(|error| {
                crate::errors::InvalidArgumentError::new(format!(
                    "invalid {LINKED_CHECKOUT_WRITE_STANCE_ENV}: {error}"
                ))
                .into()
            })
        })
        .transpose()
}

/// The declared `[project] name`, or `None` when the table or key is absent.
pub fn project_name_from_config(config: &JitConfig) -> Option<&str> {
    config
        .project
        .as_ref()?
        .name
        .as_ref()
        .map(crate::config::ProjectName::as_str)
}

/// Build the namespace registry from an already-parsed [`JitConfig`], without
/// filesystem access.
///
/// The single `config -> LabelNamespaces` conversion; the default rule family
/// derives from this same registry. The registry carries
/// exactly what the configuration declares: an absent `[namespaces]` table
/// yields no namespace, and an absent `[type_hierarchy]` table yields no
/// hierarchy.
pub fn namespaces_from_config(config: &JitConfig) -> LabelNamespaces {
    let namespaces = config
        .namespaces
        .iter()
        .flatten()
        // Only the taxonomy (description/unique) crosses into the domain
        // registry; per-namespace constraints (values/pattern/required) were
        // removed when validation became rule-driven.
        .map(|(name, ns_config)| {
            (
                name.clone(),
                LabelNamespace::new(ns_config.description.clone(), ns_config.unique),
            )
        })
        .collect();

    let mut result = LabelNamespaces {
        schema_version: config
            .version
            .as_ref()
            .map_or(crate::config::VersionConfig::DEFAULT_SCHEMA, |v| v.schema),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn config(toml: &str) -> JitConfig {
        crate::declarations::assemble_configuration(Some(toml.as_bytes()), None, None).unwrap()
    }

    #[test]
    fn test_namespaces_from_config_carries_the_declared_taxonomy() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let namespaces = namespaces_from_config(&config(&taxonomy.config_fragment()));

        assert_eq!(namespaces.schema_version, 2);
        assert_eq!(namespaces.namespaces.len(), taxonomy.namespaces.len());
        assert!(namespaces.namespaces.contains_key("type"));
        assert!(namespaces
            .namespaces
            .contains_key(taxonomy.type_at_level(1)));
        assert!(namespaces
            .namespaces
            .contains_key(taxonomy.type_at_level(2)));
        assert_eq!(
            namespaces.strategic_types.as_ref().unwrap(),
            &taxonomy.strategic_types
        );
    }

    #[test]
    fn test_namespaces_from_config_without_declarations_yields_an_empty_registry() {
        let namespaces = namespaces_from_config(&config(""));

        assert!(namespaces.namespaces.is_empty());
        assert!(namespaces.type_hierarchy.is_none());
        assert!(namespaces.label_associations.is_none());
        assert!(namespaces.strategic_types.is_none());
        assert_eq!(
            crate::domain::type_taxonomy::HierarchyConfig::from_declarations(&namespaces).unwrap(),
            crate::domain::type_taxonomy::HierarchyConfig::empty(),
        );
    }

    #[test]
    fn test_namespaces_from_config_keeps_each_namespace_uniqueness() {
        let namespaces = namespaces_from_config(&config(
            "[namespaces.type]\ndescription = \"Issue type\"\nunique = true\n\n\
             [namespaces.component]\ndescription = \"Component\"\nunique = false\n",
        ));

        assert!(namespaces.namespaces["type"].unique);
        assert!(!namespaces.namespaces["component"].unique);
    }

    #[test]
    fn test_write_policy_from_config_defaults_unless_declared() {
        [
            ("", LinkedCheckoutWriteStance::DEFAULT),
            (
                "[type_hierarchy]\ntypes = { task = 1 }\n",
                LinkedCheckoutWriteStance::DEFAULT,
            ),
            (
                "[worktree]\nenforce_leases = \"strict\"\n",
                LinkedCheckoutWriteStance::DEFAULT,
            ),
            (
                "[worktree]\nwrite_policy = \"allow\"\n",
                LinkedCheckoutWriteStance::Allow,
            ),
        ]
        .into_iter()
        .for_each(|(toml, expected)| {
            assert_eq!(write_policy_from_config(&config(toml)), expected, "{toml}");
        });
    }

    #[test]
    fn test_invocation_write_override_parses_the_stance_vocabulary() {
        assert_eq!(invocation_write_override(None).unwrap(), None);
        assert_eq!(
            invocation_write_override(Some("allow")).unwrap(),
            Some(LinkedCheckoutWriteStance::Allow)
        );
        let error = invocation_write_override(Some("sometimes")).unwrap_err();
        assert!(error
            .downcast_ref::<crate::errors::InvalidArgumentError>()
            .is_some());
        assert!(error.to_string().contains(LINKED_CHECKOUT_WRITE_STANCE_ENV));
    }

    #[test]
    fn test_project_name_from_config_reads_the_declared_name() {
        assert_eq!(project_name_from_config(&config("")), None);
        assert_eq!(project_name_from_config(&config("[project]\n")), None);
        assert_eq!(
            project_name_from_config(&config("[project]\nname = \"just-in-time\"\n")),
            Some("just-in-time")
        );
    }
}
