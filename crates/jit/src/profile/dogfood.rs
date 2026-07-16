//! Embedded production package for jit's repository-neutral dogfood workflow.

use super::{
    derive_preset_projection, project_package, EmbeddedProfilePackage, PackageProjection,
    ProfilePackageError, ProjectionError,
};
use crate::domain::Gate;
use include_dir::{include_dir, Dir};
use std::collections::{BTreeMap, BTreeSet};

static JIT_DOGFOOD_DIRECTORY: Dir<'_> =
    include_dir!("$CARGO_MANIFEST_DIR/../../profiles/jit-dogfood");

/// Package-source prefix identifying assets that also project into this source tree.
pub const JIT_DOGFOOD_LIVE_SOURCE_PREFIX: &str = "assets/live/";

/// Failures loading or projecting the embedded dogfood workflow.
#[derive(Debug, thiserror::Error)]
pub enum DogfoodProfileError {
    /// The production package failed immutable package validation.
    #[error("invalid embedded jit-dogfood package: {0}")]
    Package(#[from] ProfilePackageError),
    /// A package projection could not be rendered.
    #[error("failed to project embedded jit-dogfood package: {0}")]
    Projection(#[from] ProjectionError),
    /// A requested gate is absent from the package.
    #[error("jit-dogfood package does not declare gate '{0}'")]
    MissingGate(String),
    /// The planning template is absent or structurally invalid.
    #[error("jit-dogfood planning template is invalid: {0}")]
    InvalidPlanningTemplate(String),
    /// A package gate does not match the runtime gate wire type.
    #[error("jit-dogfood gate '{key}' is invalid: {source}")]
    InvalidGate {
        /// Requested package gate key.
        key: String,
        /// Runtime wire-format error.
        source: serde_json::Error,
    },
}

/// Load the recursively embedded, immutable `jit-dogfood` package.
pub fn jit_dogfood_package() -> Result<EmbeddedProfilePackage<'static>, DogfoodProfileError> {
    EmbeddedProfilePackage::from_dir(&JIT_DOGFOOD_DIRECTORY).map_err(Into::into)
}

/// Deserialize one gate definition from the package's authored gate inventory.
pub fn jit_dogfood_gate(key: &str) -> Result<Gate, DogfoodProfileError> {
    let package = jit_dogfood_package()?;
    let value = derive_preset_projection(&package)
        .gates
        .iter()
        .find(|value| value.get("key").and_then(serde_json::Value::as_str) == Some(key))
        .cloned()
        .ok_or_else(|| DogfoodProfileError::MissingGate(key.to_string()))?;

    serde_json::from_value(value).map_err(|source| DogfoodProfileError::InvalidGate {
        key: key.to_string(),
        source,
    })
}

/// Gate keys attached to nodes of the package-authored `plan` template.
///
/// Anchor-only gates are excluded, so this is also the compatibility preset
/// inventory used by [`crate::gate_presets::BuiltinPresets`].
pub fn jit_dogfood_planning_gate_keys() -> Result<Vec<String>, DogfoodProfileError> {
    let package = jit_dogfood_package()?;
    let template = package
        .manifest()
        .contributions
        .iter()
        .find_map(|contribution| match contribution {
            super::Contribution::KeyedArray {
                target: super::KeyedArrayTarget::Templates,
                identity,
                value,
            } if identity == "plan" => Some(value),
            _ => None,
        })
        .ok_or_else(|| {
            DogfoodProfileError::InvalidPlanningTemplate("missing 'plan' template".to_string())
        })?;
    let nodes = template
        .get("nodes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            DogfoodProfileError::InvalidPlanningTemplate(
                "'plan' template has no node array".to_string(),
            )
        })?;
    let mut keys = Vec::new();
    for node in nodes {
        let gates = node
            .get("gates")
            .and_then(serde_json::Value::as_array)
            .ok_or_else(|| {
                DogfoodProfileError::InvalidPlanningTemplate(
                    "a 'plan' template node has no gate array".to_string(),
                )
            })?;
        for gate in gates {
            let key = gate.as_str().ok_or_else(|| {
                DogfoodProfileError::InvalidPlanningTemplate(
                    "a 'plan' template gate is not a string".to_string(),
                )
            })?;
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_string());
            }
        }
    }
    Ok(keys)
}

