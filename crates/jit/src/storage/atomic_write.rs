//! The shared file-write primitives: the file publication core
//! ([`publish_file`]) and its stage writer, the control-file entries over it,
//! directory durability, and no-replace rename.
//!
//! The module lives in the storage layer because persistence is storage's
//! responsibility: command/validation/output callers produce content and hand
//! it here rather than touching the filesystem themselves.
//!
//! Every item here except [`ControlFile`] and [`publish_control_file`] is
//! `pub(in crate::storage)`. Publication into a repository root belongs to the
//! mutation-session API, so `crate::commands` must not be able to name these
//! primitives at all; the compiler, not a source-text guard, enforces that. The
//! crate-visible entry writes only the data-root leaves [`ControlFile`] closes
//! over. Export publication to a caller-chosen path outside the repository roots
//! lives in [`super::external_publish`], which reaches these primitives from
//! inside `crate::storage`.

use crate::repository_state::{FileMode, VirtualPath};
use crate::storage::worktree_paths::WorktreePaths;
use anyhow::{Context, Result};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use std::ffi::OsStr;
use std::io::Read;
use std::path::{Component, Path};

/// How much of a publication's durability [`publish_file`] completes itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::storage) enum Durability {
    /// Sync the staged file, then the parent directory after the rename.
    Full,
    /// Sync the staged file; the caller syncs the parent directory.
    Deferred,
    /// Sync nothing, for tests whose subject is coordination rather than
    /// crash durability.
    #[cfg(test)]
    Unsynced,
}

/// Replace `leaf` in `parent` with `bytes` under `mode`.
///
/// Durability argument: the stage's bytes are synced before the rename, so a
/// rename that persists names durable bytes, and a crash leaves the target
/// holding either its previous content or `bytes` (`@/inv/atomic-writes`). The
/// rename itself persists once the parent directory is synced, which `Full`
/// does here and `Deferred` leaves to the caller; a test's `Unsynced` syncs
/// neither. `before_rename` runs between the synced stage and the rename: it
/// is the caller's last refusal.
///
/// # Errors
///
/// Returns the first failing step. A failure before the rename removes the
/// stage and leaves the previous target; a `Full` directory-sync failure is
/// reported after the rename has replaced the target.
pub(in crate::storage) fn publish_file<E: From<std::io::Error>>(
    parent: &Dir,
    leaf: impl AsRef<Path>,
    bytes: &[u8],
    mode: FileMode,
    durability: Durability,
    before_rename: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    let stage = unique_stage_name();
    create_staged_file(parent, &stage, bytes, mode)
        .and_then(|file| match durability {
            #[cfg(test)]
            Durability::Unsynced => Ok(()),
            _ => file.sync_all(),
        })
        .map_err(E::from)
        .and_then(|()| before_rename())
        .and_then(|()| {
            parent
                .rename(&stage, parent, leaf.as_ref())
                .map_err(E::from)
        })
        .inspect_err(|_| {
            let _ = parent.remove_file(&stage);
        })?;
    match durability {
        Durability::Full => sync_capability_directory(parent).map_err(E::from),
        Durability::Deferred => Ok(()),
        #[cfg(test)]
        Durability::Unsynced => Ok(()),
    }
}

/// A machine-local file jit publishes directly in the data root, outside the
/// repository state the redo record carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControlFile {
    /// The identity of the data root's one checkout.
    WorktreeIdentity,
    /// The running server's process record.
    ServerPid,
}

impl ControlFile {
    fn path(self) -> VirtualPath {
        match self {
            Self::WorktreeIdentity => VirtualPath::WORKTREE_IDENTITY,
            Self::ServerPid => VirtualPath::SERVER_PID,
        }
    }
}

/// Replace `file` in the data root at `data_root` with `bytes`, fully durable.
///
/// # Errors
///
/// Fails when `data_root` is relative, absent or reached through a link, or
/// when [`publish_file`] fails.
pub(crate) fn publish_control_file(
    data_root: &Path,
    file: ControlFile,
    bytes: &[u8],
) -> Result<()> {
    let leaf = file.path();
    let root = super::capability_fs::open_absolute_dir_nofollow(data_root)
        .with_context(|| format!("opening data root {}", data_root.display()))?;
    publish_file(
        &root,
        leaf.relative().as_path(),
        bytes,
        FileMode::Regular,
        Durability::Full,
        || Ok::<(), std::io::Error>(()),
    )
    .with_context(|| {
        format!(
            "publishing {}",
            data_root.join(leaf.relative().as_path()).display()
        )
    })
}

