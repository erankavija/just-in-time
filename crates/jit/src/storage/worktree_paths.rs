use anyhow::Result;
use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::domain::identity::IssueId;
use crate::storage::git::{Git, GitCheckout, GitError, ProbedCheckout};

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

#[derive(Debug, Default)]
struct LayoutContext {
    git: Git,
    observations: Mutex<BTreeMap<PathBuf, Result<Option<ProbedCheckout>, GitError>>>,
}

impl LayoutContext {
    fn probe(&self, path: &Path) -> Result<Option<ProbedCheckout>> {
        // Hold only this context's lock across discovery; never acquire a repository
        // or identity lock here. Publication must precede another clone's lookup.
        let mut observations = self
            .observations
            .lock()
            .map_err(|_| anyhow::anyhow!("layout observation lock poisoned"))?;
        if let Some(observed) = observations.get(path) {
            return observed.clone().map_err(Into::into);
        }
        let canonical = path.canonicalize().ok();
        if let Some(observed) = canonical
            .as_ref()
            .and_then(|path| observations.get(path))
            .cloned()
        {
            observations.insert(path.to_path_buf(), observed.clone());
            return observed.map_err(Into::into);
        }
        let observed = self.git.probe(path);
        let top_level = observed
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map(|checkout| checkout.worktree_root.clone());
        let physical_top = top_level.as_ref().and_then(|path| path.canonicalize().ok());
        std::iter::once(path.to_path_buf())
            .chain(canonical)
            .chain(top_level)
            .chain(physical_top)
            .for_each(|alias| {
                observations
                    .entry(alias)
                    .or_insert_with(|| observed.clone());
            });
        observed.map_err(Into::into)
    }
}

/// Paths for worktree-aware storage.
///
/// Detects git worktree context and provides paths for both the data plane
/// (`local_jit`) and the shared control plane (`shared_jit`).
#[derive(Clone, Debug)]
pub struct WorktreePaths {
    /// Whether the selected data root belongs to a Git checkout.
    pub git_repository: bool,
    /// Git common directory of the data checkout or preserved relocated
    /// coordination checkout; selecting code never changes this authority.
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
    data_checkout_root: PathBuf,
    pub(crate) code_git: bool,
    code_top_level: PathBuf,
    selected_code_root: bool,
    layout_context: Arc<LayoutContext>,
}

impl PartialEq for WorktreePaths {
    fn eq(&self, other: &Self) -> bool {
        self.git_repository == other.git_repository
            && self.common_dir == other.common_dir
            && self.worktree_root == other.worktree_root
            && self.local_jit == other.local_jit
            && self.shared_jit == other.shared_jit
            && self.relocated_worktree_checkout == other.relocated_worktree_checkout
            && self.data_checkout_root == other.data_checkout_root
            && self.code_git == other.code_git
            && self.code_top_level == other.code_top_level
            && self.selected_code_root == other.selected_code_root
    }
}

impl WorktreePaths {
    /// Undiscovered paths never imply Git authority or claims coordination.
    pub(crate) fn non_git(data_root: &Path, worktree_root: &Path) -> Self {
        let common_dir = worktree_root.join(".git");
        Self {
            git_repository: false,
            data_checkout_root: worktree_root.to_path_buf(),
            code_git: false,
            code_top_level: worktree_root.to_path_buf(),
            selected_code_root: false,
            shared_jit: common_dir.join("jit"),
            common_dir,
            worktree_root: worktree_root.to_path_buf(),
            local_jit: data_root.to_path_buf(),
            relocated_worktree_checkout: None,
            layout_context: Arc::default(),
        }
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
        Self::detect_with_context(
            &probe,
            non_git_root,
            Some(selected_data_root),
            Arc::default(),
        )
    }