/// Project only package-authored live consumers plus managed regions.
///
/// Install-only adopter state remains excluded. `existing` supplies current
/// bytes for managed-region targets, using repository-relative keys.
pub fn jit_dogfood_live_projection(
    existing: &BTreeMap<String, Vec<u8>>,
) -> Result<PackageProjection, DogfoodProfileError> {
    let package = jit_dogfood_package()?;
    let projected = project_package(&package, existing)?;
    let live_targets = package
        .manifest()
        .assets
        .iter()
        .filter(|asset| asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
        .map(|asset| asset.target.as_str())
        .chain(
            package
                .manifest()
                .regions
                .iter()
                .map(|region| region.target.as_str()),
        )
        .collect::<BTreeSet<_>>();
    let mut files = projected
        .files()
        .iter()
        .filter(|(target, _)| live_targets.contains(target.as_str()))
        .map(|(target, file)| (target.clone(), file.clone()))
        .collect::<BTreeMap<_, _>>();
    if let (Some(current), Some(projected_agents)) =
        (existing.get("AGENTS.md"), files.get_mut("AGENTS.md"))
    {
        preserve_nested_region(
            current,
            &mut projected_agents.bytes,
            b"<!-- jit:invariants:begin -->",
            b"<!-- jit:invariants:end -->",
        );
    }
    Ok(PackageProjection::from_files(files))
}

fn preserve_nested_region(current: &[u8], projected: &mut Vec<u8>, begin: &[u8], end: &[u8]) {
    let Some(current_begin) = find_bytes(current, begin) else {
        return;
    };
    let Some(current_end_rel) = find_bytes(&current[current_begin + begin.len()..], end) else {
        return;
    };
    let current_content_start = current_begin + begin.len();
    let current_content_end = current_content_start + current_end_rel;

    let Some(projected_begin) = find_bytes(projected, begin) else {
        return;
    };
    let Some(projected_end_rel) = find_bytes(&projected[projected_begin + begin.len()..], end)
    else {
        return;
    };
    let projected_content_start = projected_begin + begin.len();
    let projected_content_end = projected_content_start + projected_end_rel;
    projected.splice(
        projected_content_start..projected_content_end,
        current[current_content_start..current_content_end]
            .iter()
            .copied(),
    );
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::config::ProjectionStyle;
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::profile::{Contribution, KeyedArrayTarget};
    use crate::storage::{GateRegistry, IssueStore, JsonFileStorage};
    use crate::validation::rules::RuleSet;
    use crate::validation::rules_gates_projection::render_rules_and_gates_markdown;
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn test_jit_dogfood_package_validates_and_has_expected_workflow_inventory() {
        let package = jit_dogfood_package().unwrap();
        assert_eq!(package.manifest().profile.id, "jit-dogfood");
        assert!(package.file_count() <= super::super::MAX_EMBEDDED_PROFILE_FILES);
        assert!(package.byte_size() <= super::super::MAX_EMBEDDED_PROFILE_BYTES);

        let gates = package
            .manifest()
            .contributions
            .iter()
            .filter_map(|contribution| match contribution {
                Contribution::KeyedArray {
                    target: KeyedArrayTarget::Gates,
                    identity,
                    ..
                } => Some(identity.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            gates,
            BTreeSet::from([
                "breakdown-review",
                "code-review",
                "coverage-preview",
                "jit-validate",
                "plan-review",
                "repo-validate",
            ])
        );

        let skill_roots = package
            .manifest()
            .assets
            .iter()
            .filter_map(|asset| {
                asset
                    .target
                    .strip_prefix(".agents/skills/")
                    .and_then(|path| path.split('/').next())
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(
            skill_roots,
            BTreeSet::from([
                "jit-breakdown",
                "jit-execution-lead",
                "jit-manage",
                "jit-migrate",
                "jit-parallel",
                "jit-planning-lead",
                "jit-project-lead",
            ])
        );
    }

    #[test]
    fn test_jit_dogfood_package_contains_no_deferred_or_checkout_local_content() {
        let package = jit_dogfood_package().unwrap();
        for declaration in package
            .manifest()
            .assets
            .iter()
            .map(|asset| (&asset.source, &asset.target))
            .chain(
                package
                    .manifest()
                    .regions
                    .iter()
                    .map(|region| (&region.source, &region.target)),
            )
        {
            let (source, target) = declaration;
            let text = std::str::from_utf8(package.source_bytes(source).unwrap()).unwrap_or("");
            for forbidden in [
                "/home/",
                "crates/jit/",
                "dev/archive/",
                "dev/vision/",
                "docs/examples/research/",
                "/evals/",
                "trigger-evals",
                "LDPC",
                "GLDPC",
                "Nexus",
                "gf2",
                "Kani",
                "AVX",
                "Zen 3",
                "PPC",
                "QAM",
                "SNR",
                "this-repo",
                "this repository",
            ] {
                assert!(
                    !source.contains(forbidden)
                        && !target.contains(forbidden)
                        && !text.contains(forbidden),
                    "{source} contains checkout-local or deferred content '{forbidden}'"
                );
            }
        }
    }

    #[test]
    fn test_live_projection_excludes_install_only_state() {
        let projection = jit_dogfood_live_projection(&BTreeMap::new()).unwrap();
        assert!(projection
            .get(".agents/skills/jit-manage/SKILL.md")
            .is_some());
        assert!(projection.get("scripts/ai-review.sh").is_some());
        assert!(projection.get("AGENTS.md").is_some());
        assert!(projection.get(".jit/invariants.toml").is_none());
        assert!(projection
            .get(".jit/schemas/jit-content-standards.json")
            .is_none());
        assert!(projection
            .get(".jit/reference/rules-and-gates.md")
            .is_none());
    }

    #[test]
    fn test_all_package_gate_values_deserialize_as_runtime_gates() {
        for key in [
            "plan-review",
            "breakdown-review",
            "code-review",
            "coverage-preview",
            "jit-validate",
            "repo-validate",
        ] {
            let gate = jit_dogfood_gate(key).unwrap();
            assert_eq!(gate.key, key);
            assert!(
                matches!(
                    gate.checker,
                    Some(
                        crate::domain::GateChecker::RepositoryValidation
                            | crate::domain::GateChecker::IssueValidation
                            | crate::domain::GateChecker::LabelTargetValidation { .. }
                            | crate::domain::GateChecker::ReviewPlaceholder
                    )
                ),
                "{key} must remain an in-process portable checker"
            );
        }
    }

    #[test]
    fn test_live_projection_matches_every_declared_source_tree_consumer() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let agents = fs::read(root.join("AGENTS.md")).unwrap();
        let projection =
            jit_dogfood_live_projection(&BTreeMap::from([("AGENTS.md".to_string(), agents)]))
                .unwrap();

        for file in projection.files().values() {
            let live = fs::read(root.join(&file.target))
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", file.target));
            assert_eq!(live, file.bytes, "{} drifted from the package", file.target);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = fs::metadata(root.join(&file.target))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111
                    != 0;
                assert_eq!(
                    executable,
                    file.mode == super::super::ProjectedFileMode::Executable,
                    "{} has the wrong executable mode",
                    file.target
                );
            }
        }
    }

    #[test]
    fn test_profile_applies_to_neutral_repo_and_ordinary_renderers_consume_config() {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        storage.init().unwrap();
        let executor = CommandExecutor::new(storage.clone());
        executor
            .seed_project_config(
                temp.path(),
                &HierarchyTemplate::default().generate_config_toml(),
            )
            .unwrap();
        fs::write(temp.path().join("AGENTS.md"), b"# Existing guidance\n").unwrap();

        let package = jit_dogfood_package().unwrap();
        let applied = executor.apply_embedded_profile(&package).unwrap();
        assert_eq!(
            applied.status,
            super::super::ProfileApplicationStatus::Applied
        );
        assert!(temp
            .path()
            .join(".agents/skills/jit-manage/SKILL.md")
            .is_file());
        assert!(temp.path().join(".jit/invariants.toml").is_file());
        assert!(temp
            .path()
            .join(".jit/reference/content-standards.md")
            .is_file());
        let guidance = fs::read_to_string(temp.path().join("AGENTS.md")).unwrap();
        assert!(guidance.starts_with("# Existing guidance\n\n"));
        assert!(guidance.contains("<!-- jit:dogfood-guidance:begin -->"));
        assert_eq!(guidance.matches("<!-- jit:invariants:begin -->").count(), 1);

        let reloaded = CommandExecutor::new(storage);
        let invariants = reloaded.render_invariants().unwrap();
        assert_eq!(invariants.target, "AGENTS.md");
        assert_eq!(invariants.count, 0);
        let reference = reloaded.render_rules_and_gates().unwrap();
        assert_eq!(reference.target, ".jit/reference/rules-and-gates.md");
        assert_eq!(reference.gates, 6);
        assert!(temp.path().join(&reference.target).is_file());

        let unchanged = reloaded.apply_embedded_profile(&package).unwrap();
        assert_eq!(
            unchanged.status,
            super::super::ProfileApplicationStatus::Unchanged
        );
    }

    #[test]
    fn test_installed_rules_gates_reference_is_derived_from_manifest_registries() {
        let package = jit_dogfood_package().unwrap();
        let rule_values = package
            .manifest()
            .contributions
            .iter()
            .filter_map(|contribution| match contribution {
                Contribution::KeyedArray {
                    target: KeyedArrayTarget::Rules,
                    value,
                    ..
                } => Some(value.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        let rules_toml = toml::to_string(&serde_json::json!({ "rules": rule_values })).unwrap();
        let temp = TempDir::new().unwrap();
        fs::create_dir_all(temp.path().join("schemas")).unwrap();
        fs::write(
            temp.path().join("schemas/jit-content-standards.json"),
            package
                .source_bytes("assets/install/.jit/schemas/jit-content-standards.json")
                .unwrap(),
        )
        .unwrap();
        let rules = RuleSet::from_toml_str(&rules_toml, temp.path()).unwrap();
        let mut gates = GateRegistry::default();
        for key in [
            "plan-review",
            "breakdown-review",
            "code-review",
            "coverage-preview",
            "jit-validate",
            "repo-validate",
        ] {
            gates
                .gates
                .insert(key.to_string(), jit_dogfood_gate(key).unwrap());
        }
        let expected = render_rules_and_gates_markdown(&rules, &gates, ProjectionStyle::Full);
        assert_eq!(
            package
                .source_bytes("assets/install/.jit/reference/rules-and-gates.md")
                .unwrap(),
            expected.as_bytes()
        );
    }
}
