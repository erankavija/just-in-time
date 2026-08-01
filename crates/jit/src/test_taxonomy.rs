//! The one type vocabulary every shared test fixture renders.
//!
//! A suite needs the vocabulary in one of three forms, and all three come from
//! [`test_taxonomy`]:
//!
//! - a [`HierarchyConfig`] for the pure domain and graph suites, which take one
//!   by value and construct no repository ([`TestTaxonomy::hierarchy_config`]);
//! - a `config.toml` fragment for a suite that writes its own configuration file
//!   and appends its own `[item_kinds.*]`, `[gates]` or `[rules]` tables to it
//!   ([`TestTaxonomy::config_fragment`]);
//! - a whole repository built from that fragment
//!   ([`crate::test_utils::setup_test_repo_with_taxonomy`]).
//!
//! This module reaches for nothing but the pure hierarchy types it renders into,
//! so a domain or graph unit test reads the declaration without linking the
//! command and storage layers that repository construction needs.
//!
//! The vocabulary names no type any shipped package declares, so a suite
//! asserting that a strategic type resolves is exercising the mechanism over a
//! vocabulary it supplied, not passing because its names match something
//! shipped. It exists only under `cfg(test)` or `feature = "test-support"`, so
//! no adopter build compiles it and no repository can receive it.

#![cfg(any(test, feature = "test-support"))]

use crate::domain::type_taxonomy::HierarchyConfig;
use crate::hierarchy_templates::HierarchyTemplate;
use std::collections::HashMap;

/// One namespace declaration in the shared test vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestNamespace {
    /// Human-readable purpose stored in the repository configuration.
    pub description: String,
    /// Whether a repository issue may carry at most one label in this namespace.
    pub unique: bool,
}

/// The vocabulary [`test_taxonomy`] declares, and the forms it renders into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestTaxonomy {
    /// Type names and their hierarchy levels.
    pub hierarchy: HashMap<String, u8>,
    /// Type assigned when a consumer omits a type label.
    pub default_type: String,
    /// Type names this vocabulary treats as strategic.
    pub strategic_types: Vec<String>,
    /// Type names and their membership-label namespaces.
    pub label_associations: HashMap<String, String>,
    /// Namespace declarations, keyed by namespace name.
    pub namespaces: HashMap<String, TestNamespace>,
}

impl TestTaxonomy {
    /// The hierarchy value the pure domain and graph entry points take.
    ///
    /// # Panics
    ///
    /// Panics if the declared levels are not a valid hierarchy, which is a
    /// defect in this fixture rather than in a consumer.
    pub fn hierarchy_config(&self) -> HierarchyConfig {
        HierarchyConfig::new(self.hierarchy.clone(), self.label_associations.clone())
            .expect("the declared test vocabulary is a valid hierarchy")
    }

    /// The type this vocabulary declares at `level`, for a consumer that reads
    /// the name it was given rather than repeating a literal.
    ///
    /// # Panics
    ///
    /// Panics if the vocabulary declares no type at `level`, or more than one.
    pub fn type_at_level(&self, level: u8) -> &str {
        let mut at_level = self
            .hierarchy
            .iter()
            .filter(|(_, declared)| **declared == level)
            .map(|(name, _)| name.as_str());
        let name = at_level
            .next()
            .unwrap_or_else(|| panic!("the declared test vocabulary has a type at level {level}"));
        assert!(
            at_level.next().is_none(),
            "the declared test vocabulary has one type at level {level}"
        );
        name
    }

    /// The initialization preset carrying this vocabulary, for the fixture that
    /// derives a repository's coupled rules and schemas from it.
    pub fn hierarchy_template(&self) -> HierarchyTemplate {
        HierarchyTemplate {
            name: "test-taxonomy".to_string(),
            description: "Shared test vocabulary".to_string(),
            hierarchy: self.hierarchy.clone(),
            label_associations: self.label_associations.clone(),
        }
    }

