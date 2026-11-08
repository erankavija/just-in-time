//! Repository-local checks over this checkout's own workflow package.

#[cfg(test)]
mod tests {
    use super::super::LIVE_ASSET_SOURCE_PREFIX;
    use crate::commands::CommandExecutor;
    use crate::config::ProjectionStyle;
    use crate::declarations::invariants::InvariantRegistry;
    use crate::declarations::GateRegistry;
    use crate::domain::repository_inputs::ExclusionPattern;
    use crate::profile::{
        LiveSourceDeclaration, ProfilePackage, RegionDeclaration, RegionPlacement,
    };
    use crate::repository_state::{
        render_managed_document, render_rules_and_gates_markdown, Contribution, KeyedArrayTarget,
        ManagedDocumentClaim,
    };
    use crate::storage::{IssueStore, JsonFileStorage};
    use std::collections::BTreeSet;
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
        assert!(!invariants.is_empty());
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
            .filter(|asset| asset.source.starts_with(LIVE_ASSET_SOURCE_PREFIX))
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
            .filter(|asset| !asset.source.starts_with(LIVE_ASSET_SOURCE_PREFIX))
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
                .filter(|asset| !asset.source.starts_with(LIVE_ASSET_SOURCE_PREFIX))
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
        let initialized = crate::test_cli::init_git_repository(empty.path());
        assert!(initialized.is_ok(), "the fixture repository initializes");

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
            .all(|region| !region.source.starts_with(LIVE_ASSET_SOURCE_PREFIX)));
    }

    #[test]
    fn test_checked_in_package_has_no_live_source_files() {
        let package_sources = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../profiles/jit-dogfood")
            .join(LIVE_ASSET_SOURCE_PREFIX);
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
            assert_eq!(gate.key.as_str(), key);
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

    /// How a rendered managed region differs from its packaged source, normalizing
    /// the nested invariants projection when the package source carries it.
    fn managed_region_drift_report(
        region: &RegionDeclaration,
        rendered: &str,
        packaged: &str,
    ) -> Option<String> {
        let rendered = normalized_managed_region_body(rendered, packaged);
        let packaged = normalized_managed_region_body(packaged, packaged);
        (rendered != packaged).then(|| {
            format!(
                "region '{}' differs: {} declares {rendered:?}, {} declares {packaged:?}; \
                 edit the two together",
                region.region_id, region.target, region.source
            )
        })
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
            region_id: region_id.try_into().expect("test region id is canonical"),
            placement: RegionPlacement::Append,
            template: false,
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
    fn test_managed_region_drift_report_names_both_carriers_and_their_text() {
        let region = region_declaration(
            "profiles/example/assets/region.md",
            "docs/example.md",
            "example",
        );

        let report = managed_region_drift_report(&region, "the rendered body", "the packaged body")
            .expect("different bodies are drift");

        [
            "docs/example.md",
            "profiles/example/assets/region.md",
            "the rendered body",
            "the packaged body",
        ]
        .iter()
        .for_each(|expected| assert!(report.contains(expected), "{report}"));
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
        package.model().regions.iter().for_each(|region| {
            let source =
                std::str::from_utf8(package.source_bytes(&region.source).unwrap()).unwrap();
            assert!(
                managed_region_drift_report(
                    region,
                    &format!("{source}\nrepository-only text"),
                    source,
                )
                .is_some(),
                "region '{}' went uncompared",
                region.region_id
            );
        });
    }

    #[test]
    fn test_managed_regions_match_every_declared_source_tree_consumer() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .unwrap();
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

    #[test]
    fn test_profile_applies_to_neutral_repo_and_ordinary_renderers_consume_config() {
        let temp = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(temp.path().join(".jit"));
        let layout =
            crate::storage::discover_repository_layout(temp.path(), storage.root()).unwrap();
        let initializer = CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .unwrap();
        initializer
            .profiles()
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
        let executor = CommandExecutor::new(storage.clone())
            .with_layout(layout)
            .unwrap();
        // Applied from inside the worktree it is applied to, because an
        // application records the package's worktree-relative location, and
        // beside the package it declares a dependency on, which is where that
        // dependency is looked for.
        crate::test_utils::capture_repository_package(
            "jit-default",
            &temp.path().join("profiles/jit-default"),
        )
        .unwrap();
        let package = crate::test_utils::capture_repository_package(
            PACKAGE_ID,
            &temp.path().join("profiles").join(PACKAGE_ID),
        )
        .unwrap();
        let applied = crate::test_utils::apply_package(&executor, &package).unwrap();
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
        // Application installs exactly the identities the package contributes:
        // both sides derive from the package, so neither restates the other.
        let contributed_ids = contributed
            .invariants
            .iter()
            .map(|invariant| invariant.id.as_str())
            .collect::<BTreeSet<_>>();
        let declared_ids = package
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
        assert_eq!(contributed_ids, declared_ids);
        assert!(contributed_ids.contains("label-format"));
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
        let reloaded = CommandExecutor::new(storage).with_layout(layout).unwrap();
        let invariants = reloaded.items().project_render(Some("invariants")).unwrap();
        let invariants = &invariants.items[0];
        assert_eq!(invariants.target, "AGENTS.md");
        assert_eq!(invariants.mode, crate::config::ProjectionMode::Region);
        assert_eq!(invariants.style, ProjectionStyle::IdAnchor);
        assert_eq!(invariants.kinds, ["invariant"]);
        assert_eq!(invariants.count, contributed.invariants.len());
        let reference = reloaded
            .items()
            .project_render(Some("rules-and-gates"))
            .unwrap();
        let reference = &reference.items[0];
        assert_eq!(reference.target, ".jit/reference/rules-and-gates.md");
        assert_eq!(reference.mode, crate::config::ProjectionMode::SeparateFile);
        assert_eq!(reference.style, ProjectionStyle::Full);
        assert_eq!(reference.kinds, ["rule", "gate"]);
        // The separate-file target carries the profile's six gates (registry
        // composition asserted through the rendered `## Gates` section).
        let rendered = fs::read_to_string(temp.path().join(&reference.target)).unwrap();
        assert_eq!(rendered.matches("@/gate/").count(), 6);

        let unchanged = crate::test_utils::apply_package(&reloaded, &package).unwrap();
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
        let rules = crate::declarations::rules::RuleSet::parse(
            &rules_toml,
            None,
            [(
                "schemas/jit-content-standards.json".to_string(),
                package
                    .source_bytes("assets/install/.jit/schemas/jit-content-standards.json")
                    .unwrap()
                    .to_vec(),
            )],
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
                .insert(key.parse().unwrap(), declared_gate(&package, key));
        }
        let expected = render_rules_and_gates_markdown(&rules, &gates, ProjectionStyle::Full);
        assert_eq!(
            package
                .source_bytes("assets/install/.jit/reference/rules-and-gates.md")
                .unwrap(),
            expected.as_bytes()
        );
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
            .filter(|asset| asset.source.starts_with(LIVE_ASSET_SOURCE_PREFIX))
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

    /// Create-time warnings the package's own type-hierarchy rules raise over
    /// one issue carrying `labels`,
    /// evaluated in memory over the declared test vocabulary.
    fn create_time_warnings(labels: Vec<String>) -> Vec<crate::validation::graph::GraphFinding> {
        let (_workspace, package) = assembled_package();
        let rules = package
            .model()
            .contributions
            .iter()
            .filter_map(|contribution| match contribution {
                Contribution::KeyedArray {
                    target: KeyedArrayTarget::Rules,
                    value,
                    ..
                } if value["assert"]["type-hierarchy"].is_object() => Some(value.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            rules.len(),
            crate::declarations::rules::TypeHierarchyKind::ALL.len(),
            "the package declares one warning per hierarchy check"
        );
        let declared = toml::to_string(&serde_json::json!({ "rules": rules }))
            .expect("the package's rules serialize as a rules file");
        let ruleset = crate::declarations::rules::RuleSet::parse(&declared, None, [])
            .expect("the package's rules parse");
        let graph_rules = ruleset.rules.iter().collect::<Vec<_>>();

        let mut issue = crate::test_utils::fixture_issue("Subject".to_string(), String::new());
        issue.labels = labels;
        crate::validation::graph::evaluate_graph(
            &graph_rules,
            std::slice::from_ref(&issue),
            &crate::test_taxonomy::test_taxonomy().hierarchy_config(),
            crate::domain::ContentFormat::Markdown,
            chrono::Utc::now(),
            &std::collections::HashMap::new(),
        )
    }

    #[test]
    fn test_workflow_package_rules_warn_for_strategic_issue_without_membership_label() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let strategic = taxonomy.type_at_level(2);

        let warnings = create_time_warnings(vec![format!("type:{strategic}")]);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].finding.rule, "strategic-consistency");
        assert!(warnings[0]
            .finding
            .message
            .contains(&format!("{}:*", taxonomy.label_associations[strategic])));
    }

    #[test]
    fn test_workflow_package_rules_warn_for_leaf_issue_without_parent_label() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();

        let warnings = create_time_warnings(vec![format!("type:{}", taxonomy.type_at_level(4))]);

        assert_eq!(warnings.len(), 1);
        assert_eq!(warnings[0].finding.rule, "orphan-leaf");
        assert!(warnings[0].finding.message.contains("orphaned leaf"));
    }

    #[test]
    fn test_workflow_package_rules_accept_strategic_issue_with_membership_label() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let strategic = taxonomy.type_at_level(2);

        let warnings = create_time_warnings(vec![
            format!("type:{strategic}"),
            format!("{}:auth", taxonomy.label_associations[strategic]),
        ]);

        assert!(warnings.is_empty(), "{warnings:?}");
    }

    #[test]
    fn test_workflow_package_rules_accept_leaf_issue_with_parent_label() {
        let taxonomy = crate::test_taxonomy::test_taxonomy();
        let parent = taxonomy.type_at_level(2);

        let warnings = create_time_warnings(vec![
            format!("type:{}", taxonomy.type_at_level(4)),
            format!("{}:auth", taxonomy.label_associations[parent]),
        ]);

        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