    fn detect_with_context(
        current: &Path,
        non_git_root: &Path,
        selected_data_root: Option<&Path>,
        layout_context: Arc<LayoutContext>,
    ) -> Result<Self> {
        let Some(probed) = layout_context.probe(current)? else {
            let worktree_checkout = selected_data_root
                .filter(|data_root| !data_root.starts_with(non_git_root))
                .map(|_| layout_context.probe(non_git_root))
                .transpose()?
                .flatten();
            let worktree_root = non_git_root.to_path_buf();
            let (common_dir, relocated_worktree_checkout): (PathBuf, Option<RelocatedCheckout>) =
                match worktree_checkout {
                    Some(checkout) => (checkout.common_dir.clone(), Some(checkout.into())),
                    None => (worktree_root.join(".git"), None),
                };
            return Ok(Self {
                layout_context,
                code_git: relocated_worktree_checkout.is_some(),
                code_top_level: relocated_worktree_checkout.as_ref().map_or_else(
                    || worktree_root.clone(),
                    |checkout| checkout.top_level.clone(),
                ),
                data_checkout_root: worktree_root.clone(),
                selected_code_root: false,
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
            layout_context,
            git_repository: true,
            data_checkout_root: worktree_root.clone(),
            code_git: true,
            code_top_level: worktree_root.clone(),
            selected_code_root: false,
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
            .then(|| GitCheckout::new(Git::default(), self.data_checkout_root.clone()))
    }

    /// Return the selected code checkout, if the code root is in Git.
    pub fn code_checkout(&self) -> Option<GitCheckout> {
        self.code_git
            .then(|| GitCheckout::new(Git::default(), self.worktree_root.clone()))
    }

    /// The top level of the Git checkout containing the code root.
    pub fn code_checkout_top_level(&self) -> Option<&Path> {
        self.code_git.then_some(self.code_top_level.as_path())
    }

    /// Select a code root while retaining data and coordination authority.
    pub fn with_worktree_root(&self, root: &Path) -> Result<Self> {
        let mut paths = self.clone();
        let checkout = self.layout_context.probe(root)?;
        paths.code_git = checkout.is_some();
        paths.code_top_level =
            checkout.map_or_else(|| root.to_path_buf(), |checkout| checkout.worktree_root);
        paths.worktree_root = root.to_path_buf();
        paths.selected_code_root = true;
        Ok(paths)
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
            None => (
                worktree_root.join(
                    self.local_jit
                        .strip_prefix(&self.data_checkout_root)
                        .unwrap_or(Path::new(".jit")),
                ),
                None,
            ),
            Some(_) => match self.layout_context.probe(&worktree_root)? {
                Some(checkout) => (self.local_jit.clone(), Some(checkout.into())),
                None => return Ok(None),
            },
        };
        Ok(Some(Self {
            layout_context: Arc::clone(&self.layout_context),
            git_repository: self.git_repository,
            common_dir: self.common_dir.clone(),
            relocated_worktree_checkout: relocated_worktree_checkout.clone(),
            data_checkout_root: worktree_root.clone(),
            code_git: if self.selected_code_root {
                self.code_git
            } else {
                self.git_repository || relocated_worktree_checkout.is_some()
            },
            selected_code_root: self.selected_code_root,
            code_top_level: if self.selected_code_root {
                self.code_top_level.clone()
            } else {
                relocated_worktree_checkout.as_ref().map_or_else(
                    || worktree_root.clone(),
                    |checkout| checkout.top_level.clone(),
                )
            },
            worktree_root: if self.selected_code_root {
                self.worktree_root.clone()
            } else {
                worktree_root
            },
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

    /// The `jit` directory under [`Self::checkout_git_dir`], holding that
    /// checkout's identity.
    pub(crate) fn checkout_control_dir(&self) -> Option<PathBuf> {
        self.checkout_git_dir().map(|git_dir| git_dir.join("jit"))
    }

    /// The git directories' `jit` control directories: the shared control
    /// plane, then [`Self::checkout_control_dir`] when there is one.
    pub(crate) fn git_control_dirs(&self) -> impl Iterator<Item = PathBuf> {
        std::iter::once(self.shared_jit.clone()).chain(self.checkout_control_dir())
    }

    /// Check if we're in a secondary worktree (not main worktree).
    ///
    /// Returns true if `common_dir != <checkout top level>/.git`.
    pub fn is_worktree(&self) -> bool {
        self.common_dir != self.checkout_top_level().join(".git")
    }

    /// The worktree root may be a subdirectory of the recorded relocated checkout.
    pub fn checkout_top_level(&self) -> &Path {
        self.relocated_worktree_checkout
            .as_ref()
            .map_or(&self.data_checkout_root, |checkout| &checkout.top_level)
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
            .strip_prefix(&self.data_checkout_root)
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

    #[cfg(unix)]
    fn counted_external_paths(
        store: &Path,
        selected: &Path,
        stand_in: &crate::test_harness::git::GitStandIn,
    ) -> WorktreePaths {
        WorktreePaths::detect_with_context(
            &existing_directory_for(store),
            selected,
            Some(store),
            Arc::new(LayoutContext {
                git: Git::with_program(stand_in.program()),
                ..LayoutContext::default()
            }),
        )
        .unwrap()
    }

    #[cfg(unix)]
    #[test]
    fn test_layout_context_reuses_exact_physical_and_git_reported_top_level_aliases() {
        use crate::test_harness::git::GitStandIn;
        let dir = tempfile::tempdir().unwrap();
        let (repository, _, store) = external_store_layout(dir.path());
        let subdirectory = repository.root().join("sub");
        std::fs::create_dir(&subdirectory).unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(repository.root(), &alias).unwrap();
        let stand_in = GitStandIn::new();
        let paths = counted_external_paths(&store, &subdirectory, &stand_in);

        for candidate in [subdirectory.as_path(), repository.root(), alias.as_path()] {
            let projected = paths
                .for_worktree_root(candidate.to_path_buf())
                .unwrap()
                .unwrap();
            assert_eq!(projected.checkout_top_level(), repository.root());
        }
        // An exact alias retains its first observation even if it is retargeted.
        std::fs::remove_file(&alias).unwrap();
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
        assert!(paths.for_worktree_root(alias).unwrap().is_some());
        assert_eq!(
            stand_in.invocations().len(),
            2,
            "{:?}",
            stand_in.invocations()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_layout_context_does_not_alias_nested_checkouts_or_git_administrative_directories() {
        use crate::test_harness::git::{GitRepository, GitStandIn};
        let dir = tempfile::tempdir().unwrap();
        let (repository, linked, store) = external_store_layout(dir.path());
        let nested = GitRepository::init(repository.root().join("nested"));
        let stand_in = GitStandIn::new();
        let paths = counted_external_paths(&store, repository.root(), &stand_in);
        let nested_paths = paths
            .for_worktree_root(nested.root().to_path_buf())
            .unwrap()
            .unwrap();
        assert_eq!(nested_paths.checkout_top_level(), nested.root());
        let sibling = paths.for_worktree_root(linked).unwrap().unwrap();

        for administrative in [
            paths.common_dir.as_path(),
            sibling.checkout_git_dir().unwrap(),
        ] {
            assert_eq!(
                paths
                    .for_worktree_root(administrative.to_path_buf())
                    .unwrap(),
                None
            );
        }
        assert_eq!(
            stand_in.invocations().len(),
            6,
            "{:?}",
            stand_in.invocations()
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_layout_context_negative_observation_does_not_classify_descendants() {
        use crate::test_harness::git::{GitRepository, GitStandIn};
        let dir = tempfile::tempdir().unwrap();
        let stand_in = GitStandIn::new();
        let context = LayoutContext {
            git: Git::with_program(stand_in.program()),
            ..LayoutContext::default()
        };
        assert_eq!(context.probe(dir.path()).unwrap(), None);
        let nested = GitRepository::init(dir.path().join("nested"));
        assert_eq!(
            context.probe(nested.root()).unwrap().unwrap().worktree_root,
            nested.root()
        );
        assert_eq!(context.probe(dir.path()).unwrap(), None);
        assert_eq!(stand_in.invocations().len(), 2);
    }

    #[cfg(unix)]
    #[test]
    fn test_layout_context_reuses_typed_errors_through_canonical_aliases() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("bad-git");
        crate::test_utils::write_executable(
            &program,
            "#!/bin/sh\nprintf x >> \"$0.calls\"\nprintf 'malformed\\n'\n",
        )
        .unwrap();
        let alias = dir.path().join("alias");
        std::os::unix::fs::symlink(dir.path(), &alias).unwrap();
        let context = LayoutContext {
            git: Git::with_program(&program),
            ..LayoutContext::default()
        };

        for path in [dir.path(), alias.as_path(), dir.path()] {
            let error = context.probe(path).unwrap_err();
            assert!(matches!(
                error.downcast_ref::<GitError>(),
                Some(GitError::UnexpectedOutput { .. })
            ));
        }
        assert_eq!(
            std::fs::read(program.with_extension("calls")).unwrap(),
            b"x"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_layout_context_keeps_an_unavailable_executable_observation() {
        let dir = tempfile::tempdir().unwrap();
        let program = dir.path().join("unavailable-git");
        let context = LayoutContext {
            git: Git::with_program(&program),
            ..LayoutContext::default()
        };
        assert_eq!(context.probe(dir.path()).unwrap(), None);
        crate::test_utils::write_executable(
            &program,
            "#!/bin/sh\nprintf x >> \"$0.calls\"\nexit 1\n",
        )
        .unwrap();

        assert_eq!(context.probe(dir.path()).unwrap(), None);
        assert!(!program.with_extension("calls").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_layout_context_concurrent_clones_publish_one_sibling_observation() {
        use crate::test_harness::git::GitStandIn;
        let dir = tempfile::tempdir().unwrap();
        let (repository, linked, store) = external_store_layout(dir.path());
        let stand_in = GitStandIn::new();
        let paths = counted_external_paths(&store, repository.root(), &stand_in);
        let barrier = std::sync::Barrier::new(8);
        let projected = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    let paths = paths.clone();
                    let linked = linked.clone();
                    let barrier = &barrier;
                    scope.spawn(move || {
                        barrier.wait();
                        paths.for_worktree_root(linked).unwrap().unwrap()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect::<Vec<_>>()
        });

        assert!(projected.windows(2).all(|pair| pair[0] == pair[1]));
        assert!(projected[0]
            .for_worktree_root(repository.root().to_path_buf())
            .unwrap()
            .is_some());
        assert_eq!(
            stand_in.invocations().len(),
            3,
            "{:?}",
            stand_in.invocations()
        );
    }

    #[test]
    fn test_layout_context_clone_reuses_observed_sibling_after_it_disappears() {
        let dir = tempfile::tempdir().unwrap();
        let (repository, linked, store) = external_store_layout(dir.path());
        let paths = WorktreePaths::detect_for_data_root(&store, repository.root()).unwrap();
        let projected = paths.for_worktree_root(linked.clone()).unwrap();
        std::fs::remove_dir_all(&linked).unwrap();

        assert_eq!(paths.clone().for_worktree_root(linked).unwrap(), projected);
    }

    #[test]
    fn test_layout_context_keeps_missing_sibling_absent_until_a_new_invocation() {
        let dir = tempfile::tempdir().unwrap();
        let (repository, _, store) = external_store_layout(dir.path());
        let missing = repository.root().join("missing");
        let paths = WorktreePaths::detect_for_data_root(&store, repository.root()).unwrap();
        assert_eq!(paths.for_worktree_root(missing.clone()).unwrap(), None);
        std::fs::create_dir(&missing).unwrap();

        assert_eq!(paths.for_worktree_root(missing.clone()).unwrap(), None);
        let fresh = WorktreePaths::detect_for_data_root(&store, repository.root()).unwrap();
        assert!(fresh.for_worktree_root(missing).unwrap().is_some());
    }

    #[cfg(unix)]
    #[test]
    fn test_selected_code_root_probes_each_distinct_context_once() {
        use crate::test_harness::git::{GitRepository, GitStandIn};
        let dir = tempfile::tempdir().unwrap();
        let data = GitRepository::init(dir.path().join("data"));
        let code = GitRepository::init(dir.path().join("code"));
        let stand_in = GitStandIn::new();
        let paths = WorktreePaths::detect_with_context(
            data.root(),
            data.root(),
            Some(&data.root().join(".jit")),
            Arc::new(LayoutContext {
                git: Git::with_program(stand_in.program()),
                ..LayoutContext::default()
            }),
        )
        .unwrap();
        let selected = paths.with_worktree_root(code.root()).unwrap();
        selected.with_worktree_root(code.root()).unwrap();
        assert_eq!(stand_in.invocations().len(), 2);
    }

    #[test]
    fn test_selected_code_root_preserves_data_and_coordination_authority() {
        let dir = tempfile::tempdir().unwrap();
        let data = crate::test_harness::git::GitRepository::init(dir.path().join("data"));
        let code = crate::test_harness::git::GitRepository::init(dir.path().join("code"));
        let paths =
            WorktreePaths::detect_for_data_root(&data.root().join(".jit"), data.root()).unwrap();
        let selected = paths.with_worktree_root(code.root()).unwrap();
        assert_eq!(selected.git().unwrap().worktree_root(), data.root());
        assert_eq!(
            selected.code_checkout().unwrap().worktree_root(),
            code.root()
        );
        assert_eq!(selected.checkout_top_level(), data.root());
        assert_eq!(selected.common_dir, paths.common_dir);
        let projected = selected
            .for_worktree_root(dir.path().join("linked"))
            .unwrap()
            .unwrap();
        assert_eq!(projected.worktree_root, code.root());
        assert_eq!(projected.checkout_top_level(), dir.path().join("linked"));
    }

    #[test]
    fn test_selected_code_subdirectory_retains_checkout_top_level() {
        let dir = tempfile::tempdir().unwrap();
        let data = crate::test_harness::git::GitRepository::init(dir.path().join("data"));
        let code = crate::test_harness::git::GitRepository::init(dir.path().join("code"));
        let sub = code.root().join("sub");
        std::fs::create_dir(&sub).unwrap();
        let paths =
            WorktreePaths::detect_for_data_root(&data.root().join(".jit"), data.root()).unwrap();
        let selected = paths.with_worktree_root(&sub).unwrap();
        assert_eq!(selected.code_checkout().unwrap().worktree_root(), sub);
        assert_eq!(selected.code_checkout_top_level(), Some(code.root()));
    }

    #[test]
    fn test_selected_non_git_code_does_not_inherit_data_checkout() {
        let dir = tempfile::tempdir().unwrap();
        let data = crate::test_harness::git::GitRepository::init(dir.path().join("data"));
        let code = dir.path().join("code");
        std::fs::create_dir(&code).unwrap();
        let paths =
            WorktreePaths::detect_for_data_root(&data.root().join(".jit"), data.root()).unwrap();
        let selected = paths.with_worktree_root(&code).unwrap();
        assert!(selected.git().is_some());
        assert!(selected.code_checkout().is_none());
    }

    #[test]
    fn test_detect_in_non_git_directory() {
        let current = tempfile::tempdir().unwrap();
        let selected_root = tempfile::tempdir().unwrap();

        let data_root = current.path().join(".jit");
        let paths = WorktreePaths::detect_for_data_root(&data_root, selected_root.path()).unwrap();

        assert_eq!(paths.worktree_root, selected_root.path());
        assert_eq!(paths.local_jit, data_root);
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
        assert!(!paths.is_worktree());
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
        assert!(!paths.is_worktree());
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
