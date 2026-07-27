//! Which artifacts under an issue-scoped area sit outside the directory their
//! owning issue owns there.
//!
//! Advice, never enforcement (`@/issue/8e071e18/decision/D-7`). Legacy flat
//! artifacts are deliberately tolerated, so this module answers a question and
//! carries no verdict a caller could gate on: it turns a listing of an area
//! into the artifacts whose location disagrees with
//! [`resolve_artifact_directory`], and says nothing about the rest.
//!
//! An **artifact** here is a path component the report resolves an owner for.
//! A component that opens with a short id — eight lowercase hexadecimal
//! characters delimited by `-`, `.`, or the end of the component, the shape
//! [`Issue::short_id`] prints — names its owner outright, and that prefix
//! decides. A component carrying none is resolved from the document references
//! naming it, where exactly one referencing issue is the owner and a component
//! no issue references is passed over rather than reported. An ownership these
//! inputs cannot settle is *unattributed* — a prefix no single issue answers
//! to, or a prefix-less name several issues reference: the artifact is named,
//! and nothing is claimed about where it belongs.
//!
//! Occurrences are located from the top down. Each component below the area is
//! examined in turn and the first one that is unattributed or misplaced is the
//! occurrence, so a misplaced directory is named once instead of once per file
//! beneath it, and a correctly placed directory still exposes a stranger filed
//! inside it. Placement itself is containment rather than equality: an artifact
//! anywhere beneath the directory its owner owns is where it belongs, so an
//! owning issue's own subdirectories conform.

use crate::config::DocumentationConfig;
use crate::domain::artifact_classifier::contains_path;
use crate::domain::artifact_directory::{resolve_artifact_directory, ArtifactDirectoryError};
use crate::domain::artifact_plan::normalize_artifact_path;
use crate::domain::type_taxonomy::HierarchyConfig;
use crate::domain::{Issue, SHORT_ID_LENGTH};
use std::collections::{BTreeMap, BTreeSet};

/// What the report says about one artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArtifactDisposition {
    /// The owning issue is known and the artifact sits outside the directory
    /// that issue owns in this area.
    Nonconforming {
        /// Full id of the owning issue.
        issue_id: String,
        /// Repository-relative directory the owner owns in this area.
        canonical_directory: String,
    },
    /// The inputs settle no single owner, so nothing is claimed about where the
    /// artifact belongs. A short-id prefix no issue answers to, one several
    /// issues share, and a prefix-less name several issues reference all land
    /// here.
    Unattributed,
}

impl ArtifactDisposition {
    /// The stable wire spelling of the disposition.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Nonconforming { .. } => "nonconforming",
            Self::Unattributed => "unattributed",
        }
    }
}

/// One artifact the report names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportedArtifact {
    /// Declared issue-scoped area the artifact was found under.
    pub area: String,
    /// Repository-relative path of the artifact, which is a directory when a
    /// whole directory is misplaced.
    pub path: String,
    /// Short id the artifact's own name opens with, empty when the name carries
    /// none and a document reference resolved the owner instead.
    pub short_id: String,
    /// What the report says about it.
    pub disposition: ArtifactDisposition,
}

