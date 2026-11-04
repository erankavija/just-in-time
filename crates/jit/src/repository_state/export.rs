//! Typed classification and pure finalization of repository export files.

use super::{
    CaptureBudget, CaptureError, CaptureSpec, ExpectedPreimage, FileMode, MaterializationPlan,
    RepositoryAction, RepositoryActionOwner, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositoryLayout, RepositoryLayoutError, VirtualPath,
};
use crate::domain::relative_path::RootRelativePath;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// A lexically normalized output path classified outside both repository roots.
///
/// This type has no public constructor. External persistence boundaries accept it
/// instead of an arbitrary [`Path`], preventing repository-owned exports from
/// accidentally bypassing [`crate::storage::RepositoryStateStore`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExternalExportPath(PathBuf);

impl ExternalExportPath {
    /// Return the normalized external filesystem path.
    pub(crate) fn as_path(&self) -> &Path {
        &self.0
    }
}

/// Classification of one requested export destination.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RepositoryExportDestination {
    /// A canonical Data- or Worktree-owned target.
    Repository(VirtualPath),
    /// A normalized target outside both repository roots.
    External(ExternalExportPath),
}

/// Exact typed input to repository export finalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RepositoryExportIntent {
    target: VirtualPath,
    payload: RepositoryExportPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RepositoryExportPayload {
    ReplaceFile(Vec<u8>),
    CreateFile(Vec<u8>),
    CreateTree {
        directories: BTreeSet<RootRelativePath>,
        files: BTreeMap<RootRelativePath, Vec<u8>>,
    },
}

impl RepositoryExportIntent {
    /// Select one canonical repository target and its complete output bytes.
    pub(crate) fn new(target: VirtualPath, bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            target,
            payload: RepositoryExportPayload::ReplaceFile(bytes.into()),
        }
    }

    /// Select an absent repository target for one snapshot archive.
    pub(crate) fn new_absent_file(target: VirtualPath, bytes: Vec<u8>) -> Self {
        Self {
            target,
            payload: RepositoryExportPayload::CreateFile(bytes),
        }
    }

    /// Select an absent repository target for one complete snapshot tree.
    ///
    /// The resulting multi-action delta is replayed forward by its redo
    /// record; it does not claim rename-style observer isolation while the
    /// individual tree entries are published.
    pub(crate) fn new_tree(
        target: VirtualPath,
        mut directories: BTreeSet<RootRelativePath>,
        mut files: BTreeMap<RootRelativePath, Vec<u8>>,
    ) -> Self {
        directories.retain(|path| !path.is_root());
        files.retain(|path, _| !path.is_root());
        Self {
            target,
            payload: RepositoryExportPayload::CreateTree { directories, files },
        }
    }

    /// Build the bounded capture declaration for this export.
    ///
    /// The exact target, its immediate parent, and the parent's complete
    /// non-recursive listing are revalidated before publication.
    pub(crate) fn capture_spec(
        &self,
        budget: CaptureBudget,
    ) -> Result<CaptureSpec, RepositoryExportError> {
        let parent = immediate_parent(&self.target)?;
        let mut paths = BTreeSet::from([self.target.clone(), parent.clone()]);
        if let RepositoryExportPayload::CreateTree { directories, files } = &self.payload {
            for relative in directories.iter().chain(files.keys()) {
                paths.insert(tree_path(&self.target, relative)?);
            }
        }
        let mut spec = CaptureSpec::new(paths, budget)?;
        spec.discover_listing(parent)?;
        Ok(spec)
    }
}

/// Export classification or pure finalization failure.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RepositoryExportError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error(transparent)]
    Delta(#[from] super::DeltaError),
    #[error("repository export target must be below a selected root: {0:?}")]
    RootTarget(VirtualPath),
    #[error("repository export parent is not an ordinary directory: {0:?}")]
    UnsafeParent(VirtualPath),
    #[error("repository export target has an unsafe occupant: {0:?}")]
    UnsafeTarget(VirtualPath),
    #[error("repository export target already exists: {0:?}")]
    OccupiedTarget(VirtualPath),
}

