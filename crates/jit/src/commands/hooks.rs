//! Git hooks installation commands

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

const PRE_COMMIT_HOOK: &str = include_str!("../../../../scripts/hooks/pre-commit");
const PRE_PUSH_HOOK: &str = include_str!("../../../../scripts/hooks/pre-push");

/// Install git hooks for lease and divergence validation
///
/// Copies hook templates to `.git/hooks/` directory and makes them executable.
///
/// # Hooks Installed
///
/// - **pre-commit**: Validates leases and divergence before commit
/// - **pre-push**: Validates leases before push
///
/// # Errors
///
/// Returns error if:
/// - Not in a git repository
/// - Cannot write to `.git/hooks/` directory
/// - File permissions cannot be set
pub fn install_hooks(git_dir: Option<PathBuf>) -> Result<InstallResult> {
    // Find .git directory
    let git_dir = if let Some(dir) = git_dir {
        dir
    } else {
        find_git_dir()?
    };

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

    // Install pre-push hook
    let pre_push_path = hooks_dir.join("pre-push");
    if pre_push_path.exists() {
        skipped.push("pre-push (already exists)".to_string());
    } else {
        fs::write(&pre_push_path, PRE_PUSH_HOOK).context("Failed to write pre-push hook")?;
        set_executable(&pre_push_path)?;
        installed.push("pre-push".to_string());
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

/// Find .git directory by walking up from current directory
fn find_git_dir() -> Result<PathBuf> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;

    let mut dir = current_dir.as_path();

    loop {
        let git_dir = dir.join(".git");

        if git_dir.is_dir() {
            return Ok(git_dir);
        }

        // Check if .git is a file (git worktree)
        if git_dir.is_file() {
            // Read gitdir from file
            let content = fs::read_to_string(&git_dir).context("Failed to read .git file")?;

            if let Some(gitdir) = content.strip_prefix("gitdir: ") {
                let gitdir = gitdir.trim();
                let gitdir_path = dir.join(gitdir);

                // For worktrees, find the common .git directory
                if let Some(parent) = gitdir_path.parent() {
                    if parent.file_name() == Some(std::ffi::OsStr::new("worktrees")) {
                        if let Some(common_git) = parent.parent() {
                            return Ok(common_git.to_path_buf());
                        }
                    }
                }

                return Ok(gitdir_path);
            }
        }

        // Move up to parent directory
        match dir.parent() {
            Some(parent) => dir = parent,
            None => anyhow::bail!("Not in a git repository (no .git directory found)"),
        }
    }
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
    fn test_find_git_dir() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        // Create subdirectory
        let subdir = temp.path().join("subdir");
        fs::create_dir(&subdir).unwrap();

        // Change to subdirectory and find .git
        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&subdir).unwrap();

        let found = find_git_dir().unwrap();

        std::env::set_current_dir(&original_dir).unwrap();

        assert_eq!(found, git_dir);
    }

    #[test]
    fn test_install_hooks() {
        let temp = TempDir::new().unwrap();
        let git_dir = temp.path().join(".git");
        fs::create_dir(&git_dir).unwrap();

        let result = install_hooks(Some(git_dir.clone())).unwrap();

        assert_eq!(result.installed.len(), 2);
        assert!(result.installed.contains(&"pre-commit".to_string()));
        assert!(result.installed.contains(&"pre-push".to_string()));
        assert!(result.skipped.is_empty());

        // Verify files exist
        assert!(git_dir.join("hooks/pre-commit").exists());
        assert!(git_dir.join("hooks/pre-push").exists());

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

        let result = install_hooks(Some(git_dir.clone())).unwrap();

        assert_eq!(result.installed.len(), 1);
        assert!(result.installed.contains(&"pre-push".to_string()));
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].contains("pre-commit"));

        // Verify existing hook wasn't overwritten
        let content = fs::read_to_string(hooks_dir.join("pre-commit")).unwrap();
        assert_eq!(content, "existing hook");
    }
}
