//! Embedded production package for jit's repository-neutral dogfood workflow.

use super::{Contribution, EmbeddedProfilePackage, KeyedArrayTarget, ProfilePackageError};
use crate::declarations::GateDefinition;
use include_dir::{include_dir, Dir};

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
pub fn jit_dogfood_gate(key: &str) -> Result<GateDefinition, DogfoodProfileError> {
    let package = jit_dogfood_package()?;
    let value = package
        .manifest()
        .contributions
        .iter()
        .find_map(|contribution| match contribution {
            Contribution::KeyedArray {
                target: KeyedArrayTarget::Gates,
                value,
                ..
            } if value.get("key").and_then(serde_json::Value::as_str) == Some(key) => {
                Some(value.clone())
            }
            _ => None,
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::config::ProjectionStyle;
    use crate::declarations::GateRegistry;
    use crate::hierarchy_templates::HierarchyTemplate;
    use crate::profile::{Contribution, KeyedArrayTarget, MapEntryTarget};
    use crate::repository_state::render_rules_and_gates_markdown;
    use crate::storage::{IssueStore, JsonFileStorage};
    use std::collections::{BTreeMap, BTreeSet};
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
    fn test_packaged_skills_only_require_declared_project_item_kinds() {
        let package = jit_dogfood_package().unwrap();
        let declares_charter = package.manifest().contributions.iter().any(|contribution| {
            matches!(
                contribution,
                Contribution::MapEntry {
                    target: MapEntryTarget::ItemKinds,
                    identity,
                    ..
                } if identity == "charter"
            )
        });
        let requires_charter_address = package
            .manifest()
            .assets
            .iter()
            .filter(|asset| asset.target.starts_with(".agents/skills/jit-project-lead/"))
            .any(|asset| {
                package.source_bytes(&asset.source).is_some_and(|bytes| {
                    bytes
                        .windows(b"@/charter/".len())
                        .any(|w| w == b"@/charter/")
                })
            });

        assert!(
            declares_charter || !requires_charter_address,
            "the project-lead skill must not require an undeclared charter item kind"
        );
    }

    #[test]
    fn test_packaged_content_standard_paths_are_repository_root_relative() {
        let package = jit_dogfood_package().unwrap();
        let prompt = package
            .source_bytes("assets/live/.agents/skills/jit-breakdown/references/analysis-prompt.md")
            .unwrap();
        let prompt = std::str::from_utf8(prompt).unwrap();
        assert!(prompt.contains("resolved from the repository root"));
        assert!(!prompt.contains("relative to this prompt file"));
    }

    #[test]
    fn test_installed_paths_do_not_require_jq() {
        let package = jit_dogfood_package().unwrap();
        for asset in &package.manifest().assets {
            let text = std::str::from_utf8(package.source_bytes(&asset.source).unwrap())
                .unwrap_or_default();
            let mentions_jq = text
                .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
                .any(|token| token == "jq");
            assert!(
                !mentions_jq,
                "{} still exposes a jq dependency",
                asset.target
            );
        }
    }

    #[test]
    fn test_packaged_qualified_item_citations_resolve_from_installed_registries() {
        let package = jit_dogfood_package().unwrap();
        let mut known = BTreeMap::<String, BTreeSet<String>>::new();
        for contribution in &package.manifest().contributions {
            if let Contribution::KeyedArray {
                target, identity, ..
            } = contribution
            {
                let kind = match target {
                    KeyedArrayTarget::Rules => Some("rule"),
                    KeyedArrayTarget::Gates => Some("gate"),
                    KeyedArrayTarget::Templates => None,
                };
                if let Some(kind) = kind {
                    known
                        .entry(kind.to_string())
                        .or_default()
                        .insert(identity.clone());
                }
            }
        }
        let invariants: toml::Value = toml::from_str(
            std::str::from_utf8(
                package
                    .source_bytes("assets/install/.jit/invariants.toml")
                    .unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
        for invariant in invariants
            .get("invariants")
            .and_then(toml::Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(id) = invariant.get("id").and_then(toml::Value::as_str) {
                known
                    .entry("invariant".to_string())
                    .or_default()
                    .insert(id.to_string());
            }
        }
        let citation =
            regex::Regex::new(r"@/([a-z][a-z0-9-]*)/([A-Za-z0-9][A-Za-z0-9-]*)").unwrap();

        for (source, target) in package
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
            let text =
                std::str::from_utf8(package.source_bytes(source).unwrap()).unwrap_or_default();
            for capture in citation.captures_iter(text) {
                let authored_kind = &capture[1];
                let kind = if authored_kind == "inv" {
                    "invariant"
                } else {
                    authored_kind
                };
                let id = &capture[2];
                assert!(
                    known.get(kind).is_some_and(|ids| ids.contains(id)),
                    "{target} cites unresolved package item @/{authored_kind}/{id}"
                );
            }
        }
    }

    #[test]
    fn test_live_assets_cover_source_consumers_and_exclude_install_only_state() {
        let package = jit_dogfood_package().unwrap();
        let live: BTreeSet<&str> = package
            .manifest()
            .assets
            .iter()
            .filter(|asset| asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
            .map(|asset| asset.target.as_str())
            .collect();
        // Live consumers are packaged from `assets/live/`.
        assert!(live.contains(".agents/skills/jit-manage/SKILL.md"));
        assert!(live.contains("scripts/ai-review.sh"));
        let regions: BTreeSet<&str> = package
            .manifest()
            .regions
            .iter()
            .map(|region| region.target.as_str())
            .collect();
        assert!(regions.contains("AGENTS.md"));
        // Install-only adopter state is never a live consumer.
        assert!(!live.contains(".jit/invariants.toml"));
        assert!(!live.contains(".jit/schemas/jit-content-standards.json"));
        assert!(!live.contains(".jit/reference/rules-and-gates.md"));
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
                        crate::declarations::GateChecker::RepositoryValidation
                            | crate::declarations::GateChecker::IssueValidation
                            | crate::declarations::GateChecker::LabelTargetValidation { .. }
                            | crate::declarations::GateChecker::ReviewPlaceholder
                    )
                ),
                "{key} must remain an in-process portable checker"
            );
        }
    }

    #[test]
    fn test_live_assets_match_every_declared_source_tree_consumer() {
        use crate::repository_state::splice_region;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let package = jit_dogfood_package().unwrap();

        // Every live asset is an exact copy of the repo file it consumes, mode included.
        for asset in package
            .manifest()
            .assets
            .iter()
            .filter(|asset| asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
        {
            let live = fs::read(root.join(&asset.target))
                .unwrap_or_else(|error| panic!("failed to read {}: {error}", asset.target));
            assert_eq!(
                live,
                package.source_bytes(&asset.source).unwrap(),
                "{} drifted from the package",
                asset.target
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let executable = fs::metadata(root.join(&asset.target))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o111
                    != 0;
                assert_eq!(
                    executable, asset.executable,
                    "{} has the wrong executable mode",
                    asset.target
                );
            }
        }

        // Each managed region's packaged source matches the live region body, modulo
        // any nested managed sub-region the repo fills (the invariants projection)
        // that the package leaves as a placeholder. Normalizing both sides' invariants
        // sub-region through the same splice makes prose drift the only difference the
        // comparison can surface.
        let inv_begin = "<!-- jit:invariants:begin -->";
        let inv_end = "<!-- jit:invariants:end -->";
        for region in &package.manifest().regions {
            let live = fs::read_to_string(root.join(&region.target)).unwrap();
            let begin = format!("<!-- jit:{}:begin -->", region.region_id);
            let end = format!("<!-- jit:{}:end -->", region.region_id);
            let body_start = live.find(&begin).expect("live region begin") + begin.len() + 1;
            let body_end = live.find(&end).expect("live region end");
            let body = &live[body_start..body_end];
            let source =
                std::str::from_utf8(package.source_bytes(&region.source).unwrap()).unwrap();
            if source.contains(inv_begin) {
                let normalized_live =
                    splice_region(body, "_No invariants declared._", inv_begin, inv_end).unwrap();
                let normalized_source =
                    splice_region(source, "_No invariants declared._", inv_begin, inv_end).unwrap();
                assert_eq!(
                    normalized_live, normalized_source,
                    "{} region prose drifted from the package",
                    region.target
                );
            } else {
                assert_eq!(
                    body.as_bytes(),
                    source.as_bytes(),
                    "{} region drifted from the package",
                    region.target
                );
            }
        }
    }

    #[test]
    fn test_profile_applies_to_neutral_repo_and_ordinary_renderers_consume_config() {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
        let initializer = CommandExecutor::new(storage.clone()).with_layout(layout);
        initializer
            .initialize_fresh_repository(temp.path(), &HierarchyTemplate::default(), None)
            .unwrap();
        fs::write(temp.path().join("AGENTS.md"), b"# Existing guidance\n").unwrap();

        let layout =
            crate::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
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

        let layout =
            crate::storage::discover_repository_layout(temp.path(), temp.path().join(".jit"))
                .unwrap();
        let reloaded = CommandExecutor::new(storage).with_layout(layout);
        let invariants = reloaded.project_render(Some("invariants")).unwrap();
        let invariants = &invariants.projections[0];
        assert_eq!(invariants.target, "AGENTS.md");
        assert_eq!(invariants.mode, "region");
        assert_eq!(invariants.style, "id-anchor");
        assert_eq!(invariants.kinds, ["invariant"]);
        assert_eq!(invariants.count, 0);
        let reference = reloaded.project_render(Some("rules-and-gates")).unwrap();
        let reference = &reference.projections[0];
        assert_eq!(reference.target, ".jit/reference/rules-and-gates.md");
        assert_eq!(reference.mode, "separate-file");
        assert_eq!(reference.style, "full");
        assert_eq!(reference.kinds, ["rule", "gate"]);
        // The separate-file target carries the profile's six gates (registry
        // composition asserted through the rendered `## Gates` section).
        let rendered = fs::read_to_string(temp.path().join(&reference.target)).unwrap();
        assert_eq!(rendered.matches("@/gate/").count(), 6);

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
        std::fs::write(temp.path().join("rules.toml"), &rules_toml).unwrap();
        let rules = crate::storage::ruleset_store::load_ruleset(
            temp.path(),
            &toml::from_str::<crate::config::JitConfig>("").unwrap(),
        )
        .unwrap();
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

    #[test]
    fn test_public_content_standards_redirect_links_profile_installation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let redirect =
            fs::read_to_string(root.join("docs/reference/jit-content-standards.md")).unwrap();
        assert!(redirect.contains("contributors to the JIT source repository"));
        assert!(redirect.contains("Ordinary `jit init` does not install it"));
        assert!(redirect.contains("profile-installed workflow policy"));
        assert!(redirect.contains("repositories that use plain initialization"));
        assert!(redirect.contains("`jit init --profile jit-dogfood`"));
        assert!(redirect.contains("`jit profile apply jit-dogfood`"));
        assert!(redirect.contains("[Repository Profiles](profiles.md)"));
        assert!(!redirect.contains("does not expose a public profile-install command"));
    }

    #[test]
    fn test_planning_bracket_docs_describe_builtin_review_placeholders() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for path in [
            "docs/concepts/planning-bracket.md",
            "docs/how-to/adopt-planning-bracket.md",
            "docs/examples/sdd/config.toml",
            "docs/examples/research/config.toml",
            "docs/reference/cli-commands.md",
        ] {
            let text = fs::read_to_string(root.join(path)).unwrap();
            assert!(
                text.contains("placeholder"),
                "{path} must describe the built-in review placeholders"
            );
            for stale in [
                "agent plan-quality gate",
                "agent breakdown-review gate",
                "agent (command-backed)",
                "JIT_SRC=",
            ] {
                assert!(!text.contains(stale), "{path} retains stale text: {stale}");
            }
        }

        for path in [
            ".jit/templates.toml",
            "docs/concepts/planning-bracket.md",
            "docs/how-to/adopt-planning-bracket.md",
            "docs/examples/sdd/templates.toml",
            "docs/examples/research/templates.toml",
            "docs/reference/cli-commands.md",
            "profiles/jit-dogfood/manifest.toml",
        ] {
            let text = fs::read_to_string(root.join(path)).unwrap();
            for stale in [
                "Agent plan-quality review",
                "approved plan",
                "approved breakdown",
                "breakdown approved",
                "plan is approved",
                "breakdown after plan approved",
            ] {
                assert!(!text.contains(stale), "{path} retains stale text: {stale}");
            }
        }
    }
}
