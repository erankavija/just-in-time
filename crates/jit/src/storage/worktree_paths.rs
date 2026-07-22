use anyhow::{Context, Result};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::errors;

/// Paths for worktree-aware storage.
///
/// Detects git worktree context and provides paths for both per-worktree data plane
/// (`.jit/`) and shared control plane (`.git/jit/`).
#[derive(Clone, Debug, PartialEq)]
pub struct WorktreePaths {
    /// Shared .git directory (common across all worktrees)
    pub common_dir: PathBuf,
    /// Root of current worktree
    pub worktree_root: PathBuf,
    /// Local data plane: <worktree_root>/.jit
    pub local_jit: PathBuf,
    /// Shared control plane: <common_dir>/jit
    pub shared_jit: PathBuf,
}

impl WorktreePaths {
    /// Detect worktree context using git commands.
    pub fn detect() -> Result<Self> {
        Self::detect_from(&env::current_dir()?)
    }

    /// Detect worktree context while using `non_git_root` as the worktree
    /// authority when Git is unavailable.
    ///
    /// The caller has already selected a JIT data root and therefore knows
    /// whether that selection came from ancestor discovery or from an explicit
    /// init/override target. Git remains authoritative whenever `current` is in
    /// a worktree.
    pub fn detect_with_non_git_root(non_git_root: &Path) -> Result<Self> {
        Self::detect_from_with_non_git_root(&env::current_dir()?, non_git_root)
    }

    /// Detect worktree context from an explicitly selected repository root.
    ///
    /// This is the non-global counterpart to [`Self::detect`]. Validation views
    /// use it so machine-local claims coordination is checked for the repository
    /// being validated, even when that repository is not the process cwd.
    pub(crate) fn detect_from(current: &Path) -> Result<Self> {
        Self::detect_from_with_non_git_root(current, current)
    }

    fn detect_from_with_non_git_root(current: &Path, non_git_root: &Path) -> Result<Self> {
        // Check if in git repo
        let is_repo = Command::new("git")
            .arg("-C")
            .arg(current)
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !is_repo {
            let worktree_root = non_git_root.to_path_buf();
            let dot_git = worktree_root.join(".git");
            return Ok(Self {
                common_dir: dot_git.clone(),
                local_jit: worktree_root.join(".jit"),
                worktree_root,
                shared_jit: dot_git.join("jit"),
            });
        }

        // Get git common dir (shared .git)
        let common_dir_output = Command::new("git")
            .arg("-C")
            .arg(current)
            .args(["rev-parse", "--git-common-dir"])
            .output()
            .context("Failed to execute git command")?;

        if !common_dir_output.status.success() {
            let stderr = String::from_utf8_lossy(&common_dir_output.stderr);
            return Err(anyhow::anyhow!(
                "{}",
                errors::git_command_failed("git rev-parse --git-common-dir", &stderr)
            ));
        }

        let common_dir_raw = PathBuf::from(String::from_utf8(common_dir_output.stdout)?.trim());

        // Canonicalize common_dir to handle relative paths (e.g., ".git" in main worktree)
        let common_dir = if common_dir_raw.is_absolute() {
            common_dir_raw
        } else {
            current.join(common_dir_raw).canonicalize()?
        };

        // Get worktree root
        let worktree_root_output = Command::new("git")
            .arg("-C")
            .arg(current)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .context("Failed to execute git command")?;

        if !worktree_root_output.status.success() {
            let stderr = String::from_utf8_lossy(&worktree_root_output.stderr);
            return Err(anyhow::anyhow!(
                "{}",
                errors::git_command_failed("git rev-parse --show-toplevel", &stderr)
            ));
        }

        let worktree_root = PathBuf::from(String::from_utf8(worktree_root_output.stdout)?.trim());

        let local_jit = worktree_root.join(".jit");
        let shared_jit = common_dir.join("jit");

        Ok(Self {
            common_dir,
            worktree_root,
            local_jit,
            shared_jit,
        })
    }

    /// Check if we're in a secondary worktree (not main worktree).
    ///
    /// Returns true if `common_dir != worktree_root/.git`.
    pub fn is_worktree(&self) -> bool {
        self.common_dir != self.worktree_root.join(".git")
    }

    /// Check if this is the main worktree.
    ///
    /// Returns true if `common_dir == worktree_root/.git`.
    pub fn is_main_worktree(&self) -> bool {
        !self.is_worktree()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_in_non_git_directory() {
        let current = tempfile::tempdir().unwrap();
        let selected_root = tempfile::tempdir().unwrap();

        let paths =
            WorktreePaths::detect_from_with_non_git_root(current.path(), selected_root.path())
                .unwrap();

        assert_eq!(paths.worktree_root, selected_root.path());
        assert_eq!(paths.local_jit, selected_root.path().join(".jit"));
    }

    #[test]
    fn test_detect_in_main_worktree() {
        let paths = WorktreePaths::detect().unwrap();

        // Test the is_worktree() invariant: it should be false IFF common_dir == worktree_root/.git
        let expected_is_main = paths.common_dir == paths.worktree_root.join(".git");
        assert_eq!(
            paths.is_worktree(),
            !expected_is_main,
            "is_worktree() should be false when common_dir == worktree_root/.git, true otherwise"
        );

        // If in main worktree, verify path structure
        if expected_is_main {
            assert_eq!(paths.local_jit, paths.worktree_root.join(".jit"));
            assert_eq!(paths.shared_jit, paths.common_dir.join("jit"));
        }
    }

    #[test]
    fn test_is_worktree_detection() {
        let main_paths = WorktreePaths {
            common_dir: PathBuf::from("/repo/.git"),
            worktree_root: PathBuf::from("/repo"),
            local_jit: PathBuf::from("/repo/.jit"),
            shared_jit: PathBuf::from("/repo/.git/jit"),
        };
        assert!(!main_paths.is_worktree());

        let secondary_paths = WorktreePaths {
            common_dir: PathBuf::from("/repo/.git"),
            worktree_root: PathBuf::from("/worktrees/feature-a"),
            local_jit: PathBuf::from("/worktrees/feature-a/.jit"),
            shared_jit: PathBuf::from("/repo/.git/jit"),
        };
        assert!(secondary_paths.is_worktree());
    }
}
