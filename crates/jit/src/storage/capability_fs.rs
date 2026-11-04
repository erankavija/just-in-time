//! No-follow capability primitives: root handles and the walks, opens and reads that keep every traversal beneath a held directory.

use crate::domain::relative_path::RootRelativePath;
use crate::repository_state::{RepositoryLayout, RepositoryRootClass};
use crate::storage::atomic_write::sync_capability_directory;
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use cap_std::{ambient_authority, fs::MetadataExt as _};
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};

/// Failure of a no-follow primitive.
#[derive(Debug, thiserror::Error)]
pub enum CapabilityFsError {
    #[error("unsafe repository mutation target: {0}")]
    UnsafeTarget(String),
    #[error("repository changed after capture: {path}")]
    RetryableConflict { path: String },
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Open no-follow handles for the selected roots, constructed only inside the
/// session boundary that verified their identity.
pub struct CapabilityRoots {
    worktree: Dir,
    data: Option<DataRoot>,
}

/// An open no-follow handle on a data root, or on the stage that becomes one.
///
/// Holding it pins the directory it opened: a record committed through it
/// lands there however the path is rebound afterwards.
pub struct DataRoot(Dir);

impl DataRoot {
    /// Open the directory at the absolute `path` without following links.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` is relative, is absent, or resolves
    /// through a link.
    pub fn open(path: &Path) -> Result<Self, CapabilityFsError> {
        open_absolute_dir_nofollow(path).map(Self)
    }

    pub(in crate::storage) fn from_dir(directory: Dir) -> Self {
        Self(directory)
    }

    pub(in crate::storage) fn dir(&self) -> &Dir {
        &self.0
    }
}

impl CapabilityRoots {
    pub(in crate::storage) fn open(
        layout: &RepositoryLayout,
    ) -> Result<(Self, Dir, String), CapabilityFsError> {
        let worktree = open_absolute_dir_nofollow(layout.worktree_root())?;
        ensure_capability_identity(
            &worktree,
            layout.worktree_identity(),
            layout.worktree_root(),
        )?;
        let data_parent_path = layout.data_root().parent().ok_or_else(|| {
            CapabilityFsError::UnsafeTarget(layout.data_root().display().to_string())
        })?;
        let data_parent = open_absolute_dir_nofollow(data_parent_path)?;
        let data_leaf = layout
            .data_root()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                CapabilityFsError::UnsafeTarget(layout.data_root().display().to_string())
            })?
            .to_string();
        let data = match data_parent.symlink_metadata(&data_leaf) {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
                let data = open_child_dir_nofollow(&data_parent, &data_leaf)?;
                ensure_capability_identity(&data, layout.data_identity(), layout.data_root())?;
                Some(DataRoot::from_dir(data))
            }
            Ok(_) => {
                return Err(CapabilityFsError::UnsafeTarget(
                    layout.data_root().display().to_string(),
                ))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        // Bind the held parent capability to the absent-root parent recorded in
        // the layout. The fd already resists path redirection; this additionally
        // rejects a parent swapped out from under the layout before capture, so a
        // staged root is never published beneath a directory the layout never saw.
        if data.is_none() {
            if let Some(parent_identity) = capability_dir_identity(&data_parent) {
                let expected = format!("absent:{parent_identity}:{data_leaf}");
                if expected != layout.data_identity() {
                    return Err(CapabilityFsError::RetryableConflict {
                        path: layout.data_root().display().to_string(),
                    });
                }
            }
        }
        Ok((Self { worktree, data }, data_parent, data_leaf))
    }

    pub(in crate::storage) fn data_root(&self) -> Option<&DataRoot> {
        self.data.as_ref()
    }

    pub(in crate::storage) fn root(&self, class: RepositoryRootClass) -> Option<&Dir> {
        match class {
            RepositoryRootClass::Worktree => Some(&self.worktree),
            RepositoryRootClass::Data => self.data.as_ref().map(DataRoot::dir),
        }
    }
}

/// Open the parent directory of a repository-relative target for capture.
///
/// `Ok(None)` means an ancestor directory does not exist, so the target captures
/// as absence. A symlinked or non-directory ancestor stays a hard error: it is an
/// unsafe target, not a clean absence.
pub(in crate::storage) fn open_capability_parent(
    root: &Dir,
    path: &Path,
) -> Result<Option<(Dir, String)>, CapabilityFsError> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) = components
        .split_last()
        .ok_or_else(|| CapabilityFsError::UnsafeTarget(path.display().to_string()))?;
    let mut current = root.try_clone()?;
    for component in parents {
        match current.symlink_metadata(component) {
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                return Err(CapabilityFsError::UnsafeTarget(component.clone()))
            }
            Ok(_) => current = open_child_dir_nofollow(&current, component)?,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some((current, leaf.clone())))
}

