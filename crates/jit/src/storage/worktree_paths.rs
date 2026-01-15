use anyhow::{Context, Result};
use std::env;
use std::path::PathBuf;
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
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use jit::storage::worktree_paths::WorktreePaths;
    ///
    /// let paths = WorktreePaths::detect().unwrap();
    /// println!("Local .jit: {:?}", paths.local_jit);
    /// println!("Shared .git/jit: {:?}", paths.shared_jit);
    /// ```
    pub fn detect() -> Result<Self> {
        // Check if in git repo
        let is_repo = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !is_repo {
            // Not in git repo, use current dir
            let current = env::current_dir()?;
            let dot_git = current.join(".git");
            return Ok(Self {
                common_dir: dot_git.clone(),
                worktree_root: current.clone(),
                local_jit: current.join(".jit"),
                shared_jit: dot_git.join("jit"),
            });
        }

        // Get git common dir (shared .git)
        let common_dir_output = Command::new("git")
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
            env::current_dir()?.join(common_dir_raw).canonicalize()?
        };

        // Get worktree root
        let worktree_root_output = Command::new("git")
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_in_non_git_directory() {
        let temp = TempDir::new().unwrap();
        let original_dir = env::current_dir().unwrap();

        env::set_current_dir(&temp).unwrap();

        let paths = WorktreePaths::detect().unwrap();

        // Should use current directory when not in git repo
        let temp_canon = temp.path().canonicalize().unwrap();
        assert_eq!(paths.worktree_root, temp_canon);
        assert_eq!(paths.common_dir, temp_canon.join(".git"));
        assert_eq!(paths.local_jit, temp_canon.join(".jit"));
        assert_eq!(paths.shared_jit, temp_canon.join(".git/jit"));
        assert!(!paths.is_worktree());

        env::set_current_dir(original_dir).unwrap();
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