/// Replace `target`, a file under one of `paths`' git control directories
/// ([`WorktreePaths::git_control_dirs`]), with `bytes`. Callers pass
/// `Durability::Full`, or `Unsynced` in a test.
///
/// The control directories lie outside every repository root, so the parent is
/// opened following links; it must exist. `before_rename` is
/// [`publish_file`]'s last refusal.
///
/// # Errors
///
/// Fails when `target` lies outside the control directories, its parent is
/// absent, or [`publish_file`] fails.
pub(in crate::storage) fn publish_git_control_file(
    paths: &WorktreePaths,
    target: &Path,
    bytes: &[u8],
    durability: Durability,
    before_rename: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let (parent, leaf) = open_git_control_parent(paths, target)?;
    publish_file(
        &parent,
        leaf,
        bytes,
        FileMode::Regular,
        durability,
        before_rename,
    )
    .with_context(|| format!("publishing {}", target.display()))
}

/// Remove `target`, a file under one of `paths`' git control directories, and
/// sync its directory unless `durability` is `Unsynced`; an absent file is
/// already removed.
///
/// # Errors
///
/// Fails when `target` lies outside the control directories, its parent is
/// absent, or the removal or the sync fails.
pub(in crate::storage) fn remove_git_control_file(
    paths: &WorktreePaths,
    target: &Path,
    durability: Durability,
) -> Result<()> {
    let (parent, leaf) = open_git_control_parent(paths, target)?;
    match parent.remove_file(leaf) {
        Ok(()) => sync_directory_for(&parent, durability),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
    .with_context(|| format!("removing {}", target.display()))
}

/// The parent of `target`, opened following links, and its leaf name, once
/// `target` is shown to lie beneath one of `paths`' git control directories.
fn open_git_control_parent<'t>(
    paths: &WorktreePaths,
    target: &'t Path,
) -> Result<(Dir, &'t OsStr)> {
    let beneath = |dir: &Path| {
        target.strip_prefix(dir).is_ok_and(|rest| {
            rest.components().next().is_some()
                && rest
                    .components()
                    .all(|component| matches!(component, Component::Normal(_)))
        })
    };
    let (Some(parent), Some(leaf)) = (target.parent(), target.file_name()) else {
        anyhow::bail!("{} names no control file", target.display());
    };
    if !paths.git_control_dirs().any(|dir| beneath(&dir)) {
        anyhow::bail!(
            "{} lies outside the git control directories",
            target.display()
        );
    }
    let parent =
        open_dir_following(parent).with_context(|| format!("opening {}", parent.display()))?;
    Ok((parent, leaf))
}

/// Open the directory at `path`, following links, as a handle that can be
/// synced; `Dir::open_ambient_dir` yields one fsync refuses.
pub(in crate::storage) fn open_dir_following(path: &Path) -> std::io::Result<Dir> {
    let directory = std::fs::File::open(path)?;
    match directory.metadata()?.is_dir() {
        true => Ok(Dir::from_std_file(directory)),
        false => Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("{} is not a directory", path.display()),
        )),
    }
}

const STAGE_PREFIX: &str = ".jit-stage.";
const STAGE_SUFFIX: &str = ".tmp";

/// A stage name for a publication beside its target: a random UUID, so no
/// other writer draws it and nothing can be waiting at it, and `.tmp`, so a
/// stage its publication never renamed is reclaimed by the orphan sweep in
/// [`super::temp_cleanup`].
pub(in crate::storage) fn unique_stage_name() -> String {
    format!(
        "{STAGE_PREFIX}{}{STAGE_SUFFIX}",
        uuid::Uuid::new_v4().simple()
    )
}

/// Whether `name` is one [`unique_stage_name`] draws.
#[cfg(test)]
pub(in crate::storage) fn is_stage_name(name: &str) -> bool {
    name.strip_prefix(STAGE_PREFIX)
        .and_then(|rest| rest.strip_suffix(STAGE_SUFFIX))
        .is_some_and(|id| uuid::Uuid::try_parse(id).is_ok())
}

/// [`create_staged_file`], returning once the file is durable.
pub(in crate::storage) fn write_staged_file(
    parent: &Dir,
    leaf: impl AsRef<Path>,
    content: impl Read,
    mode: FileMode,
) -> std::io::Result<()> {
    create_staged_file(parent, leaf, content, mode)?.sync_all()
}

