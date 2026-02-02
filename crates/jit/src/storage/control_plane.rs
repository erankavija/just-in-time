use anyhow::Result;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Initialize the control plane directory structure.
///
/// Creates `.git/jit/` with subdirectories for coordination state:
/// - `locks/` - File locks for atomic operations
/// - `heartbeat/` - Agent heartbeat files
/// - `events/` - Control plane event log
///
/// All directories created with 0700 permissions (owner-only access).
///
/// # Examples
///
/// ```no_run
/// use jit::storage::control_plane::init_control_plane;
/// use std::path::Path;
///
/// let git_dir = Path::new(".git");
/// init_control_plane(git_dir).unwrap();
/// ```
pub fn init_control_plane(git_dir: &Path) -> Result<()> {
    let control_dir = git_dir.join("jit");

    // Create main control plane directory with 0700 permissions
    create_dir_with_permissions(&control_dir, 0o700)?;

    // Create subdirectories
    create_dir_with_permissions(&control_dir.join("locks"), 0o700)?;
    create_dir_with_permissions(&control_dir.join("heartbeat"), 0o700)?;
    create_dir_with_permissions(&control_dir.join("events"), 0o700)?;

    Ok(())
}

/// Create directory with specific permissions, idempotent.
#[cfg(unix)]
fn create_dir_with_permissions(path: &Path, mode: u32) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    fs::create_dir_all(path)?;
    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions)?;

    Ok(())
}

/// Create directory (Windows - no permission control)
#[cfg(not(unix))]
fn create_dir_with_permissions(path: &Path, _mode: u32) -> Result<()> {
    if path.exists() {
        return Ok(());
    }

    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_init_control_plane_creates_directories() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        init_control_plane(&git_dir).unwrap();

        // Verify main directory exists
        let jit_dir = git_dir.join("jit");
        assert!(jit_dir.exists());
        assert!(jit_dir.is_dir());

        // Verify subdirectories exist
        assert!(jit_dir.join("locks").exists());
        assert!(jit_dir.join("heartbeat").exists());
        assert!(jit_dir.join("events").exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_control_plane_has_correct_permissions() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        init_control_plane(&git_dir).unwrap();

        let jit_dir = git_dir.join("jit");
        let metadata = fs::metadata(&jit_dir).unwrap();
        let permissions = metadata.permissions();

        // Should be 0700 (owner read/write/execute only)
        assert_eq!(permissions.mode() & 0o777, 0o700);
    }

    #[test]
    fn test_init_control_plane_idempotent() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        // First initialization
        init_control_plane(&git_dir).unwrap();

        // Second initialization should succeed (idempotent)
        init_control_plane(&git_dir).unwrap();

        // Directories should still exist
        let jit_dir = git_dir.join("jit");
        assert!(jit_dir.exists());
        assert!(jit_dir.join("locks").exists());
    }

    #[test]
    fn test_init_control_plane_creates_parent_if_missing() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        // Don't create git_dir - test that it handles this

        // Should create both .git and .git/jit
        fs::create_dir(&git_dir).unwrap();
        init_control_plane(&git_dir).unwrap();

        assert!(git_dir.join("jit").exists());
    }
}