/// Lexically normalize and classify a requested output path.
///
/// Relative paths are resolved against the explicit invocation directory. Data
/// classification is authoritative when the selected data root is nested under
/// the worktree; only `OutsideRepositoryRoots` becomes an external destination.
pub(crate) fn classify_repository_export(
    layout: &RepositoryLayout,
    invocation_dir: &Path,
    requested: &Path,
) -> Result<RepositoryExportDestination, RepositoryExportError> {
    let physical = normalize_absolute(if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        invocation_dir.join(requested)
    })?;
    match layout.classify_and_canonicalize(&physical) {
        Ok(path) => Ok(RepositoryExportDestination::Repository(path)),
        Err(RepositoryLayoutError::OutsideRepositoryRoots(_)) => Ok(
            RepositoryExportDestination::External(ExternalExportPath(physical)),
        ),
        Err(error) => Err(error.into()),
    }
}

/// Finalize one repository-contained export as an exact recoverable plan.
pub(crate) fn finalize_repository_export(
    base: &RepositoryImage,
    intent: &RepositoryExportIntent,
) -> Result<MaterializationPlan, RepositoryExportError> {
    finalize_export_payload(base, &intent.target, intent.payload.clone())
}

fn finalize_export_payload(
    base: &RepositoryImage,
    target: &VirtualPath,
    payload: RepositoryExportPayload,
) -> Result<MaterializationPlan, RepositoryExportError> {
    base.layout().ensure_canonical(target)?;
    let parent = immediate_parent(target)?;
    if !base.directory_listings().contains_key(&parent) {
        return Err(CaptureError::UndiscoveredRepositoryPath(parent).into());
    }
    if !matches!(base.entry(&parent)?, RepositoryEntry::Directory { .. }) {
        return Err(RepositoryExportError::UnsafeParent(parent));
    }
    let target_entry = base.entry(target)?;
    let actions = match payload {
        RepositoryExportPayload::ReplaceFile(bytes) => {
            let mode = match target_entry {
                RepositoryEntry::Absent => FileMode::Regular,
                RepositoryEntry::File { mode, .. } => *mode,
                _ => return Err(RepositoryExportError::UnsafeTarget(target.clone())),
            };
            vec![RepositoryAction::WriteFile {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryExport,
                expected: ExpectedPreimage::of(target_entry),
                bytes,
                mode,
            }]
        }
        RepositoryExportPayload::CreateFile(bytes) => {
            require_absent(target_entry, target)?;
            vec![RepositoryAction::WriteFile {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryExport,
                expected: ExpectedPreimage::Absent,
                bytes,
                mode: FileMode::Regular,
            }]
        }
        RepositoryExportPayload::CreateTree { directories, files } => {
            require_absent(target_entry, target)?;
            let mut actions = vec![RepositoryAction::CreateDirectory {
                path: target.clone(),
                owner: RepositoryActionOwner::RepositoryExport,
                expected: ExpectedPreimage::Absent,
            }];
            for relative in directories {
                let path = tree_path(target, &relative)?;
                require_absent(base.entry(&path)?, &path)?;
                actions.push(RepositoryAction::CreateDirectory {
                    path,
                    owner: RepositoryActionOwner::RepositoryExport,
                    expected: ExpectedPreimage::Absent,
                });
            }
            for (relative, bytes) in files {
                let path = tree_path(target, &relative)?;
                require_absent(base.entry(&path)?, &path)?;
                actions.push(RepositoryAction::WriteFile {
                    path,
                    owner: RepositoryActionOwner::RepositoryExport,
                    expected: ExpectedPreimage::Absent,
                    bytes,
                    mode: FileMode::Regular,
                });
            }
            actions
        }
    };
    let delta = RepositoryDelta::new(base.layout(), actions)?;
    Ok(MaterializationPlan::new(base, delta))
}

fn require_absent(
    entry: &RepositoryEntry,
    path: &VirtualPath,
) -> Result<(), RepositoryExportError> {
    if matches!(entry, RepositoryEntry::Absent) {
        Ok(())
    } else {
        Err(RepositoryExportError::OccupiedTarget(path.clone()))
    }
}