/// Open the parent directory of a repository-relative target, creating the
/// ancestors it lacks.
///
/// Each directory it creates is synced into the parent that now names it before
/// the walk descends, so a file published underneath is reachable after a crash
/// rather than hanging off an entry the filesystem never wrote
/// (`@/inv/atomic-writes`). An ancestor that already exists costs no sync.
pub(in crate::storage) fn open_or_create_capability_parent(
    root: &Dir,
    path: &Path,
) -> Result<(Dir, String), CapabilityFsError> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) = components
        .split_last()
        .ok_or_else(|| CapabilityFsError::UnsafeTarget(path.display().to_string()))?;
    parents
        .iter()
        .try_fold(root.try_clone()?, |current, component| {
            match current.symlink_metadata(component) {
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    current.create_dir(component)?;
                    sync_capability_directory(&current)?;
                }
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            open_child_dir_nofollow(&current, component)
        })
        .map(|parent| (parent, leaf.clone()))
}

pub(in crate::storage) fn open_descendant_dir_nofollow(
    root: &Dir,
    path: &Path,
) -> Result<Dir, CapabilityFsError> {
    let mut current = root.try_clone()?;
    for component in path.components() {
        current = open_child_dir_nofollow(&current, Path::new(component.as_os_str()))?;
    }
    Ok(current)
}

pub(crate) fn open_child_dir_nofollow(
    parent: &Dir,
    name: impl AsRef<Path>,
) -> Result<Dir, CapabilityFsError> {
    let name = name.as_ref();
    let metadata = parent.symlink_metadata(name)?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err(CapabilityFsError::UnsafeTarget(name.display().to_string()));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    options._cap_fs_ext_maybe_dir(true);
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(CapabilityFsError::UnsafeTarget(name.display().to_string()));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

/// Open `leaf` under `parent` as an ordinary file, refusing a symlink or any
/// non-regular target without following it.
pub(crate) fn open_regular_file_nofollow(
    parent: &Dir,
    leaf: &str,
) -> Result<cap_std::fs::File, CapabilityFsError> {
    let metadata = parent.symlink_metadata(leaf)?;
    if metadata.is_symlink() || !metadata.is_file() {
        return Err(CapabilityFsError::UnsafeTarget(leaf.to_string()));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let file = parent.open_with(leaf, &options)?;
    if !file.metadata()?.is_file() {
        return Err(CapabilityFsError::UnsafeTarget(leaf.to_string()));
    }
    Ok(file)
}

pub(crate) fn open_absolute_dir_nofollow(path: &Path) -> Result<Dir, CapabilityFsError> {
    if !path.is_absolute() {
        return Err(CapabilityFsError::UnsafeTarget(path.display().to_string()));
    }
    let mut anchor = PathBuf::new();
    let mut descendants = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => anchor.push(component.as_os_str()),
            Component::Normal(name) => descendants.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(CapabilityFsError::UnsafeTarget(path.display().to_string()))
            }
        }
    }
    let mut current = Dir::open_ambient_dir(&anchor, ambient_authority())?;
    for name in descendants {
        current = open_child_dir_nofollow(&current, name)?;
    }
    Ok(current)
}

