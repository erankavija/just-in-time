//! Deterministic resolution of a participating profile-package graph.
//!
//! Package loading is a command concern. This module receives the complete
//! candidate set — selected packages and the dependencies loaded for them —
//! and settles the graph without consulting the filesystem or choosing a winner
//! from selector order.

use super::ProfilePackage;
use crate::domain::identity::ProfileId;
use semver::Version;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The semantic version of the running JIT engine.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct EngineVersion(Version);

impl EngineVersion {
    /// Parse an engine version at the boundary where it enters graph policy.
    pub fn parse(value: &str) -> Result<Self, semver::Error> {
        Version::parse(value).map(Self)
    }

    /// Return the version of this compiled JIT engine.
    pub fn running() -> Result<Self, semver::Error> {
        Self::parse(env!("CARGO_PKG_VERSION"))
    }
}

impl std::fmt::Display for EngineVersion {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The complete, validated package graph for one profile operation.
#[derive(Debug, Clone)]
pub struct ResolvedProfileGraph {
    packages: BTreeMap<ProfileId, ProfilePackage>,
    dependencies: BTreeMap<ProfileId, BTreeSet<ProfileId>>,
    order: Vec<ProfileId>,
    selected: BTreeSet<ProfileId>,
}

impl ResolvedProfileGraph {
    /// Resolve every supplied package and return a dependency-first order.
    ///
    /// The supplied map must contain the selected packages and their dependency
    /// closure. The resolver still verifies that closure, compatibility, and
    /// incompatibility claims are all satisfied before returning.
    pub fn resolve(
        packages: BTreeMap<ProfileId, ProfilePackage>,
        selected: BTreeSet<ProfileId>,
        engine: &EngineVersion,
    ) -> Result<Self, ProfileGraphError> {
        let missing_selected = selected
            .iter()
            .find(|id| !packages.contains_key(*id))
            .cloned();
        if let Some(id) = missing_selected {
            return Err(ProfileGraphError::SelectedPackageMissing { id: id.to_string() });
        }

        for package in packages.values() {
            let model = package.model();
            if !model.compatible_jit.matches(&engine.0) {
                return Err(ProfileGraphError::IncompatibleEngine {
                    package: model.id.to_string(),
                    required: model.compatible_jit.to_string(),
                    actual: engine.to_string(),
                });
            }
        }

        let mut dependencies = BTreeMap::new();
        for (id, package) in &packages {
            let mut declared = BTreeSet::new();
            for dependency in &package.model().dependencies {
                let Some(resolved) = packages.get(&dependency.id) else {
                    return Err(ProfileGraphError::MissingDependency {
                        package: id.to_string(),
                        dependency: dependency.id.to_string(),
                    });
                };
                if !dependency.version.matches(&resolved.model().version) {
                    return Err(ProfileGraphError::DependencyVersionMismatch {
                        package: id.to_string(),
                        dependency: dependency.id.to_string(),
                        required: dependency.version.to_string(),
                        found: resolved.model().version.to_string(),
                    });
                }
                declared.insert(dependency.id.clone());
            }
            dependencies.insert(id.clone(), declared);
        }

        for (id, package) in &packages {
            let mut incompatibilities = package.model().incompatibilities.clone();
            incompatibilities.sort_by_cached_key(|incompatibility| {
                (
                    incompatibility.id.clone(),
                    incompatibility.version.to_string(),
                )
            });
            for incompatibility in incompatibilities {
                let Some(other) = packages.get(&incompatibility.id) else {
                    continue;
                };
                if incompatibility.version.matches(&other.model().version) {
                    return Err(ProfileGraphError::IncompatiblePackages {
                        package: id.to_string(),
                        other: incompatibility.id.to_string(),
                        requirement: incompatibility.version.to_string(),
                        other_version: other.model().version.to_string(),
                    });
                }
            }
        }

        let adjacency = dependencies
            .iter()
            .map(|(id, required)| (id.clone(), required.iter().cloned().collect()))
            .collect::<Vec<_>>();
        let order = crate::graph::keyed_topological_order(&adjacency).map_err(|cycle| {
            ProfileGraphError::DependencyCycle {
                cycle: cycle.into_iter().map(|id| id.to_string()).collect(),
            }
        })?;

        Ok(Self {
            packages,
            dependencies,
            order,
            selected,
        })
    }

