//! Repository-local checks and package-source conventions for this checkout's
//! workflow package.

/// Package-source prefix identifying assets that also project into this source tree.
pub const JIT_DOGFOOD_LIVE_SOURCE_PREFIX: &str = "assets/live/";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CommandExecutor;
    use crate::config::ProjectionStyle;
    use crate::declarations::invariants::InvariantRegistry;
    use crate::declarations::GateRegistry;
    use crate::profile::contribution_drift::{DeclaredOverride, OverrideScope};
    use crate::profile::drift_report::{DriftCarrier, DriftReport, DriftSubject};
    use crate::profile::{
        ExclusionPattern, LiveSourceDeclaration, ProfilePackage, RegionDeclaration, RegionPlacement,
    };
    use crate::repository_state::{
        render_managed_document, render_rules_and_gates_markdown, Contribution, KeyedArrayTarget,
        ManagedDocumentClaim, MapEntryTarget,
    };
    use crate::storage::{IssueStore, JsonFileStorage};
    use crate::templates::{GraphTemplate, TemplateRegistry};
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use tempfile::TempDir;

    /// The id of the workflow package this repository declares its own workflow
    /// in, which is the package every assertion below is about.
    const PACKAGE_ID: &str = "jit-dogfood";

    /// This repository's workflow package, assembled from its checkout, with
    /// the directory holding the assembled tree.
    ///
    /// Every assertion here is about the package this repository authors, so
    /// it is assembled from the checkout it is drawn from rather than read from
    /// a copy of it.
    fn assembled_package() -> (TempDir, ProfilePackage) {
        crate::test_utils::temporary_repository_package(PACKAGE_ID)
    }

    /// The runtime gate definition `package`'s manifest declares under `key`.
    ///
    /// Deserializing the declaration through the runtime wire type is the
    /// assertion these callers make about it, so a declaration that is absent
    /// or does not match that type panics naming the key.
    fn declared_gate(package: &ProfilePackage, key: &str) -> crate::declarations::GateDefinition {
        let value = package
            .model()
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
            .unwrap_or_else(|| panic!("the package declares gate '{key}'"));

        serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("package gate '{key}' is a runtime gate: {error}"))
    }

    #[test]
    fn test_jit_dogfood_package_validates_and_has_expected_workflow_inventory() {
        let (_workspace, package) = assembled_package();
        assert_eq!(package.model().id.as_str(), "jit-dogfood");
        assert!(package.file_count() <= super::super::MAX_PROFILE_PACKAGE_FILES);
        assert!(package.byte_size() <= super::super::MAX_PROFILE_PACKAGE_BYTES);

        let gates = package
            .model()
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

        let invariants = package
            .model()
            .contributions
            .iter()
            .filter_map(|contribution| match contribution {
                Contribution::KeyedArray {
                    target: KeyedArrayTarget::Invariants,
                    identity,
                    ..
                } => Some(identity.as_str()),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(invariants.len(), 18);
        assert!(invariants.contains("label-format"));
        assert!(invariants.contains("convention-convergence"));

        let skill_roots = package
            .model()
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
        let (_workspace, package) = assembled_package();
        for declaration in package
            .model()
            .assets
            .iter()
            .map(|asset| (&asset.source, &asset.target))
            .chain(
                package
                    .model()
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
        let (_workspace, package) = assembled_package();
        let declares_charter = package.model().contributions.iter().any(|contribution| {
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
            .model()
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
        let (_workspace, package) = assembled_package();
        let prompt = package
            .source_bytes("assets/live/.agents/skills/jit-breakdown/references/analysis-prompt.md")
            .unwrap();
        let prompt = std::str::from_utf8(prompt).unwrap();
        assert!(prompt.contains("resolved from the repository root"));
        assert!(!prompt.contains("relative to this prompt file"));
    }

    #[test]
    fn test_installed_paths_do_not_require_jq() {
        let (_workspace, package) = assembled_package();
        for asset in &package.model().assets {
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
        let (_workspace, package) = assembled_package();
        let mut known = BTreeMap::<String, BTreeSet<String>>::new();
        for contribution in &package.model().contributions {
            if let Contribution::KeyedArray {
                target, identity, ..
            } = contribution
            {
                let kind = match target {
                    KeyedArrayTarget::Rules => Some("rule"),
                    KeyedArrayTarget::Gates => Some("gate"),
                    KeyedArrayTarget::Invariants => Some("invariant"),
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
        let citation =
            regex::Regex::new(r"@/([a-z][a-z0-9-]*)/([A-Za-z0-9][A-Za-z0-9-]*)").unwrap();

        for (source, target) in package
            .model()
            .assets
            .iter()
            .map(|asset| (&asset.source, &asset.target))
            .chain(
                package
                    .model()
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

    /// Which repository files are packaged is settled by the walk over the
    /// declared live-source roots, so what is left here is the other side: the
    /// adopter state the package installs is never drawn from this repository.
    #[test]
    fn test_live_assets_exclude_install_only_adopter_state() {
        let (_workspace, package) = assembled_package();
        let live: BTreeSet<&str> = live_asset_targets(&package).into_iter().collect();
        assert!(!live.is_empty(), "the package declares live assets");
        let regions: BTreeSet<&str> = package
            .model()
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

    /// Every live asset's repository-relative target.
    fn live_asset_targets(package: &ProfilePackage) -> Vec<&str> {
        package
            .model()
            .assets
            .iter()
            .filter(|asset| asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
            .map(|asset| asset.target.as_str())
            .collect()
    }

    /// Every package-relative source the package authors itself: the
    /// install-only assets and the managed-region sources, which have no
    /// repository counterpart the assembly could draw them from.
    fn package_authored_sources(package: &ProfilePackage) -> Vec<&str> {
        package
            .model()
            .assets
            .iter()
            .filter(|asset| !asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
            .map(|asset| asset.source.as_str())
            .chain(
                package
                    .model()
                    .regions
                    .iter()
                    .map(|region| region.source.as_str()),
            )
            .collect()
    }

    /// The declared roots are exactly the repository domain the live assets are
    /// drawn from: each target falls under one of them, no target falls under
    /// two, and no root is declared that nothing is drawn from.
    #[test]
    fn test_declared_live_source_roots_claim_every_live_asset_target_exactly_once() {
        let (_workspace, package) = assembled_package();
        let roots = &package.model().live_sources;
        assert!(
            !roots.is_empty(),
            "the package declares its live-source roots"
        );

        let targets = live_asset_targets(&package);
        assert!(!targets.is_empty(), "the package declares live assets");
        let unclaimed: Vec<(&str, usize)> = targets
            .iter()
            .map(|target| {
                (
                    *target,
                    roots
                        .iter()
                        .filter(|declaration| declaration.root.relative_path(target).is_some())
                        .count(),
                )
            })
            .filter(|(_, claiming)| *claiming != 1)
            .collect();
        assert_eq!(
            unclaimed,
            Vec::<(&str, usize)>::new(),
            "each entry names a live asset target and the number of declared roots claiming it"
        );

        let empty: Vec<&str> = roots
            .iter()
            .filter(|declaration| {
                !targets
                    .iter()
                    .any(|target| declaration.root.relative_path(target).is_some())
            })
            .map(|declaration| declaration.root.as_str())
            .collect();
        assert_eq!(
            empty,
            Vec::<&str>::new(),
            "each entry names a declared root no live asset is drawn from"
        );
    }

    /// No declared root claims a file the package authors itself.
    ///
    /// The rule is about sources rather than targets, and this package shows
    /// why the distinction is load-bearing: an install-only asset writes into a
    /// declared root, so a rule stated over targets would either forbid that
    /// asset or admit a package-authored source as a live consumer.
    #[test]
    fn test_declared_live_source_roots_claim_no_package_authored_source() {
        let (_workspace, package) = assembled_package();
        let roots = &package.model().live_sources;
        let authored = package_authored_sources(&package);
        assert!(
            !authored.is_empty(),
            "the package authors install-only and region sources"
        );

        let claimed: Vec<&str> = authored
            .iter()
            .copied()
            .filter(|source| {
                roots
                    .iter()
                    .any(|declaration| declaration.root.relative_path(source).is_some())
            })
            .collect();
        assert_eq!(
            claimed,
            Vec::<&str>::new(),
            "each entry names a package-authored source a declared root claims as a live consumer"
        );

        assert!(
            package
                .model()
                .assets
                .iter()
                .filter(|asset| !asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
                .any(|asset| roots
                    .iter()
                    .any(|declaration| declaration.root.relative_path(&asset.target).is_some())),
            "an install-only asset writes into a declared root, which is what makes \
             the source-side rule distinct from a target-side one"
        );
    }

    /// The repository-relative paths the repository at `worktree` tracks.
    ///
    /// Tracked rather than present, so a contributor's scratch file beneath a
    /// packaged root is not read as a packaging defect. The provenance
    /// fixtures' repository-input inventory lists untracked non-ignored paths
    /// beside tracked ones, which is the opposite property, so this listing
    /// takes its own flags rather than that idiom's.
    ///
    /// Panics when the listing cannot be obtained, and when it comes back
    /// empty: a walk over nothing reports nothing, so either would let the
    /// checks below pass by vacuity instead of by the property holding.
    fn tracked_repository_paths(worktree: &Path) -> Vec<String> {
        let listing = Command::new("git")
            .current_dir(worktree)
            .args(["ls-files", "--cached", "--full-name", "-z"])
            .output()
            .unwrap_or_else(|error| {
                panic!(
                    "failed to list the tracked files under {}: {error}",
                    worktree.display()
                )
            });
        assert!(
            listing.status.success(),
            "failed to list the tracked files under {}: {}",
            worktree.display(),
            String::from_utf8_lossy(&listing.stderr)
        );

        let paths: Vec<String> = String::from_utf8(listing.stdout)
            .expect("tracked repository paths are valid UTF-8")
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_string)
            .collect();
        assert!(
            !paths.is_empty(),
            "{} tracks no file, so a walk over the listing would report nothing \
             rather than fail",
            worktree.display()
        );
        paths
    }

    /// Every path in `tracked` that lies beneath a declared root, is packaged
    /// by none of the live-asset targets in `packaged`, and is matched by none
    /// of that root's exclusion patterns.
    ///
    /// The root decides membership and the patterns decide coverage, both over
    /// the same repository-relative path: one glob names a repository directory
    /// of unpackaged material however many files it holds.
    ///
    /// `packaged` holds live-asset targets alone. An install-only asset is
    /// authored by the package rather than drawn from the repository, so a
    /// repository file at such a target is unpackaged material a declared
    /// exclusion covers, not a file the declaration claims.
    fn unpackaged_files_under_declared_roots<'a>(
        roots: &[LiveSourceDeclaration],
        packaged: &BTreeSet<&str>,
        tracked: &'a [String],
    ) -> Vec<&'a str> {
        tracked
            .iter()
            .map(String::as_str)
            .filter(|path| !packaged.contains(path))
            .filter(|path| {
                roots.iter().any(|declaration| {
                    declaration.root.relative_path(path).is_some() && !declaration.excludes(path)
                })
            })
            .collect()
    }

    /// Every path in `tracked` that lies beneath a declared root and is matched
    /// by one of that root's exclusion patterns.
    fn excluded_files_under_declared_roots<'a>(
        roots: &[LiveSourceDeclaration],
        tracked: &'a [String],
    ) -> Vec<&'a str> {
        tracked
            .iter()
            .map(String::as_str)
            .filter(|path| {
                roots.iter().any(|declaration| {
                    declaration.root.relative_path(path).is_some() && declaration.excludes(path)
                })
            })
            .collect()
    }

    /// Every tracked repository file beneath a declared live-source root is
    /// either packaged as a live asset or covered by a declared exclusion.
    ///
    /// This is the direction the asset declarations do not walk: each declared
    /// asset names its repository file, so a live consumer added under a
    /// packaged root and declared nowhere is absent from every adopter install
    /// with nothing reporting it.
    #[test]
    fn test_unpackaged_files_under_declared_roots_is_empty_across_the_tracked_repository_tree() {
        let worktree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (_workspace, package) = assembled_package();
        let roots = &package.model().live_sources;
        let tracked = tracked_repository_paths(&worktree);
        let packaged: BTreeSet<&str> = live_asset_targets(&package).into_iter().collect();

        // The walk's domain covers the whole packaged live surface: every live
        // asset is a tracked repository file beneath a declared root, so no
        // packaged file sits where the walk never looks.
        let unwalked: Vec<&str> = packaged
            .iter()
            .copied()
            .filter(|target| {
                !tracked.iter().any(|path| path == target)
                    || !roots
                        .iter()
                        .any(|declaration| declaration.root.relative_path(target).is_some())
            })
            .collect();
        assert_eq!(
            unwalked,
            Vec::<&str>::new(),
            "each entry names a packaged live asset that is not a tracked file \
             beneath a declared root, so the walk never visits it"
        );

        assert_eq!(
            unpackaged_files_under_declared_roots(roots, &packaged, &tracked),
            Vec::<&str>::new(),
            "each entry names a tracked repository file under a declared \
             live-source root that no live asset packages and no declared \
             exclusion matches. Package it as a live asset, or add a pattern \
             covering its category to that root's exclusions in \
             profiles/jit-dogfood/manifest.toml"
        );
    }

    /// A consumer added under any declared root, packaged nowhere and of no
    /// shape that root's exclusions describe, is reported by name — which is
    /// what fails the walk above, whose assertion is that nothing is reported.
    #[test]
    fn test_unpackaged_files_under_declared_roots_names_a_tracked_file_no_declaration_covers() {
        let (_workspace, package) = assembled_package();
        let roots = &package.model().live_sources;
        assert!(!roots.is_empty(), "the package declares live-source roots");
        let packaged: BTreeSet<&str> = live_asset_targets(&package).into_iter().collect();

        for declaration in roots {
            let added = format!("{}/a-new-consumer/SKILL.md", declaration.root);
            let tracked: Vec<String> = packaged
                .iter()
                .map(|target| (*target).to_string())
                .chain([added.clone()])
                .collect();

            assert_eq!(
                unpackaged_files_under_declared_roots(roots, &packaged, &tracked),
                vec![added.as_str()],
                "a consumer added under {} is not reported by name",
                declaration.root
            );
        }
    }

    /// The walk consults a declaration's patterns only for paths that
    /// declaration's own root contains, so an exclusion cannot reach across
    /// into another root.
    ///
    /// Membership and coverage are separate terms of one conjunction:
    /// `relative_path(path).is_some() && excludes(path)`. Since matching moved
    /// to repository-relative paths, a pattern's text can name a location
    /// outside the root that declares it, which is what makes the membership
    /// term load-bearing rather than decorative — [`LiveSourceDeclaration::excludes`]
    /// alone would match such a path. Package validation rejects a root
    /// declared twice or nested inside another
    /// ([`ProfilePackageError::DuplicateLiveSourceRoot`],
    /// [`ProfilePackageError::NestedLiveSourceRoot`]), so every repository path
    /// lies under exactly one declared root; that is what makes the conjunction
    /// sufficient rather than merely conventional, since there is never a
    /// second declaration whose patterns could also claim the path.
    #[test]
    fn test_unpackaged_files_under_declared_roots_consults_only_the_declaration_owning_the_path() {
        let (_workspace, package) = assembled_package();
        let declared = &package.model().live_sources;
        assert!(
            declared.len() >= 2,
            "the package declares two roots to reach between"
        );
        let packaged: BTreeSet<&str> = live_asset_targets(&package).into_iter().collect();

        // An unpackaged consumer under the last declared root, and a pattern
        // naming everything under that root. Whichever declaration carries the
        // pattern, its text matches the path.
        let owner = declared.last().expect("a declared root");
        let added = format!("{}/a-new-consumer/SKILL.md", owner.root);
        let tracked: Vec<String> = packaged
            .iter()
            .map(|target| (*target).to_string())
            .chain([added.clone()])
            .collect();
        let reaching =
            ExclusionPattern::try_from(format!("{}/**", owner.root)).expect("a compilable pattern");
        assert!(
            reaching.matches(&added),
            "the pattern must match the added path for this test to say anything"
        );

        // Carried by a declaration whose root does not contain the path, the
        // pattern is never consulted and the path is still reported.
        let reaching_across: Vec<LiveSourceDeclaration> = declared
            .iter()
            .enumerate()
            .map(|(position, declaration)| {
                let mut declaration = declaration.clone();
                if position == 0 {
                    declaration.exclude.push(reaching.clone());
                }
                declaration
            })
            .collect();
        assert_eq!(
            unpackaged_files_under_declared_roots(&reaching_across, &packaged, &tracked),
            vec![added.as_str()],
            "a pattern declared under {} suppressed a path under {}",
            reaching_across[0].root,
            owner.root
        );

        // Carried by the declaration that does contain it, the same pattern
        // covers it. The walk's silence turns on which declaration owns the
        // path, not on which one spells a matching pattern.
        let owning: Vec<LiveSourceDeclaration> = declared
            .iter()
            .map(|declaration| {
                let mut declaration = declaration.clone();
                if declaration.root == owner.root {
                    declaration.exclude.push(reaching.clone());
                }
                declaration
            })
            .collect();
        assert_eq!(
            unpackaged_files_under_declared_roots(&owning, &packaged, &tracked),
            Vec::<&str>::new(),
            "the owning root's own exclusion did not cover the path"
        );
    }

    /// The exclusions describe categories rather than files: they cover more
    /// repository files than there are patterns, which a list of path literals
    /// could not do, and none of them shadows a packaged live asset.
    #[test]
    fn test_excluded_files_under_declared_roots_covers_more_files_than_there_are_patterns() {
        let worktree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (_workspace, package) = assembled_package();
        let roots = &package.model().live_sources;
        let tracked = tracked_repository_paths(&worktree);
        let covered = excluded_files_under_declared_roots(roots, &tracked);
        let patterns: usize = roots
            .iter()
            .map(|declaration| declaration.exclude.len())
            .sum();

        assert!(
            covered.len() > patterns,
            "{patterns} declared patterns cover {} repository files, so the \
             declaration reads as a path inventory rather than a set of \
             categories",
            covered.len()
        );

        // A pattern widened until the walk passes would drop a live consumer
        // from the check while leaving it packaged, so no exclusion may reach
        // one.
        let packaged: BTreeSet<&str> = live_asset_targets(&package).into_iter().collect();
        let shadowed: Vec<&str> = covered
            .iter()
            .copied()
            .filter(|path| packaged.contains(path))
            .collect();
        assert_eq!(
            shadowed,
            Vec::<&str>::new(),
            "each entry names a packaged live asset that a declared exclusion \
             also matches"
        );
    }

    /// Every declared pattern is authored beneath the root that declares it.
    ///
    /// Patterns and roots are both repository-relative, which is what lets a
    /// pattern read as the repository location it names — and what allows one
    /// to be written outside its own root, where the walk consults it for no
    /// path and it silently covers nothing. A pattern's text has to open with
    /// its root for the exclusion to bound the root it is declared under.
    #[test]
    fn test_excluded_files_under_declared_roots_matches_only_patterns_authored_beneath_their_root()
    {
        let (_workspace, package) = assembled_package();
        let roots = &package.model().live_sources;

        let stray: Vec<(&str, &str)> = roots
            .iter()
            .flat_map(|declaration| {
                declaration
                    .exclude
                    .iter()
                    .map(|pattern| (declaration.root.as_str(), pattern.as_str()))
            })
            .filter(|(root, pattern)| !pattern.starts_with(&format!("{root}/")))
            .collect();
        assert_eq!(
            stray,
            Vec::<(&str, &str)>::new(),
            "each entry pairs a declared root with a pattern authored outside \
             it, which the walk consults for no path"
        );

        // The patterns are repository-relative, so each one matches the very
        // paths the walk hands it: a root-relative spelling of the same
        // category would match nothing.
        let worktree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tracked = tracked_repository_paths(&worktree);
        assert!(
            !excluded_files_under_declared_roots(roots, &tracked).is_empty(),
            "no declared pattern matches any repository path"
        );
    }

    #[test]
    #[should_panic(expected = "failed to list the tracked files under")]
    fn test_tracked_repository_paths_panics_when_the_listing_cannot_be_obtained() {
        let unlistable = TempDir::new().unwrap();
        // A malformed gitfile stops the repository search here and fails it,
        // whatever the enclosing directories are.
        fs::write(unlistable.path().join(".git"), b"not a gitfile\n").unwrap();

        let _ = tracked_repository_paths(unlistable.path());
    }

    #[test]
    #[should_panic(expected = "tracks no file")]
    fn test_tracked_repository_paths_panics_when_the_listing_is_empty() {
        let empty = TempDir::new().unwrap();
        let initialized = Command::new("git")
            .current_dir(empty.path())
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(initialized.success(), "the fixture repository initializes");

        let _ = tracked_repository_paths(empty.path());
    }

    #[test]
    fn test_managed_region_sources_stay_outside_live_asset_prefix() {
        let (_workspace, package) = assembled_package();
        assert!(!package.model().regions.is_empty());
        assert!(package
            .model()
            .regions
            .iter()
            .all(|region| !region.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX)));
    }

    #[test]
    fn test_checked_in_package_has_no_live_source_files() {
        let package_sources = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../profiles/jit-dogfood")
            .join(JIT_DOGFOOD_LIVE_SOURCE_PREFIX);
        fn contains_file(path: &Path) -> bool {
            let entries = match fs::read_dir(path) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return false,
                Err(error) => panic!("failed to inspect package sources beneath {path:?}: {error}"),
            };
            entries.flatten().any(|entry| {
                let path = entry.path();
                path.is_file() || (path.is_dir() && contains_file(&path))
            })
        }
        assert!(
            !contains_file(&package_sources),
            "checked-in package sources must not contain live asset files: {}",
            package_sources.display()
        );
    }

    #[test]
    fn test_all_package_gate_values_deserialize_as_runtime_gates() {
        let (_workspace, package) = assembled_package();
        for key in [
            "plan-review",
            "breakdown-review",
            "code-review",
            "coverage-preview",
            "jit-validate",
            "repo-validate",
        ] {
            let gate = declared_gate(&package, key);
            assert_eq!(gate.key, key);
            assert!(
                matches!(
                    gate.checker,
                    Some(
                        crate::declarations::GateChecker::RepositoryValidation
                            | crate::declarations::GateChecker::IssueValidation
                            | crate::declarations::GateChecker::LabelTargetValidation { .. }
                            | crate::declarations::GateChecker::RuleValidation { .. }
                            | crate::declarations::GateChecker::ReviewPlaceholder
                    )
                ),
                "{key} must remain an in-process portable checker"
            );
        }
    }

    /// Compare a rendered managed region with its packaged source through the
    /// shared drift report, normalizing the nested invariants projection when
    /// the package source carries it.
    fn managed_region_drift_report(
        region: &RegionDeclaration,
        rendered: &str,
        packaged: &str,
    ) -> Option<DriftReport> {
        let subject = DriftSubject {
            repository: DriftCarrier::new(
                region.target.clone(),
                format!("region '{}'", region.region_id),
            ),
            packaged: DriftCarrier::new(
                region.source.clone(),
                format!("region '{}'", region.region_id),
            ),
            field_root: format!("region '{}'", region.region_id),
            remedy: format!(
                "edit the rendered region in '{}' and the packaged source in '{}' together so both carriers state the same region body",
                region.target, region.source
            ),
        };
        let rendered = normalized_managed_region_body(rendered, packaged);
        let packaged = normalized_managed_region_body(packaged, packaged);
        let differences = if rendered == packaged {
            Vec::new()
        } else {
            vec![format!(
                "{}: the repository declares {rendered:?}, the package declares {packaged:?}",
                subject.field_root
            )]
        };

        subject.reporting(differences)
    }

    /// Apply the invariants placeholder splice used by the live repository
    /// check to a region body when its packaged source declares that child.
    fn normalized_managed_region_body(body: &str, packaged: &str) -> String {
        let inv_begin = "<!-- jit:invariants:begin -->";
        let inv_end = "<!-- jit:invariants:end -->";
        if !packaged.contains(inv_begin) {
            return body.to_string();
        }

        let claims = [ManagedDocumentClaim::Region {
            owner: "test".into(),
            region_id: "invariants".into(),
            begin: inv_begin.as_bytes().to_vec(),
            end: inv_end.as_bytes().to_vec(),
            content: b"_No invariants declared._".to_vec(),
            placement: crate::repository_state::RegionPlacement::RequireExisting,
        }];
        String::from_utf8(render_managed_document(body.as_bytes(), &claims).unwrap()).unwrap()
    }

    fn region_declaration(source: &str, target: &str, region_id: &str) -> RegionDeclaration {
        RegionDeclaration {
            source: source.to_string(),
            target: target.to_string(),
            region_id: region_id.to_string(),
            placement: RegionPlacement::Append,
        }
    }

    #[test]
    fn test_managed_region_drift_report_is_silent_when_bodies_agree() {
        let region = region_declaration(
            "profiles/example/assets/region.md",
            "docs/example.md",
            "example",
        );

        assert!(managed_region_drift_report(&region, "the same body", "the same body").is_none());
    }

    #[test]
    fn test_managed_region_drift_report_names_rendered_and_packaged_carriers() {
        let region = region_declaration(
            "profiles/example/assets/region.md",
            "docs/example.md",
            "example",
        );
        let report = managed_region_drift_report(&region, "the rendered body", "the packaged body")
            .expect("different bodies are drift");

        assert_eq!(report.repository().path, region.target);
        assert_eq!(report.packaged().path, region.source);
        assert!(report.repository().entry.contains("example"));
        assert!(report.packaged().entry.contains("example"));
        let rendered = report.to_string();
        assert!(rendered.contains("repository: docs/example.md"));
        assert!(rendered.contains("packaged:   profiles/example/assets/region.md"));
        assert!(rendered.contains("edit the rendered region"));
    }

    #[test]
    fn test_managed_region_drift_report_attributes_differing_text_to_each_carrier() {
        let region = region_declaration(
            "profiles/example/assets/region.md",
            "docs/example.md",
            "example",
        );
        let report = managed_region_drift_report(&region, "the rendered body", "the packaged body")
            .expect("different bodies are drift");
        let difference = report.differences().join("\n");

        assert!(difference.contains("the repository declares \"the rendered body\""));
        assert!(
            difference.contains("the package declares \"the packaged body\""),
            "{difference}"
        );
    }

    #[test]
    fn test_managed_region_drift_report_normalizes_a_nested_managed_subregion_on_both_sides() {
        let region = region_declaration(
            "profiles/example/assets/region.md",
            "docs/example.md",
            "example",
        );
        let rendered = "before\n<!-- jit:invariants:begin -->\n- repository projection\n<!-- jit:invariants:end -->\nafter\n";
        let packaged = "before\n<!-- jit:invariants:begin -->\n_No invariants declared._\n<!-- jit:invariants:end -->\nafter\n";

        assert!(managed_region_drift_report(&region, rendered, packaged).is_none());
    }

    #[test]
    fn test_managed_region_drift_report_compares_every_manifest_region_declaration() {
        let (_workspace, package) = assembled_package();
        let expected = package
            .model()
            .regions
            .iter()
            .map(|region| (region.target.clone(), region.source.clone()))
            .collect::<BTreeSet<_>>();
        let compared = package
            .model()
            .regions
            .iter()
            .map(|region| {
                let source =
                    std::str::from_utf8(package.source_bytes(&region.source).unwrap()).unwrap();
                let report = managed_region_drift_report(
                    region,
                    &format!("{source}\nrepository-only text"),
                    source,
                )
                .expect("the synthetic repository-only text is drift");
                (
                    report.repository().path.clone(),
                    report.packaged().path.clone(),
                )
            })
            .collect::<BTreeSet<_>>();

        assert_eq!(compared, expected);
    }

    #[test]
    fn test_managed_regions_match_every_declared_source_tree_consumer() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (_workspace, package) = assembled_package();

        // Each managed region's packaged source matches the live region body, modulo
        // any nested managed sub-region the repo fills (the invariants projection)
        // that the package leaves as a placeholder. Normalizing both sides' invariants
        // sub-region through the same splice makes prose drift the only difference the
        // comparison can surface.
        for region in &package.model().regions {
            let live = fs::read_to_string(root.join(&region.target)).unwrap();
            let begin = format!("<!-- jit:{}:begin -->", region.region_id);
            let end = format!("<!-- jit:{}:end -->", region.region_id);
            let body_start = live.find(&begin).expect("live region begin") + begin.len() + 1;
            let body_end = live.find(&end).expect("live region end");
            let body = &live[body_start..body_end];
            let source =
                std::str::from_utf8(package.source_bytes(&region.source).unwrap()).unwrap();
            if let Some(report) = managed_region_drift_report(region, body, source) {
                panic!("{report}");
            }
        }
    }

    /// This repository's committed template registry carries the packaged
    /// declarations: its generated region equals the render, the render owns
    /// that region and no byte outside it, the result loads, and rendering
    /// again changes nothing.
    ///
    /// The assertion never writes. Generation is the generator script the
    /// failure message names, so an edit to either declaration alone fails here
    /// instead of being repaired.
    #[test]
    fn test_committed_template_registry_carries_the_packaged_declarations() {
        use crate::profile::template_region::{
            outside_template_region, packaged_templates, render_template_block,
            render_template_registry, splice_template_region, TEMPLATE_REGION_GENERATOR,
            TEMPLATE_REGISTRY_PATH,
        };
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let existing = fs::read(root.join(TEMPLATE_REGISTRY_PATH)).unwrap();
        let (_workspace, package) = assembled_package();
        let block = render_template_block(&packaged_templates(&package).unwrap()).unwrap();
        // The render the generator publishes, so the two cannot disagree about
        // what the registry should hold.
        let rendered = render_template_registry(&existing, &package).unwrap();

        // The committed registry IS the render. On drift the generator, not
        // this assertion, is what brings the two back into agreement.
        assert_eq!(
            String::from_utf8(rendered.clone()).unwrap(),
            String::from_utf8(existing.clone()).unwrap(),
            "{TEMPLATE_REGISTRY_PATH} no longer carries the packaged template \
             declarations. The block between the region delimiters is \
             generated from profiles/jit-dogfood/manifest.toml: edit the \
             packaged declaration, then regenerate with {TEMPLATE_REGION_GENERATOR}"
        );

        // Rendering owns the region and nothing else.
        assert_eq!(
            outside_template_region(&rendered).unwrap(),
            outside_template_region(&existing).unwrap(),
            "rendering changed registry bytes outside the region delimiters"
        );

        // Whatever the region happens to hold, the packaged block replaces it
        // whole while the authored bytes around it survive: the region body is
        // read from the package rather than carried over from the file.
        let stale =
            splice_template_region(&existing, "# a declaration the package does not make\n")
                .unwrap();
        assert_eq!(
            outside_template_region(&stale).unwrap(),
            outside_template_region(&existing).unwrap(),
            "replacing the region changed registry bytes outside the delimiters"
        );
        assert_eq!(
            splice_template_region(&stale, &block).unwrap(),
            rendered,
            "rendering over a diverged region did not restore the packaged block"
        );

        // The rendered registry loads, and every container type a declaration
        // brackets is still bracketed by that declaration.
        let unchecked_hierarchy: [&str; 0] = [];
        let before = TemplateRegistry::from_toml_str(
            std::str::from_utf8(&existing).unwrap(),
            &unchecked_hierarchy,
        )
        .unwrap();
        let after = TemplateRegistry::from_toml_str(
            std::str::from_utf8(&rendered).unwrap(),
            &unchecked_hierarchy,
        )
        .unwrap();
        for template in &before.templates {
            for container_type in &template.applies_to {
                assert_eq!(
                    after
                        .template_for_container(container_type)
                        .map(|applied| applied.name.as_str()),
                    Some(template.name.as_str()),
                    "the rendered registry stops bracketing container type '{container_type}'"
                );
            }
        }

        // A second rendering run finds nothing to change.
        assert_eq!(
            splice_template_region(&rendered, &block).unwrap(),
            rendered,
            "a second rendering run changed the registry"
        );
    }

    /// The declarations this repository deliberately holds differently from
    /// the packaged contribution that names them.
    ///
    /// Every other contribution is bound: a packaged value and the registry
    /// entry it restates are two copies of one declaration, and an edit to
    /// either alone is reported. An entry here says the two are not copies of
    /// one declaration at all, and why. Each is as narrow as its reason —
    /// unless the whole entry is this repository's own, only the single field
    /// the two carriers state differently is named, and every remaining field
    /// of that entry stays bound.
    const DECLARED_OVERRIDES: &[DeclaredOverride] = &[
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Gates),
            identity: None,
            field: Some("checker"),
            reason: "The package ships every review gate as a portable placeholder an \
                     adopter replaces without changing the workflow graph, which the \
                     manifest declares above its gate contributions. This checkout is \
                     such an adopter: each of its gates runs a real command.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Gates),
            identity: None,
            field: Some("title"),
            reason: "A replaced checker is titled for the command it runs rather than \
                     for the placeholder it replaced.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Gates),
            identity: None,
            field: Some("description"),
            reason: "A replaced checker is described by what it runs; the packaged \
                     description is bound instead to the reference the package \
                     installs, which is rendered from the same contributions.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Invariants),
            identity: None,
            field: Some("enforced-by"),
            reason: "The checkout-only cargo-ci gate is not package content, so the \
                     packaged enforcement claims bind to the portable repo-validate \
                     gate instead, which the manifest declares above its invariant \
                     contributions. The statement each entry carries stays bound.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Rules),
            identity: None,
            field: Some("origin"),
            reason: "A rule's origin records which declaration authored it, so a rule \
                     this repository authored before the package carries its own \
                     provenance marker rather than the package's.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::KeyedArray(KeyedArrayTarget::Rules),
            identity: None,
            field: Some("description"),
            reason: "Each carrier describes the rule for its own reader; the packaged \
                     description is bound instead to the reference the package \
                     installs, which is rendered from the same contributions. What the \
                     rule asserts stays bound.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::MapEntry(MapEntryTarget::Namespaces),
            identity: None,
            field: Some("examples"),
            reason: "Namespace examples are documentation and never enforced, so each \
                     carrier names its own vocabulary: this repository's examples cite \
                     its own issues, which an adopter has no counterpart for.",
        },
        DeclaredOverride {
            package: PACKAGE_ID,
            scope: OverrideScope::Projection,
            identity: Some("rules-and-gates"),
            field: None,
            reason: "The two are different projections of one registry rather than one \
                     projection declared twice: this repository renders the reference \
                     into a region of its published documentation, and the package \
                     installs it as a separate file under .jit/reference/.",
        },
    ];

    /// This repository's registries, keyed by the repository-relative path the
    /// contributions target.
    ///
    /// Panics when a targeted registry cannot be read: a registry the
    /// comparison cannot consult is a contribution left unbound, which is what
    /// the check exists to prevent.
    fn contributed_registries(contributions: &[Contribution]) -> BTreeMap<String, String> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        crate::profile::contribution_drift::contributed_registry_paths(contributions)
            .into_iter()
            .map(|registry| {
                let text = fs::read_to_string(root.join(registry))
                    .unwrap_or_else(|error| panic!("failed to read {registry}: {error}"));
                (registry.to_string(), text)
            })
            .collect()
    }

    /// Every package this repository publishes, assembled from the checkout,
    /// with the directories holding the assembled trees.
    ///
    /// The set comes from the checkout's package-source listing, so a package
    /// added there joins the walks below without an edit here — which is the
    /// property that keeps a second published package from going unbound the
    /// way the first one did.
    fn published_packages() -> (Vec<TempDir>, Vec<(String, ProfilePackage)>) {
        crate::test_utils::published_package_ids()
            .into_iter()
            .map(|id| {
                let (workspace, package) = crate::test_utils::temporary_repository_package(&id);
                (workspace, (id, package))
            })
            .unzip()
    }

    /// Every contribution of `package` whose repository counterpart states
    /// something else, given the registries the repository carries.
    fn contribution_drift(
        id: &str,
        package: &ProfilePackage,
        registries: &BTreeMap<String, String>,
    ) -> Vec<crate::profile::drift_report::DriftReport> {
        crate::profile::contribution_drift::contribution_drift_reports(
            id,
            &package.model().contributions,
            registries,
            DECLARED_OVERRIDES,
        )
        .expect("every contributed registry entry is comparable")
    }

    /// Every contribution of every published package, and the reports they
    /// produce against the registries this repository carries.
    fn published_contribution_drift(
        packages: &[(String, ProfilePackage)],
    ) -> Vec<crate::profile::drift_report::DriftReport> {
        packages
            .iter()
            .flat_map(|(id, package)| {
                let registries = contributed_registries(&package.model().contributions);
                contribution_drift(id, package, &registries)
            })
            .collect()
    }

    /// Every contribution of every package this repository publishes agrees
    /// with the repository registry entry it restates.
    ///
    /// This is the direction the manifests do not walk: a contribution and the
    /// entry it restates are two copies of one declaration, and nothing else
    /// keeps them in step. The repository's copy is what this checkout's own
    /// validation reads and the packaged copy is what an adopter receives, so
    /// once they come apart one of them carries a ruling the other never
    /// recorded. Binding one package and not the next left exactly that gap.
    #[test]
    fn test_contribution_drift_is_empty_across_every_published_package() {
        let (_workspaces, packages) = published_packages();
        assert!(
            packages.len() >= 2,
            "this repository publishes more than one package, which is what makes \
             the walk over the published set distinct from a walk over one package"
        );

        let reported = published_contribution_drift(&packages)
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        assert_eq!(
            reported,
            Vec::<String>::new(),
            "each report names a packaged contribution and the repository entry it \
             restates, which no longer state the same thing"
        );
    }

    /// Every published package contributes to a registry the walk reads, so no
    /// package is carried by the walk without being compared.
    #[test]
    fn test_every_published_package_contributes_to_a_registry_the_walk_reads() {
        let (_workspaces, packages) = published_packages();

        let uncompared: Vec<&str> = packages
            .iter()
            .filter(|(_, package)| {
                contributed_registries(&package.model().contributions).is_empty()
            })
            .map(|(id, _)| id.as_str())
            .collect();

        assert_eq!(
            uncompared,
            Vec::<&str>::new(),
            "each entry names a published package the walk visits and compares \
             nothing for"
        );
    }

    /// Every target the manifest contributes to is bound: an edit to the
    /// packaged declaration alone is reported for each of them.
    ///
    /// A comparison that reached only some targets would pass this repository
    /// while leaving the rest free to drift, which is the state the check
    /// replaces. The seeded value is a field of the contribution itself, so no
    /// target is bound by an expectation stated here about what it declares.
    #[test]
    fn test_contribution_drift_compares_a_seeded_edit_on_every_declared_target() {
        let (_workspaces, packages) = published_packages();

        // Each target is named with the package declaring it, so a target one
        // package binds does not vouch for the same target in another.
        let declared: BTreeSet<(String, String)> = packages
            .iter()
            .flat_map(|(id, package)| {
                package
                    .model()
                    .contributions
                    .iter()
                    .map(|contribution| (id.clone(), declared_target(contribution)))
            })
            .collect();
        assert!(
            !declared.is_empty(),
            "the published packages declare contributions"
        );

        let bound: BTreeSet<(String, String)> = packages
            .iter()
            .flat_map(|(id, package)| {
                let contributions = &package.model().contributions;
                let registries = contributed_registries(contributions);
                contributions
                    .iter()
                    .filter(|contribution| {
                        seeded_packaged_edits(contribution)
                            .iter()
                            .any(|seeded| seeded_drift(id, seeded, &registries).len() == 1)
                    })
                    .map(|contribution| (id.clone(), declared_target(contribution)))
                    .collect::<Vec<_>>()
            })
            .collect();

        assert_eq!(
            bound, declared,
            "each pair the two sets differ by names a package and a target where a \
             seeded edit to the packaged declaration went unreported, so nothing \
             binds it"
        );
    }

    /// An edit to the repository's registry alone is reported, naming both
    /// carriers and the differing text.
    ///
    /// The edit is made the way a hand would make it — the entry's own text is
    /// rewritten in the registry file — and the check reads the entry back out
    /// of that text, so no seam stands between the seeded edit and the
    /// comparison. The entry is the one whose amendment went unreported for
    /// want of this check.
    #[test]
    fn test_contribution_drift_report_names_both_carriers_of_a_seeded_repository_edit() {
        let (_workspace, package) = assembled_package();
        let contributions = &package.model().contributions;
        let registry = KeyedArrayTarget::Invariants.registry_path();
        let mut registries = contributed_registries(contributions);
        let committed = registries
            .get(registry)
            .expect("the invariant registry")
            .clone();
        registries.insert(
            registry.to_string(),
            seeded_invariant_statement(&committed, "domain-agnostic"),
        );

        let reported = contribution_drift(PACKAGE_ID, &package, &registries);

        assert_eq!(reported.len(), 1, "{reported:?}");
        let report = &reported[0];
        assert_eq!(report.repository().path, registry);
        assert_eq!(
            report.packaged().path,
            crate::profile::contribution_drift::packaged_manifest_path(PACKAGE_ID)
        );
        let rendered = report.to_string();
        assert!(rendered.contains(SEEDED_EDIT), "{rendered}");
        for carrier in [report.repository(), report.packaged()] {
            assert!(rendered.contains(&carrier.path), "{rendered}");
            assert!(rendered.contains(&carrier.entry), "{rendered}");
        }
    }

    /// Every declared override still names a contribution its own package
    /// makes and, when it names a field, one that contribution declares.
    ///
    /// An override outliving the declaration it excuses would go on suppressing
    /// a comparison nothing asked for. Because an override names its package,
    /// an entry whose reason was written about one package and whose package no
    /// longer declares the contribution is reported here even while another
    /// package declares the same thing.
    #[test]
    fn test_declared_overrides_each_name_a_contribution_its_package_still_declares() {
        let (_workspaces, packages) = published_packages();

        let stale: Vec<(&str, &str)> = DECLARED_OVERRIDES
            .iter()
            .filter(|declared| {
                !packages.iter().any(|(id, package)| {
                    package
                        .model()
                        .contributions
                        .iter()
                        .any(|contribution| declared.applies_to(id, contribution))
                })
            })
            .map(|declared| (declared.package, declared.reason))
            .collect();

        assert_eq!(
            stale,
            Vec::<(&str, &str)>::new(),
            "each entry pairs a package with a declared override whose contribution \
             or field that package no longer declares"
        );
    }

    /// The text a seeded edit writes into a declaration.
    const SEEDED_EDIT: &str = "a value the package does not carry";

    /// The target a contribution declares, spelled the way the manifest spells
    /// it, so the walk above states its coverage in the manifest's vocabulary.
    fn declared_target(contribution: &Contribution) -> String {
        match serde_json::to_value(contribution) {
            Ok(serde_json::Value::Object(fields)) => fields
                .get("target")
                .or_else(|| fields.get("kind"))
                .and_then(|target| target.as_str().map(str::to_string)),
            _ => None,
        }
        .expect("a contribution declares its target")
    }

    /// The reports one edited contribution produces against `registries`.
    ///
    /// A seed that leaves the declaration uninterpretable — a token that is not
    /// one of a closed vocabulary, say — reports nothing here and the walk
    /// above tries the next field.
    fn seeded_drift(
        package: &str,
        contribution: &Contribution,
        registries: &BTreeMap<String, String>,
    ) -> Vec<crate::profile::drift_report::DriftReport> {
        crate::profile::contribution_drift::contribution_drift_reports(
            package,
            std::slice::from_ref(contribution),
            registries,
            DECLARED_OVERRIDES,
        )
        .unwrap_or_default()
    }

    /// Every way this walk knows to edit one packaged declaration away from the
    /// repository's, one edited contribution per field it can change.
    ///
    /// The field values are the contribution's own, changed in place and in
    /// kind, so a declaration of any shape yields candidates without this
    /// helper knowing what its fields mean.
    fn seeded_packaged_edits(contribution: &Contribution) -> Vec<Contribution> {
        let value = crate::profile::contribution_drift::contributed_value(contribution);
        let candidates: Vec<serde_json::Value> = match &value {
            serde_json::Value::Object(fields) => fields
                .iter()
                .filter_map(|(key, field)| {
                    let mut edited = fields.clone();
                    edited.insert(key.clone(), seeded_value(field)?);
                    Some(serde_json::Value::Object(edited))
                })
                .collect(),
            scalar => seeded_value(scalar).into_iter().collect(),
        };
        candidates
            .into_iter()
            .filter_map(|value| reseated(contribution, value))
            .collect()
    }

    /// `value` changed in place and in kind, or `None` when this walk has no
    /// change for its shape.
    fn seeded_value(value: &serde_json::Value) -> Option<serde_json::Value> {
        use serde_json::Value;
        match value {
            Value::String(_) => Some(Value::String(SEEDED_EDIT.to_string())),
            Value::Bool(carried) => Some(Value::Bool(!carried)),
            Value::Number(carried) => carried
                .as_i64()
                .map(|carried| Value::Number((carried + 1).into())),
            Value::Array(carried) => Some(Value::Array(
                [
                    carried.clone(),
                    vec![Value::String(SEEDED_EDIT.to_string())],
                ]
                .concat(),
            )),
            Value::Object(_) | Value::Null => None,
        }
    }

    /// `contribution` carrying `value` instead of its own.
    ///
    /// A projection's value is a complete configuration rather than a free
    /// value, so a seed that is not one is no candidate.
    fn reseated(contribution: &Contribution, value: serde_json::Value) -> Option<Contribution> {
        Some(match contribution {
            Contribution::KeyedArray {
                target, identity, ..
            } => Contribution::KeyedArray {
                target: *target,
                identity: identity.clone(),
                value,
            },
            Contribution::MapEntry {
                target, identity, ..
            } => Contribution::MapEntry {
                target: *target,
                identity: identity.clone(),
                value,
            },
            Contribution::Projection { name, .. } => Contribution::Projection {
                name: name.clone(),
                value: serde_json::from_value(value).ok()?,
            },
            Contribution::Scalar { target, .. } => Contribution::Scalar {
                target: *target,
                value: value.as_str()?.to_string(),
            },
            Contribution::SetString { target, .. } => Contribution::SetString {
                target: *target,
                value: value.as_str()?.to_string(),
            },
        })
    }

    /// `registry` with the statement of invariant `identity` rewritten.
    fn seeded_invariant_statement(registry: &str, identity: &str) -> String {
        let mut document = registry
            .parse::<toml_edit::DocumentMut>()
            .expect("the invariant registry parses");
        let entry = document[KeyedArrayTarget::Invariants.array_name()]
            .as_array_of_tables_mut()
            .expect("the registry declares an invariant array")
            .iter_mut()
            .find(|entry| {
                entry
                    .get(KeyedArrayTarget::Invariants.identity_field())
                    .and_then(toml_edit::Item::as_str)
                    == Some(identity)
            })
            .unwrap_or_else(|| panic!("the repository declares invariant '{identity}'"));
        entry["statement"] = toml_edit::value(SEEDED_EDIT);
        document.to_string()
    }

    /// This repository's committed template registry.
    fn committed_template_registry() -> Vec<u8> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        fs::read(root.join(crate::profile::template_region::TEMPLATE_REGISTRY_PATH)).unwrap()
    }

    /// The declarations a template registry's text carries, as the registry
    /// loader parses them. The type check is left to the repository's own
    /// config load: what is compared here is the declaration, not the hierarchy
    /// it names.
    fn template_declarations_of(registry: &[u8]) -> Vec<GraphTemplate> {
        let unchecked_hierarchy: [&str; 0] = [];
        TemplateRegistry::from_toml_str(
            std::str::from_utf8(registry).unwrap(),
            &unchecked_hierarchy,
        )
        .unwrap()
        .templates
    }

    /// The declarations a registry's generated region carries: the extent the
    /// package is the authority for (`@/issue/e204e63d/decision/D-1`), which is
    /// what the drift guard compares.
    fn region_declarations_of(registry: &[u8]) -> Vec<GraphTemplate> {
        use crate::profile::template_region::inside_template_region;
        template_declarations_of(inside_template_region(registry).unwrap().as_bytes())
    }

    /// The committed region's declarations agree with the packaged ones field
    /// for field, description strings included.
    ///
    /// Each side is compared as the value it parses into — the registry's
    /// generated region through the templates loader, the package through its
    /// manifest — so the assertion carries no expectation of its own about what
    /// the bracket declares, and it holds whether or not the region was
    /// regenerated: a hand edit inside the region fails here as readily as a
    /// change to the packaged authority.
    #[test]
    fn test_committed_template_declarations_agree_with_the_packaged_declarations() {
        use crate::profile::template_region::{packaged_templates, template_drift_report};
        let (_workspace, package) = assembled_package();
        let repository = region_declarations_of(&committed_template_registry());
        let packaged = packaged_templates(&package).unwrap();

        if let Some(report) = template_drift_report(&repository, &packaged).unwrap() {
            panic!("{report}");
        }
    }

    /// A hand edit to the registry's generated region alone is reported, naming
    /// the field it changed.
    ///
    /// The edit is made the way a hand would make it: the changed declaration
    /// is written back into the committed registry's region, and the guard
    /// reads its declarations out of that text.
    #[test]
    fn test_template_drift_report_names_the_field_an_edited_repository_declaration_changed() {
        use crate::profile::template_region::{
            packaged_templates, render_template_block, splice_template_region,
            template_drift_report, TEMPLATE_REGION_GENERATOR,
        };
        let (_workspace, package) = assembled_package();
        let committed = committed_template_registry();
        let mut edited = region_declarations_of(&committed);
        let node = edited
            .first_mut()
            .and_then(|template| template.nodes.first_mut())
            .expect("the repository declares a template node");
        node.description = Some(format!(
            "{} A sentence the package does not carry.",
            node.description.as_deref().unwrap_or_default()
        ));
        let hand_edited =
            splice_template_region(&committed, &render_template_block(&edited).unwrap()).unwrap();

        let report = template_drift_report(
            &region_declarations_of(&hand_edited),
            &packaged_templates(&package).unwrap(),
        )
        .unwrap()
        .expect("an edited repository declaration is drift");

        assert!(
            report.contains("template[0].nodes[0].description"),
            "{report}"
        );
        assert!(
            report.contains("A sentence the package does not carry."),
            "{report}"
        );
        assert!(report.contains(TEMPLATE_REGION_GENERATOR), "{report}");
    }

    /// An edit to the packaged declaration alone is reported, naming the field
    /// it changed.
    #[test]
    fn test_template_drift_report_names_the_field_an_edited_packaged_declaration_changed() {
        use crate::profile::template_region::{
            packaged_templates, template_drift_report, TEMPLATE_REGION_GENERATOR,
        };
        let (_workspace, package) = assembled_package();
        let repository = region_declarations_of(&committed_template_registry());
        let mut packaged = packaged_templates(&package).unwrap();
        let anchor = packaged
            .first_mut()
            .and_then(|template| template.anchors.first_mut())
            .expect("the package declares a template anchor");
        anchor.gates.push("a-gate-the-repository-omits".to_string());
        let appended = anchor.gates.len() - 1;

        let report = template_drift_report(&repository, &packaged)
            .unwrap()
            .expect("an edited packaged declaration is drift");

        assert!(
            report.contains(&format!("template[0].anchors[0].gates[{appended}]")),
            "{report}"
        );
        assert!(report.contains("a-gate-the-repository-omits"), "{report}");
        assert!(report.contains(TEMPLATE_REGION_GENERATOR), "{report}");
    }

    /// A template the repository authors outside the delimiters is not drift.
    ///
    /// The package is the authority for the generated region, not for the
    /// registry's whole template inventory (`@/issue/e204e63d/decision/D-1`),
    /// so a second declaration beyond the region leaves the guard silent even
    /// though the file then carries more declarations than the package does.
    #[test]
    fn test_template_drift_report_ignores_a_template_authored_outside_the_region() {
        use crate::profile::template_region::{
            packaged_templates, render_template_block, template_drift_report,
        };
        let (_workspace, package) = assembled_package();
        let packaged = packaged_templates(&package).unwrap();
        let mut authored = packaged.clone();
        authored
            .first_mut()
            .expect("the package declares a template")
            .name = "authored-outside-the-region".to_string();
        let registry = [
            committed_template_registry(),
            b"\n".to_vec(),
            render_template_block(&authored).unwrap().into_bytes(),
        ]
        .concat();

        assert!(
            template_declarations_of(&registry).len() > region_declarations_of(&registry).len(),
            "the registry under test must carry a declaration beyond its region"
        );
        if let Some(report) =
            template_drift_report(&region_declarations_of(&registry), &packaged).unwrap()
        {
            panic!("a declaration authored outside the region was reported as drift:\n{report}");
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
            .initialize_fresh_repository(temp.path(), None)
            .unwrap();
        fs::write(temp.path().join("AGENTS.md"), b"# Existing guidance\n").unwrap();
        let invariants_path = temp.path().join(".jit/invariants.toml");
        let scaffolded_invariants = fs::read_to_string(&invariants_path).unwrap();
        assert!(InvariantRegistry::from_toml_str(&scaffolded_invariants)
            .unwrap()
            .invariants
            .is_empty());

        let layout =
            crate::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
        let executor = CommandExecutor::new(storage.clone()).with_layout(layout);
        // Applied from inside the worktree it is applied to, because an
        // application records the package's worktree-relative location, and
        // beside the package it declares a dependency on, which is where that
        // dependency is looked for.
        crate::test_utils::assemble_repository_package(
            "jit-default",
            &temp.path().join("profiles/jit-default"),
        )
        .unwrap();
        let package = crate::test_utils::assemble_repository_package(
            PACKAGE_ID,
            &temp.path().join("profiles").join(PACKAGE_ID),
        )
        .unwrap();
        let applied = executor.apply_profile_package(&package).unwrap();
        assert_eq!(
            applied.requested().unwrap().status,
            super::super::ProfileApplicationStatus::Applied
        );
        assert!(temp
            .path()
            .join(".agents/skills/jit-manage/SKILL.md")
            .is_file());
        // The invariant registry belongs to the repository, which already carried
        // one: the application contributes entries into it and does not publish a
        // replacement asset.
        assert!(!package
            .model()
            .assets
            .iter()
            .any(|asset| asset.target == ".jit/invariants.toml"));
        let contributed =
            InvariantRegistry::from_toml_str(&fs::read_to_string(&invariants_path).unwrap())
                .unwrap();
        assert_eq!(contributed.invariants.len(), 18);
        assert!(contributed
            .invariants
            .iter()
            .any(|invariant| invariant.id == "label-format"));
        assert_ne!(
            fs::read_to_string(&invariants_path).unwrap(),
            scaffolded_invariants
        );
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
        assert_eq!(invariants.count, 18);
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

        let unchanged = reloaded.apply_profile_package(&package).unwrap();
        assert_eq!(
            unchanged.requested().unwrap().status,
            super::super::ProfileApplicationStatus::Unchanged
        );
    }

    #[test]
    fn test_installed_rules_gates_reference_is_derived_from_manifest_registries() {
        let (_workspace, package) = assembled_package();
        let rule_values = package
            .model()
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
                .insert(key.to_string(), declared_gate(&package, key));
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
        assert!(redirect.contains("`jit init --profile path:packages/jit-dogfood`"));
        assert!(redirect.contains("`jit profile apply --profile path:packages/jit-dogfood`"));
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
    fn live_asset_executable_declarations(package: &ProfilePackage) -> Vec<(&str, bool)> {
        package
            .model()
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
        let (_workspace, package) = assembled_package();
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
        let (_workspace, package) = assembled_package();
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
        let (_workspace, package) = assembled_package();
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
        let (_workspace, package) = assembled_package();
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
