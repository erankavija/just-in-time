use anyhow::Result;
use std::env;
use std::path::{Path, PathBuf};

use crate::storage::git::{Git, GitCheckout, ProbedCheckout};

/// Paths for worktree-aware storage.
///
/// Detects git worktree context and provides paths for both per-worktree data plane
/// (`.jit/`) and shared control plane (`.git/jit/`).
#[derive(Clone, Debug, PartialEq)]
pub struct WorktreePaths {
    /// Whether the selected data root belongs to a Git checkout.
    pub git_repository: bool,
    /// Shared .git directory (common across all worktrees)
    pub common_dir: PathBuf,
    /// Root of current worktree
    pub worktree_root: PathBuf,
    /// Local data plane: <worktree_root>/.jit
    pub local_jit: PathBuf,
    /// Shared control plane: <common_dir>/jit
    pub shared_jit: PathBuf,
    /// Top level of the checkout holding the worktree root when the selected
    /// data root lies outside every checkout and outside the worktree root.
    pub relocated_worktree_checkout: Option<PathBuf>,
}

impl WorktreePaths {
    /// Detect worktree context using git commands.
    pub fn detect() -> Result<Self> {
        Self::detect_from(&env::current_dir()?)
    }

    /// Detect the checkout containing the selected JIT data root.
    ///
    /// `selected_data_root` is the authority for Git classification, including
    /// when it was selected through `JIT_DATA_DIR` and differs from the process
    /// working directory. `non_git_root` remains the repository worktree root
    /// used by Git-optional commands when the selected store is outside version
    /// control.
    ///
    /// # Errors
    ///
    /// Returns an error when Git identifies a checkout but its common directory
    /// or top-level path cannot be resolved.
    pub fn detect_for_data_root(selected_data_root: &Path, non_git_root: &Path) -> Result<Self> {
        let probe = existing_directory_for(selected_data_root);
        Self::detect_from_with_non_git_root(&probe, non_git_root, Some(selected_data_root))
    }

    /// Detect worktree context from an explicitly selected repository root.
    ///
    /// This is the non-global counterpart to [`Self::detect`]. Validation views
    /// use it so machine-local claims coordination is checked for the repository
    /// being validated, even when that repository is not the process cwd.
    pub(crate) fn detect_from(current: &Path) -> Result<Self> {
        Self::detect_from_with_non_git_root(current, current, None)
    }

    fn detect_from_with_non_git_root(
        current: &Path,
        non_git_root: &Path,
        selected_data_root: Option<&Path>,
    ) -> Result<Self> {
        let Some(probed) = Git::default().probe(current)? else {
            let relocated_worktree_checkout = selected_data_root
                .filter(|data_root| !data_root.starts_with(non_git_root))
                .map(|_| Git::default().probe(non_git_root))
                .transpose()?
                .flatten()
                .map(|checkout| checkout.worktree_root);
            let worktree_root = non_git_root.to_path_buf();
            let dot_git = worktree_root.join(".git");
            return Ok(Self {
                relocated_worktree_checkout,
                git_repository: false,
                common_dir: dot_git.clone(),
                local_jit: selected_data_root
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| worktree_root.join(".jit")),
                worktree_root,
                shared_jit: dot_git.join("jit"),
            });
        };
        let ProbedCheckout {
            common_dir,
            worktree_root,
        } = probed;

        let local_jit = selected_data_root
            .map(Path::to_path_buf)
            .unwrap_or_else(|| worktree_root.join(".jit"));
        let shared_jit = common_dir.join("jit");

        Ok(Self {
            git_repository: true,
            common_dir,
            worktree_root,
            local_jit,
            shared_jit,
            relocated_worktree_checkout: None,
        })
    }

    /// Return whether the selected data root belongs to a Git checkout.
    pub fn is_git_repository(&self) -> bool {
        self.git_repository
    }

    /// Return this checkout's git operations, run by the default executable,
    /// or `None` outside a git checkout.
    pub fn git(&self) -> Option<GitCheckout> {
        self.git_repository
            .then(|| GitCheckout::new(Git::default(), self.worktree_root.clone()))
    }

    /// Return the checkout holding the paths a mutation writes: this checkout,
    /// or the worktree's own when the data root lies outside every checkout.
    pub fn write_checkout(&self) -> Option<GitCheckout> {
        self.git().or_else(|| {
            self.relocated_worktree_checkout
                .clone()
                .map(|root| GitCheckout::new(Git::default(), root))
        })
    }

    /// Project this repository authority onto one worktree reported by Git.
    ///
    /// This derives per-worktree identity paths without running a second
    /// repository-layout probe. The projected store uses the conventional
    /// `.jit` location reported by `git worktree list` consumers.
    pub fn for_worktree_root(&self, worktree_root: PathBuf) -> Self {
        let local_jit = worktree_root.join(".jit");
        Self {
            git_repository: self.git_repository,
            common_dir: self.common_dir.clone(),
            worktree_root,
            local_jit,
            shared_jit: self.shared_jit.clone(),
            relocated_worktree_checkout: self.relocated_worktree_checkout.clone(),
        }
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

    /// Return the primary checkout root when this is a linked non-primary checkout.
    ///
    /// # Errors
    ///
    /// Returns an error for a linked checkout whose common directory cannot be
    /// mapped to a primary checkout.
    pub fn primary_worktree_root(&self) -> Result<Option<&Path>> {
        if !self.is_worktree() {
            return Ok(None);
        }
        if self.common_dir.file_name().and_then(|name| name.to_str()) != Some(".git") {
            anyhow::bail!(
                "git common directory is not a primary-checkout .git directory: {}",
                self.common_dir.display()
            );
        }
        self.common_dir.parent().map(Some).ok_or_else(|| {
            anyhow::anyhow!(
                "git common directory has no primary-checkout parent: {}",
                self.common_dir.display()
            )
        })
    }

    /// Map this checkout's selected data root to the primary checkout's store.
    ///
    /// A store outside the selected checkout has no primary-checkout counterpart,
    /// as does a primary or non-Git checkout.
    ///
    /// # Errors
    ///
    /// Returns an error when the primary checkout cannot be resolved.
    pub fn primary_data_root(&self) -> Result<Option<PathBuf>> {
        let Some(primary_root) = self.primary_worktree_root()? else {
            return Ok(None);
        };
        Ok(self
            .local_jit
            .strip_prefix(&self.worktree_root)
            .ok()
            .map(|relative| primary_root.join(relative)))
    }

    /// Return whether `candidate` is this repository's primary checkout.
    ///
    /// # Errors
    ///
    /// Returns an error when the primary checkout cannot be resolved.
    pub fn is_primary_worktree_path(&self, candidate: &Path) -> Result<bool> {
        Ok(match self.primary_worktree_root()? {
            Some(primary) => candidate == primary,
            None => candidate == self.worktree_root,
        })
    }
}

