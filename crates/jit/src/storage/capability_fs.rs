//! No-follow capability primitives: root handles and the walks, opens and reads that keep every traversal beneath a held directory.

use crate::domain::relative_path::RootRelativePath;
use crate::repository_state::{RepositoryLayout, RepositoryRootClass, RepositoryRootEvidence};
use crate::storage::atomic_write::sync_capability_directory;
use crate::storage::redo_record::{RedoRecord, WorktreeRootBinding};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, DirEntry, File, Metadata, OpenOptions};
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
    worktree: Option<(Dir, WorktreeRootBinding)>,
    data: Option<DataRoot>,
}

/// An open no-follow handle on a data root, or on the stage that becomes one.
///
/// Holding it pins the directory it opened: a record committed through it
/// lands there however the path is rebound afterwards.
pub struct DataRoot {
    directory: Dir,
    #[cfg(test)]
    refuse_sync: bool,
}

impl DataRoot {
    /// Open the directory at the absolute `path` without following links.
    ///
    /// # Errors
    ///
    /// Returns an error when `path` is relative, is absent, or resolves
    /// through a link.
    pub fn open(path: &Path) -> Result<Self, CapabilityFsError> {
        open_absolute_dir_nofollow(path).map(Self::from_dir)
    }

    pub(in crate::storage) fn from_dir(directory: Dir) -> Self {
        Self {
            directory,
            #[cfg(test)]
            refuse_sync: false,
        }
    }

    pub(in crate::storage) fn dir(&self) -> &Dir {
        &self.directory
    }

    /// Fsync the root, so entries created in it are durable.
    pub(in crate::storage) fn sync(&self) -> std::io::Result<()> {
        #[cfg(test)]
        if self.refuse_sync {
            return Err(std::io::Error::other("injected root fsync failure"));
        }
        sync_capability_directory(&self.directory)
    }

    /// Test seam: a handle whose root fsync fails.
    #[cfg(test)]
    pub(in crate::storage) fn refusing_sync(mut self) -> Self {
        self.refuse_sync = true;
        self
    }
}

impl CapabilityRoots {
    pub(in crate::storage) fn open(
        layout: &RepositoryLayout,
    ) -> Result<(Self, Dir, String), CapabilityFsError> {
        let evidence =
            RepositoryRootEvidence::new(layout.data_root(), layout.data_identity(), true);
        let (mut roots, parent, leaf) = Self::open_data(&evidence)?;
        roots.bind_worktree(&WorktreeRootBinding::from_layout(layout))?;
        Ok((roots, parent, leaf))
    }

    pub(in crate::storage) fn open_data(
        evidence: &RepositoryRootEvidence,
    ) -> Result<(Self, Dir, String), CapabilityFsError> {
        let data_parent_path = evidence.path().parent().ok_or_else(|| {
            CapabilityFsError::UnsafeTarget(evidence.path().display().to_string())
        })?;
        let data_parent = open_absolute_dir_nofollow(data_parent_path)?;
        let data_leaf = evidence
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| CapabilityFsError::UnsafeTarget(evidence.path().display().to_string()))?
            .to_string();
        let data = match data_parent.symlink_metadata(&data_leaf) {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
                let data = open_child_dir_nofollow(&data_parent, &data_leaf)?;
                ensure_capability_identity(&data, evidence.identity(), evidence.path())?;
                Some(DataRoot::from_dir(data))
            }
            Ok(_) => {
                return Err(CapabilityFsError::UnsafeTarget(
                    evidence.path().display().to_string(),
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
                if expected != evidence.identity() {
                    return Err(CapabilityFsError::RetryableConflict {
                        path: evidence.path().display().to_string(),
                    });
                }
            }
        }
        Ok((
            Self {
                worktree: None,
                data,
            },
            data_parent,
            data_leaf,
        ))
    }

    pub(in crate::storage) fn bind_worktree(
        &mut self,
        binding: &WorktreeRootBinding,
    ) -> Result<(), CapabilityFsError> {
        let directory = open_absolute_dir_nofollow(binding.path())?;
        ensure_capability_identity(&directory, binding.identity(), binding.path())?;
        self.worktree = Some((directory, binding.clone()));
        Ok(())
    }

    pub(in crate::storage) fn validate_record_root(
        &self,
        record: &RedoRecord,
    ) -> Result<(), crate::storage::RedoApplyError> {
        let Some(binding) = record.worktree_binding() else {
            return Ok(());
        };
        let failure = |source: anyhow::Error| crate::storage::RedoApplyError::OriginalWorktree {
            path: binding.path().to_path_buf(),
            source,
        };
        if self.worktree.as_ref().map(|(_, held)| held) != Some(binding) {
            return Err(failure(anyhow::anyhow!(
                "held worktree does not match the committed binding"
            )));
        }
        let current =
            open_absolute_dir_nofollow(binding.path()).map_err(|error| failure(error.into()))?;
        ensure_capability_identity(&current, binding.identity(), binding.path())
            .map_err(|error| failure(error.into()))
    }

