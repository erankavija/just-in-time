//! No-follow filesystem evidence acquisition for pure archive planning.

use crate::domain::artifact_classifier::{
    artifact_archive_destination, classification_facts_from_evidence,
    resolve_container_destination as derive_container_destination, ArtifactClassificationFacts,
    ArtifactClassificationPolicy, CitationScanEvidence, EmbeddedArtifactOwner,
    ResolvedContainerDestination,
};
use crate::domain::artifact_discovery::{
    ArtifactEvidence, ArtifactEvidenceMap, ArtifactListingScope,
};
use crate::domain::artifact_plan::{
    normalize_artifact_path, ArtifactPlanEntry, ArtifactVersion, PlanTarget,
};
use crate::repository_state::RootRelativePath;
use crate::storage::file_transaction::open_regular_file_nofollow;
use crate::storage::repository_state_store::{open_absolute_dir_nofollow, open_child_dir_nofollow};
use crate::storage::{validate_repo_relative_path, IssueStore, PathReadError};
use anyhow::{anyhow, Result};
use cap_std::fs::Dir;
use std::io::{ErrorKind, Read};
use std::path::Path;

/// Acquire the exact marker, child, and legacy evidence needed to resolve a destination.
pub fn resolve_container_destination<S: IssueStore>(
    storage: &S,
    preferred_root: &str,
    legacy_root: &str,
    container_id: &str,
) -> Result<ResolvedContainerDestination> {
    validate_repo_relative_path(preferred_root)?;
    validate_repo_relative_path(legacy_root)?;
    let archive_root = Path::new(legacy_root)
        .parent()
        .ok_or_else(|| anyhow!("container archive destination has no archive root"))?;
    let archive_root = normalize_artifact_path(&archive_root.to_string_lossy());
    validate_repo_relative_path(&archive_root)?;

    let mut evidence = ArtifactEvidenceMap::new();
    let archive = inspect_artifact_evidence(
        storage,
        &archive_root,
        ArtifactListingScope::ImmediateChildren,
    )?;
    if let ArtifactEvidence::Directory { entries, .. } = &archive {
        for child in entries {
            let child_evidence =
                inspect_artifact_evidence(storage, child, ArtifactListingScope::MetadataOnly)?;
            if matches!(child_evidence, ArtifactEvidence::Directory { .. }) {
                let marker = format!("{child}/.jit-container");
                evidence.insert(
                    marker.clone(),
                    inspect_artifact_evidence(
                        storage,
                        &marker,
                        ArtifactListingScope::MetadataOnly,
                    )?,
                );
            }
            evidence.insert(child.clone(), child_evidence);
        }
    }
    evidence.insert(archive_root, archive);
    if !evidence.contains_key(legacy_root) {
        evidence.insert(
            legacy_root.to_string(),
            inspect_artifact_evidence(storage, legacy_root, ArtifactListingScope::MetadataOnly)?,
        );
    }
    Ok(derive_container_destination(
        preferred_root,
        legacy_root,
        container_id,
        &evidence,
    )?)
}

