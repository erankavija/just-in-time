//! Git-backed canonical commit resolution and pinned artifact reads.
//!
//! Revision resolution deliberately shells out to Git with the exact
//! `rev-parse --verify <rev>^{commit}` contract. This preserves Git's own
//! symbolic and abbreviated revision semantics while publishing only a full,
//! hash-algorithm-agnostic OID to the domain layer.

use crate::domain::artifact_plan::ArtifactVersion;
use crate::repository_state::RepositoryTargetKind;
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
    /// Git could not enumerate paths changed between the build and review
    /// trees, or in the current working tree.
    #[error("git changed-path query failed for {operation}: {stderr}")]
    ChangedPathsFailed { operation: String, stderr: String },
    /// The artifact path violates the repository-relative storage contract.
    #[error(transparent)]
    InvalidPath(#[from] crate::storage::PathReadError),
}

/// Bytes read at one canonical historical commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedArtifactRead {
    version: ArtifactVersion,
    blob_oid: String,
    bytes: Vec<u8>,
}

/// No-follow classification of one target at a resolved historical commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinnedTargetRead {
    version: ArtifactVersion,
    target_kind: RepositoryTargetKind,
    object_oid: Option<String>,
    bytes: Option<Vec<u8>>,
    unavailable_reason: Option<String>,
}

impl PinnedTargetRead {
    /// Canonical historical commit used for this classification.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }

    /// Captured filesystem kind at the named commit.
    pub fn target_kind(&self) -> RepositoryTargetKind {
        self.target_kind
    }

    /// Git object id for a present target.
    pub fn object_oid(&self) -> Option<&str> {
        self.object_oid.as_deref()
    }

    /// Exact bytes for an ordinary file target.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// Stable reason when the target is missing at a resolved commit.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }
}

impl PinnedArtifactRead {
    /// Canonical pinned version associated with the bytes.
    pub fn version(&self) -> &ArtifactVersion {
        &self.version
    }

    /// True Git blob object id of the read content, so capture identity binds to
    /// Git's own object identity rather than a synthesized surrogate.
    pub fn blob_oid(&self) -> &str {
        &self.blob_oid
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
        let oid_output = self.run(["rev-parse", "--verify", object.as_str()])?;
        if !oid_output.status.success() {
            return Err(GitRevisionError::PinnedReadFailed {
                revision: revision.to_string(),
                path: path.to_string(),
                stderr: stderr(&oid_output),
            });
        }
        let blob_oid = String::from_utf8_lossy(&oid_output.stdout)
            .trim()
            .to_string();
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
            blob_oid,
            bytes: output.stdout,
        })
    }

    /// Resolve and classify `path` at `revision` without consulting the
    /// working tree.
    ///
    /// Unlike [`read_pinned_path`](Self::read_pinned_path), directories and
    /// unsupported Git tree entries are successful, typed results. A path
    /// absent from an otherwise resolved commit is likewise represented as
    /// [`RepositoryTargetKind::Missing`] with its boundary diagnostic.
    pub fn inspect_pinned_target(
        &self,
        revision: &str,
        path: &str,
    ) -> Result<PinnedTargetRead, GitRevisionError> {
        validate_repo_relative_path(path)?;
        let version = self.resolve_commit(revision)?;
        let object = format!("{}:{path}", version.as_str());
        let oid_output = self.run(["rev-parse", "--verify", object.as_str()])?;
        if !oid_output.status.success() {
            return Ok(PinnedTargetRead {
                version,
                target_kind: RepositoryTargetKind::Missing,
                object_oid: None,
                bytes: None,
                unavailable_reason: Some(stderr(&oid_output)),
            });
        }
        let object_oid = String::from_utf8_lossy(&oid_output.stdout)
            .trim()
            .to_string();
        let kind_output = self.run(["cat-file", "-t", object.as_str()])?;
        if !kind_output.status.success() {
            return Err(GitRevisionError::PinnedReadFailed {
                revision: revision.to_string(),
                path: path.to_string(),
                stderr: stderr(&kind_output),
            });
        }
        let target_kind = match String::from_utf8_lossy(&kind_output.stdout).trim() {
            "blob" => RepositoryTargetKind::File,
            "tree" => RepositoryTargetKind::Directory,
            _ => RepositoryTargetKind::Unsupported,
        };
        let bytes = if target_kind.is_file() {
            let output = self.run(["cat-file", "blob", object.as_str()])?;
            if !output.status.success() {
                return Err(GitRevisionError::PinnedReadFailed {
                    revision: revision.to_string(),
                    path: path.to_string(),
                    stderr: stderr(&output),
                });
            }
            Some(output.stdout)
        } else {
            None
        };
        Ok(PinnedTargetRead {
            version,
            target_kind,
            object_oid: Some(object_oid),
            bytes,
            unavailable_reason: None,
        })
    }

    /// Return paths changed between two revisions, using NUL-delimited Git
    /// output so unusual but valid repository paths cannot be confused with
    /// record separators.
    pub fn changed_paths_between(
        &self,
        from: &str,
        to: &str,
    ) -> Result<Vec<String>, GitRevisionError> {
        let output = self.run(["diff", "--name-only", "--no-renames", "-z", from, to])?;
        parse_changed_paths(output, format!("{from}..{to}"))
    }

    /// Return tracked and untracked paths that differ from `HEAD` in the
    /// working tree. Ignored paths are included because a compile-time input
    /// can be locally ignored while still changing the produced binary.
    pub fn changed_worktree_paths(&self) -> Result<Vec<String>, GitRevisionError> {
        let tracked = self.run(["diff", "--name-only", "--no-renames", "-z", "HEAD"])?;
        let untracked = self.run(["ls-files", "--others", "--exclude-standard", "-z"])?;
        let ignored = self.run([
            "ls-files",
            "--others",
            "--ignored",
            "--exclude-standard",
            "-z",
        ])?;
        let mut paths = parse_changed_paths(tracked, "working tree".to_string())?;
        paths.extend(parse_changed_paths(
            untracked,
            "untracked working tree".to_string(),
        )?);
        paths.extend(parse_changed_paths(
            ignored,
            "ignored working tree".to_string(),
        )?);
        Ok(paths)
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

fn parse_changed_paths(output: Output, operation: String) -> Result<Vec<String>, GitRevisionError> {
    if !output.status.success() {
        return Err(GitRevisionError::ChangedPathsFailed {
            operation,
            stderr: stderr(&output),
        });
    }

    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
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
