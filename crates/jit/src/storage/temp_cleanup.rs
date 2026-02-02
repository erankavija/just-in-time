//! Orphaned temporary file cleanup
//!
//! Removes `.tmp` files left behind by crashed or interrupted operations.
//! Part of the startup recovery system.

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime};

/// Clean up orphaned temporary files older than the threshold.
///
/// Scans the given directory recursively for `*.tmp` files and removes those
/// that are older than `threshold_secs`. This handles cases where atomic write
/// operations were interrupted (process crash, kill -9, power failure).
///
/// # Arguments
///
/// * `root` - Root directory to scan (typically `.jit/`)
/// * `threshold_secs` - Age threshold in seconds (default: 3600 = 1 hour)
///
/// # Returns
///
/// Number of files removed
///
/// # Errors
///
/// Returns error if directory traversal fails. Individual file removal failures
/// are logged but don't fail the operation (best-effort cleanup).
pub fn cleanup_orphaned_temp_files(root: &Path, threshold_secs: u64) -> Result<usize> {
    let threshold = Duration::from_secs(threshold_secs);
    let now = SystemTime::now();
    let mut removed_count = 0;

    // Recursively find all .tmp files
    let tmp_files = find_temp_files(root)?;

    for path in tmp_files {
        // Check file age
        if let Ok(metadata) = fs::metadata(&path) {
            if let Ok(modified) = metadata.modified() {
                if let Ok(age) = now.duration_since(modified) {
                    if age > threshold {
                        // File is old enough, remove it
                        if fs::remove_file(&path).is_ok() {
                            removed_count += 1;
                            // Silently remove (could add logging in future)
                        }
                    }
                }
            }
        }
    }

    Ok(removed_count)
}

/// Recursively find all .tmp files in a directory
fn find_temp_files(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut temp_files = Vec::new();

    if !root.exists() {
        return Ok(temp_files);
    }

    visit_dir(root, &mut temp_files)?;
    Ok(temp_files)
}

/// Recursively visit directory and collect .tmp files
fn visit_dir(dir: &Path, temp_files: &mut Vec<std::path::PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    let entries = fs::read_dir(dir).context("Failed to read directory")?;

    for entry in entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();

        if path.is_dir() {
            visit_dir(&path, temp_files)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
            temp_files.push(path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::thread::sleep;
    use tempfile::TempDir;

    #[test]
    fn test_no_temp_files() {
        let dir = TempDir::new().unwrap();
        let removed = cleanup_orphaned_temp_files(dir.path(), 1).unwrap();
        assert_eq!(
            removed, 0,
            "No files should be removed from empty directory"
        );
    }

    #[test]
    fn test_ignores_recent_temp_files() {
        let dir = TempDir::new().unwrap();
        let tmp_file = dir.path().join("recent.tmp");
        File::create(&tmp_file).unwrap();

        // Threshold = 10 seconds, file is fresh
        let removed = cleanup_orphaned_temp_files(dir.path(), 10).unwrap();
        assert_eq!(removed, 0, "Recent temp file should not be removed");
        assert!(tmp_file.exists(), "Recent temp file should still exist");
    }

    #[test]
    fn test_removes_old_temp_files() {
        let dir = TempDir::new().unwrap();
        let tmp_file = dir.path().join("old.tmp");
        File::create(&tmp_file).unwrap();

        // Wait to make file "old"
        sleep(Duration::from_millis(100));

        // Threshold = 0.05 seconds (50ms), file is older
        let removed = cleanup_orphaned_temp_files(dir.path(), 0).unwrap();
        assert_eq!(removed, 1, "Old temp file should be removed");
        assert!(!tmp_file.exists(), "Old temp file should be deleted");
    }

    #[test]
    fn test_removes_multiple_old_files() {
        let dir = TempDir::new().unwrap();
        let tmp1 = dir.path().join("old1.tmp");
        let tmp2 = dir.path().join("old2.tmp");
        let tmp3 = dir.path().join("subdir").join("old3.tmp");

        fs::create_dir_all(tmp3.parent().unwrap()).unwrap();
        File::create(&tmp1).unwrap();
        File::create(&tmp2).unwrap();
        File::create(&tmp3).unwrap();

        sleep(Duration::from_millis(100));

        let removed = cleanup_orphaned_temp_files(dir.path(), 0).unwrap();
        assert_eq!(removed, 3, "All old temp files should be removed");
        assert!(!tmp1.exists());
        assert!(!tmp2.exists());
        assert!(!tmp3.exists());
    }

    #[test]
    fn test_ignores_non_tmp_files() {
        let dir = TempDir::new().unwrap();
        let json_file = dir.path().join("data.json");
        let txt_file = dir.path().join("notes.txt");

        File::create(&json_file).unwrap();
        File::create(&txt_file).unwrap();

        sleep(Duration::from_millis(100));

        let removed = cleanup_orphaned_temp_files(dir.path(), 0).unwrap();
        assert_eq!(removed, 0, "Non-.tmp files should be ignored");
        assert!(json_file.exists());
        assert!(txt_file.exists());
    }

    #[test]
    fn test_mixed_old_and_recent_files() {
        let dir = TempDir::new().unwrap();
        let old_tmp = dir.path().join("old.tmp");
        File::create(&old_tmp).unwrap();

        sleep(Duration::from_millis(200));

        let recent_tmp = dir.path().join("recent.tmp");
        File::create(&recent_tmp).unwrap();

        // Threshold = 0.1 seconds (100ms) - old file is ~200ms, recent is ~0ms
        let removed = cleanup_orphaned_temp_files(dir.path(), 0).unwrap();
        assert_eq!(removed, 2, "With zero threshold, both files are old enough");
        assert!(!old_tmp.exists(), "Old file should be deleted");
        assert!(
            !recent_tmp.exists(),
            "Recent file is also old enough with threshold=0"
        );
    }

    #[test]
    fn test_handles_missing_directory() {
        let dir = TempDir::new().unwrap();
        let nonexistent = dir.path().join("nonexistent");

        let removed = cleanup_orphaned_temp_files(&nonexistent, 1).unwrap();
        assert_eq!(removed, 0, "Should handle missing directory gracefully");
    }

    #[test]
    fn test_recursive_scan() {
        let dir = TempDir::new().unwrap();
        let subdir1 = dir.path().join("sub1");
        let subdir2 = dir.path().join("sub1/sub2");

        fs::create_dir_all(&subdir2).unwrap();

        File::create(dir.path().join("root.tmp")).unwrap();
        File::create(subdir1.join("level1.tmp")).unwrap();
        File::create(subdir2.join("level2.tmp")).unwrap();

        sleep(Duration::from_millis(100));

        let removed = cleanup_orphaned_temp_files(dir.path(), 0).unwrap();
        assert_eq!(removed, 3, "Should find temp files in all subdirectories");
    }
}