/// Acquire every source, mirror, marker, and destination listing used by
/// classification, plus the text of the declared citation scan universe.
pub fn collect_artifact_classification_facts<S: IssueStore>(
    storage: &S,
    target: &PlanTarget,
    destination_root: &str,
    artifacts: &[ArtifactPlanEntry],
    policy: &ArtifactClassificationPolicy,
    embedded_owners: Vec<EmbeddedArtifactOwner>,
) -> Result<ArtifactClassificationFacts> {
    let inspect_destinations = !policy.archive_root.is_empty();
    let mut evidence = ArtifactEvidenceMap::new();
    for artifact in artifacts
        .iter()
        .filter(|artifact| artifact.version() == &ArtifactVersion::WorkingTree)
    {
        let source = artifact.source();
        evidence.insert(
            source.to_string(),
            inspect_artifact_evidence(storage, source, ArtifactListingScope::MetadataOnly)?,
        );
        if inspect_destinations {
            if let Some(destination) =
                artifact_archive_destination(destination_root, &policy.development_root, source)
            {
                evidence.insert(
                    destination.clone(),
                    inspect_artifact_evidence(
                        storage,
                        &destination,
                        ArtifactListingScope::MetadataOnly,
                    )?,
                );
            }
        }
    }
    if matches!(target, PlanTarget::Container { .. }) && inspect_destinations {
        let destination = inspect_artifact_evidence(
            storage,
            destination_root,
            ArtifactListingScope::RecursiveFiles,
        )?;
        if matches!(destination, ArtifactEvidence::Directory { .. }) {
            let marker = format!("{destination_root}/.jit-container");
            evidence.insert(
                marker.clone(),
                inspect_artifact_evidence(storage, &marker, ArtifactListingScope::MetadataOnly)?,
            );
        }
        evidence.insert(destination_root.to_string(), destination);
    }
    Ok(ArtifactClassificationFacts {
        citations: collect_citation_scan_evidence(storage, &policy.citation_scan_roots),
        ..classification_facts_from_evidence(
            target,
            destination_root,
            artifacts,
            policy,
            embedded_owners,
            &evidence,
        )?
    })
}

/// Read the text of every file the declared citation scan roots reach.
///
/// Advisory plan evidence, so no path failure reaches the caller: a declared
/// root that is absent, a path component that is a symbolic link, a file whose
/// bytes are not valid UTF-8, and any other unreadable path are skipped, and the
/// plan is produced from whatever the scan did reach. A root naming one file
/// contributes that file; a root naming a directory contributes every file
/// beneath it.
///
/// The walk is [`inspect_artifact_evidence`]'s recursive listing, which opens
/// every component with the no-follow directory handles the rest of the planner
/// uses, so the scan escapes the repository through no symbolic link and starts
/// no external process.
pub(crate) fn collect_citation_scan_evidence<S: IssueStore>(
    storage: &S,
    roots: &[String],
) -> CitationScanEvidence {
    roots
        .iter()
        .map(|root| normalize_artifact_path(root))
        .filter(|root| !root.is_empty())
        .flat_map(|root| {
            match inspect_artifact_evidence(storage, &root, ArtifactListingScope::RecursiveFiles) {
                Ok(ArtifactEvidence::File(bytes)) => vec![(root, bytes)],
                Ok(ArtifactEvidence::Directory { entries, .. }) => entries
                    .into_iter()
                    .filter_map(|entry| read_scanned_file(storage, entry))
                    .collect(),
                _ => Vec::new(),
            }
        })
        .filter_map(|(path, bytes)| String::from_utf8(bytes).ok().map(|text| (path, text)))
        .collect()
}

/// Read one listed scan path, skipping everything that is not a regular file.
fn read_scanned_file<S: IssueStore>(storage: &S, path: String) -> Option<(String, Vec<u8>)> {
    match inspect_artifact_evidence(storage, &path, ArtifactListingScope::MetadataOnly) {
        Ok(ArtifactEvidence::File(bytes)) => Some((path, bytes)),
        _ => None,
    }
}

/// Inspect one repository-relative path without following any symlink component.
pub(crate) fn inspect_artifact_evidence<S: IssueStore>(
    storage: &S,
    path: &str,
    listing_scope: ArtifactListingScope,
) -> Result<ArtifactEvidence, PathReadError> {
    if matches!(
        validate_repo_relative_path(path),
        Err(PathReadError::InvalidPath(_) | PathReadError::OutsideRepoRoot(_))
    ) {
        return Ok(ArtifactEvidence::InvalidPath);
    }
    validate_repo_relative_path(path)?;
    let layout = storage.repository_layout().map_err(PathReadError::Other)?;
    let root = open_absolute_dir_nofollow(layout.worktree_root()).map_err(other)?;
    inspect_artifact_from_root(&root, path, listing_scope)
}

