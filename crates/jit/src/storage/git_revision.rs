//! Git-backed canonical commit resolution and pinned artifact reads.
//!
//! Revision resolution deliberately shells out to Git with the exact
//! `rev-parse --verify <rev>^{commit}` contract. This preserves Git's own
//! symbolic and abbreviated revision semantics while publishing only a full,
//! hash-algorithm-agnostic OID to the domain layer.

use crate::domain::artifact_inventory::PinnedRootResolver;
use crate::domain::artifact_plan::ArtifactVersion;
use crate::storage::validate_repo_relative_path;
use std::path::PathBuf;
use std::process::{Command, Output};
use thiserror::Error;

/// Errors from canonical revision resolution and historical blob reads.
#[derive(Debug, Error)]
pub enum GitRevisionError {
    /// The configured Git executable could not be started.
    #[error("git is unavailable at {program}: {source}")]
    GitUnavailable {
        program: PathBuf,
        #[source]
        source: std::io::Error,
    },
    /// Git rejected the supplied revision under commit-peeling semantics.
    #[error("revision could not be resolved to a commit: {revision}: {stderr}")]
    RevisionNotFound { revision: String, stderr: String },
    /// Git returned output that is not a canonical SHA-1 or SHA-256 commit OID.
    #[error("git returned a non-canonical commit OID for {revision}: {output}")]
    NonCanonicalOid { revision: String, output: String },
    /// The historical path could not be read at the resolved commit.
    #[error("pinned artifact read failed for {path} at {revision}: {stderr}")]
    PinnedReadFailed {
        revision: String,
        path: String,
        stderr: String,
    },
    /// The artifact path violates the repository-relative storage contract.
    #[error(transparent)]
    InvalidPath(#[from] crate::storage::PathReadError),
}

/// Bytes read at one canonical historical commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedArtifactRead {
    version: ArtifactVersion,
    bytes: Vec<u8>,
}

impl PinnedArtifactRead {
    /// Canonical pinned version associated with the bytes.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }

    /// Byte-faithful blob content read from Git history.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

/// Reusable Git boundary for version-aware artifact planning.
#[derive(Debug, Clone)]
pub struct GitRevisionResolver {
    repo_root: PathBuf,
    git_program: PathBuf,
}

impl GitRevisionResolver {
    /// Resolve revisions relative to `repo_root` using the `git` found on PATH.
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self::with_git_program(repo_root, "git")
    }

    /// Resolve revisions with an explicit Git executable.
    ///
    /// This supports hermetic embedders and makes the unavailable-Git failure
    /// contract directly testable without modifying process-global PATH.
    pub fn with_git_program(
        repo_root: impl Into<PathBuf>,
        git_program: impl Into<PathBuf>,
    ) -> Self {
        Self {
            repo_root: repo_root.into(),
            git_program: git_program.into(),
        }
    }

    /// Canonicalize `revision` with exact
    /// `git rev-parse --verify <revision>^{commit}` semantics.
    ///
    /// Both full SHA-1 and SHA-256 OIDs are accepted; abbreviated and symbolic
    /// inputs are returned only as Git's full canonical lowercase commit OID.
    pub fn resolve_commit(&self, revision: &str) -> Result<ArtifactVersion, GitRevisionError> {
        let peeled = format!("{revision}^{{commit}}");
        let output = self.run(["rev-parse", "--verify", peeled.as_str()])?;
        if !output.status.success() {
            return Err(GitRevisionError::RevisionNotFound {
                revision: revision.to_string(),
                stderr: stderr(&output),
            });
        }

        canonical_version(revision, &String::from_utf8_lossy(&output.stdout))
    }

    /// Resolve `revision` and read `path` from that exact commit.
    ///
    /// The method never consults the working tree. Resolution failures and
    /// commit-tree/blob failures are returned to the caller as blockers.
    pub fn read_pinned_path(
        &self,
        revision: &str,
        path: &str,
    ) -> Result<PinnedArtifactRead, GitRevisionError> {
        validate_repo_relative_path(path)?;
        let version = self.resolve_commit(revision)?;
        let object = format!("{}:{path}", version.as_str());
        let output = self.run(["cat-file", "blob", object.as_str()])?;
        if !output.status.success() {
            return Err(GitRevisionError::PinnedReadFailed {
                revision: revision.to_string(),
                path: path.to_string(),
                stderr: stderr(&output),
            });
        }
        Ok(PinnedArtifactRead {
            version,
            bytes: output.stdout,
        })
    }

    fn run<'a>(
        &self,
        arguments: impl IntoIterator<Item = &'a str>,
    ) -> Result<Output, GitRevisionError> {
        Command::new(&self.git_program)
            .current_dir(&self.repo_root)
            .args(arguments)
            .output()
            .map_err(|source| GitRevisionError::GitUnavailable {
                program: self.git_program.clone(),
                source,
            })
    }
}

impl PinnedRootResolver for GitRevisionResolver {
    type Error = GitRevisionError;

    fn resolve_and_read(&self, revision: &str, path: &str) -> Result<ArtifactVersion, Self::Error> {
        self.read_pinned_path(revision, path)
            .map(|read| read.version)
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).trim().to_string()
}

fn canonical_version(revision: &str, output: &str) -> Result<ArtifactVersion, GitRevisionError> {
    let oid = output.trim().to_string();
    ArtifactVersion::pinned(oid.clone()).map_err(|_| GitRevisionError::NonCanonicalOid {
        revision: revision.to_string(),
        output: oid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_version_accepts_full_sha1_and_sha256_git_output() {
        let sha1 = "0123456789abcdef0123456789abcdef01234567";
        let sha256 = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

        assert_eq!(canonical_version("HEAD", sha1).unwrap().as_str(), sha1);
        assert_eq!(
            canonical_version("HEAD", &format!("{sha256}\n"))
                .unwrap()
                .as_str(),
            sha256
        );
    }

    #[test]
    fn test_canonical_version_rejects_abbreviated_or_noncanonical_git_output() {
        assert!(matches!(
            canonical_version("HEAD", "01234567"),
            Err(GitRevisionError::NonCanonicalOid { .. })
        ));
        assert!(matches!(
            canonical_version("HEAD", "0123456789ABCDEF0123456789ABCDEF01234567"),
            Err(GitRevisionError::NonCanonicalOid { .. })
        ));
    }
}
