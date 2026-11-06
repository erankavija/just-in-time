//! The shared file-write primitives: the file publication core
//! ([`publish_file`]) and its stage writer, durability ([`write_file_synced`],
//! [`sync_directory`]), and no-replace rename.
//!
//! A caller that must not proceed until its bytes survive a crash reaches the
//! synced operations. The module lives in the storage layer because persistence
//! is storage's responsibility: command/validation/output callers produce
//! content and hand it here rather than touching the filesystem themselves.
//!
//! Every item here is `pub(in crate::storage)`. Publication into a repository
//! root belongs to the mutation-session API, so `crate::commands` must not be
//! able to name these primitives at all; the compiler, not a source-text guard,
//! enforces that. Export publication to a caller-chosen path outside the
//! repository roots lives in [`super::external_publish`], which reaches these
//! primitives from inside `crate::storage`.

use crate::repository_state::FileMode;
use anyhow::{Context, Result};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, File, OpenOptions};
use std::ffi::OsStr;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

/// How much of a publication's durability [`publish_file`] completes itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::storage) enum Durability {
    /// Sync the staged file, then the parent directory after the rename.
    Full,
    /// Sync the staged file; the caller syncs the parent directory.
    Deferred,
}

/// Replace `leaf` in `parent` with `bytes` under `mode`.
///
/// Durability argument: the stage's bytes are synced before the rename, so a
/// rename that persists names durable bytes, and a crash leaves the target
/// holding either its previous content or `bytes` (`@/inv/atomic-writes`). The
/// rename itself persists once the parent directory is synced, which `Full`
/// does here and `Deferred` leaves to the caller. `before_rename` runs between
/// the synced stage and the rename: it is the caller's last refusal.
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
    write_staged_file(parent, &stage, bytes, mode)
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

/// Write `content` under `mode` as a new file named `leaf` and return once it
/// is durable. Every staged file in storage is written here, whichever
/// publication writes it.
///
/// The open creates the file or fails: an occupied name is never opened
/// through, so no symlink carries the bytes elsewhere and no occupant is
/// truncated (`@/inv/atomic-writes`).
pub(in crate::storage) fn write_staged_file(
    parent: &Dir,
    leaf: impl AsRef<Path>,
    mut content: impl Read,
    mode: FileMode,
) -> std::io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(leaf, &options)?;
    std::io::copy(&mut content, &mut file)?;
    set_mode(&file, mode)?;
    file.sync_all()
}

#[cfg(unix)]
fn set_mode(file: &File, mode: FileMode) -> std::io::Result<()> {
    use cap_std::fs::PermissionsExt;

    let bits = match mode {
        FileMode::Regular => 0o644,
        FileMode::Executable => 0o755,
    };
    file.set_permissions(cap_std::fs::Permissions::from_mode(bits))
}

#[cfg(not(unix))]
fn set_mode(_file: &File, _mode: FileMode) -> std::io::Result<()> {
    Ok(())
}

/// Write `content` to `path`, truncating it, and return once those bytes are on
/// the device.
///
/// An interrupted write leaves a prefix of `content`, never a mix with what the
/// path held. A caller that creates the file needs [`sync_directory`] on the
/// parent naming it.
pub(in crate::storage) fn write_file_synced(path: &Path, content: &[u8]) -> Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("opening {}", path.display()))?;
    file.write_all(content)
        .with_context(|| format!("writing {}", path.display()))?;
    file.sync_all()
        .with_context(|| format!("syncing {}", path.display()))
}

/// Fsync the directory at `path`, so entries created, renamed or removed in it
/// survive a crash; a file's own bytes need one of the synced writes.
pub(in crate::storage) fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .and_then(|directory| directory.sync_all())
        .with_context(|| format!("syncing directory {}", path.display()))
}

/// [`sync_directory`] for a caller holding the directory as a capability rather
/// than as a path.
pub(in crate::storage) fn sync_capability_directory(directory: &Dir) -> std::io::Result<()> {
    directory.try_clone()?.into_std_file().sync_all()
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

    /// A character device the kernel refuses to fsync, so a call reporting that
    /// refusal is a call that reached fsync rather than one that only opened the
    /// path. Linux-only: the refusal is this kernel's contract.
    #[cfg(target_os = "linux")]
    const UNSYNCABLE: &str = "/dev/null";

    #[test]
    fn test_write_file_synced_writes_the_bytes_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("redo.record");
        let content = [0, 159, 146, 150, 255];

        write_file_synced(&path, &content).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), content);
    }

    #[test]
    fn test_write_file_synced_leaves_no_bytes_of_a_longer_predecessor() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("redo.record");
        write_file_synced(&path, b"a considerably longer predecessor").unwrap();

        write_file_synced(&path, b"short").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"short".to_vec());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_write_file_synced_reports_a_sync_the_kernel_refuses() {
        assert!(
            write_file_synced(Path::new(UNSYNCABLE), b"bytes").is_err(),
            "the write returned before the bytes were synced"
        );
    }

    #[test]
    fn test_sync_directory_accepts_the_directory_it_is_given() {
        let dir = tempfile::tempdir().unwrap();

        sync_directory(dir.path()).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_sync_directory_reports_a_sync_the_kernel_refuses() {
        assert!(
            sync_directory(Path::new(UNSYNCABLE)).is_err(),
            "the directory sync returned without reaching fsync"
        );
    }
}