fn tree_path(
    target: &VirtualPath,
    relative: &RootRelativePath,
) -> Result<VirtualPath, RepositoryExportError> {
    if relative.is_root() {
        return Ok(target.clone());
    }
    let joined = target.relative().as_path().join(relative.as_path());
    Ok(VirtualPath::from_root(
        target.root_class(),
        RootRelativePath::parse(joined).map_err(RepositoryLayoutError::from)?,
    )?)
}

fn immediate_parent(path: &VirtualPath) -> Result<VirtualPath, RepositoryExportError> {
    let relative = path.relative().as_path();
    if relative.as_os_str().is_empty() {
        return Err(RepositoryExportError::RootTarget(path.clone()));
    }
    Ok(VirtualPath::from_root(
        path.root_class(),
        RootRelativePath::parse(relative.parent().unwrap_or_else(|| Path::new("")))
            .map_err(RepositoryLayoutError::from)?,
    )?)
}

fn normalize_absolute(path: PathBuf) -> Result<PathBuf, RepositoryLayoutError> {
    if !path.is_absolute() {
        return Err(RepositoryLayoutError::RelativeRoot(path));
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::Normal(value) => {
                if value.to_string_lossy().chars().any(char::is_control) {
                    return Err(RepositoryLayoutError::LexicalEscape(
                        path.to_string_lossy().into_owned(),
                    ));
                }
                normalized.push(value);
            }
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(RepositoryLayoutError::LexicalEscape(
                        path.to_string_lossy().into_owned(),
                    ));
                }
            }
            Component::Prefix(_) => {
                return Err(RepositoryLayoutError::LexicalEscape(
                    path.to_string_lossy().into_owned(),
                ));
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{CapturedBytes, DirectoryListing, EntryIdentity};
    use crate::test_harness::image::{synthetic_layout, ImageBuilder};

    const CAPTURE_BUDGET: CaptureBudget = CaptureBudget {
        max_listings: 1,
        max_bytes: 1024,
        max_depth: 8,
    };

    fn identity(name: &str, bytes: &[u8]) -> EntryIdentity {
        EntryIdentity::for_bytes(name, bytes).unwrap()
    }

    #[test]
    fn test_classify_repository_export_prefers_data_then_worktree_then_external() {
        let layout = synthetic_layout();
        assert!(matches!(
            classify_repository_export(&layout, Path::new("/repo"), Path::new(".jit/out.json"))
                .unwrap(),
            RepositoryExportDestination::Repository(path)
                if path == VirtualPath::data("out.json").unwrap()
        ));
        assert!(matches!(
            classify_repository_export(&layout, Path::new("/repo/sub"), Path::new("../out.json"))
                .unwrap(),
            RepositoryExportDestination::Repository(path)
                if path == VirtualPath::worktree("out.json").unwrap()
        ));
        assert!(matches!(
            classify_repository_export(&layout, Path::new("/repo"), Path::new("../out.json"))
                .unwrap(),
            RepositoryExportDestination::External(_)
        ));
    }

    #[test]
    fn test_finalize_repository_export_preserves_existing_mode_and_preimage() {
        let target = VirtualPath::worktree("out.json").unwrap();
        let old = b"old";
        let intent = RepositoryExportIntent::new(target.clone(), b"new".to_vec());
        let parent = immediate_parent(&target).unwrap();
        let image = ImageBuilder::new()
            .with_spec(intent.capture_spec(CAPTURE_BUDGET).unwrap())
            .entry(
                target.clone(),
                RepositoryEntry::File {
                    content: CapturedBytes::new("target", old.to_vec()).unwrap(),
                    mode: FileMode::Executable,
                },
            )
            .entry(
                parent.clone(),
                RepositoryEntry::Directory {
                    identity: identity("parent", b"directory"),
                    mode: FileMode::Regular,
                },
            )
            .listing(parent, DirectoryListing::default())
            .build();
        let plan = finalize_repository_export(&image, &intent).unwrap();
        assert!(matches!(
            &plan.delta().actions()[0],
            RepositoryAction::WriteFile {
                owner,
                expected: ExpectedPreimage::File { .. },
                bytes,
                mode: FileMode::Executable,
                ..
            } if *owner == RepositoryActionOwner::RepositoryExport && bytes == b"new"
        ));
    }

    #[test]
    fn test_finalize_repository_export_rejects_wrong_kind_target() {
        let target = VirtualPath::worktree("out.json").unwrap();
        let intent = RepositoryExportIntent::new(target.clone(), b"new".to_vec());
        let parent = immediate_parent(&target).unwrap();
        let image = ImageBuilder::new()
            .with_spec(intent.capture_spec(CAPTURE_BUDGET).unwrap())
            .entry(
                target.clone(),
                RepositoryEntry::Symlink {
                    target: CapturedBytes::new("target", b"elsewhere".to_vec()).unwrap(),
                    mode: FileMode::Regular,
                },
            )
            .entry(
                parent.clone(),
                RepositoryEntry::Directory {
                    identity: identity("parent", b"directory"),
                    mode: FileMode::Regular,
                },
            )
            .listing(parent, DirectoryListing::default())
            .build();
        let result = finalize_repository_export(&image, &intent);
        assert!(matches!(
            result,
            Err(RepositoryExportError::UnsafeTarget(path)) if path == target
        ));
    }

    #[test]
    fn test_snapshot_tree_capture_and_plan_cover_every_absent_path() {
        let target = VirtualPath::data("exports/snapshot").unwrap();
        let parent = VirtualPath::data("exports").unwrap();
        let intent = RepositoryExportIntent::new_tree(
            target.clone(),
            BTreeSet::from([
                RootRelativePath::parse("").unwrap(),
                RootRelativePath::parse("a").unwrap(),
                RootRelativePath::parse("a/b").unwrap(),
            ]),
            BTreeMap::from([(
                RootRelativePath::parse("a/b/file").unwrap(),
                b"content".to_vec(),
            )]),
        );
        let spec = intent.capture_spec(CAPTURE_BUDGET).unwrap();
        let captured = spec.paths().cloned().collect::<BTreeSet<_>>();
        assert_eq!(
            captured,
            BTreeSet::from([
                parent.clone(),
                target.clone(),
                VirtualPath::data("exports/snapshot/a").unwrap(),
                VirtualPath::data("exports/snapshot/a/b").unwrap(),
                VirtualPath::data("exports/snapshot/a/b/file").unwrap(),
            ])
        );
        assert_eq!(spec.listings(), &BTreeSet::from([parent.clone()]));

        let image = captured
            .into_iter()
            .fold(ImageBuilder::new().with_spec(spec), |builder, path| {
                let entry = if path == parent {
                    RepositoryEntry::Directory {
                        identity: identity("parent", b"directory"),
                        mode: FileMode::Regular,
                    }
                } else {
                    RepositoryEntry::Absent
                };
                builder.entry(path, entry)
            })
            .listing(parent, DirectoryListing::default())
            .build();
        let plan = finalize_repository_export(&image, &intent).unwrap();
        assert_eq!(plan.delta().actions().len(), 4);
        assert!(matches!(
            &plan.delta().actions()[0],
            RepositoryAction::CreateDirectory { path, .. } if path == &target
        ));
        assert!(matches!(
            &plan.delta().actions()[3],
            RepositoryAction::WriteFile { path, bytes, .. }
                if path == &VirtualPath::data("exports/snapshot/a/b/file").unwrap()
                    && bytes == b"content"
        ));
    }

    #[test]
    fn test_snapshot_file_requires_an_absent_target() {
        let target = VirtualPath::worktree("snapshot.tar").unwrap();
        let intent = RepositoryExportIntent::new_absent_file(target.clone(), b"new".to_vec());
        let parent = immediate_parent(&target).unwrap();
        let image = ImageBuilder::new()
            .with_spec(intent.capture_spec(CAPTURE_BUDGET).unwrap())
            .file(target.clone(), b"old")
            .entry(
                parent.clone(),
                RepositoryEntry::Directory {
                    identity: identity("parent", b"directory"),
                    mode: FileMode::Regular,
                },
            )
            .listing(parent, DirectoryListing::default())
            .build();
        let result = finalize_repository_export(&image, &intent);
        assert!(matches!(
            result,
            Err(RepositoryExportError::OccupiedTarget(path)) if path == target
        ));
    }
}
