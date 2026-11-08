//! No-follow inspection of repository paths, and the advisory citation scan
//! built on it.

use crate::domain::archive_advisory::{AppliedChanges, CitationIndex, CitationScanner};
use crate::domain::artifact_classifier::contains_path;
use crate::domain::artifact_discovery::{ArtifactEvidence, ArtifactListingScope};
use crate::domain::artifact_plan::normalize_artifact_path;
use crate::domain::relative_path::RootRelativePath;
use crate::storage::capability_fs::{
    open_absolute_dir_nofollow, open_child_dir_nofollow, open_regular_file_nofollow,
    CapabilityFsError,
};
use crate::storage::repository_state_store::{
    is_advisory_permission_denied, RepositoryStateStoreError,
};
use crate::storage::{IssueStore, PathReadError};
use anyhow::anyhow;
use cap_std::fs::Dir;
use std::collections::BTreeSet;
use std::io::{ErrorKind, Read};

/// Index the citations of `sources` in every file the declared citation scan
/// roots reach, in one [`CitationScanner`] pass holding one file's bytes at a
/// time.
///
/// A declared root that is absent, a path component that is a symbolic link, a
/// permission-denied subtree, and a file whose bytes are not valid UTF-8 are
/// skipped. Other I/O failures stay hard errors. A root naming one file
/// contributes that file; a root naming a directory contributes every readable
/// file beneath it; a root nested in another declared root is walked once.
///
/// `applied` names what an execution changed before the scan, so the roots are
/// read as they stood before it: a file the execution created is skipped, and
/// a removed source within a root is read at its destination under its source
/// path.
///
/// The walk is [`inspect_artifact_evidence`]'s recursive listing, which opens
/// every component with no-follow directory handles, so the scan escapes the
/// repository through no symbolic link and starts no external process.
pub(crate) fn scan_citation_index<S: IssueStore>(
    storage: &S,
    roots: &[String],
    sources: impl IntoIterator<Item = String>,
    applied: &AppliedChanges,
) -> Result<CitationIndex, PathReadError> {
    let scanner = CitationScanner::new(sources).map_err(other)?;
    let roots = roots
        .iter()
        .map(|root| normalize_artifact_path(root))
        .filter(|root| !root.is_empty())
        .collect::<BTreeSet<_>>();
    let mut index = CitationIndex::default();
    let mut record = |path: &str, bytes: &[u8]| {
        if !applied.is_created(path) {
            scanner.scan(path, bytes, &mut index);
        }
    };
    for root in roots.iter().filter(|root| {
        !roots
            .iter()
            .any(|outer| outer != *root && contains_path(outer, root))
    }) {
        match inspect_artifact_evidence_with_policy(
            storage,
            root,
            ArtifactListingScope::RecursiveFiles,
            PermissionDeniedPolicy::Skip,
        )? {
            ArtifactEvidence::File(bytes) => record(root, &bytes),
            ArtifactEvidence::Directory { entries, .. } => {
                for entry in entries {
                    if let Some((path, bytes)) = read_scanned_file(storage, entry)? {
                        record(&path, &bytes);
                    }
                }
            }
            _ => {}
        }
    }
    for (source, destination) in applied
        .moved()
        .filter(|(source, _)| roots.iter().any(|root| contains_path(root, source)))
    {
        if let Some((_, bytes)) = read_scanned_file(storage, destination.to_string())? {
            scanner.scan(source, &bytes, &mut index);
        }
    }
    Ok(index)
}