    /// Each participating package's transitive dependencies.
    pub fn transitive_dependencies(&self) -> BTreeMap<ProfileId, BTreeSet<ProfileId>> {
        self.order.iter().fold(BTreeMap::new(), |mut closure, id| {
            let required = self.dependencies.get(id).into_iter().flatten();
            let reached = required
                .flat_map(|dependency| {
                    closure
                        .get(dependency)
                        .into_iter()
                        .flatten()
                        .chain(std::iter::once(dependency))
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .collect();
            closure.insert(id.clone(), reached);
            closure
        })
    }

    /// Return the selected roots and their dependency closure in canonical
    /// dependency-first order.
    pub fn selected_packages(&self) -> Vec<ProfilePackage> {
        let mut reachable = self.selected.clone();
        let mut pending = VecDeque::from_iter(self.selected.iter().cloned());
        while let Some(id) = pending.pop_front() {
            if let Some(required) = self.dependencies.get(&id) {
                for dependency in required {
                    if reachable.insert(dependency.clone()) {
                        pending.push_back(dependency.clone());
                    }
                }
            }
        }
        self.order
            .iter()
            .filter(|id| reachable.contains(*id))
            .filter_map(|id| self.packages.get(id).cloned())
            .collect()
    }
}

/// A graph claim that prevented publication.
#[derive(Debug, thiserror::Error)]
pub enum ProfileGraphError {
    /// A selected id was not present in the candidate map.
    #[error("selected profile '{id}' is missing from the package graph")]
    SelectedPackageMissing { id: String },
    /// A package's engine range excludes the running engine.
    #[error(
        "profile '{package}' requires compatible-jit '{required}', but the running engine is '{actual}'"
    )]
    IncompatibleEngine {
        package: String,
        required: String,
        actual: String,
    },
    /// A declared dependency is not in the participating map.
    #[error("profile '{package}' declares missing dependency '{dependency}'")]
    MissingDependency { package: String, dependency: String },
    /// A dependency exists but has the wrong version.
    #[error(
        "profile '{package}' requires dependency '{dependency}' at '{required}', but the participating package provides '{found}'"
    )]
    DependencyVersionMismatch {
        package: String,
        dependency: String,
        required: String,
        found: String,
    },
    /// A participating package pair violates an incompatibility declaration.
    #[error(
        "profile '{package}' is incompatible with profile '{other}' (requirement '{requirement}', other version '{other_version}')"
    )]
    IncompatiblePackages {
        package: String,
        other: String,
        requirement: String,
        other_version: String,
    },
    /// The participating dependency graph contains a cycle, named as a closed
    /// path whose first id repeats as the last.
    #[error("profile dependency cycle: {}", cycle.join(" -> "))]
    DependencyCycle { cycle: Vec<String> },
    /// Two different package images claim one profile identity.
    #[error(
        "profile id '{id}' resolves to different package images '{first_hash}' and '{second_hash}'"
    )]
    ConflictingPackageIdentity {
        id: String,
        first_hash: String,
        second_hash: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use tempfile::TempDir;

    fn package_tree(temp: &TempDir, id: &str) -> ProfilePackage {
        let root = temp.path().join(id);
        std::fs::create_dir_all(root.join("assets")).unwrap();
        std::fs::write(
            root.join("manifest.toml"),
            format!(
                "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"1.0.0\"\ncompatible-jit = \">=0.2.0, <2.0.0\"\n\n[[asset]]\nsource = \"assets/content.txt\"\ntarget = \"docs/{id}.txt\"\n"
            ),
        )
        .unwrap();
        std::fs::write(root.join("assets/content.txt"), id).unwrap();
        ProfilePackage::from_directory(&root).unwrap()
    }

    fn package_map(ids: &[&str]) -> BTreeMap<ProfileId, ProfilePackage> {
        let temp = TempDir::new().unwrap();
        let packages = ids
            .iter()
            .map(|id| package_tree(&temp, id))
            .collect::<Vec<_>>();
        // Keep the trees alive through package construction and return owned
        // packages; package readers own all admitted bytes after construction.
        drop(temp);
        packages
            .into_iter()
            .map(|package| (package.model().id.clone(), package))
            .collect()
    }

    fn selected(ids: &[&str]) -> BTreeSet<ProfileId> {
        ids.iter()
            .map(|id| ProfileId::try_from(*id).unwrap())
            .collect()
    }

    #[test]
    fn test_resolved_profile_graph_orders_packages_by_id_when_they_are_independent() {
        let graph = ResolvedProfileGraph::resolve(
            package_map(&["zulu", "alpha", "middle"]),
            selected(&["zulu", "alpha"]),
            &EngineVersion::running().unwrap(),
        )
        .unwrap();

        assert_eq!(
            graph
                .selected_packages()
                .into_iter()
                .map(|package| package.model().id.to_string())
                .collect::<Vec<_>>(),
            vec!["alpha", "zulu"]
        );
    }

    #[test]
    fn test_transitive_dependencies_closes_over_a_chain() {
        let temp = TempDir::new().unwrap();
        let packages = [("top", Some("middle")), ("middle", Some("base")), ("base", None)]
            .map(|(id, dependency)| {
                let _ = package_tree(&temp, id);
                let declared = dependency
                    .map(|dependency| format!("\n[[dependency]]\nid = \"{dependency}\"\nversion = \"*\"\n"))
                    .unwrap_or_default();
                std::fs::write(
                    temp.path().join(id).join("manifest.toml"),
                    format!(
                        "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"1.0.0\"\ncompatible-jit = \">=0.2.0, <2.0.0\"\n{declared}\n[[asset]]\nsource = \"assets/content.txt\"\ntarget = \"docs/{id}.txt\"\n"
                    ),
                )
                .unwrap();
                let package = ProfilePackage::from_directory(&temp.path().join(id)).unwrap();
                (package.model().id.clone(), package)
            })
            .into_iter()
            .collect();

        let closure = ResolvedProfileGraph::resolve(
            packages,
            selected(&["top"]),
            &EngineVersion::running().unwrap(),
        )
        .unwrap()
        .transitive_dependencies();

        assert_eq!(
            closure[&ProfileId::try_from("top").unwrap()],
            selected(&["middle", "base"])
        );
        assert_eq!(
            closure[&ProfileId::try_from("base").unwrap()],
            selected(&[])
        );
    }

    #[test]
    fn test_resolved_profile_graph_reports_a_cycle_with_all_package_ids() {
        let temp = TempDir::new().unwrap();
        let _ = package_tree(&temp, "first");
        let _ = package_tree(&temp, "second");
        for (id, dependency) in [("first", "second"), ("second", "first")] {
            std::fs::write(
                temp.path().join(id).join("manifest.toml"),
                format!(
                    "[profile]\nmanifest-version = 2\nid = \"{id}\"\nversion = \"1.0.0\"\ncompatible-jit = \">=0.2.0, <2.0.0\"\n\n[[dependency]]\nid = \"{dependency}\"\nversion = \"*\"\n\n[[asset]]\nsource = \"assets/content.txt\"\ntarget = \"docs/{id}.txt\"\n"
                ),
            )
            .unwrap();
        }
        let first = ProfilePackage::from_directory(&temp.path().join("first")).unwrap();
        let second = ProfilePackage::from_directory(&temp.path().join("second")).unwrap();
        let packages = [first, second]
            .into_iter()
            .map(|package| (package.model().id.clone(), package))
            .collect();

        let error = ResolvedProfileGraph::resolve(
            packages,
            selected(&["first"]),
            &EngineVersion::running().unwrap(),
        )
        .expect_err("a two-node cycle has no order");
        assert!(
            matches!(
                error,
                ProfileGraphError::DependencyCycle { ref cycle }
                    if *cycle
                        == vec![
                            "first".to_string(),
                            "second".to_string(),
                            "first".to_string(),
                        ]
            ),
            "the cycle must name both packages: {error}"
        );
    }

    proptest! {
        #[test]
        fn prop_resolved_profile_graph_ignores_selector_occurrence_order(
            reverse in any::<bool>(),
        ) {
            let ids = ["alpha", "beta", "gamma"];
            let selection = if reverse {
                ["gamma", "alpha"]
            } else {
                ["alpha", "gamma"]
            };
            let graph = ResolvedProfileGraph::resolve(
                package_map(&ids),
                selected(&selection),
                &EngineVersion::running().unwrap(),
            ).unwrap();
            let order = graph.selected_packages().into_iter()
                .map(|package| package.model().id.to_string())
                .collect::<Vec<_>>();
            prop_assert_eq!(order, vec!["alpha", "gamma"]);
        }
    }
}
