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
    use crate::profile::{ExclusionPattern, LiveSourceDeclaration, ProfilePackage};
    use crate::repository_state::{
        render_rules_and_gates_markdown, Contribution, KeyedArrayTarget, MapEntryTarget,
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
            .unwrap_or_else(|| panic!("the package declares gate '{key}'"));

        serde_json::from_value(value)
            .unwrap_or_else(|error| panic!("package gate '{key}' is a runtime gate: {error}"))
    }

    #[test]
    fn test_jit_dogfood_package_validates_and_has_expected_workflow_inventory() {
        let (_workspace, package) = assembled_package();
        assert_eq!(package.manifest().profile.id.as_str(), "jit-dogfood");
        assert!(package.file_count() <= super::super::MAX_PROFILE_PACKAGE_FILES);
        assert!(package.byte_size() <= super::super::MAX_PROFILE_PACKAGE_BYTES);

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

        let invariants = package
            .manifest()
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
        let (_workspace, package) = assembled_package();
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
        let (_workspace, package) = assembled_package();
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
        let (_workspace, package) = assembled_package();
        let mut known = BTreeMap::<String, BTreeSet<String>>::new();
        for contribution in &package.manifest().contributions {
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

    /// Which repository files are packaged is settled by the walk over the
    /// declared live-source roots, so what is left here is the other side: the
    /// adopter state the package installs is never drawn from this repository.
    #[test]
    fn test_live_assets_exclude_install_only_adopter_state() {
        let (_workspace, package) = assembled_package();
        let live: BTreeSet<&str> = live_asset_targets(&package).into_iter().collect();
        assert!(!live.is_empty(), "the package declares live assets");
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

    /// Every live asset's repository-relative target.
    fn live_asset_targets(package: &ProfilePackage) -> Vec<&str> {
        package
            .manifest()
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
            .manifest()
            .assets
            .iter()
            .filter(|asset| !asset.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX))
            .map(|asset| asset.source.as_str())
            .chain(
                package
                    .manifest()
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
        let roots = &package.manifest().live_sources;
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
        let roots = &package.manifest().live_sources;
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
                .manifest()
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
    /// This is the direction the drift assertion does not walk: that one reads
    /// each declared asset's repository file, so a live consumer added under a
    /// packaged root and declared nowhere is absent from every adopter install
    /// with nothing reporting it.
    #[test]
    fn test_unpackaged_files_under_declared_roots_is_empty_across_the_tracked_repository_tree() {
        let worktree = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let (_workspace, package) = assembled_package();
        let roots = &package.manifest().live_sources;
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
        let roots = &package.manifest().live_sources;
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
        let declared = &package.manifest().live_sources;
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
        let roots = &package.manifest().live_sources;
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
        let roots = &package.manifest().live_sources;

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
        assert!(!package.manifest().regions.is_empty());
        assert!(package
            .manifest()
            .regions
            .iter()
            .all(|region| !region.source.starts_with(JIT_DOGFOOD_LIVE_SOURCE_PREFIX)));
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
        let (_workspace, package) = assembled_package();

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
            .manifest()
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
    fn live_asset_executable_declarations(package: &ProfilePackage) -> Vec<(&str, bool)> {
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
