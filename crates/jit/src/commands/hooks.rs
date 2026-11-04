//! Git hooks installation commands

use anyhow::{Context, Result};
use std::fs;
use std::path::Path;

const PRE_COMMIT_HOOK: &str = include_str!("../../../../scripts/hooks/pre-commit");

/// Install the git hook for branch-drift validation
///
/// Copies hook templates into `<git_dir>/hooks/` and makes them executable.
/// `git_dir` is the common git directory of the selected repository.
///
/// # Hooks Installed
///
/// - **pre-commit**: Validates branch drift before commit
///
/// # Errors
///
/// Returns error if the hooks directory cannot be written or permissions cannot be set.
pub fn install_hooks(git_dir: &Path) -> Result<InstallResult> {
    let hooks_dir = git_dir.join("hooks");

    // Create hooks directory if it doesn't exist
    if !hooks_dir.exists() {
        fs::create_dir_all(&hooks_dir).context("Failed to create .git/hooks directory")?;
    }

    let mut installed = Vec::new();
    let mut skipped = Vec::new();

    // Install pre-commit hook
    let pre_commit_path = hooks_dir.join("pre-commit");
    if pre_commit_path.exists() {
        skipped.push("pre-commit (already exists)".to_string());
    } else {
        fs::write(&pre_commit_path, PRE_COMMIT_HOOK).context("Failed to write pre-commit hook")?;
        set_executable(&pre_commit_path)?;
        installed.push("pre-commit".to_string());
    }

    Ok(InstallResult {
        hooks_dir: hooks_dir.to_string_lossy().to_string(),
        installed,
        skipped,
    })
}

/// Result of hook installation
#[derive(Debug, serde::Serialize)]
pub struct InstallResult {
    pub hooks_dir: String,
    pub installed: Vec<String>,
    pub skipped: Vec<String>,
}

/// Set executable permissions on a file (Unix only)
#[cfg(unix)]
fn set_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut perms = fs::metadata(path)
        .context("Failed to get file metadata")?
        .permissions();

    // Add execute permission for owner, group, and others
    let mode = perms.mode();
    perms.set_mode(mode | 0o111);

    fs::set_permissions(path, perms).context("Failed to set executable permission")?;

    Ok(())
}

/// Set executable permissions on a file (Windows - no-op)
#[cfg(not(unix))]
fn set_executable(_path: &Path) -> Result<()> {
    // Windows doesn't use Unix-style permissions
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_install_hooks() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        let result = install_hooks(&git_dir).unwrap();

        assert_eq!(result.installed, ["pre-commit"]);
        assert!(result.skipped.is_empty());

        // Verify files exist
        assert!(git_dir.join("hooks/pre-commit").exists());

        // Verify executable (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = fs::metadata(git_dir.join("hooks/pre-commit"))
                .unwrap()
                .permissions();
            assert_ne!(perms.mode() & 0o111, 0);
        }
    }

    #[test]
    fn test_install_hooks_skips_existing() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        let hooks_dir = git_dir.join("hooks");
        fs::create_dir_all(&hooks_dir).unwrap();

        // Create existing pre-commit
        fs::write(hooks_dir.join("pre-commit"), "existing hook").unwrap();

        let result = install_hooks(&git_dir).unwrap();

        assert!(result.installed.is_empty());
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].contains("pre-commit"));

        // Verify existing hook wasn't overwritten
        let content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
        assert_eq!(content, "existing hook");
    }
}
