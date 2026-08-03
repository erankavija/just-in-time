//! The canonical artifact directory an issue owns inside a named issue-scoped area.
//!
//! A repository declares which development areas organize their artifacts one
//! directory per issue ([`DocumentationConfig::issue_scoped_areas()`]). Several
//! areas adopt the convention at once, so the area is an input: a caller names
//! the area it wants and [`resolve_artifact_directory`] returns the directory
//! the issue owns inside it. An area the registry does not declare is an error
//! rather than a silently accepted path, and the directory name itself is
//! never something a caller composes.
//!
//! The name is the issue's short id, suffixed with a membership slug when one
//! is unambiguous. Precedence has three steps: exactly one `type:*` label, then
//! that type's configured membership namespace
//! ([`HierarchyConfig::get_membership_namespace`]), then exactly one value of
//! that namespace on the issue. Every other shape — no type label, several of
//! them, an unmapped type, no membership value, several of them, or a value
//! that normalizes to nothing — resolves to the bare short-id directory, which
//! is the shape archival already adopts for identifier-only directories
//! ([`crate::domain::artifact_classifier::preferred_container_destination_root`]).
//! Naming reads labels and the short id alone, so renaming an issue leaves its
//! directory where it is.
//!
//! Resolution is pure. It states where an artifact belongs and reads nothing,
//! so an issue whose artifacts still sit flat in the area resolves the same
//! canonical directory while those artifacts stay where they are.

use crate::config::DocumentationConfig;
use crate::domain::artifact_classifier::archive_container_slug;
use crate::domain::artifact_plan::normalize_artifact_path;
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::Issue;
use crate::labels::type_value_of;

/// Stable failures while resolving an issue's canonical artifact directory.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ArtifactDirectoryError {
    /// The named area is absent from the configured issue-scoped registry.
    #[error("'{area}' is not a declared issue-scoped area (declared: {})",
        if declared.is_empty() { "none".to_string() } else { declared.join(", ") })]
    UndeclaredArea {
        /// The area the caller named.
        area: String,
        /// The registry the area was matched against.
        declared: Vec<String>,
    },
}

/// Resolve the directory `issue` owns inside the declared issue-scoped `area`.
///
/// The repository-relative result is `area` joined with the issue's short id,
/// suffixed with the slug of its single membership value when the three-step
/// precedence resolves one (module documentation). `area` is matched against
/// the configured registry under lexical path normalization
/// ([`DocumentationConfig::is_issue_scoped_area`]), so `x`, `./x`, and `x/`
/// name the same area while a path inside one names no area at all.
///
/// # Errors
///
/// [`ArtifactDirectoryError::UndeclaredArea`] when `documentation` declares no
/// such area, naming both the area the caller asked for and the registry it
/// was matched against.
pub fn resolve_artifact_directory(
    issue: &Issue,
    area: &str,
    documentation: &DocumentationConfig,
    hierarchy: &HierarchyConfig,
) -> Result<String, ArtifactDirectoryError> {
    documentation
        .is_issue_scoped_area(area)
        .then(|| {
            let short_id = issue.short_id();
            let directory = membership_slug(issue, hierarchy)
                .map(|slug| format!("{short_id}-{slug}"))
                .unwrap_or(short_id);
            normalize_artifact_path(&format!("{area}/{directory}"))
        })
        .ok_or_else(|| ArtifactDirectoryError::UndeclaredArea {
            area: area.to_string(),
            declared: documentation.issue_scoped_areas(),
        })
}