fn inspect_artifact_from_root(
    root: &Dir,
    path: &str,
    listing_scope: ArtifactListingScope,
) -> Result<ArtifactEvidence, PathReadError> {
    let relative = RootRelativePath::parse(path)
        .map_err(|error| PathReadError::InvalidPath(error.to_string()))?;
    let components = relative
        .as_path()
        .components()
        .map(|component| component.as_os_str().to_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) = components
        .split_last()
        .ok_or_else(|| PathReadError::InvalidPath(path.to_string()))?;
    let mut parent = root.try_clone().map_err(PathReadError::from)?;
    for component in parents {
        let metadata = match parent.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Ok(ArtifactEvidence::Missing)
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            return Ok(ArtifactEvidence::Symlink);
        }
        parent = open_child_dir_nofollow(&parent, component).map_err(other)?;
    }
    let metadata = match parent.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ArtifactEvidence::Missing),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() {
        return Ok(ArtifactEvidence::Symlink);
    }
    if metadata.is_file() {
        let mut file =
            open_regular_file_nofollow(&parent, &leaf.to_string_lossy()).map_err(other)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(PathReadError::from)?;
        return Ok(ArtifactEvidence::File(bytes));
    }
    if !metadata.is_dir() {
        return Ok(ArtifactEvidence::Unsupported);
    }
    let directory = open_child_dir_nofollow(&parent, leaf).map_err(other)?;
    let entries = match listing_scope {
        ArtifactListingScope::MetadataOnly => Vec::new(),
        ArtifactListingScope::ImmediateChildren => list_immediate_children(&directory, path)?,
        ArtifactListingScope::RecursiveFiles => list_recursive(&directory, path, false)?,
        ArtifactListingScope::RecursiveEntries => list_recursive(&directory, path, true)?,
    };
    Ok(ArtifactEvidence::Directory {
        scope: listing_scope,
        entries,
    })
}

fn list_immediate_children(directory: &Dir, relative: &str) -> Result<Vec<String>, PathReadError> {
    let mut entries = directory
        .entries()?
        .map(|entry| {
            entry
                .map_err(PathReadError::from)
                .and_then(|entry| child_path(relative, entry.file_name()))
        })
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    Ok(entries)
}

/// Walk `directory` with no-follow handles, naming every entry it reaches.
///
/// `name_directories` decides whether a directory is named as well as descended
/// through, which is the difference between
/// [`ArtifactListingScope::RecursiveFiles`] and
/// [`ArtifactListingScope::RecursiveEntries`]: without it a subtree holding no
/// file contributes nothing at all. A symbolic link is named as a leaf and
/// never descended into under either.
fn list_recursive(
    directory: &Dir,
    relative: &str,
    name_directories: bool,
) -> Result<Vec<String>, PathReadError> {
    let mut pending = vec![(directory.try_clone()?, relative.to_string())];
    let mut listed = Vec::new();
    while let Some((current, prefix)) = pending.pop() {
        for entry in current.entries()? {
            let entry = entry?;
            let name = entry.file_name();
            let path = child_path(&prefix, name.clone())?;
            let metadata = current.symlink_metadata(&name)?;
            if metadata.is_dir() && !metadata.is_symlink() {
                let child = open_child_dir_nofollow(&current, &name).map_err(other)?;
                pending.push((child, path.clone()));
                if name_directories {
                    listed.push(path);
                }
            } else {
                listed.push(path);
            }
        }
    }
    listed.sort();
    Ok(listed)
}

fn child_path(relative: &str, name: std::ffi::OsString) -> Result<String, PathReadError> {
    name.to_str()
        .ok_or_else(|| anyhow!("artifact path is not valid UTF-8"))
        .map(|name| format!("{relative}/{name}"))
        .map_err(PathReadError::Other)
}