/// Write `content` under `mode` as a new file named `leaf`, unsynced. Every
/// staged file in storage is created here, whichever publication writes it.
///
/// The open creates the file or fails: an occupied name is never opened
/// through, so no symlink carries the bytes elsewhere and no occupant is
/// truncated (`@/inv/atomic-writes`).
fn create_staged_file(
    parent: &Dir,
    leaf: impl AsRef<Path>,
    mut content: impl Read,
    mode: FileMode,
) -> std::io::Result<cap_std::fs::File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(leaf, &options)?;
    std::io::copy(&mut content, &mut file)?;
    #[cfg(unix)]
    file.set_permissions(permissions(mode))?;
    #[cfg(not(unix))]
    let _ = mode;
    Ok(file)
}

/// The permission bits a staged file carries.
#[cfg(unix)]
fn permissions(mode: FileMode) -> cap_std::fs::Permissions {
    use cap_std::fs::PermissionsExt;

    cap_std::fs::Permissions::from_mode(match mode {
        FileMode::Regular => 0o644,
        FileMode::Executable => 0o755,
    })
}

/// Fsync `directory`, so entries created, renamed or removed in it survive a
/// crash; a file's own bytes need their own sync.
pub(in crate::storage) fn sync_capability_directory(directory: &Dir) -> std::io::Result<()> {
    directory.try_clone()?.into_std_file().sync_all()
}

/// [`sync_capability_directory`], skipped for `Unsynced`.
pub(in crate::storage) fn sync_directory_for(
    directory: &Dir,
    durability: Durability,
) -> std::io::Result<()> {
    match durability {
        #[cfg(test)]
        Durability::Unsynced => Ok(()),
        _ => sync_capability_directory(directory),
    }
}

