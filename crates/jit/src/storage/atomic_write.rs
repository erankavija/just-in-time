//! The shared atomic file-write primitive (temp file + rename).
//!
//! Storage writes go through the helpers in this module so a reader never
//! observes a partially written file (the JIT "atomic file writes" invariant).
//! It lives in the storage layer because persistence is storage's responsibility:
//! command/validation/output callers produce content and hand it here rather
//! than touching the filesystem themselves.

use anyhow::{Context, Result};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
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
pub fn write_file_atomic(path: &Path, content: &str) -> Result<()> {
    write_file_atomic_bytes(path, content.as_bytes())
}

/// Write arbitrary bytes to `path` atomically (temp file + rename).
pub fn write_file_atomic_bytes(path: &Path, content: &[u8]) -> Result<()> {
    write_file_atomic_bytes_with_permissions(path, content, None)
}

/// Atomically publish an export proven to be outside the repository roots.
pub(crate) fn write_external_export_atomic(
    path: &crate::repository_state::ExternalExportPath,
    content: &[u8],
) -> Result<()> {
    let target = path.as_path();
    let parent_path = target
        .parent()
        .ok_or_else(|| anyhow::anyhow!("external export target has no parent"))?;
    let leaf = target
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("external export target has no file name"))?;
    let parent = super::repository_state_store::open_absolute_dir_nofollow(parent_path)
        .with_context(|| format!("opening external export parent {}", parent_path.display()))?;
    match parent.symlink_metadata(leaf) {
        Ok(metadata) if metadata.is_file() && !metadata.is_symlink() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(_) => anyhow::bail!("external export target is not an ordinary file"),
        Err(error) => return Err(error.into()),
    }

    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = leaf.to_string_lossy();
    let tmp_name = format!(".{file_name}.{}.{seq}.tmp", std::process::id());
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut temp = parent
        .open_with(&tmp_name, &options)
        .with_context(|| format!("creating external export temporary file {tmp_name}"))?;
    let publish = (|| -> Result<()> {
        temp.write_all(content)?;
        temp.sync_all()?;
        drop(temp);
        revalidate_external_parent(parent_path, &parent)?;
        parent.rename(&tmp_name, &parent, leaf)?;
        parent.try_clone()?.into_std_file().sync_all()?;
        Ok(())
    })();
    if publish.is_err() {
        let _ = parent.remove_file(&tmp_name);
    }
    publish
}

fn revalidate_external_parent(path: &Path, held: &Dir) -> Result<()> {
    let fresh = super::repository_state_store::open_absolute_dir_nofollow(path)
        .with_context(|| format!("revalidating external export parent {}", path.display()))?;
    let held_identity = super::repository_state_store::capability_dir_identity(held)
        .ok_or_else(|| anyhow::anyhow!("external parent identity is unavailable"))?;
    let fresh_identity = super::repository_state_store::capability_dir_identity(&fresh)
        .ok_or_else(|| anyhow::anyhow!("external parent identity is unavailable"))?;
    if held_identity != fresh_identity {
        anyhow::bail!("external export parent changed before publication");
    }
    Ok(())
}

/// Write bytes and optional permissions to `path` as one atomic publication.
///
/// Content and permissions are applied to the same-directory temporary file
/// before it is renamed onto the target. A failed write, permission update, or
/// rename removes the temporary file on a best-effort basis and leaves an
/// existing target untouched.
pub fn write_file_atomic_bytes_with_permissions(
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