/// Read one validated root-relative ordinary file without following links.
///
/// `Ok(None)` means the leaf or one of its parents is absent. Symlinked
/// ancestors, symlink leaves, directories, and special files are unsafe rather
/// than absence.
pub(crate) fn read_repository_file_nofollow(
    root: &Path,
    relative: &RootRelativePath,
) -> Result<Option<Vec<u8>>, CapabilityFsError> {
    if relative.is_root() {
        return Err(CapabilityFsError::UnsafeTarget(root.display().to_string()));
    }
    let root = open_absolute_dir_nofollow(root)?;
    let Some((parent, leaf)) = open_capability_parent(&root, relative.as_path())? else {
        return Ok(None);
    };
    let metadata = match parent.symlink_metadata(&leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() || !metadata.is_file() {
        return Err(CapabilityFsError::UnsafeTarget(
            relative.as_path().display().to_string(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(&leaf, &options)?;
    if !file.metadata()?.is_file() {
        return Err(CapabilityFsError::UnsafeTarget(
            relative.as_path().display().to_string(),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Read one validated root-relative ordinary file through a confined root handle.
///
/// Symlinks may be followed, but capability resolution keeps every traversal
/// beneath `root`; a raced path cannot redirect the final open outside it.
pub(crate) fn read_repository_file_confined(
    root: &Path,
    relative: &RootRelativePath,
) -> Result<Option<Vec<u8>>, CapabilityFsError> {
    if relative.is_root() {
        return Err(CapabilityFsError::UnsafeTarget(root.display().to_string()));
    }
    let root_dir = open_absolute_dir_nofollow(root)?;
    read_confined_relative(root, &root_dir, relative.as_path(), 0)
}

fn read_confined_relative(
    root_path: &Path,
    root: &Dir,
    relative: &Path,
    symlink_depth: usize,
) -> Result<Option<Vec<u8>>, CapabilityFsError> {
    if symlink_depth >= 40 {
        return Err(CapabilityFsError::UnsafeTarget(
            relative.display().to_string(),
        ));
    }
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(CapabilityFsError::UnsafeTarget(
                relative.display().to_string(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = root.try_clone()?;
    let mut traversed = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        let metadata = match current.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            let target = current.read_link_contents(component)?;
            let resolved =
                resolve_confined_symlink(root_path, &traversed, &target, &components[index + 1..])?;
            return read_confined_relative(root_path, root, &resolved, symlink_depth + 1);
        }
        if index + 1 == components.len() {
            if !metadata.is_file() {
                return Err(CapabilityFsError::UnsafeTarget(
                    relative.display().to_string(),
                ));
            }
            let mut options = OpenOptions::new();
            options.read(true);
            options._cap_fs_ext_follow(FollowSymlinks::No);
            let mut file = current.open_with(component, &options)?;
            if !file.metadata()?.is_file() {
                return Err(CapabilityFsError::UnsafeTarget(
                    relative.display().to_string(),
                ));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok(Some(bytes));
        }
        if !metadata.is_dir() {
            return Err(CapabilityFsError::UnsafeTarget(
                relative.display().to_string(),
            ));
        }
        current = open_child_dir_nofollow(&current, component)?;
        traversed.push(component);
    }
    Err(CapabilityFsError::UnsafeTarget(
        relative.display().to_string(),
    ))
}

fn resolve_confined_symlink(
    root: &Path,
    parent: &Path,
    target: &Path,
    tail: &[std::ffi::OsString],
) -> Result<PathBuf, CapabilityFsError> {
    let is_absolute = target.is_absolute();
    let target = if is_absolute {
        target
            .strip_prefix(root)
            .map_err(|_| CapabilityFsError::UnsafeTarget(target.display().to_string()))?
    } else {
        target
    };
    let mut resolved = if is_absolute {
        PathBuf::new()
    } else {
        parent.to_path_buf()
    };
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => resolved.push(name),
            Component::ParentDir if resolved.pop() => {}
            _ => {
                return Err(CapabilityFsError::UnsafeTarget(
                    target.display().to_string(),
                ))
            }
        }
    }
    resolved.extend(tail);
    Ok(resolved)
}

fn ensure_capability_identity(
    directory: &Dir,
    expected: &str,
    path: &Path,
) -> Result<(), CapabilityFsError> {
    let actual = capability_metadata_identity(&directory.dir_metadata()?)?;
    if actual == expected {
        Ok(())
    } else {
        Err(CapabilityFsError::RetryableConflict {
            path: path.display().to_string(),
        })
    }
}

/// Directory-capability identity in the same `dev:ino` form the layout records
/// for the absent-root parent. `None` on platforms without stable inode identity,
/// where the absent-root parent binding is skipped.
pub(crate) fn capability_dir_identity(directory: &Dir) -> Option<String> {
    #[cfg(any(unix, windows))]
    {
        let metadata = directory.dir_metadata().ok()?;
        capability_metadata_identity(&metadata).ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = directory;
        None
    }
}

#[cfg(unix)]
pub(in crate::storage) fn capability_metadata_identity(
    metadata: &cap_std::fs::Metadata,
) -> Result<String, CapabilityFsError> {
    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
pub(in crate::storage) fn capability_metadata_identity(
    metadata: &cap_std::fs::Metadata,
) -> Result<String, CapabilityFsError> {
    let volume =
        cap_primitives::fs::_WindowsByHandle::volume_serial_number(metadata).ok_or_else(|| {
            CapabilityFsError::UnsafeTarget("Windows volume identity unavailable".into())
        })?;
    let index = cap_primitives::fs::_WindowsByHandle::file_index(metadata).ok_or_else(|| {
        CapabilityFsError::UnsafeTarget("Windows file identity unavailable".into())
    })?;
    Ok(format!("{volume}:{index}"))
}

#[cfg(not(any(unix, windows)))]
pub(in crate::storage) fn capability_metadata_identity(
    metadata: &cap_std::fs::Metadata,
) -> Result<String, CapabilityFsError> {
    Ok(format!(
        "{}:{}",
        metadata.len(),
        metadata.permissions().readonly()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::repository_state_store::RepositoryStateStoreError;
    use crate::storage::RedoApplyError;

    #[test]
    fn test_capability_fs_error_conversions_preserve_variant_and_message() {
        let errors = || {
            [
                CapabilityFsError::UnsafeTarget("link".into()),
                CapabilityFsError::RetryableConflict {
                    path: "root".into(),
                },
                CapabilityFsError::Io(std::io::Error::other("disk")),
            ]
        };
        let messages = errors().map(|error| error.to_string());

        let store = errors().map(RepositoryStateStoreError::from);
        assert!(matches!(
            store.as_slice(),
            [
                RepositoryStateStoreError::UnsafeTarget(_),
                RepositoryStateStoreError::RetryableConflict { .. },
                RepositoryStateStoreError::Io(_)
            ]
        ));
        assert_eq!(store.map(|error| error.to_string()), messages);

        let redo = errors().map(RedoApplyError::from);
        assert!(redo
            .iter()
            .all(|error| matches!(error, RedoApplyError::Roots(_))));
        assert_eq!(redo.map(|error| error.to_string()), messages);
    }
}