    #[cfg(test)]
    pub(in crate::storage) fn record_layout(&self) -> RepositoryLayout {
        let binding = &self.worktree.as_ref().unwrap().1;
        RepositoryLayout::new(
            RepositoryRootEvidence::new(binding.path(), binding.identity(), true),
            RepositoryRootEvidence::new(binding.path().join(".jit"), "fixture-data", true),
        )
        .unwrap()
    }

    pub(in crate::storage) fn data_root(&self) -> Option<&DataRoot> {
        self.data.as_ref()
    }

    pub(in crate::storage) fn root(&self, class: RepositoryRootClass) -> Option<&Dir> {
        match class {
            RepositoryRootClass::Worktree => self.worktree.as_ref().map(|(directory, _)| directory),
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

/// The kind required of an opened leaf handle.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LeafKind {
    File,
    Directory,
}

#[derive(Debug, thiserror::Error)]
#[error("leaf is not of the required filesystem kind")]
struct UnexpectedLeafKind;

/// Open without following the leaf; non-blocking mode lets metadata reject
/// pipes and devices without waiting for a peer.
fn open_leaf_nofollow(
    kind: LeafKind,
    open: impl FnOnce(&OpenOptions) -> std::io::Result<File>,
) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    options._cap_fs_ext_maybe_dir(matches!(kind, LeafKind::Directory));
    options._cap_fs_ext_nonblock(true);
    let file = open(&options)?;
    let metadata = file.metadata()?;
    let expected = match kind {
        LeafKind::File => metadata.is_file(),
        LeafKind::Directory => metadata.is_dir(),
    };
    if expected {
        Ok(file)
    } else {
        Err(std::io::Error::new(
            ErrorKind::InvalidInput,
            UnexpectedLeafKind,
        ))
    }
}

/// Open a listed leaf through the directory handle that listed it.
pub(crate) fn open_entry_nofollow(entry: &DirEntry, kind: LeafKind) -> std::io::Result<File> {
    open_leaf_nofollow(kind, |options| entry.open_with(options))
}

/// Open beneath a held directory, refusing a leaf link or an escaping path.
pub(crate) fn open_path_nofollow(
    directory: &Dir,
    relative: &Path,
    kind: LeafKind,
) -> std::io::Result<File> {
    open_leaf_nofollow(kind, |options| directory.open_with(relative, options))
}

fn leaf_open_error(path: &Path, error: std::io::Error) -> CapabilityFsError {
    match classify_open_failure(&error) {
        OpenRefusal::Io => error.into(),
        _ => CapabilityFsError::UnsafeTarget(path.display().to_string()),
    }
}

pub(crate) fn open_child_dir_nofollow(
    parent: &Dir,
    name: impl AsRef<Path>,
) -> Result<Dir, CapabilityFsError> {
    let name = name.as_ref();
    let file = open_path_nofollow(parent, name, LeafKind::Directory)
        .map_err(|error| leaf_open_error(name, error))?;
    Ok(Dir::from_std_file(file.into_std()))
}

/// Open an ordinary file under a held parent, refusing links and special files.
pub(crate) fn open_regular_file_nofollow(
    parent: &Dir,
    leaf: &str,
) -> Result<File, CapabilityFsError> {
    open_path_nofollow(parent, Path::new(leaf), LeafKind::File)
        .map_err(|error| leaf_open_error(Path::new(leaf), error))
}

/// Read an ordinary leaf; report refusals at `reported_path` and absence as no bytes.
pub(crate) fn read_leaf_nofollow(
    parent: &Dir,
    leaf: impl AsRef<Path>,
    reported_path: &Path,
) -> Result<Option<Vec<u8>>, CapabilityFsError> {
    let leaf = leaf.as_ref();
    let mut file = match open_path_nofollow(parent, leaf, LeafKind::File)
        .map_err(|error| leaf_open_error(reported_path, error))
    {
        Ok(file) => file,
        Err(CapabilityFsError::Io(error)) if error.kind() == ErrorKind::NotFound => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// What a failed no-follow open says about the name it was given.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenRefusal {
    /// The name is a symbolic link, which a no-follow open refuses outright.
    Symlink,
    /// The leaf cannot serve the required filesystem kind.
    Unopenable,
    /// Resolving the name left the directory the open was anchored at.
    Escape,
    /// Ordinary I/O: the name is absent, unreadable, or the open failed for a
    /// reason that says nothing about what the name is.
    Io,
}

/// Distinguish kind refusals, links and capability escapes from ordinary I/O.
#[cfg(unix)]
pub(crate) fn classify_open_failure(error: &std::io::Error) -> OpenRefusal {
    if error
        .get_ref()
        .is_some_and(|source| source.is::<UnexpectedLeafKind>())
    {
        return OpenRefusal::Unopenable;
    }
    match error.raw_os_error() {
        Some(code) if code == nix::libc::ELOOP || code == nix::libc::EMLINK => OpenRefusal::Symlink,
        Some(code) if code == nix::libc::ENXIO => OpenRefusal::Unopenable,
        None if error.kind() == std::io::ErrorKind::PermissionDenied => OpenRefusal::Escape,
        _ => OpenRefusal::Io,
    }
}

/// Non-Unix platforms distinguish handle-kind and capability-escape refusals.
#[cfg(not(unix))]
pub(crate) fn classify_open_failure(error: &std::io::Error) -> OpenRefusal {
    if error
        .get_ref()
        .is_some_and(|source| source.is::<UnexpectedLeafKind>())
    {
        return OpenRefusal::Unopenable;
    }
    if error.raw_os_error().is_none() && error.kind() == std::io::ErrorKind::PermissionDenied {
        OpenRefusal::Escape
    } else {
        OpenRefusal::Io
    }
}

/// Whether the opened file carries an executable Unix mode.
#[cfg(unix)]
pub(crate) fn is_executable(metadata: &Metadata) -> bool {
    use cap_std::fs::PermissionsExt as _;

    metadata.permissions().mode() & 0o111 != 0
}

/// Windows has no executable mode bit for package-reader purposes.
#[cfg(not(unix))]
pub(crate) fn is_executable(_metadata: &Metadata) -> bool {
    false
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
    read_leaf_nofollow(&parent, &leaf, relative.as_path())
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
            return read_leaf_nofollow(&current, component, relative);
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
    use tempfile::TempDir;

    #[cfg(unix)]
    #[test]
    fn test_open_regular_file_nofollow_refuses_a_symbolic_link() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let target = temp.path().join("target.txt");
        std::fs::write(&target, b"outside content").unwrap();
        symlink(&target, temp.path().join("link.txt")).unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();

        let error = open_regular_file_nofollow(&dir, "link.txt").unwrap_err();

        assert!(matches!(error, CapabilityFsError::UnsafeTarget(_)));
    }

    #[test]
    fn test_open_regular_file_nofollow_refuses_a_directory() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join("subdir")).unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();

