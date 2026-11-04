use anyhow::Result;
use std::env;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::domain::identity::IssueId;
use crate::storage::git::{Git, GitCheckout, ProbedCheckout};

/// The checkout a command runs in when its data root lies outside every checkout
/// and outside the worktree root.
#[derive(Clone, Debug, PartialEq)]
pub struct RelocatedCheckout {
    /// The checkout's top level; the worktree root may be a subdirectory of it.
    pub top_level: PathBuf,
    /// The checkout's own git directory, which holds its identity because the
    /// data root may serve many checkouts.
    pub git_dir: PathBuf,
}

impl From<ProbedCheckout> for RelocatedCheckout {
    fn from(probed: ProbedCheckout) -> Self {
        Self {
            top_level: probed.worktree_root,
            git_dir: probed.git_dir,
        }
    }
}

/// Paths for worktree-aware storage.
///
/// Detects git worktree context and provides paths for both the data plane
/// (`local_jit`) and the shared control plane (`shared_jit`).
#[derive(Clone, Debug, PartialEq)]
pub struct WorktreePaths {
    /// Whether the selected data root belongs to a Git checkout.
    pub git_repository: bool,
    /// Git common directory shared by all worktrees: of the checkout holding the
    /// data root, else of the checkout holding the worktree root, else the
    /// unbacked `<worktree_root>/.git`.
    pub common_dir: PathBuf,
    /// Root of current worktree
    pub worktree_root: PathBuf,
    /// Selected data root; `<worktree_root>/.jit` unless selected elsewhere.
    pub local_jit: PathBuf,
    /// Shared control plane: <common_dir>/jit
    pub shared_jit: PathBuf,
    /// The checkout holding the worktree root when the selected data root lies
    /// outside every checkout and outside the worktree root.
    pub relocated_worktree_checkout: Option<RelocatedCheckout>,
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
    /// control; its checkout, if any, then supplies the common directory.
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
            let worktree_checkout = selected_data_root
                .filter(|data_root| !data_root.starts_with(non_git_root))
                .map(|_| Git::default().probe(non_git_root))
                .transpose()?
                .flatten();
            let worktree_root = non_git_root.to_path_buf();
            let (common_dir, relocated_worktree_checkout) = match worktree_checkout {
                Some(checkout) => (checkout.common_dir.clone(), Some(checkout.into())),
                None => (worktree_root.join(".git"), None),
            };
            return Ok(Self {
                relocated_worktree_checkout,
                git_repository: false,
                shared_jit: common_dir.join("jit"),
                common_dir,
                local_jit: selected_data_root
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| worktree_root.join(".jit")),
                worktree_root,
            });
        };
        let ProbedCheckout {
            common_dir,
            worktree_root,
            ..
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

    /// Return whether a git common directory backs `shared_jit`, so claims and
    /// leases can coordinate here. Unlike [`Self::is_git_repository`], this holds
    /// for a store outside every checkout invoked from inside one.
    pub fn has_claims_coordination(&self) -> bool {
        self.git_repository || self.relocated_worktree_checkout.is_some()
    }

    /// The directory of lease records under the control plane `shared_jit`.
    pub fn leases_dir(shared_jit: &Path) -> PathBuf {
        shared_jit.join("leases")
    }

    /// The issue's lease record under `shared_jit`; an absent file means no lease.
    pub fn lease_path(shared_jit: &Path, issue_id: &IssueId) -> PathBuf {
        Self::leases_dir(shared_jit).join(format!("{issue_id}.json"))
    }

    /// The stage file a change to the issue's lease record is written to before
    /// its rename. Its `.tmp` suffix puts an interrupted operation's residue
    /// under `jit recover`'s temp-file cleanup.
    pub fn lease_stage_path(shared_jit: &Path, issue_id: &IssueId) -> PathBuf {
        Self::leases_dir(shared_jit).join(format!("{issue_id}.json.tmp"))
    }

    /// The issue a [`Self::lease_path`] file name names, or `None` for any
    /// other name in the lease directory.
    pub fn lease_record_issue(file_name: &OsStr) -> Option<IssueId> {
        file_name
            .to_str()?
            .strip_suffix(".json")
            .and_then(|stem| IssueId::try_from(stem).ok())
    }

    /// Return whether the selected store belongs to a linked checkout. A store
    /// outside every checkout is shared, so no linked checkout holds it.
    pub fn holds_linked_store(&self) -> bool {
        self.git_repository && self.is_worktree()
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
                .as_ref()
                .map(|checkout| GitCheckout::new(Git::default(), checkout.top_level.clone()))
        })
    }

    /// Project this repository authority onto one worktree reported by Git.
    ///
    /// A checkout holds its store at the conventional `.jit` location and needs
    /// no process. Over a store outside every checkout, the projection keeps that
    /// store and probes the worktree for its git directory, answering `None` when
    /// the worktree is no longer a checkout, as a stale `git worktree list` entry
    /// can be.
    ///
    /// # Errors
    ///
    /// Returns an error when git reports a checkout whose paths cannot be resolved.
    pub fn for_worktree_root(&self, worktree_root: PathBuf) -> Result<Option<Self>> {
        let (local_jit, relocated_worktree_checkout) = match self.relocated_worktree_checkout {
            None => (worktree_root.join(".jit"), None),
            Some(_) => match Git::default().probe(&worktree_root)? {
                Some(checkout) => (self.local_jit.clone(), Some(checkout.into())),
                None => return Ok(None),
            },
        };
        Ok(Some(Self {
            git_repository: self.git_repository,
            common_dir: self.common_dir.clone(),
            relocated_worktree_checkout,
            worktree_root,
            local_jit,
            shared_jit: self.shared_jit.clone(),
        }))
    }

    /// The git directory holding this checkout's identity, for a store outside
    /// every checkout. Any other store holds the identity of its one checkout.
    pub(crate) fn checkout_git_dir(&self) -> Option<&Path> {
        self.relocated_worktree_checkout
            .as_ref()
            .map(|checkout| checkout.git_dir.as_path())
    }

    /// Check if we're in a secondary worktree (not main worktree).
    ///
    /// Returns true if `common_dir != <checkout top level>/.git`.
    pub fn is_worktree(&self) -> bool {
        self.common_dir != self.checkout_top_level().join(".git")
    }

    /// The worktree root may be a subdirectory of the recorded relocated checkout.
    pub(crate) fn checkout_top_level(&self) -> &Path {
        self.relocated_worktree_checkout
            .as_ref()
            .map_or(&self.worktree_root, |checkout| &checkout.top_level)
    }

    /// Check if this is the main worktree.
    ///
    /// Returns true if `common_dir == <checkout top level>/.git`.
    pub fn is_main_worktree(&self) -> bool {
        !self.is_worktree()
    }

    /// Return the primary checkout root when this is a linked non-primary checkout.
    ///
    /// A store outside every checkout needs no primary checkout, so an unmappable
    /// common directory answers `None` there.
    ///
    /// # Errors
    ///
    /// Returns an error for a linked checkout holding its store whose common
    /// directory cannot be mapped to a primary checkout.
    pub fn primary_worktree_root(&self) -> Result<Option<&Path>> {
        if !self.is_worktree() {
            return Ok(None);
        }
        if self.common_dir.file_name().and_then(|name| name.to_str()) != Some(".git") {
            if !self.git_repository {
                return Ok(None);
            }
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
            None => !self.is_worktree() && candidate == self.checkout_top_level(),
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

    /// `(repository, linked checkout, store outside both)` under `dir`.
    fn external_store_layout(
        dir: &Path,
    ) -> (crate::test_harness::git::GitRepository, PathBuf, PathBuf) {
        let repository = crate::test_harness::git::GitRepository::init(dir.join("repo"));
        repository.write("base", "base").commit("base");
        let linked = dir.join("linked");
        repository.git(&["worktree", "add", "-q", linked.to_str().unwrap()]);
        let store = dir.join("store");
        (repository, linked, store)
    }

    #[test]
    fn test_detect_for_external_store_from_primary_locates_claims_in_its_common_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, _, store) = external_store_layout(&dir);

        let paths = WorktreePaths::detect_for_data_root(&store, repository.root()).unwrap();

        assert_eq!(paths.common_dir, repository.root().join(".git"));
        assert_eq!(paths.shared_jit, repository.root().join(".git/jit"));
        assert!(paths.has_claims_coordination());
        assert!(!paths.is_git_repository());
        assert!(paths.is_main_worktree());
        assert!(!paths.holds_linked_store());
    }

    #[test]
    fn test_detect_for_external_store_from_primary_subdirectory_is_the_main_worktree() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, _, store) = external_store_layout(&dir);
        let subdirectory = repository.root().join("sub");
        std::fs::create_dir(&subdirectory).unwrap();

        let paths = WorktreePaths::detect_for_data_root(&store, &subdirectory).unwrap();

        assert_eq!(paths.worktree_root, subdirectory);
        assert_eq!(paths.common_dir, repository.root().join(".git"));
        assert!(paths.is_main_worktree());
        assert!(paths.is_primary_worktree_path(repository.root()).unwrap());
    }

    #[test]
    fn test_detect_for_external_store_from_linked_checkout_shares_the_primary_common_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, linked, store) = external_store_layout(&dir);

        let paths = WorktreePaths::detect_for_data_root(&store, &linked).unwrap();

        assert_eq!(paths.common_dir, repository.root().join(".git"));
        assert_eq!(paths.shared_jit, repository.root().join(".git/jit"));
        assert!(paths.has_claims_coordination());
        assert_eq!(paths.git(), None);
        assert!(paths.is_worktree());
        assert!(!paths.holds_linked_store());
        assert_eq!(
            paths.primary_worktree_root().unwrap(),
            Some(repository.root())
        );
        assert_eq!(paths.primary_data_root().unwrap(), None);
        assert!(paths.is_primary_worktree_path(repository.root()).unwrap());
    }

    /// A bare repository has no primary checkout, and an external store needs none.
    #[test]
    fn test_detect_for_external_store_from_linked_checkout_of_bare_repository_resolves_no_primary()
    {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, _, store) = external_store_layout(&dir);
        let bare = dir.join("bare.git");
        let linked = dir.join("bare-linked");
        repository.git(&["clone", "-q", "--bare", ".", bare.to_str().unwrap()]);
        repository.git(&[
            "--git-dir",
            bare.to_str().unwrap(),
            "worktree",
            "add",
            "-q",
            "--detach",
            linked.to_str().unwrap(),
        ]);

        let paths = WorktreePaths::detect_for_data_root(&store, &linked).unwrap();

        assert_eq!(paths.common_dir, bare);
        assert!(paths.is_worktree());
        assert_eq!(paths.primary_worktree_root().unwrap(), None);
        assert_eq!(paths.primary_data_root().unwrap(), None);
        assert!(!paths.is_primary_worktree_path(&linked).unwrap());
    }

    #[test]
    fn test_for_worktree_root_over_external_store_projects_the_checkout_over_the_same_store() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, linked, store) = external_store_layout(&dir);
        let paths = WorktreePaths::detect_for_data_root(&store, repository.root()).unwrap();

        let projected = paths.for_worktree_root(linked.clone()).unwrap().unwrap();

        assert_eq!(
            projected,
            WorktreePaths::detect_for_data_root(&store, &linked).unwrap()
        );
    }

    #[test]
    fn test_for_worktree_root_over_external_store_answers_none_for_a_removed_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, linked, store) = external_store_layout(&dir);
        let paths = WorktreePaths::detect_for_data_root(&store, repository.root()).unwrap();
        std::fs::remove_dir_all(&linked).unwrap();

        assert_eq!(paths.for_worktree_root(linked).unwrap(), None);
    }

    #[test]
    fn test_for_worktree_root_of_a_store_in_its_checkout_uses_the_conventional_location() {
        let paths = crate::test_utils::primary_checkout_paths("/repo");

        let projected = paths
            .for_worktree_root(PathBuf::from("/worktrees/feature"))
            .unwrap()
            .unwrap();

        assert_eq!(projected.local_jit, Path::new("/worktrees/feature/.jit"));
        assert_eq!(projected.checkout_git_dir(), None);
    }

    #[test]
    fn test_detect_for_data_root_inside_a_checkout_keeps_claims_in_its_common_dir() {
        let dir = tempfile::tempdir().unwrap();
        let dir = dir.path().canonicalize().unwrap();
        let (repository, linked, _) = external_store_layout(&dir);

        let paths = WorktreePaths::detect_for_data_root(&linked.join(".jit"), &linked).unwrap();

        assert_eq!(paths.shared_jit, repository.root().join(".git/jit"));
        assert!(paths.is_git_repository());
        assert!(paths.has_claims_coordination());
        assert!(paths.holds_linked_store());
    }

    #[test]
    fn test_detect_for_data_root_outside_every_checkout_and_invoked_outside_has_no_claims() {
        let store = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();

        let paths = WorktreePaths::detect_for_data_root(store.path(), outside.path()).unwrap();

        assert!(!paths.has_claims_coordination());
        assert!(!paths.holds_linked_store());
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
