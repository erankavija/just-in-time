//! The shared file-write primitives: atomic publication (temp file + rename)
//! and durability ([`write_file_synced`], [`create_file_synced`], [`sync_directory`]).
//!
//! Storage writes go through the helpers in this module so a reader never
//! observes a partially written file (the JIT "atomic file writes" invariant).
//! Atomicity is reader visibility, not durability: a caller that must not
//! proceed until its bytes survive a crash reaches the synced operations.
//! It lives in the storage layer because persistence is storage's responsibility:
//! command/validation/output callers produce content and hand it here rather
//! than touching the filesystem themselves.
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
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `content` to `path` atomically (temp file + rename).
///
/// The write goes to a temp file in the SAME directory as `path` and is then
/// renamed onto `path`, so a reader never observes a partially written file.
/// The temp filename is UNIQUE per process and per call — it embeds the OS
/// process id and a process-local monotonic counter — so concurrent writers
/// targeting the same path never collide on a shared temp file before the
/// rename. The parent directory must already exist; the rename is atomic only
/// within a single filesystem (the temp file stays in the target's directory to
/// guarantee that).
pub(in crate::storage) fn write_file_atomic(path: &Path, content: &str) -> Result<()> {
    write_file_atomic_bytes(path, content.as_bytes())
}

/// Write arbitrary bytes to `path` atomically (temp file + rename).
pub(in crate::storage) fn write_file_atomic_bytes(path: &Path, content: &[u8]) -> Result<()> {
    write_file_atomic_bytes_with_permissions(path, content, None)
}

/// Write `content` to `path`, truncating it, and return once those bytes are on
/// the device.
///
/// An interrupted write leaves a prefix of `content`, never a mix with what the
/// path held. A caller whose path may hold something it must not lose reaches
/// [`create_file_synced`]; one that creates the file needs [`sync_directory`]
/// on the parent naming it.
pub(in crate::storage) fn write_file_synced(path: &Path, content: &[u8]) -> Result<()> {
    write_synced(
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
        path,
        content,
    )
}

/// Write `content` to `path` as a new file, refusing an occupied path, and
/// return once those bytes are on the device.
///
/// The refusal is the open itself, so it holds against a concurrent writer a
/// caller-side existence check would race. What occupies a commit point — a
/// redo record — is a mutation nobody applied, and truncating it destroys that
/// mutation; resolving the leftover is the occupying caller's business.
pub(in crate::storage) fn create_file_synced(path: &Path, content: &[u8]) -> Result<()> {
    write_synced(
        fs::OpenOptions::new().write(true).create_new(true),
        path,
        content,
    )
}

/// The synced write both operations share; `options` decides what an occupied
/// path means.
fn write_synced(options: &fs::OpenOptions, path: &Path, content: &[u8]) -> Result<()> {
    let mut file = options
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

/// Write bytes and optional permissions to `path` as one atomic publication.
///
/// Content and permissions are applied to the same-directory temporary file
/// before it is renamed onto the target. A failed write, permission update, or
/// rename removes the temporary file on a best-effort basis and leaves an
/// existing target untouched.
fn write_file_atomic_bytes_with_permissions(
    path: &Path,
    content: &[u8],
    permissions: Option<fs::Permissions>,
) -> Result<()> {
    // Per-process monotonic counter so two calls within the same process get
    // distinct temp names even at the same instant; combined with the process id
    // it is unique across concurrent writers to the same target.
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("write");
    let tmp_name = format!(".{file_name}.{}.{seq}.tmp", std::process::id());
    // Keep the temp file in the SAME directory as the target so the rename is a
    // same-filesystem (atomic) operation.
    let tmp = match path.parent() {
        Some(dir) => dir.join(tmp_name),
        None => PathBuf::from(tmp_name),
    };

    if let Err(source) = fs::write(&tmp, content) {
        cleanup_temp(&tmp);
        return Err(source).with_context(|| format!("writing {}", tmp.display()));
    }
    if let Some(permissions) = permissions {
        if let Err(source) = fs::set_permissions(&tmp, permissions) {
            cleanup_temp(&tmp);
            return Err(source)
                .with_context(|| format!("setting permissions on {}", tmp.display()));
        }
    }
    if let Err(source) = fs::rename(&tmp, path) {
        cleanup_temp(&tmp);
        return Err(source)
            .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()));
    }
    Ok(())
}

fn cleanup_temp(path: &Path) {
    let _ = fs::remove_file(path);
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
    fn test_create_file_synced_writes_the_bytes_it_was_given() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("redo.record");
        let content = [0, 159, 146, 150, 255];

        create_file_synced(&path, &content).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), content);
    }

    #[test]
    fn test_create_file_synced_leaves_an_occupied_path_as_it_found_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("redo.record");
        let occupant = b"a record nobody applied yet";
        write_file_synced(&path, occupant).unwrap();

        assert!(create_file_synced(&path, b"replacement").is_err());

        assert_eq!(std::fs::read(&path).unwrap(), occupant.to_vec());
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

    #[test]
    fn test_write_file_atomic_writes_full_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.json");
        write_file_atomic(&path, "{\"nodes\":[]}").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"nodes\":[]}");
    }

    #[test]
    fn test_write_file_atomic_leaves_no_temp_file() {
        // The rename consumes the temp file, so a completed write leaves ONLY the
        // target in the directory — evidence the temp-file + rename path ran and
        // no partial `.tmp` sibling lingers.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        write_file_atomic(&path, "hello").unwrap();

        let entries: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["out.txt".to_string()]);
    }

    #[test]
    fn test_write_file_atomic_overwrites_existing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.txt");
        write_file_atomic(&path, "first").unwrap();
        write_file_atomic(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        // Still exactly one file: the overwrite left no temp residue.
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn test_write_file_atomic_bytes_preserves_non_utf8_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("asset.bin");
        let content = [0, 159, 146, 150, 255];

        write_file_atomic_bytes(&path, &content).unwrap();

        assert_eq!(std::fs::read(path).unwrap(), content);
    }

    #[test]
    fn test_write_file_atomic_cleans_temp_after_rename_failure() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("occupied");
        std::fs::create_dir(&target).unwrap();

        assert!(write_file_atomic_bytes(&target, b"replacement").is_err());

        assert!(target.is_dir());
        assert_eq!(
            std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect::<Vec<_>>(),
            vec![std::ffi::OsString::from("occupied")]
        );
    }
}