/// Read one listed scan path, skipping everything that is not a regular file.
fn read_scanned_file<S: IssueStore>(
    storage: &S,
    path: String,
) -> Result<Option<(String, Vec<u8>)>, PathReadError> {
    match inspect_artifact_evidence_with_policy(
        storage,
        &path,
        ArtifactListingScope::MetadataOnly,
        PermissionDeniedPolicy::Skip,
    )? {
        ArtifactEvidence::File(bytes) => Ok(Some((path, bytes))),
        _ => Ok(None),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PermissionDeniedPolicy {
    Hard,
    Skip,
}

impl PermissionDeniedPolicy {
    fn skips(self, error: &(dyn std::error::Error + 'static)) -> bool {
        self == Self::Skip && is_permission_denied(error)
    }
}

fn is_permission_denied(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(cause) = current {
        if cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == ErrorKind::PermissionDenied)
            || cause
                .downcast_ref::<CapabilityFsError>()
                .is_some_and(|error| {
                    matches!(error, CapabilityFsError::Io(error)
                        if error.kind() == ErrorKind::PermissionDenied)
                })
            || cause
                .downcast_ref::<RepositoryStateStoreError>()
                .is_some_and(is_advisory_permission_denied)
        {
            return true;
        }
        current = cause.source();
    }
    false
}

/// Inspect one repository-relative path without following any symlink component.
pub(crate) fn inspect_artifact_evidence<S: IssueStore>(
    storage: &S,
    path: &str,
    listing_scope: ArtifactListingScope,
) -> Result<ArtifactEvidence, PathReadError> {
    inspect_artifact_evidence_with_policy(
        storage,
        path,
        listing_scope,
        PermissionDeniedPolicy::Hard,
    )
}

fn inspect_artifact_evidence_with_policy<S: IssueStore>(
    storage: &S,
    path: &str,
    listing_scope: ArtifactListingScope,
    permission_denied: PermissionDeniedPolicy,
) -> Result<ArtifactEvidence, PathReadError> {
    let relative = match RootRelativePath::parse(path) {
        Ok(relative) if !relative.is_root() => relative,
        _ => return Ok(ArtifactEvidence::InvalidPath),
    };
    let layout = storage.repository_layout().map_err(PathReadError::Other)?;
    let root = open_absolute_dir_nofollow(layout.worktree_root()).map_err(other)?;
    inspect_artifact_from_root(&root, path, &relative, listing_scope, permission_denied)
}

fn inspect_artifact_from_root(
    root: &Dir,
    path: &str,
    relative: &RootRelativePath,
    listing_scope: ArtifactListingScope,
    permission_denied: PermissionDeniedPolicy,
) -> Result<ArtifactEvidence, PathReadError> {
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
            Err(error) if permission_denied.skips(&error) => {
                return Ok(ArtifactEvidence::Unsupported)
            }
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            return Ok(ArtifactEvidence::Symlink);
        }
        parent = match open_child_dir_nofollow(&parent, component) {
            Ok(parent) => parent,
            Err(error) if permission_denied.skips(&error) => {
                return Ok(ArtifactEvidence::Unsupported)
            }
            Err(error) => return Err(other(error)),
        };
    }
    let metadata = match parent.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(ArtifactEvidence::Missing),
        Err(error) if permission_denied.skips(&error) => return Ok(ArtifactEvidence::Unsupported),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() {
        return Ok(ArtifactEvidence::Symlink);
    }
    if metadata.is_file() {
        let mut file = match open_regular_file_nofollow(&parent, &leaf.to_string_lossy()) {
            Ok(file) => file,
            Err(error) if permission_denied.skips(&error) => {
                return Ok(ArtifactEvidence::Unsupported)
            }
            Err(error) => return Err(other(error)),
        };
        let mut bytes = Vec::new();
        if let Err(error) = file.read_to_end(&mut bytes) {
            if permission_denied.skips(&error) {
                return Ok(ArtifactEvidence::Unsupported);
            }
            return Err(error.into());
        }
        return Ok(ArtifactEvidence::File(bytes));
    }
    if !metadata.is_dir() {
        return Ok(ArtifactEvidence::Unsupported);
    }
    let directory = match open_child_dir_nofollow(&parent, leaf) {
        Ok(directory) => directory,
        Err(error) if permission_denied.skips(&error) => return Ok(ArtifactEvidence::Unsupported),
        Err(error) => return Err(other(error)),
    };
    let entries = match listing_scope {
        ArtifactListingScope::MetadataOnly => Vec::new(),
        ArtifactListingScope::ImmediateChildren => list_immediate_children(&directory, path)?,
        ArtifactListingScope::RecursiveFiles => {
            list_recursive(&directory, path, false, permission_denied)?
        }
        ArtifactListingScope::RecursiveEntries => {
            list_recursive(&directory, path, true, permission_denied)?
        }
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
    permission_denied: PermissionDeniedPolicy,
) -> Result<Vec<String>, PathReadError> {
    let mut pending = vec![(directory.try_clone()?, relative.to_string())];
    let mut listed = Vec::new();
    while let Some((current, prefix)) = pending.pop() {
        let entries = match current.entries() {
            Ok(entries) => entries,
            Err(error) if permission_denied.skips(&error) => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) if permission_denied.skips(&error) => continue,
                Err(error) => return Err(error.into()),
            };
            let name = entry.file_name();
            let path = child_path(&prefix, name.clone())?;
            let metadata = match current.symlink_metadata(&name) {
                Ok(metadata) => metadata,
                Err(error) if permission_denied.skips(&error) => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.is_dir() && !metadata.is_symlink() {
                let child = match open_child_dir_nofollow(&current, &name) {
                    Ok(child) => child,
                    Err(error) if permission_denied.skips(&error) => continue,
                    Err(error) => return Err(other(error)),
                };
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
    use crate::storage::JsonFileStorage;
    use std::collections::BTreeSet;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    const CITED_SOURCE: &str = "dev/active/plan.md";

    fn repository() -> (TempDir, JsonFileStorage) {
        let repo = TempDir::new().unwrap();
        let storage = JsonFileStorage::new(repo.path().join(".jit"));
        fs::create_dir(storage.root()).unwrap();
        let layout =
            crate::storage::discover_repository_layout(repo.path(), storage.root()).unwrap();
        storage.configure_repository_layout(&layout).unwrap();
        (repo, storage)
    }

    fn write(repo: &TempDir, path: &str, contents: impl AsRef<[u8]>) {
        let file = repo.path().join(path);
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(file, contents).unwrap();
    }

    /// A shell-comment citation of [`CITED_SOURCE`].
    fn citation() -> String {
        format!("#!/usr/bin/env bash\n# see {CITED_SOURCE} for the design\n")
    }

    /// The scanned files citing [`CITED_SOURCE`] under `roots`.
    fn citing_files(storage: &JsonFileStorage, roots: &[&str]) -> BTreeSet<String> {
        let roots = roots
            .iter()
            .map(|root| (*root).to_string())
            .collect::<Vec<_>>();
        scan_citation_index(
            storage,
            &roots,
            [CITED_SOURCE.to_string()],
            &AppliedChanges::default(),
        )
        .unwrap()
        .citations(CITED_SOURCE)
        .map(|location| location.citing.clone())
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
    fn test_inspect_artifact_evidence_does_not_follow_leaf_or_traversal_symlinks() {
        let (repo, storage) = repository();
        write(&repo, "real/file.md", "content");
        fs::create_dir(repo.path().join("directory-artifact")).unwrap();
        symlink(
            repo.path().join("real/file.md"),
            repo.path().join("leaf.md"),
        )
        .unwrap();
        symlink(repo.path().join("real"), repo.path().join("linked-dir")).unwrap();
        let inspect = |path: &str| {
            inspect_artifact_evidence(&storage, path, ArtifactListingScope::MetadataOnly)
        };

        assert_eq!(inspect("leaf.md").unwrap(), ArtifactEvidence::Symlink);
        assert_eq!(
            inspect("linked-dir/file.md").unwrap(),
            ArtifactEvidence::Symlink
        );
        assert_eq!(inspect("missing.md").unwrap(), ArtifactEvidence::Missing);
        assert_eq!(
            inspect("real/file.md").unwrap(),
            ArtifactEvidence::File(b"content".to_vec())
        );
        assert!(matches!(
            inspect("directory-artifact").unwrap(),
            ArtifactEvidence::Directory { .. }
        ));
        assert!(matches!(
            inspect("real/file.md/child"),
            Err(PathReadError::Other(_))
        ));
    }

    #[test]
    fn test_scan_citation_index_reads_a_declared_root_outside_the_development_root() {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "scripts/nested/benchmark.sh", citation());
        write(&repo, "CHANGELOG.md", citation());
        write(&repo, "dev/notes.md", citation());

        let citing = citing_files(&storage, &["scripts", "CHANGELOG.md"]);

        // Both declared roots sit outside the configured development root, one
        // of them naming a single file rather than a directory. The citing file
        // inside the development root that no root declares stays unread, so the
        // universe is the declared list and not the development root.
        assert_eq!(
            citing,
            paths(&["CHANGELOG.md", "scripts/nested/benchmark.sh"])
        );
    }

    #[test]
    fn test_scan_citation_index_skips_an_absent_declared_root_without_failing_the_scan() {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "scripts/present.sh", citation());

        let citing = citing_files(&storage, &["docs/absent", "scripts"]);

        // The readable root still contributes, so the absent root is skipped
        // rather than emptying or failing the scan.
        assert_eq!(citing, paths(&["scripts/present.sh"]));
    }

    #[test]
    fn test_scan_citation_index_skips_a_symbolic_link_path_component_without_failing_the_scan() {
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

        let citing = citing_files(&storage, &["linked-root", "scripts"]);

        // The same citing file sits behind a symlinked declared root and behind
        // a symlinked component of a walked root; neither reaches it, while the
        // regular file beside the link is still read.
        assert_eq!(citing, paths(&["scripts/real.sh"]));
        // Declared by its real path, that same file is read, so the skip is the
        // symbolic link and not the file behind it.
        assert_eq!(
            citing_files(&storage, &["outside"]),
            paths(&["outside/cited.sh"])
        );
    }

    #[test]
    fn test_scan_citation_index_skips_a_file_whose_bytes_are_not_valid_utf8_without_failing_the_scan(
    ) {
        let (repo, storage) = repository();
        write(&repo, CITED_SOURCE, "plan");
        write(&repo, "scripts/real.sh", citation());
        write(
            &repo,
            "scripts/deck.pdf",
            [b"\xff", citation().as_bytes(), b"\xfe"].concat(),
        );

        let citing = citing_files(&storage, &["scripts"]);

        // The undecodable file carries the cited path in its bytes, so a lossy
        // read would report it; only the text file is.
        assert_eq!(citing, paths(&["scripts/real.sh"]));
    }

    #[test]
    fn test_scan_citation_index_after_an_execution_reads_the_roots_as_they_stood_before_it() {
        use crate::domain::artifact_execution::{ArchiveExecutionResult, ArchivePublication};
        use crate::domain::artifact_plan::{
            ArtifactAction, ArtifactPlan, ArtifactPlanEntry, ArtifactVersion, ContentIdentity,
            PlanTarget, PolicyStatus,
        };
        let (repo, storage) = repository();
        let (moved, destination, created) = (
            "scripts/moved.sh",
            "archive/scripts/moved.sh",
            "scripts/created.sh",
        );
        write(&repo, moved, citation());
        write(&repo, "scripts/stays.sh", citation());
        let roots = ["scripts".to_string()];
        let scan = |applied: &AppliedChanges| {
            scan_citation_index(&storage, &roots, [CITED_SOURCE.to_string()], applied).unwrap()
        };
        let before = scan(&AppliedChanges::default());

        // The execution relocates one citing file out of the root, byte for
        // byte, and creates another citing file inside it.
        fs::create_dir_all(repo.path().join("archive/scripts")).unwrap();
        fs::rename(repo.path().join(moved), repo.path().join(destination)).unwrap();
        write(&repo, created, citation());
        let identity = ContentIdentity::from_bytes(citation().as_bytes());
        let plan = ArtifactPlan::new(
            PlanTarget::Document { path: moved.into() },
            "archive",
            PolicyStatus::Configured,
            vec![
                ArtifactPlanEntry::new(moved, ArtifactVersion::WorkingTree, ArtifactAction::Move)
                    .with_destination(destination)
                    .with_content_identity(identity.clone()),
            ],
            Vec::new(),
            Vec::new(),
        )
        .unwrap();
        let publication = |source: Option<&str>, destination: &str| ArchivePublication {
            source: source.map(str::to_string),
            destination: destination.to_string(),
            content_identity: identity.clone(),
            adopted: false,
        };
        let result = ArchiveExecutionResult {
            schema_version: 1,
            target: plan.target().clone(),
            destination_root: "archive".into(),
            publications: vec![
                publication(Some(moved), destination),
                publication(None, created),
            ],
            reference_changes: Vec::new(),
            planned_deletions: Vec::new(),
            deleted_sources: vec![moved.to_string()],
            plan_warnings: Vec::new(),
            event_appended: true,
            reconciling: false,
        };

        assert_eq!(before.citations(CITED_SOURCE).count(), 2);
        assert_eq!(scan(&AppliedChanges::of(&plan, &result)), before);
    }
}