    /// This vocabulary as `.jit/config.toml` text: a complete configuration on
    /// its own, and a fragment a suite composes with.
    ///
    /// The render opens and closes whole tables, so a suite appends its own
    /// `[item_kinds.*]`, `[gates]` or `[rules]` tables after it — no key a suite
    /// writes lands in a table this fragment left open.
    pub fn config_fragment(&self) -> String {
        let mut types: Vec<_> = self.hierarchy.iter().collect();
        types.sort_by(|(left_name, left_level), (right_name, right_level)| {
            left_level.cmp(right_level).then(left_name.cmp(right_name))
        });
        let types = types
            .into_iter()
            .map(|(name, level)| format!("{name} = {level}"))
            .collect::<Vec<_>>()
            .join(", ");

        let strategic_types = self
            .strategic_types
            .iter()
            .map(|name| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");

        let mut associations: Vec<_> = self.label_associations.iter().collect();
        associations.sort_by_key(|(name, _)| *name);
        let associations = associations
            .into_iter()
            .map(|(type_name, namespace)| format!("{type_name} = \"{namespace}\""))
            .collect::<Vec<_>>()
            .join("\n");

        let mut namespaces: Vec<_> = self.namespaces.iter().collect();
        namespaces.sort_by_key(|(name, _)| *name);
        let namespaces = namespaces
            .into_iter()
            .map(|(name, namespace)| {
                format!(
                    "[namespaces.{name}]\ndescription = \"{}\"\nunique = {}",
                    namespace.description, namespace.unique
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");

        format!(
            "[version]\nschema = 2\n\n[type_hierarchy]\ntypes = {{ {types} }}\nstrategic_types = [{strategic_types}]\n\n[type_hierarchy.label_associations]\n{associations}\n\n{namespaces}\n\n[validation]\nstrictness = \"loose\"\ndefault_type = \"{}\"\n",
            self.default_type
        )
    }
}

/// The declared test vocabulary: a four-level hierarchy, the membership
/// namespace each container type owns, the type a consumer gets by default, and
/// the namespaces a repository built from it declares.
pub fn test_taxonomy() -> TestTaxonomy {
    let hierarchy = [
        ("objective".to_string(), 1),
        ("initiative".to_string(), 2),
        ("deliverable".to_string(), 3),
        ("action".to_string(), 4),
    ]
    .into_iter()
    .collect();
    let strategic_types = ["objective", "initiative"]
        .into_iter()
        .map(str::to_string)
        .collect();
    let label_associations = [
        ("objective".to_string(), "objective".to_string()),
        ("initiative".to_string(), "initiative".to_string()),
        ("deliverable".to_string(), "deliverable".to_string()),
    ]
    .into_iter()
    .collect();
    let namespaces = [
        (
            "type".to_string(),
            TestNamespace {
                description: "Issue type".to_string(),
                unique: true,
            },
        ),
        (
            "area".to_string(),
            TestNamespace {
                description: "Cross-cutting test area".to_string(),
                unique: false,
            },
        ),
        (
            "crew".to_string(),
            TestNamespace {
                description: "Owning test crew".to_string(),
                unique: true,
            },
        ),
        (
            "objective".to_string(),
            TestNamespace {
                description: "Objective membership".to_string(),
                unique: false,
            },
        ),
        (
            "initiative".to_string(),
            TestNamespace {
                description: "Initiative membership".to_string(),
                unique: false,
            },
        ),
        (
            "deliverable".to_string(),
            TestNamespace {
                description: "Deliverable membership".to_string(),
                unique: false,
            },
        ),
    ]
    .into_iter()
    .collect();

    TestTaxonomy {
        hierarchy,
        default_type: "action".to_string(),
        strategic_types,
        label_associations,
        namespaces,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::JitConfig;

    /// The tables a suite of its own writes after the fragment.
    const SUITE_TABLES: &str = "\n[item_kinds.requirement]\nsection = \"success_criteria\"\n\
                                id-pattern = \"[A-Z][A-Z0-9]*-[0-9]+\"\n\
                                markers = [\"[hard]\"]\nscope = \"issue\"\n\
                                source-of-truth = \"markdown-first\"\n\n\
                                [worktree]\nenforce_leases = \"off\"\n";

    #[test]
    fn test_config_fragment_parses_with_suite_tables_appended() {
        let taxonomy = test_taxonomy();
        let composed = format!("{}{SUITE_TABLES}", taxonomy.config_fragment());

        let config: JitConfig = toml::from_str(&composed)
            .expect("a suite's own tables append to the fragment without conflict");

        let hierarchy = config
            .type_hierarchy
            .expect("the fragment declares the vocabulary");
        assert_eq!(hierarchy.types, taxonomy.hierarchy);
        assert!(
            config
                .item_kinds
                .is_some_and(|kinds| kinds.contains_key("requirement")),
            "the suite's own table survives composition"
        );
        assert!(
            config.worktree.is_some(),
            "a suite table opened after the fragment is its own, not the fragment's"
        );
    }

    #[test]
    fn test_hierarchy_config_declares_what_config_fragment_configures() {
        let taxonomy = test_taxonomy();
        let hierarchy = taxonomy.hierarchy_config();
        let configured = toml::from_str::<JitConfig>(&taxonomy.config_fragment())
            .expect("the fragment is a parseable configuration")
            .type_hierarchy
            .expect("the fragment declares the vocabulary");

        assert_eq!(
            hierarchy
                .types()
                .map(|(name, level)| (name.clone(), *level))
                .collect::<HashMap<_, _>>(),
            configured.types,
            "the pure suites' hierarchy carries the configured types"
        );
        assert_eq!(
            hierarchy
                .membership_namespaces()
                .map(|(type_name, namespace)| (type_name.clone(), namespace.clone()))
                .collect::<HashMap<_, _>>(),
            configured.label_associations.unwrap_or_default(),
            "the pure suites' hierarchy carries the configured associations"
        );
    }

    #[test]
    fn test_hierarchy_template_carries_the_declared_vocabulary() {
        let taxonomy = test_taxonomy();
        let template = taxonomy.hierarchy_template();

        assert_eq!(template.hierarchy, taxonomy.hierarchy);
        assert_eq!(template.label_associations, taxonomy.label_associations);
        assert_eq!(
            template.get_strategic_types(),
            taxonomy.strategic_types,
            "the preset's strategic types are the declared ones"
        );
    }
}