/// Report the artifacts among `paths` that disagree with their owner's
/// canonical directory in `area`.
///
/// `paths` are repository-relative paths found beneath `area`, files and
/// directories alike; a path that lies outside `area` contributes nothing.
/// Naming a directory is what lets an empty one be reported, since it reaches
/// this function through no path of its own otherwise. `issues` is the
/// repository's issue set, which owners are resolved against: a short-id prefix
/// against the ids those issues answer to, and a name carrying no prefix
/// against the document references they hold. Exactly one match attributes the
/// artifact. A prefix zero or several issues match lands on
/// [`ArtifactDisposition::Unattributed`], as does a prefix-less name several
/// issues reference; a prefix-less name no issue references is passed over.
///
/// Results are ordered by path and hold one entry per reported artifact, so a
/// misplaced directory is named once however many of its own descendants also
/// appear in `paths`.
///
/// # Errors
///
/// [`ArtifactDirectoryError::UndeclaredArea`] when `documentation` declares no
/// such area. A caller reports over the declared registry, so this states that
/// the area it walked and the area it resolved against disagree.
pub fn report_area_artifacts(
    area: &str,
    paths: &[String],
    issues: &[Issue],
    documentation: &DocumentationConfig,
    hierarchy: &HierarchyConfig,
) -> Result<Vec<ReportedArtifact>, ArtifactDirectoryError> {
    let area = normalize_artifact_path(area);
    // Stated before any issue is consulted, so a repository holding no issues
    // disagrees about an undeclared area exactly as a populated one does.
    if !documentation.is_issue_scoped_area(&area) {
        return Err(ArtifactDirectoryError::UndeclaredArea {
            area,
            declared: documentation.issue_scoped_areas(),
        });
    }

    let owners = owners_by_short_id(issues);
    let references = owners_by_referenced_path(issues);
    let canonical_directories = issues
        .iter()
        .map(|issue| {
            resolve_artifact_directory(issue, &area, documentation, hierarchy)
                .map(|directory| (issue.id.clone(), directory))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    Ok(paths
        .iter()
        .filter_map(|path| {
            first_reported_occurrence(&area, path, &owners, &references, &canonical_directories)
        })
        .map(|artifact| (artifact.path.clone(), artifact))
        .collect::<BTreeMap<_, _>>()
        .into_values()
        .collect())
}

/// The topmost component of `path` below `area` that the report names, if any.
///
/// Walking top-down is what makes a misplaced directory one occurrence rather
/// than one per file: the first component that is unattributed or misplaced
/// ends the walk, and the components beneath it move with it.
fn first_reported_occurrence(
    area: &str,
    path: &str,
    owners: &BTreeMap<String, Vec<&Issue>>,
    references: &BTreeMap<String, Vec<&Issue>>,
    canonical_directories: &BTreeMap<String, String>,
) -> Option<ReportedArtifact> {
    let path = normalize_artifact_path(path);
    let relative = path
        .strip_prefix(area)
        .and_then(|suffix| suffix.strip_prefix('/'))?;

    relative
        .split('/')
        .scan(area.to_string(), |occurrence, component| {
            occurrence.push('/');
            occurrence.push_str(component);
            Some((occurrence.clone(), component.to_string()))
        })
        .find_map(|(occurrence, component)| {
            let (short_id, owner) = resolve_owner(&component, &occurrence, owners, references)?;
            let disposition = match owner {
                ComponentOwner::Single(issue) => {
                    let canonical = canonical_directories.get(&issue.id)?;
                    (!contains_path(canonical, &occurrence)).then(|| {
                        ArtifactDisposition::Nonconforming {
                            issue_id: issue.id.clone(),
                            canonical_directory: canonical.clone(),
                        }
                    })
                }
                ComponentOwner::Unsettled => Some(ArtifactDisposition::Unattributed),
            }?;
            Some(ReportedArtifact {
                area: area.to_string(),
                path: occurrence,
                short_id,
                disposition,
            })
        })
}

/// What the report's inputs say about one path component's owner.
enum ComponentOwner<'a> {
    /// Exactly one issue answers for the component.
    Single(&'a Issue),
    /// Several issues answer for it, or the short id it carries answers to
    /// none, so the inputs settle no owner.
    Unsettled,
}

/// The owner of the component `occurrence` ends in, with the short id the
/// report carries for it, or `None` when the component is passed over.
///
/// The name's own short-id prefix decides first and is resolved against
/// `owners`. A name carrying none is resolved against `references`, the issues
/// whose document references name that repository-relative path — the durable
/// statement of ownership a name without a prefix leaves unsaid. A prefix-less
/// name no issue references resolves to nothing at all, which is what the walk
/// passes over: an unreferenced file is invisible to archival too, and the
/// report claims nothing about it.
fn resolve_owner<'a>(
    component: &str,
    occurrence: &str,
    owners: &BTreeMap<String, Vec<&'a Issue>>,
    references: &BTreeMap<String, Vec<&'a Issue>>,
) -> Option<(String, ComponentOwner<'a>)> {
    let (short_id, candidates) = match short_id_prefix(component) {
        Some(short_id) => (
            short_id.to_string(),
            owners.get(short_id).map(Vec::as_slice).unwrap_or_default(),
        ),
        // The empty short id says the name carries none, so an entry states
        // which of the two rules attributed it.
        None => (String::new(), references.get(occurrence)?.as_slice()),
    };

    Some(match candidates {
        [issue] => (short_id, ComponentOwner::Single(issue)),
        _ => (short_id, ComponentOwner::Unsettled),
    })
}

/// The short id a path component opens with, if it opens with one.
///
/// The prefix runs to the first `-` or `.` and must be exactly
/// [`SHORT_ID_LENGTH`] lowercase hexadecimal characters, so `deadbeef-plan.md`
/// and `deadbeef` carry one while `deadbeefish-plan.md` and `plan-deadbeef.md`
/// do not. Matching the printed spelling exactly is what keeps an ordinary word
/// from being read as an identifier.
fn short_id_prefix(component: &str) -> Option<&str> {
    let candidate = component.split(['-', '.']).next()?;
    (candidate.len() == SHORT_ID_LENGTH
        && candidate
            .chars()
            .all(|character| matches!(character, '0'..='9' | 'a'..='f')))
    .then_some(candidate)
}

/// Every issue grouped under the short id it answers to.
///
/// Short ids are a truncation of the full id, so a group holding more than one
/// issue is a prefix no single issue owns.
fn owners_by_short_id(issues: &[Issue]) -> BTreeMap<String, Vec<&Issue>> {
    issues.iter().fold(BTreeMap::new(), |mut owners, issue| {
        owners.entry(issue.short_id()).or_default().push(issue);
        owners
    })
}

/// Every issue grouped under the repository-relative path each of its document
/// references names.
///
/// Paths are normalized to the spelling the walk carries, so a reference and an
/// occurrence naming the same file meet. An issue naming one path from several
/// of its own references counts once, so a group holding more than one issue is
/// a path several issues claim.
fn owners_by_referenced_path(issues: &[Issue]) -> BTreeMap<String, Vec<&Issue>> {
    issues.iter().fold(BTreeMap::new(), |references, issue| {
        issue
            .documents
            .iter()
            .map(|document| normalize_artifact_path(&document.path))
            .filter(|path| !path.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .fold(references, |mut references, path| {
                references.entry(path).or_default().push(issue);
                references
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::types::fixture_issue;
    use proptest::prelude::*;
    use std::collections::HashMap;

    /// A registry vocabulary unrelated to the shipped areas, so a `dev/`-shaped
    /// assumption fails these tests rather than passing by coincidence.
    const AREA: &str = "workspace/notes";
    const TYPE: &str = "workstream";
    const NAMESPACE: &str = "initiative";

    fn documentation() -> DocumentationConfig {
        DocumentationConfig {
            development_root: None,
            managed_paths: None,
            archive_root: None,
            permanent_paths: None,
            citation_scan_roots: None,
            issue_scoped_areas: Some(vec![AREA.to_string()]),
        }
    }

    fn hierarchy() -> HierarchyConfig {
        HierarchyConfig::new(
            HashMap::from([(TYPE.to_string(), 1)]),
            HashMap::from([(TYPE.to_string(), NAMESPACE.to_string())]),
        )
        .unwrap()
    }

    /// An issue naming a single membership value, so the directory it owns is a
    /// slug-suffixed name distinguishable from the bare area.
    fn issue(membership: &str) -> Issue {
        let mut issue = fixture_issue("Artifact owner".to_string(), String::new());
        issue.labels = vec![format!("type:{TYPE}"), format!("{NAMESPACE}:{membership}")];
        issue
    }

    /// `issue` stating, through its document references, that it owns each of
    /// `paths` — the claim a name carrying no short id leaves unsaid.
    fn referencing(issue: Issue, paths: &[&str]) -> Issue {
        Issue {
            documents: paths
                .iter()
                .map(|path| crate::domain::DocumentReference::new((*path).to_string()))
                .collect(),
            ..issue
        }
    }

    fn canonical(issue: &Issue) -> String {
        resolve_artifact_directory(issue, AREA, &documentation(), &hierarchy()).unwrap()
    }

    fn report(paths: &[&str], issues: &[Issue]) -> Vec<ReportedArtifact> {
        report_area_artifacts(
            AREA,
            &paths
                .iter()
                .map(|path| (*path).to_string())
                .collect::<Vec<_>>(),
            issues,
            &documentation(),
            &hierarchy(),
        )
        .unwrap()
    }

    fn reported_paths(artifacts: &[ReportedArtifact]) -> Vec<&str> {
        artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect()
    }

    #[test]
    fn test_report_area_artifacts_names_a_prefixed_artifact_outside_the_directory_its_owner_owns() {
        let owner = issue("artifact-layout");
        let canonical = canonical(&owner);
        let flat = format!("{AREA}/{}-plan.md", owner.short_id());

        let reported = report(&[&flat], std::slice::from_ref(&owner));

        assert_eq!(reported_paths(&reported), vec![flat.as_str()]);
        assert_eq!(
            reported[0].disposition,
            ArtifactDisposition::Nonconforming {
                issue_id: owner.id.clone(),
                canonical_directory: canonical.clone(),
            },
            "the artifact is attributed to its owner and told where it belongs"
        );
        assert_eq!(reported[0].short_id, owner.short_id());
        assert_eq!(reported[0].area, AREA);

        // The same bytes under the same owner, inside the directory it owns,
        // are not a finding, so location alone decides.
        assert!(report(
            &[&format!("{canonical}/plan.md")],
            std::slice::from_ref(&owner)
        )
        .is_empty());
    }

    #[test]
    fn test_report_area_artifacts_accepts_every_depth_beneath_the_directory_its_owner_owns() {
        let owner = issue("artifact-layout");
        let canonical = canonical(&owner);
        let short_id = owner.short_id();

        let conforming = [
            format!("{canonical}/plan.md"),
            format!("{canonical}/{short_id}-plan.md"),
            format!("{canonical}/rounds/{short_id}-review-2.md"),
            format!("{canonical}/{short_id}-rounds/notes.md"),
        ];

        assert!(
            report(
                &conforming.iter().map(String::as_str).collect::<Vec<_>>(),
                std::slice::from_ref(&owner),
            )
            .is_empty(),
            "an artifact anywhere beneath the directory its owner owns conforms"
        );
    }

    #[test]
    fn test_report_area_artifacts_names_a_misplaced_directory_once_rather_than_each_file_under_it()
    {
        let owner = issue("artifact-layout");
        let short_id = owner.short_id();
        let misplaced = format!("{AREA}/{short_id}-wrong-slug");

        let reported = report(
            &[
                &format!("{misplaced}/plan.md"),
                &format!("{misplaced}/notes.md"),
                &format!("{misplaced}/rounds/review.md"),
            ],
            std::slice::from_ref(&owner),
        );

        assert_eq!(
            reported_paths(&reported),
            vec![misplaced.as_str()],
            "the whole directory is one occurrence"
        );
        assert!(matches!(
            reported[0].disposition,
            ArtifactDisposition::Nonconforming { .. }
        ));
    }

    #[test]
    fn test_report_area_artifacts_names_a_stranger_filed_inside_a_conforming_directory() {
        let owner = issue("artifact-layout");
        let stranger = issue("other-initiative");
        let inside = format!("{}/{}-plan.md", canonical(&owner), stranger.short_id());

        let reported = report(&[&inside], &[owner.clone(), stranger.clone()]);

        assert_eq!(reported_paths(&reported), vec![inside.as_str()]);
        assert_eq!(
            reported[0].disposition,
            ArtifactDisposition::Nonconforming {
                issue_id: stranger.id.clone(),
                canonical_directory: canonical(&stranger),
            },
            "a conforming enclosing directory does not excuse a differently owned artifact"
        );

        // The same holds when the stranger's prefix answers to nothing: an
        // enclosing directory attributes only itself.
        let orphan = format!("{}/deadbeef-plan.md", canonical(&owner));
        let reported = report(&[&orphan], std::slice::from_ref(&owner));
        assert_eq!(reported_paths(&reported), vec![orphan.as_str()]);
        assert_eq!(reported[0].disposition, ArtifactDisposition::Unattributed);
    }

    #[test]
    fn test_report_area_artifacts_reports_an_unresolvable_prefix_as_unattributed() {
        let owner = issue("artifact-layout");
        let twin = {
            // Two issues sharing a short id: the prefix answers to no single one.
            let mut twin = issue("other-initiative");
            twin.id = format!("{}{}", owner.short_id(), &twin.id[SHORT_ID_LENGTH..]);
            twin
        };
        let unknown = format!("{AREA}/deadbeef-plan.md");
        let shared = format!("{AREA}/{}-plan.md", owner.short_id());

        [
            (
                "a prefix no issue answers to",
                vec![owner.clone()],
                &unknown,
            ),
            (
                "a prefix several issues answer to",
                vec![owner.clone(), twin],
                &shared,
            ),
        ]
        .iter()
        .for_each(|(shape, issues, path)| {
            let reported = report(&[path.as_str()], issues);
            assert_eq!(reported_paths(&reported), vec![path.as_str()], "{shape}");
            assert_eq!(
                reported[0].disposition,
                ArtifactDisposition::Unattributed,
                "{shape} claims no owner and no directory"
            );
        });

        // Not vacuous: the same path shape resolves an owner when exactly one
        // issue answers to the prefix.
        assert!(matches!(
            report(&[shared.as_str()], std::slice::from_ref(&owner))[0].disposition,
            ArtifactDisposition::Nonconforming { .. }
        ));
    }

    #[test]
    fn test_report_area_artifacts_attributes_a_prefixless_artifact_to_the_issue_referencing_it() {
        let host = issue("artifact-layout");
        // Filed inside the directory another issue owns, under a name that says
        // nothing about who owns it.
        let filed_inside = format!("{}/disposition-record.md", canonical(&host));
        let owner = referencing(issue("other-initiative"), &[&filed_inside]);

        let reported = report(&[&filed_inside], &[host.clone(), owner.clone()]);

        assert_eq!(reported_paths(&reported), vec![filed_inside.as_str()]);
        assert_eq!(
            reported[0].disposition,
            ArtifactDisposition::Nonconforming {
                issue_id: owner.id.clone(),
                canonical_directory: canonical(&owner),
            },
            "the reference names the owner, and a conforming enclosing directory does not excuse it"
        );
        assert!(
            reported[0].short_id.is_empty(),
            "the entry states that the name itself carries no short id: {:?}",
            reported[0].short_id
        );

        // Location still decides: the same name, referenced by the same issue,
        // inside the directory that issue owns is not a finding.
        let filed_at_home = format!("{}/disposition-record.md", canonical(&owner));
        assert!(
            report(
                &[&filed_at_home],
                &[host, referencing(owner, &[&filed_at_home])]
            )
            .is_empty(),
            "an artifact its owner references inside the directory that owner owns conforms"
        );
    }

    #[test]
    fn test_report_area_artifacts_reports_a_prefixless_artifact_several_issues_reference_as_unattributed(
    ) {
        let contested = format!("{AREA}/disposition-record.md");
        let first = referencing(issue("artifact-layout"), &[&contested]);
        let second = referencing(issue("other-initiative"), &[&contested]);

        let reported = report(&[&contested], &[first.clone(), second]);

        assert_eq!(reported_paths(&reported), vec![contested.as_str()]);
        assert_eq!(
            reported[0].disposition,
            ArtifactDisposition::Unattributed,
            "an ownership two issues both claim names neither an owner nor a directory"
        );

        // One issue naming the same artifact from two of its own references is
        // one claim, however each reference spells the path.
        let twice = referencing(first, &[&contested, &format!("./{contested}")]);
        assert!(
            matches!(
                report(&[&contested], std::slice::from_ref(&twice))[0].disposition,
                ArtifactDisposition::Nonconforming { .. }
            ),
            "the claim is counted per issue rather than per reference"
        );
    }

    #[test]
    fn test_report_area_artifacts_resolves_a_name_carrying_a_short_id_from_that_prefix_alone() {
        let owner = issue("artifact-layout");
        let flat = format!("{AREA}/{}-plan.md", owner.short_id());
        let claimant = referencing(issue("other-initiative"), &[&flat]);

        let reported = report(&[&flat], &[owner.clone(), claimant]);

        assert_eq!(
            reported[0].disposition,
            ArtifactDisposition::Nonconforming {
                issue_id: owner.id.clone(),
                canonical_directory: canonical(&owner),
            },
            "the prefix the name carries decides, and a reference from elsewhere does not move it"
        );
        assert_eq!(reported[0].short_id, owner.short_id());

        // The same where the prefix resolves nothing: an unsettled prefix is
        // unattributed rather than falling through to whoever references it.
        let unknown = format!("{AREA}/deadbeef-plan.md");
        let claimant = referencing(issue("other-initiative"), &[&unknown]);
        assert_eq!(
            report(&[&unknown], &[owner, claimant])[0].disposition,
            ArtifactDisposition::Unattributed
        );
    }

    #[test]
    fn test_report_area_artifacts_passes_over_an_unreferenced_name_carrying_no_short_id_prefix() {
        let owner = issue("artifact-layout");
        let short_id = owner.short_id();

        let unprefixed = [
            // No identifier at all.
            format!("{AREA}/architecture-pitfalls.md"),
            // The identifier is a suffix, not a prefix.
            format!("{AREA}/benchmarks-{short_id}.md"),
            // The leading run is not delimited from what follows it.
            format!("{AREA}/{short_id}notes.md"),
            // Hexadecimal, but not the printed width.
            format!("{AREA}/{}-plan.md", &short_id[..SHORT_ID_LENGTH - 1]),
            // The printed width, but outside the printed alphabet.
            format!("{AREA}/DEADBEEF-plan.md"),
            format!("{AREA}/deadbeefs-plan.md"),
        ];

        assert!(
            report(
                &unprefixed.iter().map(String::as_str).collect::<Vec<_>>(),
                std::slice::from_ref(&owner),
            )
            .is_empty(),
            "a name no prefix and no reference attributes is passed over, not reported"
        );

        // Not vacuous: what the report has nothing to say about is the missing
        // claim rather than the shape of the name. Each of the same names is
        // reported once one issue's reference states it owns it.
        unprefixed.iter().for_each(|path| {
            let claimant = referencing(issue("other-initiative"), &[path]);
            assert_eq!(
                reported_paths(&report(&[path.as_str()], &[owner.clone(), claimant])),
                vec![path.as_str()],
                "a reference attributes the artifact its name leaves unattributed"
            );
        });
    }

    #[test]
    fn test_report_area_artifacts_ignores_paths_outside_the_area_it_was_given() {
        let owner = issue("artifact-layout");
        let short_id = owner.short_id();

        let outside = [
            format!("elsewhere/{short_id}-plan.md"),
            // A sibling whose name opens with the area's spelling.
            format!("{AREA}-archive/{short_id}-plan.md"),
            // The area itself is not an artifact under it.
            AREA.to_string(),
        ];

        assert!(report(
            &outside.iter().map(String::as_str).collect::<Vec<_>>(),
            std::slice::from_ref(&owner),
        )
        .is_empty());
        // The same file directly under the area is reported, so the filter is
        // the area boundary rather than the path shape.
        assert!(!report(
            &[&format!("{AREA}/{short_id}-plan.md")],
            std::slice::from_ref(&owner)
        )
        .is_empty());
    }

    #[test]
    fn test_report_area_artifacts_rejects_an_area_the_registry_does_not_declare() {
        let owner = issue("artifact-layout");
        let path = format!("workspace/elsewhere/{}-plan.md", owner.short_id());

        // An empty repository disagrees about the area exactly as a populated
        // one does, so the rejection does not depend on having an issue to
        // resolve a directory for.
        [
            ("a repository holding issues", vec![owner.clone()]),
            ("a repository holding none", Vec::new()),
        ]
        .iter()
        .for_each(|(shape, issues)| {
            let error = report_area_artifacts(
                "workspace/elsewhere",
                std::slice::from_ref(&path),
                issues,
                &documentation(),
                &hierarchy(),
            )
            .unwrap_err();

            assert!(
                matches!(error, ArtifactDirectoryError::UndeclaredArea { .. }),
                "walking an undeclared area is a stated disagreement in {shape}: {error}"
            );
        });
    }

    proptest! {
        /// Property: an artifact beneath the directory an issue owns is never
        /// reported for that issue, however deep it sits and however the path
        /// below it is spelled. The components are drawn outside the
        /// hexadecimal alphabet so none of them names a second owner, which is
        /// a finding of its own and covered by the cases above.
        #[test]
        fn test_report_area_artifacts_never_reports_an_owner_inside_its_own_directory(
            membership in "[a-z]{1,12}",
            tail in prop::collection::vec("[g-z]{1,8}", 1..4),
        ) {
            let owner = issue(&membership);
            let path = format!("{}/{}", canonical(&owner), tail.join("/"));
            prop_assert!(report(&[path.as_str()], std::slice::from_ref(&owner)).is_empty());
        }

        /// Property: a prefixed artifact directly under the area is always
        /// reported, whatever follows the prefix in its name.
        #[test]
        fn test_report_area_artifacts_always_reports_a_flat_artifact_of_its_owner(
            membership in "[a-z]{1,12}",
            suffix in "-[a-z0-9-]{0,20}\\.md",
        ) {
            let owner = issue(&membership);
            let path = format!("{AREA}/{}{suffix}", owner.short_id());
            let reported = report(&[path.as_str()], std::slice::from_ref(&owner));
            prop_assert_eq!(reported.len(), 1);
            prop_assert_eq!(&reported[0].path, &path);
        }

        /// Property: what the report says about an artifact whose name carries
        /// a short id is the same whether or not another issue's reference
        /// names that artifact. The two runs differ in the reference alone, so
        /// the prefix rule owns the verdict outright.
        #[test]
        fn test_report_area_artifacts_leaves_a_prefixed_artifact_unmoved_by_a_reference_naming_it(
            membership in "[a-z]{1,12}",
            suffix in "-[a-z0-9-]{0,20}\\.md",
        ) {
            let owner = issue(&membership);
            let path = format!("{AREA}/{}{suffix}", owner.short_id());
            let claimant = issue("other-initiative");

            prop_assert_eq!(
                report(&[path.as_str()], &[owner.clone(), referencing(claimant.clone(), &[&path])]),
                report(&[path.as_str()], &[owner, claimant]),
            );
        }

        /// Property: a name carrying no short id that exactly one issue
        /// references is reported wherever it sits outside that issue's
        /// directory, whatever it is called.
        #[test]
        fn test_report_area_artifacts_always_reports_a_referenced_artifact_outside_its_owner(
            membership in "[a-z]{1,12}",
            name in "[g-z]{1,10}\\.md",
        ) {
            let path = format!("{AREA}/{name}");
            let owner = referencing(issue(&membership), &[&path]);
            let reported = report(&[path.as_str()], std::slice::from_ref(&owner));

            prop_assert_eq!(reported.len(), 1);
            prop_assert_eq!(&reported[0].path, &path);
            prop_assert_eq!(
                &reported[0].disposition,
                &ArtifactDisposition::Nonconforming {
                    issue_id: owner.id.clone(),
                    canonical_directory: canonical(&owner),
                }
            );
        }
    }
}