#[cfg(target_os = "linux")]
pub(in crate::storage) fn rename_noreplace_cap(
    source_dir: &Dir,
    source: impl AsRef<OsStr>,
    target_dir: &Dir,
    target: impl AsRef<OsStr>,
) -> std::io::Result<()> {
    use nix::fcntl::{renameat2, RenameFlags};
    renameat2(
        source_dir,
        source.as_ref(),
        target_dir,
        target.as_ref(),
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(std::io::Error::from)
}

#[cfg(not(target_os = "linux"))]
pub(in crate::storage) fn rename_noreplace_cap(
    _source_dir: &Dir,
    _source: impl AsRef<OsStr>,
    _target_dir: &Dir,
    _target: impl AsRef<OsStr>,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace rename is unsupported on this target",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::temp_cleanup::cleanup_orphaned_temp_files;
    use std::collections::HashSet;
    use tempfile::TempDir;

    /// A stage name a test can occupy, standing in for one drawn at random.
    const OCCUPIED_STAGE: &str = ".jit-stage.occupied.tmp";

    fn directory_handle(path: &Path) -> Dir {
        crate::storage::capability_fs::open_absolute_dir_nofollow(path).unwrap()
    }

    /// A handle to `path` that resolves names beneath it but cannot be synced:
    /// an `O_PATH` descriptor, which fsync refuses.
    #[cfg(target_os = "linux")]
    fn unsyncable_directory(path: &Path) -> Dir {
        use nix::fcntl::{open, OFlag};
        use nix::sys::stat::Mode;

        let descriptor = open(
            path,
            OFlag::O_PATH | OFlag::O_DIRECTORY | OFlag::O_CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        Dir::from_std_file(std::fs::File::from(descriptor))
    }

    /// Every entry name in `path`.
    fn entries(path: &Path) -> HashSet<String> {
        std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect()
    }

    fn publish(parent: &Dir, durability: Durability) -> std::io::Result<()> {
        publish_file(
            parent,
            "target.json",
            b"published",
            FileMode::Regular,
            durability,
            || Ok(()),
        )
    }

    #[test]
    fn test_publish_file_replaces_the_target_and_leaves_no_stage() {
        let directory = TempDir::new().unwrap();
        std::fs::write(directory.path().join("target.json"), b"previous").unwrap();

        publish(&directory_handle(directory.path()), Durability::Full).unwrap();

        assert_eq!(
            std::fs::read(directory.path().join("target.json")).unwrap(),
            b"published"
        );
        assert_eq!(
            entries(directory.path()),
            HashSet::from(["target.json".into()])
        );
    }

    /// A parent the process may not write into refuses the stage itself.
    #[cfg(unix)]
    #[test]
    fn test_publish_file_failed_stage_write_leaves_the_previous_target_and_no_stage() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TempDir::new().unwrap();
        let parent = directory.path().join("parent");
        std::fs::create_dir(&parent).unwrap();
        std::fs::write(parent.join("target.json"), b"previous").unwrap();
        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o500)).unwrap();

        let outcome = publish(&directory_handle(&parent), Durability::Full);

        std::fs::set_permissions(&parent, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(
            outcome.is_err(),
            "the stage was written into a read-only parent"
        );
        assert_eq!(
            std::fs::read(parent.join("target.json")).unwrap(),
            b"previous"
        );
        assert_eq!(entries(&parent), HashSet::from(["target.json".into()]));
    }

    /// A non-empty directory at the target is one the rename cannot replace.
    #[test]
    fn test_publish_file_failed_rename_leaves_the_previous_target_and_no_stage() {
        let directory = TempDir::new().unwrap();
        std::fs::create_dir(directory.path().join("target.json")).unwrap();
        std::fs::write(directory.path().join("target.json/held"), b"previous").unwrap();

        let outcome = publish(&directory_handle(directory.path()), Durability::Full);

        assert!(
            outcome.is_err(),
            "the rename replaced a non-empty directory"
        );
        assert_eq!(
            std::fs::read(directory.path().join("target.json/held")).unwrap(),
            b"previous"
        );
        assert_eq!(
            entries(directory.path()),
            HashSet::from(["target.json".into()])
        );
    }

    #[test]
    fn test_publish_file_refused_before_rename_leaves_the_previous_target_and_no_stage() {
        let directory = TempDir::new().unwrap();
        std::fs::write(directory.path().join("target.json"), b"previous").unwrap();

        let outcome = publish_file(
            &directory_handle(directory.path()),
            "target.json",
            b"published",
            FileMode::Regular,
            Durability::Full,
            || Err(std::io::Error::other("refused")),
        );

        assert!(outcome.is_err());
        assert_eq!(
            std::fs::read(directory.path().join("target.json")).unwrap(),
            b"previous"
        );
        assert_eq!(
            entries(directory.path()),
            HashSet::from(["target.json".into()])
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_publish_file_full_reports_a_directory_sync_failure() {
        let directory = TempDir::new().unwrap();

        let outcome = publish(&unsyncable_directory(directory.path()), Durability::Full);

        assert!(
            outcome.is_err(),
            "the publication returned without syncing its parent"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_publish_file_deferred_leaves_the_directory_sync_to_its_caller() {
        let directory = TempDir::new().unwrap();

        publish(
            &unsyncable_directory(directory.path()),
            Durability::Deferred,
        )
        .unwrap();

        assert_eq!(
            std::fs::read(directory.path().join("target.json")).unwrap(),
            b"published"
        );
    }

    #[test]
    fn test_unique_stage_name_is_distinct_across_concurrent_writers_to_one_target() {
        let names = std::thread::scope(|scope| {
            let writers = (0..16)
                .map(|_| scope.spawn(|| (0..16).map(|_| unique_stage_name()).collect::<Vec<_>>()))
                .collect::<Vec<_>>();
            writers
                .into_iter()
                .flat_map(|writer| writer.join().unwrap())
                .collect::<HashSet<_>>()
        });

        assert_eq!(
            names.len(),
            256,
            "two writers drew one stage name, which a create-new open turns into a refused publication"
        );
    }

    /// A stage is reconstructible from what its publication was given, so the
    /// sweep is its reclaim path.
    #[test]
    fn test_unique_stage_name_is_reclaimed_by_the_orphan_temp_sweep() {
        let directory = TempDir::new().unwrap();
        let orphan = directory.path().join(unique_stage_name());
        std::fs::write(&orphan, b"a stage its publication never renamed").unwrap();

        assert_eq!(cleanup_orphaned_temp_files(directory.path(), 0).unwrap(), 1);
        assert!(!orphan.exists(), "the sweep left the orphaned stage");
    }

    #[cfg(unix)]
    #[test]
    fn test_write_staged_file_refuses_a_symlink_occupying_the_staged_name() {
        let directory = TempDir::new().unwrap();
        let unrelated = directory.path().join("unrelated.json");
        std::fs::write(&unrelated, b"a file the record never named").unwrap();
        // Relative and in-tree: a confined handle resolves it, where it refuses
        // an absolute target outright.
        std::os::unix::fs::symlink("unrelated.json", directory.path().join(OCCUPIED_STAGE))
            .unwrap();
        let parent = directory_handle(directory.path());

        let outcome = write_staged_file(
            &parent,
            OCCUPIED_STAGE,
            &b"published"[..],
            FileMode::Regular,
        );

        assert!(
            outcome.is_err(),
            "the staging opened through a symlink waiting at its name"
        );
        assert_eq!(
            std::fs::read(&unrelated).unwrap(),
            b"a file the record never named",
            "the staged bytes landed on a path the record never named"
        );
    }

    #[test]
    fn test_write_staged_file_refuses_a_file_occupying_the_staged_name() {
        let directory = TempDir::new().unwrap();
        let occupant = directory.path().join(OCCUPIED_STAGE);
        std::fs::write(&occupant, b"a staging another application holds").unwrap();
        let parent = directory_handle(directory.path());

        let outcome = write_staged_file(
            &parent,
            OCCUPIED_STAGE,
            &b"published"[..],
            FileMode::Regular,
        );

        assert!(outcome.is_err(), "the staging truncated an occupied name");
        assert_eq!(
            std::fs::read(&occupant).unwrap(),
            b"a staging another application holds"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_publish_control_file_refuses_a_symlinked_data_root() {
        let directory = TempDir::new().unwrap();
        let real = directory.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let linked = directory.path().join(".jit");
        std::os::unix::fs::symlink(&real, &linked).unwrap();

        let outcome = publish_control_file(&linked, ControlFile::ServerPid, b"published");

        assert!(
            outcome.is_err(),
            "the publication followed the data root link"
        );
        assert!(entries(&real).is_empty());
    }

    #[test]
    fn test_publish_control_file_over_an_absent_data_root_creates_nothing() {
        let directory = TempDir::new().unwrap();
        let data_root = directory.path().join(".jit");

        let outcome = publish_control_file(&data_root, ControlFile::WorktreeIdentity, b"published");

        assert!(outcome.is_err());
        assert!(!data_root.exists(), "the publication created the data root");
    }

    fn publish_git(paths: &WorktreePaths, target: &Path) -> Result<()> {
        publish_git_control_file(paths, target, b"published", Durability::Full, || Ok(()))
    }

    #[test]
    fn test_publish_git_control_file_refuses_a_target_outside_the_control_directories() {
        let directory = TempDir::new().unwrap();
        let paths = crate::test_utils::primary_checkout_paths(directory.path());
        std::fs::create_dir_all(&paths.shared_jit).unwrap();
        let outside = [
            paths.local_jit.join("worktree.json"),
            paths.common_dir.join("config"),
            paths.shared_jit.join("../config"),
            paths.shared_jit.clone(),
        ];

        for target in outside {
            assert!(
                publish_git(&paths, &target).is_err(),
                "{}",
                target.display()
            );
        }
        assert!(entries(&paths.shared_jit).is_empty());
        assert_eq!(entries(&paths.common_dir), HashSet::from(["jit".into()]));
    }

    #[test]
    fn test_publish_git_control_file_into_an_absent_directory_creates_nothing() {
        let directory = TempDir::new().unwrap();
        let paths = crate::test_utils::primary_checkout_paths(directory.path());
        std::fs::create_dir_all(&paths.shared_jit).unwrap();
        let leases = paths.shared_jit.join("leases");

        assert!(publish_git(&paths, &leases.join("lease.json")).is_err());
        assert!(!leases.exists(), "the publication created its parent");
    }

    #[cfg(unix)]
    #[test]
    fn test_publish_git_control_file_follows_a_linked_control_directory() {
        let directory = TempDir::new().unwrap();
        let paths = crate::test_utils::primary_checkout_paths(directory.path());
        let elsewhere = directory.path().join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();
        std::fs::create_dir_all(&paths.common_dir).unwrap();
        std::os::unix::fs::symlink(&elsewhere, &paths.shared_jit).unwrap();

        publish_git(&paths, &paths.shared_jit.join("claims.seq")).unwrap();

        assert_eq!(
            std::fs::read(elsewhere.join("claims.seq")).unwrap(),
            b"published"
        );
        assert_eq!(entries(&elsewhere), HashSet::from(["claims.seq".into()]));
    }
}
