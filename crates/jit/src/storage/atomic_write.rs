//! The shared file-write primitives: durability ([`write_file_synced`],
//! [`sync_directory`]) and no-replace rename.
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
//! repository roots lives in [`super::external_publish`], which reaches
//! [`rename_noreplace_cap`] from inside `crate::storage`.

use anyhow::{Context, Result};
use cap_std::fs::Dir;
use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::Path;

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
