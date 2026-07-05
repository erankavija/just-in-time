//! The shared atomic file-write primitive (temp file + rename).
//!
//! All storage writes go through [`write_file_atomic`] so a reader never
//! observes a partially written file (the JIT "atomic file writes" invariant).
//! It lives in the storage layer because persistence is storage's
//! responsibility: command/validation/output callers produce content and hand
//! it here rather than touching the filesystem themselves.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

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
///
/// # Examples
///
/// ```
/// use jit::storage::atomic_write::write_file_atomic;
///
/// let dir = tempfile::tempdir().unwrap();
/// let path = dir.path().join("out.txt");
/// write_file_atomic(&path, "hello").unwrap();
/// assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
/// ```
pub fn write_file_atomic(path: &Path, content: &str) -> Result<()> {
    // Per-process monotonic counter so two calls within the same process get
    // distinct temp names even at the same instant; combined with the process id
    // it is unique across concurrent writers to the same target.
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let seq = COUNTER.fetch_add(1, Ordering::Relaxed);

    let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("write");
    let tmp_name = format!(".{file_name}.{}.{seq}.tmp", std::process::id());
    // Keep the temp file in the SAME directory as the target so the rename is a
    // same-filesystem (atomic) operation.
    let tmp = match path.parent() {
        Some(dir) => dir.join(tmp_name),
        None => PathBuf::from(tmp_name),
    };

    std::fs::write(&tmp, content).with_context(|| format!("writing {}", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("renaming {} -> {}", tmp.display(), path.display()))?;
    Ok(())
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
}
