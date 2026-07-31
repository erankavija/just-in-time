//! Embedded production package for jit's repository-neutral dogfood workflow.

use super::{EmbeddedProfilePackage, ProfilePackageError};
use crate::declarations::GateDefinition;
use crate::repository_state::{Contribution, KeyedArrayTarget};
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
            Contribution::KeyedArray {
                target: KeyedArrayTarget::Templates,
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
    use crate::repository_state::{
        render_rules_and_gates_markdown, Contribution, KeyedArrayTarget, MapEntryTarget,
    };
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
        assert!(live.contains(
            ".agents/skills/jit-planning-lead/references/breakdown-manifest.schema.json"
        ));
        assert!(live.contains(".agents/skills/jit-planning-lead/scripts/breakdown_manifest.py"));
        assert!(live.contains("contrib/gates/ai-review.sh"));
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
        use crate::repository_state::{
            render_managed_document, ManagedDocumentClaim, RegionPlacement,
        };
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
                let claim = ManagedDocumentClaim::Region {
                    owner: "test".into(),
                    region_id: "invariants".into(),
                    begin: inv_begin.as_bytes().to_vec(),
                    end: inv_end.as_bytes().to_vec(),
                    content: b"_No invariants declared._".to_vec(),
                    placement: RegionPlacement::RequireExisting,
                };
                let normalized_live = String::from_utf8(
                    render_managed_document(body.as_bytes(), std::slice::from_ref(&claim)).unwrap(),
                )
                .unwrap();
                let normalized_source = String::from_utf8(
                    render_managed_document(source.as_bytes(), &[claim]).unwrap(),
                )
                .unwrap();
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

    /// Every live asset's repository-relative target paired with the executable
    /// bit the package declares for it.
    ///
    /// Derived from the manifest, so a newly declared live asset joins the
    /// executable-mode contract without an edit here.
    #[cfg(unix)]
    fn live_asset_executable_declarations<'a>(
        package: &'a EmbeddedProfilePackage<'_>,
    ) -> Vec<(&'a str, bool)> {
        package
            .manifest()
            .assets
            .iter()
            .filter(|asset| asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
            .map(|asset| (asset.target.as_str(), asset.executable))
            .collect()
    }

    /// Declarations whose repository file carries the opposite executable bit,
    /// each paired with the mode found under `root`.
    ///
    /// Panics when a declared target is unreadable, since a declaration that
    /// cannot be compared against a file is not a declaration that holds.
    #[cfg(unix)]
    fn executable_mode_mismatches<'a>(
        root: &Path,
        declarations: impl IntoIterator<Item = (&'a str, bool)>,
    ) -> Vec<(&'a str, bool)> {
        use std::os::unix::fs::PermissionsExt;
        declarations
            .into_iter()
            .filter_map(|(target, declared)| {
                let metadata = fs::metadata(root.join(target))
                    .unwrap_or_else(|error| panic!("failed to stat {target}: {error}"));
                let executable = metadata.permissions().mode() & 0o111 != 0;
                (executable != declared).then_some((target, executable))
            })
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn test_executable_mode_mismatches_is_empty_across_every_declared_live_asset() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let package = jit_dogfood_package().unwrap();
        let declarations = live_asset_executable_declarations(&package);
        assert!(
            !declarations.is_empty(),
            "the package declares live assets to check"
        );
        assert_eq!(
            executable_mode_mismatches(&root, declarations),
            Vec::new(),
            "each entry names a live asset whose declared executable bit contradicts its repository file"
        );
    }

    #[cfg(unix)]
    #[test]
    #[should_panic(expected = "failed to stat")]
    fn test_executable_mode_mismatches_panics_when_a_declared_target_is_absent() {
        let package = jit_dogfood_package().unwrap();
        let declarations = live_asset_executable_declarations(&package);
        let empty = TempDir::new().unwrap();
        // No declared target exists under an empty root, so a declaration that
        // cannot be compared against a file is reported rather than skipped.
        let _ = executable_mode_mismatches(empty.path(), declarations);
    }

    #[cfg(unix)]
    #[test]
    fn test_executable_mode_mismatches_reports_a_permission_change_on_a_live_source() {
        use std::os::unix::fs::PermissionsExt;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let package = jit_dogfood_package().unwrap();
        let declarations = live_asset_executable_declarations(&package);

        // A faithful copy of the live sources, mode included, so a permission
        // change can be made without touching the repository.
        let mirror = TempDir::new().unwrap();
        for (target, _) in &declarations {
            let destination = mirror.path().join(target);
            fs::create_dir_all(destination.parent().unwrap()).unwrap();
            fs::copy(root.join(target), &destination).unwrap();
        }
        assert_eq!(
            executable_mode_mismatches(mirror.path(), declarations.clone()),
            Vec::new(),
            "the copy reproduces the repository modes"
        );

        for declared in [true, false] {
            let target = declarations
                .iter()
                .find_map(|(target, executable)| (*executable == declared).then_some(*target))
                .unwrap_or_else(|| panic!("no live asset is declared executable={declared}"));
            let path = mirror.path().join(target);
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            let changed = if declared {
                mode & !0o111
            } else {
                mode | 0o100
            };

            fs::set_permissions(&path, fs::Permissions::from_mode(changed)).unwrap();
            assert_eq!(
                executable_mode_mismatches(mirror.path(), declarations.clone()),
                vec![(target, !declared)],
                "a permission change on {target} alone goes unreported"
            );

            fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
            assert_eq!(
                executable_mode_mismatches(mirror.path(), declarations.clone()),
                Vec::new(),
                "restoring {target}'s mode clears the mismatch"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_executable_mode_mismatches_reports_a_declaration_inverted_against_its_repository_file()
    {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let package = jit_dogfood_package().unwrap();
        let declarations = live_asset_executable_declarations(&package);

        for executable in [true, false] {
            let subject = declarations
                .iter()
                .find_map(|(target, declared)| (*declared == executable).then_some(*target))
                .unwrap_or_else(|| panic!("no live asset is declared executable={executable}"));
            let inverted = declarations.iter().map(|(target, declared)| {
                (
                    *target,
                    if *target == subject {
                        !declared
                    } else {
                        *declared
                    },
                )
            });
            assert_eq!(
                executable_mode_mismatches(&root, inverted),
                vec![(subject, executable)],
                "a declaration claiming executable={} for {subject} goes unreported",
                !executable
            );
        }
    }
}