fn existing_directory_for(path: &Path) -> PathBuf {
    let mut candidate = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.to_path_buf())
    };
    while !candidate.is_dir() {
        if !candidate.pop() {
            break;
        }
    }
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_in_non_git_directory() {
        let current = tempfile::tempdir().unwrap();
        let selected_root = tempfile::tempdir().unwrap();

        let paths = WorktreePaths::detect_from_with_non_git_root(
            current.path(),
            selected_root.path(),
            None,
        )
        .unwrap();

        assert_eq!(paths.worktree_root, selected_root.path());
        assert_eq!(paths.local_jit, selected_root.path().join(".jit"));
    }

    #[test]
    fn test_detect_for_data_root_inside_git_directory_is_outside_version_control() {
        let dir = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(dir.path().join("repo"));
        let git_dir = repository.root().join(".git");
        let non_git_root = tempfile::tempdir().unwrap();

        let paths = WorktreePaths::detect_for_data_root(&git_dir, non_git_root.path()).unwrap();

        assert!(!paths.is_git_repository());
        assert_eq!(paths.worktree_root, non_git_root.path());
        assert_eq!(paths.local_jit, git_dir);
    }

    #[test]
    fn test_detect_for_data_root_outside_every_checkout_records_the_worktree_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(dir.path().join("repo"));
        let store = tempfile::tempdir().unwrap();
        let data_root = store.path().join("store");

        let paths = WorktreePaths::detect_for_data_root(&data_root, repository.root()).unwrap();

        assert!(!paths.is_git_repository());
        assert_eq!(paths.git(), None);
        let top_level = repository.root().canonicalize().unwrap();
        assert_eq!(
            paths.write_checkout().unwrap().worktree_root(),
            top_level.as_path()
        );
    }

    #[test]
    fn test_detect_for_data_root_records_no_relocated_checkout_otherwise() {
        let dir = tempfile::tempdir().unwrap();
        let repository = crate::test_harness::git::GitRepository::init(dir.path().join("repo"));
        let no_git = tempfile::tempdir().unwrap();

        let in_checkout =
            WorktreePaths::detect_for_data_root(&repository.root().join(".jit"), no_git.path())
                .unwrap();
        let without_git =
            WorktreePaths::detect_for_data_root(&no_git.path().join(".jit"), no_git.path())
                .unwrap();

        assert_eq!(in_checkout.write_checkout(), in_checkout.git());
        assert_eq!(in_checkout.relocated_worktree_checkout, None);
        assert_eq!(without_git.write_checkout(), None);
    }

    #[test]
    fn test_detect_in_main_worktree() {
        let paths = WorktreePaths::detect_from(Path::new(env!("CARGO_MANIFEST_DIR"))).unwrap();

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
        let main_paths = crate::test_utils::primary_checkout_paths("/repo");
        assert!(!main_paths.is_worktree());

        let secondary_paths =
            crate::test_utils::linked_checkout_paths("/repo", "/worktrees/feature-a");
        assert!(secondary_paths.is_worktree());
        assert_eq!(
            secondary_paths.primary_worktree_root().unwrap(),
            Some(Path::new("/repo"))
        );
        assert_eq!(
            secondary_paths.primary_data_root().unwrap(),
            Some(PathBuf::from("/repo/.jit"))
        );
        assert!(secondary_paths
            .is_primary_worktree_path(Path::new("/repo"))
            .unwrap());
    }

    #[test]
    fn test_git_pairs_default_executable_with_worktree_root_only_for_git_checkout() {
        let checkout = crate::test_utils::primary_checkout_paths("/repo");
        let outside = WorktreePaths {
            git_repository: false,
            ..checkout.clone()
        };

        assert_eq!(
            checkout.git(),
            Some(GitCheckout::new(Git::default(), PathBuf::from("/repo")))
        );
        assert_eq!(outside.git(), None);
    }

    #[test]
    fn test_detect_for_data_root_outside_version_control_is_not_linked() {
        let selected_root = tempfile::tempdir().unwrap();
        let repository_root = tempfile::tempdir().unwrap();

        let paths =
            WorktreePaths::detect_for_data_root(selected_root.path(), repository_root.path())
                .unwrap();

        assert!(!paths.is_worktree());
        assert!(!paths.is_git_repository());
        assert_eq!(paths.worktree_root, repository_root.path());
        assert_eq!(paths.local_jit, selected_root.path());
        assert_eq!(paths.primary_data_root().unwrap(), None);
    }
}