        let error = open_regular_file_nofollow(&dir, "subdir").unwrap_err();

        assert!(matches!(error, CapabilityFsError::UnsafeTarget(_)));
    }

    #[test]
    fn test_open_child_dir_nofollow_requires_a_directory_handle() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join("directory")).unwrap();
        std::fs::write(temp.path().join("file"), b"content").unwrap();
        let parent = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        let directory = open_child_dir_nofollow(&parent, "directory").unwrap();
        assert!(directory.dir_metadata().unwrap().is_dir());
        assert!(matches!(
            open_child_dir_nofollow(&parent, "file"),
            Err(CapabilityFsError::UnsafeTarget(_))
        ));
    }

    #[test]
    fn test_read_leaf_nofollow_returns_absence_or_the_regular_files_bytes() {
        let temp = TempDir::new().unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        assert_eq!(
            read_leaf_nofollow(&dir, "missing", Path::new("missing")).unwrap(),
            None
        );
        std::fs::write(temp.path().join("record"), b"record bytes").unwrap();
        assert_eq!(
            read_leaf_nofollow(&dir, "record", Path::new("record")).unwrap(),
            Some(b"record bytes".to_vec())
        );
    }

    #[test]
    fn test_read_repository_file_reports_the_root_relative_unsafe_leaf_path() {
        let temp = TempDir::new().unwrap();
        let relative = RootRelativePath::parse("issues/abc.json").unwrap();
        std::fs::create_dir_all(temp.path().join(relative.as_path())).unwrap();

        for read in [read_repository_file_nofollow, read_repository_file_confined] {
            let error = read(temp.path(), &relative).unwrap_err();
            assert!(matches!(
                error,
                CapabilityFsError::UnsafeTarget(path) if Path::new(&path) == relative.as_path()
            ));
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_open_regular_file_nofollow_refuses_a_fifo_without_waiting_for_a_writer() {
        let temp = TempDir::new().unwrap();
        nix::unistd::mkfifo(&temp.path().join("pipe"), nix::sys::stat::Mode::S_IRWXU).unwrap();
        let dir = Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            sender
                .send(open_regular_file_nofollow(&dir, "pipe"))
                .unwrap()
        });
        let error = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("a pipe must be refused without waiting for a writer")
            .unwrap_err();
        assert!(matches!(error, CapabilityFsError::UnsafeTarget(_)));
    }

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
                RepositoryStateStoreError::UnsafeTarget(target),
                RepositoryStateStoreError::RetryableConflict { path },
                RepositoryStateStoreError::Io(error)
            ] if target == "link" && path == "root" && error.to_string() == "disk"
        ));
        assert_eq!(store[0].to_string(), messages[0]);
        assert_eq!(store[2].to_string(), messages[2]);

        let redo = errors().map(RedoApplyError::from);
        assert!(redo
            .iter()
            .all(|error| matches!(error, RedoApplyError::Roots(_))));
        assert_eq!(redo.map(|error| error.to_string()), messages);
    }
}
