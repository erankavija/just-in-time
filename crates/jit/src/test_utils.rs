//! Shared test utilities
//!
//! Common helpers used across multiple test modules to reduce duplication.

#![cfg(any(test, feature = "test-support"))]

use crate::commands::CommandExecutor;
use crate::hierarchy_templates::HierarchyTemplate;
use crate::storage::worktree_paths::WorktreePaths;
use crate::storage::{discover_repository_layout, JsonFileStorage};
use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

/// One namespace declaration in the shared custom-taxonomy fixture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestNamespace {
    /// Human-readable purpose stored in the repository configuration.
    pub description: String,
    /// Whether a repository issue may carry at most one label in this namespace.
    pub unique: bool,
}

/// Vocabulary authored by [`setup_test_repo_with_taxonomy`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestTaxonomy {
    /// Type names and their hierarchy levels.
    pub hierarchy: HashMap<String, u8>,
    /// Type assigned when a fixture consumer omits a type label.
    pub default_type: String,
    /// Type names that the fixture treats as strategic.
    pub strategic_types: Vec<String>,
    /// Type names and their membership-label namespaces.
    pub label_associations: HashMap<String, String>,
    /// Explicit namespace declarations in the fixture repository.
    pub namespaces: HashMap<String, TestNamespace>,
}

impl TestTaxonomy {
    fn hierarchy_template(&self) -> HierarchyTemplate {
        HierarchyTemplate {
            name: "test-taxonomy".to_string(),
            description: "Shared test vocabulary".to_string(),
            hierarchy: self.hierarchy.clone(),
            label_associations: self.label_associations.clone(),
        }
    }

    fn config_toml(&self) -> String {
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

/// Return the vocabulary used by the shared custom-taxonomy fixture.
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

/// Standard test repository setup with .jit and .git directories
///
/// Creates a temporary directory with initialized jit storage.
/// Returns both the TempDir (which cleans up on drop) and the storage instance.
///
/// # Example
/// ```no_run
/// use jit::test_utils::setup_test_repo;
///
/// let (temp, storage) = setup_test_repo().unwrap();
/// // Use temp.path() and storage for tests
/// ```
pub fn setup_test_repo() -> Result<(TempDir, JsonFileStorage)> {
    let temp = TempDir::new()?;

    let jit_root = temp.path().join(".jit");
    let storage = JsonFileStorage::new(&jit_root);
    let layout = discover_repository_layout(temp.path(), &jit_root)?;
    CommandExecutor::new(storage.clone())
        .with_layout(layout)
        .initialize_fresh_repository(temp.path(), &HierarchyTemplate::default(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, storage))
}

/// Build a repository from the fixture's explicitly declared custom taxonomy.
///
/// The configuration is written before initialization, so the initializer's
/// existing-configuration path consumes these declarations and derives the
/// repository's coupled rules and schemas from them. The returned taxonomy is
/// the same value used to author that configuration, allowing callers to build
/// labels and assertions without repeating vocabulary literals.
pub fn setup_test_repo_with_taxonomy() -> Result<(TempDir, JsonFileStorage, TestTaxonomy)> {
    let taxonomy = test_taxonomy();
    let temp = TempDir::new()?;
    let jit_root = temp.path().join(".jit");
    fs::create_dir_all(&jit_root)?;
    fs::write(jit_root.join("config.toml"), taxonomy.config_toml())?;

    let storage = JsonFileStorage::new(&jit_root);
    let layout = discover_repository_layout(temp.path(), &jit_root)?;
    CommandExecutor::new(storage.clone())
        .with_layout(layout)
        .initialize_fresh_repository(temp.path(), &taxonomy.hierarchy_template(), None)?;
    // Claim coordination tests use this as a synthetic Git control directory.
    fs::create_dir(temp.path().join(".git"))?;

    Ok((temp, storage, taxonomy))
}

/// Create test WorktreePaths from a TempDir
///
/// Generates a WorktreePaths structure suitable for testing,
/// with standard paths relative to the temp directory.
///
/// # Example
/// ```no_run
/// use jit::test_utils::{setup_test_repo, create_test_paths};
///
/// let (temp, _storage) = setup_test_repo().unwrap();
/// let paths = create_test_paths(&temp);
/// assert_eq!(paths.worktree_root, temp.path());
/// ```
pub fn create_test_paths(temp: &TempDir) -> WorktreePaths {
    WorktreePaths {
        common_dir: temp.path().join(".git"),
        worktree_root: temp.path().to_path_buf(),
        local_jit: temp.path().join(".jit"),
        shared_jit: temp.path().join(".git/jit"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::IssueStore;

    #[test]
    fn test_setup_test_repo_with_taxonomy_declares_and_exposes_exact_vocabulary() {
        let (_temp, storage, taxonomy) = setup_test_repo_with_taxonomy().unwrap();
        let config = crate::config::JitConfig::load(storage.root()).unwrap();
        let hierarchy = config
            .type_hierarchy
            .expect("taxonomy fixture must declare a type hierarchy");
        let namespaces = config
            .namespaces
            .expect("taxonomy fixture must declare namespaces");

        assert_eq!(hierarchy.types, taxonomy.hierarchy);
        assert_eq!(
            hierarchy.strategic_types,
            Some(taxonomy.strategic_types.clone())
        );
        assert_eq!(
            hierarchy.label_associations,
            Some(taxonomy.label_associations.clone())
        );
        assert_eq!(
            config
                .validation
                .as_ref()
                .and_then(|validation| validation.default_type.as_deref()),
            Some(taxonomy.default_type.as_str())
        );
        assert_eq!(namespaces.len(), taxonomy.namespaces.len());
        assert!(taxonomy.namespaces.iter().all(|(name, expected)| {
            namespaces.get(name).is_some_and(|actual| {
                actual.description == expected.description && actual.unique == expected.unique
            })
        }));
    }
}
