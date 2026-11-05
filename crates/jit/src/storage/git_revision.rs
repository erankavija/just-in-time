//! Temporary revision-resolver bridge; removed by jit:e6ec58f3 after storage,
//! archive and suite callers migrate (jit:c16c834c, jit:ec9e812c).

use crate::domain::artifact_plan::ArtifactVersion;
use crate::storage::git::{Git, GitCheckout};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

pub use crate::storage::git::{GitError as GitRevisionError, PinnedArtifactRead, PinnedTargetRead};

/// Delegates version-aware artifact planning to the storage git module.
#[derive(Debug, Clone)]
pub struct GitRevisionResolver {
    repo_root: PathBuf,
    git: Git,
}

impl GitRevisionResolver {
    /// Resolve revisions relative to `repo_root` using the default executable.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            git: Git::default(),
        }
    }

    /// Resolve revisions with an explicit Git executable.
    pub fn with_git_program(
        repo_root: impl Into<PathBuf>,
        git_program: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo_root: repo_root.into(),
            git: Git::with_program(git_program),
        }
    }

    /// See [`GitCheckout::resolve_commit`].
    pub fn resolve_commit(&self, revision: &str) -> Result<ArtifactVersion, GitRevisionError> {
        self.checkout().resolve_commit(revision)
    }

    /// See [`GitCheckout::read_pinned_path`].
    pub fn read_pinned_path(
        &self,
        revision: &str,
        path: &str,
    ) -> Result<PinnedArtifactRead, GitRevisionError> {
        self.checkout().read_pinned_path(revision, path)
    }

    /// See [`GitCheckout::inspect_pinned_targets`].
    pub fn inspect_pinned_targets(
        &self,
        requests: &BTreeSet<(String, String)>,
        kind_only: &BTreeSet<(String, String)>,
        cache: &mut BTreeMap<(String, String), PinnedTargetRead>,
    ) -> BTreeMap<(String, String), Result<PinnedTargetRead, GitRevisionError>> {
        self.checkout()
            .inspect_pinned_targets(requests, kind_only, cache)
    }

    fn checkout(&self) -> GitCheckout {
        GitCheckout::new(self.git.clone(), self.repo_root.clone())
    }
}