fn other(error: impl Into<anyhow::Error>) -> PathReadError {
    PathReadError::Other(error.into())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::domain::artifact_classifier::{classify_artifacts, ArtifactClassificationInventory};
    use crate::domain::artifact_plan::{
        ArtifactAction, ArtifactOwner, ArtifactPlan, ArtifactProvenance, WarningCode,
    };
    use crate::domain::State;
    use crate::storage::JsonFileStorage;
    use std::collections::BTreeSet;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    const CONTAINER: &str = "abcdef12-3456-7890-abcd-ef1234567890";
    const CITED_SOURCE: &str = "dev/active/plan.md";
    const DESTINATION_ROOT: &str = "dev/archive/abcdef12";

    fn repository() -> (TempDir, JsonFileStorage) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        storage.configure_repository_layout(&layout);
        (repo, storage)
    }

    fn write(repo: &TempDir, path: &str, contents: impl AsRef<[u8]>) {
        let file = repo.path().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, contents).unwrap();
    }

    /// A shell-comment citation of the artifact the fixture plan relocates.
    fn citation() -> String {
        format!("#!/usr/bin/env bash\n# see {CITED_SOURCE} for the design\n")
    }

    fn scan_policy(roots: &[&str]) -> ArtifactClassificationPolicy {
        ArtifactClassificationPolicy::configured(
            "dev",
            vec!["dev/active".into()],
            vec!["docs".into()],
            "dev/archive",
        )
        .with_citation_scan_roots(roots.iter().map(|root| (*root).to_string()).collect())
    }

    /// Plan the relocation of one cited artifact with `roots` declared as the
    /// citation scan universe, acquiring evidence exactly as an archive preview
    /// does.
    fn plan_with_scan(storage: &JsonFileStorage, roots: &[&str]) -> ArtifactPlan {
        let target = PlanTarget::Container {
            id: CONTAINER.to_string(),
        };
        let artifacts = vec![ArtifactPlanEntry::new(
            CITED_SOURCE,
            ArtifactVersion::WorkingTree,
            ArtifactAction::Retain,
        )
        .with_provenance(vec![ArtifactProvenance::Explicit])
        .with_owners(vec![ArtifactOwner {
            issue: "owner".to_string(),
            document_index: 0,
            state: State::Done,
            archived_from: None,
            inside_subtree: true,
            pinned: false,
            selected_for_relink: false,
        }])];
        let policy = scan_policy(roots);
        let facts = collect_artifact_classification_facts(
            storage,
            &target,
            DESTINATION_ROOT,
            &artifacts,
            &policy,
            Vec::new(),
        )
        .unwrap();
        classify_artifacts(
            ArtifactClassificationInventory::new(target, artifacts, Vec::new())
                .with_destination_root(DESTINATION_ROOT),
            policy,
            facts,
        )
        .unwrap()
    }

    /// The scanned files a produced plan reports citations in, having confirmed
    /// the plan does relocate the cited artifact — a plan that leaves it in
    /// place would report nothing whatever the scan reached.
    fn citing_files(plan: &ArtifactPlan) -> BTreeSet<String> {
        assert!(
            plan.artifacts()
                .iter()
                .any(|artifact| artifact.action() == ArtifactAction::Move),
            "the planned artifact must relocate for its citations to be reported"
        );
        plan.artifacts()
            .iter()
            .flat_map(ArtifactPlanEntry::warnings)
            .filter(|warning| warning.code == WarningCode::MovingPathCitation)
            .filter_map(|warning| warning.path.as_deref())
            .filter_map(|location| location.rsplitn(3, ':').nth(2))
            .map(str::to_string)
            .collect()
    }

    fn paths(paths: &[&str]) -> BTreeSet<String> {
        paths.iter().map(|path| (*path).to_string()).collect()
    }

    #[test]
    fn test_inspect_artifact_evidence_distinguishes_complete_listing_scopes_without_following_symlinks(
    ) {
        let (repo, storage) = repository();
        fs::create_dir_all(repo.path().join("archive/sub")).unwrap();
        fs::write(repo.path().join("archive/sub/nested.md"), "nested").unwrap();
        // Holds no file at any depth, so only a listing that names the
        // directories it descends through can see it.
        fs::create_dir(repo.path().join("archive/empty")).unwrap();
        fs::create_dir(repo.path().join("outside")).unwrap();
        fs::write(repo.path().join("outside/hidden.md"), "hidden").unwrap();
        symlink(
            repo.path().join("outside"),
            repo.path().join("archive/link"),
        )
        .unwrap();

        let immediate =
            inspect_artifact_evidence(&storage, "archive", ArtifactListingScope::ImmediateChildren)
                .unwrap();
        assert_eq!(
            immediate,
            ArtifactEvidence::Directory {
                scope: ArtifactListingScope::ImmediateChildren,
                entries: vec![
                    "archive/empty".into(),
                    "archive/link".into(),
                    "archive/sub".into()
                ],
            }
        );
        assert_eq!(
            inspect_artifact_evidence(
                &storage,
                "archive",
                ArtifactListingScope::ImmediateChildren,
            )
            .unwrap(),
            immediate
        );
        assert_eq!(
            inspect_artifact_evidence(&storage, "archive", ArtifactListingScope::RecursiveFiles,)
                .unwrap(),
            ArtifactEvidence::Directory {
                scope: ArtifactListingScope::RecursiveFiles,
                entries: vec!["archive/link".into(), "archive/sub/nested.md".into()],
            }
        );
        // The same walk, naming the directories it descends through: the empty
        // one the files-only listing above cannot see, and the populated one it
        // reports only through the file inside.
        assert_eq!(
            inspect_artifact_evidence(&storage, "archive", ArtifactListingScope::RecursiveEntries)
                .unwrap(),
            ArtifactEvidence::Directory {
                scope: ArtifactListingScope::RecursiveEntries,
                entries: vec![
                    "archive/empty".into(),
                    "archive/link".into(),
                    "archive/sub".into(),
                    "archive/sub/nested.md".into(),
                ],
            }
        );
    }

    #[test]
    fn test_collect_citation_scan_evidence_reads_a_declared_root_outside_the_development_root() {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "scripts/nested/benchmark.sh", citation());
        write(&repo, "CHANGELOG.md", citation());
        write(&repo, "dev/notes.md", citation());

        let plan = plan_with_scan(&storage, &["scripts", "CHANGELOG.md"]);

        // Both declared roots sit outside the configured development root, one
        // of them naming a single file rather than a directory. The citing file
        // inside the development root that no root declares stays unread, so the
        // universe is the declared list and not the development root.
        assert_eq!(
            citing_files(&plan),
            paths(&["CHANGELOG.md", "scripts/nested/benchmark.sh"])
        );
    }

    #[test]
    fn test_collect_citation_scan_evidence_skips_an_absent_declared_root_without_failing_the_plan()
    {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "scripts/present.sh", citation());

        let plan = plan_with_scan(&storage, &["docs/absent", "scripts"]);

        // The plan is produced and the readable root still contributes, so the
        // absent root skipped rather than emptying or failing the scan.
        assert_eq!(citing_files(&plan), paths(&["scripts/present.sh"]));
    }

    #[test]
    fn test_collect_citation_scan_evidence_skips_a_symbolic_link_path_component_without_failing_the_plan(
    ) {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "outside/cited.sh", citation());
        write(&repo, "scripts/real.sh", citation());
        symlink(
            repo.path().join("outside"),
            repo.path().join("scripts/linked"),
        )
        .unwrap();
        symlink(repo.path().join("outside"), repo.path().join("linked-root")).unwrap();

        let plan = plan_with_scan(&storage, &["linked-root", "scripts"]);

        // The same citing file sits behind a symlinked declared root and behind
        // a symlinked component of a walked root; neither reaches it, while the
        // regular file beside the link is still read.
        assert_eq!(citing_files(&plan), paths(&["scripts/real.sh"]));
        // Declared by its real path, that same file is read, so the skip is the
        // symbolic link and not the file behind it.
        assert_eq!(
            citing_files(&plan_with_scan(&storage, &["outside"])),
            paths(&["outside/cited.sh"])
        );
    }

    #[test]
    fn test_collect_citation_scan_evidence_skips_a_file_whose_bytes_are_not_valid_utf8_without_failing_the_plan(
    ) {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "scripts/real.sh", citation());
        write(
            &repo,
            "scripts/deck.pdf",
            [b"\xff", citation().as_bytes(), b"\xfe"].concat(),
        );

        let plan = plan_with_scan(&storage, &["scripts"]);

        // The undecodable file carries the cited path in its bytes, so a lossy
        // read would report it; the plan is produced and only the text file is.
        assert_eq!(citing_files(&plan), paths(&["scripts/real.sh"]));
    }
}