/// The slug of the issue's single membership value, when the three-step
/// precedence resolves one.
///
/// `None` reports that the issue names no single membership value, which is the
/// bare short-id directory. The slug itself comes from the crate's one
/// normalizer ([`archive_container_slug`]), so a value holding no alphanumeric
/// character names nothing and lands on the same fallback.
fn membership_slug(issue: &Issue, hierarchy: &HierarchyConfig) -> Option<String> {
    let issue_types = issue
        .labels
        .iter()
        .filter_map(|label| type_value_of(label))
        .collect::<Vec<_>>();
    let namespace = (issue_types.len() == 1)
        .then(|| hierarchy.get_membership_namespace(issue_types[0]))
        .flatten()?;
    let values = issue
        .labels
        .iter()
        .filter_map(|label| label.split_once(':'))
        .filter_map(|(candidate, value)| (candidate == namespace).then_some(value))
        .collect::<Vec<_>>();
    (values.len() == 1)
        .then(|| values[0])
        .and_then(archive_container_slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::fixture_issue;
    use proptest::prelude::*;
    use std::collections::HashMap;

    /// A registry vocabulary unrelated to any area this project itself uses, so
    /// any `dev/`-shaped assumption in the resolver fails these tests rather
    /// than passing by coincidence.
    const AREA: &str = "workspace/notes";
    /// A second registry vocabulary, disjoint from [`AREA`], so a case can show
    /// that each registry accepts its own areas and rejects the other's.
    const OTHER_AREA: &str = "records/logbook";
    const TYPE: &str = "workstream";
    const NAMESPACE: &str = "workstream-group";
    const MEMBERSHIP: &str = "platform-archive";

    fn documentation(areas: Option<&[&str]>) -> DocumentationConfig {
        DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            citation_scan_roots: None,
            issue_scoped_areas: areas
                .map(|areas| areas.iter().map(|area| (*area).to_string()).collect()),
        }
    }

    fn hierarchy(namespace: &str) -> HierarchyConfig {
        HierarchyConfig::new(
            HashMap::from([(TYPE.to_string(), 1)]),
            HashMap::from([(TYPE.to_string(), namespace.to_string())]),
        )
        .unwrap()
    }

    fn fixture(title: &str, labels: &[String]) -> Issue {
        let mut issue = fixture_issue(title.to_string(), String::new());
        issue.labels = labels.to_vec();
        issue
    }

    fn type_label() -> String {
        format!("type:{TYPE}")
    }

    fn membership_label(value: &str) -> String {
        format!("{NAMESPACE}:{value}")
    }

    fn single_membership() -> Vec<String> {
        vec![type_label(), membership_label(MEMBERSHIP)]
    }

    fn resolve(issue: &Issue, area: &str) -> Result<String, ArtifactDirectoryError> {
        resolve_artifact_directory(
            issue,
            area,
            &documentation(Some(&[AREA])),
            &hierarchy(NAMESPACE),
        )
    }

    #[test]
    fn test_resolve_artifact_directory_names_the_area_short_id_and_single_membership_slug() {
        let overlong = "a".repeat(80);
        let values = [
            MEMBERSHIP,
            "Platform/Archive_V2",
            "Résumé Δοκιμή",
            overlong.as_str(),
        ];

        values.iter().for_each(|value| {
            let issue = fixture(
                "Canonical Directory Resolver",
                &[type_label(), membership_label(value)],
            );
            let resolved = resolve(&issue, AREA).unwrap();
            let (parent, directory) = resolved.rsplit_once('/').unwrap();

            assert_eq!(
                parent, AREA,
                "the resolved directory sits in the named area"
            );
            // Agreement with the crate's slug normalizer across punctuation,
            // Unicode, and the length bound is what a second normalizer would
            // have to reproduce exactly, so equality here is the reuse claim.
            assert_eq!(
                directory,
                format!(
                    "{}-{}",
                    issue.short_id(),
                    archive_container_slug(value).unwrap()
                ),
                "{value} names the short-id directory's slug suffix"
            );
            assert_ne!(
                directory,
                issue.short_id(),
                "a resolved membership value suffixes the short id"
            );
        });
    }

    #[test]
    fn test_resolve_artifact_directory_falls_back_to_the_bare_short_id_for_absent_or_ambiguous_membership(
    ) {
        let shapes = [
            ("no membership label", vec![type_label()]),
            (
                "several type labels",
                vec![
                    type_label(),
                    "type:squad".to_string(),
                    membership_label(MEMBERSHIP),
                ],
            ),
            (
                "several membership values",
                vec![
                    type_label(),
                    membership_label("first"),
                    membership_label("second"),
                ],
            ),
            (
                "a type the hierarchy maps to no namespace",
                vec!["type:unmapped".to_string(), membership_label(MEMBERSHIP)],
            ),
            (
                "a membership value that normalizes to nothing",
                vec![type_label(), membership_label("///")],
            ),
        ];

        shapes.iter().for_each(|(shape, labels)| {
            let issue = fixture("Canonical Directory Resolver", labels);
            assert_eq!(
                resolve(&issue, AREA).unwrap(),
                format!("{AREA}/{}", issue.short_id()),
                "{shape} resolves no single membership value"
            );
        });

        // The fallback is not vacuous: the same area and hierarchy give a
        // different name once exactly one membership value is resolvable.
        let unambiguous = fixture("Canonical Directory Resolver", &single_membership());
        assert_ne!(
            resolve(&unambiguous, AREA).unwrap(),
            format!("{AREA}/{}", unambiguous.short_id())
        );
    }

    #[test]
    fn test_resolve_artifact_directory_rejects_an_area_the_registry_does_not_declare() {
        let issue = fixture("Canonical Directory Resolver", &single_membership());
        let declared_directory = resolve(&issue, AREA).unwrap();

        // Lexical spellings of the declared area name the same area.
        [format!("./{AREA}"), format!("{AREA}/")]
            .iter()
            .for_each(|spelling| {
                assert_eq!(
                    resolve(&issue, spelling).unwrap(),
                    declared_directory,
                    "{spelling} spells the declared area"
                );
            });

        let undeclared = [
            (
                "a sibling whose name starts with the declared area",
                format!("{AREA}-archive"),
            ),
            (
                "a path inside the declared area",
                declared_directory.clone(),
            ),
            (
                "an area a different registry declares",
                OTHER_AREA.to_string(),
            ),
        ];

        undeclared.iter().for_each(|(shape, area)| {
            let message = resolve(&issue, area).unwrap_err().to_string();
            assert!(
                message.contains(area.as_str()),
                "the error for {shape} names the offending area: {message}"
            );
            assert!(
                message.contains(AREA),
                "the error for {shape} names the registry it was matched against: {message}"
            );
        });

        assert!(
            resolve(&issue, "").is_err(),
            "the empty string names no area"
        );
    }

    #[test]
    fn test_resolve_artifact_directory_ignores_the_issue_title() {
        // Both titles slug into distinctive words that the resolved path would
        // carry if any of it came from the title.
        let titles = ["Canonical Directory Resolver", "Entirely Renamed Container"];
        let shapes = [single_membership(), vec![type_label()]];

        shapes.iter().for_each(|labels| {
            let mut issue = fixture(titles[0], labels);
            let before = resolve(&issue, AREA).unwrap();
            issue.title = titles[1].to_string();
            let after = resolve(&issue, AREA).unwrap();

            assert_eq!(
                before, after,
                "renaming the issue must not rename its directory"
            );
            assert!(
                before
                    .rsplit_once('/')
                    .is_some_and(|(_, directory)| directory.starts_with(&issue.short_id())),
                "the name the title cannot reach is the issue's own: {before}"
            );
            assert!(
                titles
                    .iter()
                    .flat_map(|title| title.split_whitespace())
                    .all(|word| !before.contains(&word.to_lowercase())),
                "no title word may reach the directory name: {before}"
            );
        });
    }

    #[test]
    fn test_resolve_artifact_directory_reads_the_area_registry_and_membership_namespace_from_configuration(
    ) {
        let issue = fixture("Canonical Directory Resolver", &single_membership());
        let authored = documentation(Some(&[AREA]));
        let other = documentation(Some(&[OTHER_AREA]));

        // Each registry accepts its own areas and rejects the other's, so the
        // accepted set is whatever configuration declares.
        [
            (&authored, AREA, true),
            (&authored, OTHER_AREA, false),
            (&other, OTHER_AREA, true),
            (&other, AREA, false),
        ]
        .iter()
        .for_each(|(documentation, area, accepted)| {
            assert_eq!(
                resolve_artifact_directory(&issue, area, documentation, &hierarchy(NAMESPACE))
                    .is_ok(),
                *accepted,
                "{area} against the registry {:?}",
                documentation.issue_scoped_areas()
            );
        });

        // The same labels slug under a mapping that names their namespace and
        // fall back under one that names another, so the namespace is read
        // from the configured type mapping.
        let mapped =
            resolve_artifact_directory(&issue, AREA, &authored, &hierarchy(NAMESPACE)).unwrap();
        let unmapped =
            resolve_artifact_directory(&issue, AREA, &authored, &hierarchy("squad")).unwrap();
        assert_ne!(mapped, unmapped);
        assert_eq!(unmapped, format!("{AREA}/{}", issue.short_id()));
    }

    #[test]
    fn test_resolve_artifact_directory_reports_the_canonical_directory_for_a_flat_artifact_and_leaves_it_in_place(
    ) {
        // The filesystem witnesses inaction rather than enabling the behaviour:
        // the resolver is handed repository-relative names and never learns
        // where this tree is, so a tree that survives the call unchanged is the
        // observable form of "reports the canonical location and moves
        // nothing".
        let repository = tempfile::tempdir().unwrap();
        let area = repository.path().join(AREA);
        std::fs::create_dir_all(&area).unwrap();
        let issue = fixture("Canonical Directory Resolver", &single_membership());
        let flat = area.join(format!("{}-plan.md", issue.short_id()));
        std::fs::write(&flat, b"a flat artifact").unwrap();

        let resolved = resolve(&issue, AREA).unwrap();

        let canonical = repository.path().join(&resolved);
        assert_ne!(
            canonical, area,
            "the canonical directory is not the flat area the artifact sits in"
        );
        assert!(
            !canonical.exists(),
            "the resolver names {resolved} without creating it"
        );
        assert_eq!(
            std::fs::read_dir(&area)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>(),
            vec![flat.clone()],
            "the flat artifact stays where it is"
        );
        assert_eq!(std::fs::read(&flat).unwrap(), b"a flat artifact");

        // The answer is a statement about naming, not a reading of the tree: it
        // is unchanged once the tree is gone.
        std::fs::remove_dir_all(&area).unwrap();
        assert_eq!(resolve(&issue, AREA).unwrap(), resolved);
    }

    proptest! {
        /// Property: no title, however chosen, reaches the resolved directory.
        #[test]
        fn test_resolve_artifact_directory_is_invariant_under_any_title(
            first in ".*",
            second in ".*",
        ) {
            let mut issue = fixture(&first, &single_membership());
            let before = resolve(&issue, AREA).unwrap();
            prop_assert!(before
                .rsplit_once('/')
                .is_some_and(|(_, directory)| directory.starts_with(&issue.short_id())));
            issue.title = second;
            prop_assert_eq!(before, resolve(&issue, AREA).unwrap());
        }
    }
}
