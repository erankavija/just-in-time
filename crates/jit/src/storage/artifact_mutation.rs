//! Narrow filesystem mutation primitives for artifact archival.
//!
//! This module deliberately owns only path containment, staging, content
//! verification, no-replace publication, and identity-verified deletion. It
//! does not plan an archive operation, acquire an operation-wide lock, mutate
//! issues or events, or decide whether a source is eligible for deletion.

use super::{validate_repo_relative_path, IssueStore, JsonFileStorage};
use crate::domain::artifact_plan::ContentIdentity;
use crate::errors::AlreadyExistsError;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A completely written repository-local staging file.
///
/// The path is intentionally opaque: callers can verify and publish the staged
/// bytes but cannot redirect publication to an arbitrary filesystem object.
/// Dropping the handle removes an unpublished stage file best-effort.
#[derive(Debug)]
pub struct StagedArtifact {
    path: PathBuf,
}

impl Drop for StagedArtifact {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Typed failures specific to storage-owned artifact mutations.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactMutationError {
    /// A repository-relative artifact path traverses a symbolic link.
    #[error("artifact path traverses a symbolic link: {path}")]
    SymlinkArtifact { path: PathBuf },

    /// An artifact source or stage is not a regular file.
    #[error("artifact path is not a regular file: {path}")]
    NotRegularFile { path: PathBuf },

    /// Bytes no longer match the identity recorded by the caller.
    #[error(
        "artifact identity mismatch at {path}: expected sha256={expected_sha256}, byte_size={expected_byte_size}; actual sha256={actual_sha256}, byte_size={actual_byte_size}"
    )]
    IdentityMismatch {
        path: PathBuf,
        expected_sha256: String,
        expected_byte_size: u64,
        actual_sha256: String,
        actual_byte_size: u64,
    },

    /// Atomic hard-link publication cannot cross filesystem boundaries.
    #[error(
        "cannot atomically publish staged artifact across filesystems: {staged_path} -> {destination_path}"
    )]
    CrossFilesystem {
        staged_path: PathBuf,
        destination_path: PathBuf,
    },
}

impl JsonFileStorage {
    /// Resolve a repository-relative artifact path without following symlinks.
    ///
    /// Existing components, including the leaf, are inspected with
    /// `symlink_metadata`; any symbolic link is rejected even when it points
    /// back inside the repository. Missing trailing components are allowed so
    /// the same operation can validate a not-yet-created destination.
    ///
    /// # Errors
    ///
    /// Returns a typed path error for an empty, absolute, or parent-traversing
    /// input, [`ArtifactMutationError::SymlinkArtifact`] for any symlink
    /// component, and an I/O error when path metadata cannot be inspected.
    pub fn physical_repo_path(&self, relative: &str) -> Result<PathBuf> {
        validate_repo_relative_path(relative).map_err(anyhow::Error::new)?;
        let repo_root = repository_root(self)?;
        let canonical_root = fs::canonicalize(&repo_root)
            .with_context(|| format!("canonicalizing repository root {}", repo_root.display()))?;

        let mut candidate = repo_root.clone();
        let mut missing_prefix = false;
        for component in Path::new(relative).components() {
            candidate.push(component.as_os_str());
            if missing_prefix {
                continue;
            }
            match fs::symlink_metadata(&candidate) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(ArtifactMutationError::SymlinkArtifact {
                        path: candidate.clone(),
                    }
                    .into());
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    missing_prefix = true;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("inspecting artifact path {}", candidate.display())
                    });
                }
            }
        }

        if !missing_prefix {
            let canonical_candidate = fs::canonicalize(&candidate)
                .with_context(|| format!("canonicalizing artifact path {}", candidate.display()))?;
            if !canonical_candidate.starts_with(&canonical_root) {
                return Err(
                    crate::storage::PathReadError::OutsideRepoRoot(relative.to_string()).into(),
                );
            }
        }

        Ok(candidate)
    }

    /// Copy one regular repository file into an opaque, unique staging file.
    ///
    /// Staging performs no identity decision; callers separately compare the
    /// completed stage with their recorded identity using
    /// [`JsonFileStorage::verify_staged_artifact`].
    pub fn stage_artifact(&self, source: &str) -> Result<StagedArtifact> {
        let source_path = self.physical_repo_path(source)?;
        ensure_regular_file(&source_path)?;
        let bytes = fs::read(&source_path)
            .with_context(|| format!("reading artifact source {}", source_path.display()))?;

        let stage_relative = format!(".jit/tmp/artifact-{}", uuid::Uuid::new_v4());
        let stage_path = self.physical_repo_path(&stage_relative)?;
        let stage_parent = stage_path.parent().ok_or_else(|| {
            anyhow::anyhow!("staging path has no parent: {}", stage_path.display())
        })?;
        fs::create_dir_all(stage_parent).with_context(|| {
            format!(
                "creating artifact staging directory {}",
                stage_parent.display()
            )
        })?;
        // Re-check after directory creation so a pre-existing symlink component
        // can never be used as the staging location.
        let stage_path = self.physical_repo_path(&stage_relative)?;
        let write_result = (|| -> Result<()> {
            let mut stage_file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&stage_path)
                .with_context(|| format!("creating artifact stage {}", stage_path.display()))?;
            stage_file
                .write_all(&bytes)
                .with_context(|| format!("writing artifact stage {}", stage_path.display()))?;
            stage_file
                .flush()
                .with_context(|| format!("flushing artifact stage {}", stage_path.display()))
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&stage_path);
            return Err(error);
        }

        Ok(StagedArtifact { path: stage_path })
    }

    /// Verify completed staged bytes against a caller-supplied recorded identity.
    ///
    /// A mismatch is typed and leaves the stage available for caller diagnostics
    /// until its handle is dropped.
    pub fn verify_staged_artifact(
        &self,
        staged: &StagedArtifact,
        expected: &ContentIdentity,
    ) -> Result<()> {
        ensure_stage_belongs_to_storage(self, staged)?;
        verify_path_identity(&staged.path, expected)
    }

    /// Atomically publish a verified stage without replacing any destination.
    ///
    /// Publication uses a hard link from the complete stage file. Creating the
    /// destination directory entry is therefore atomic and the filesystem
    /// itself arbitrates races. Any occupied destination, including identical
    /// bytes, returns [`AlreadyExistsError`]. A destination on another
    /// filesystem returns [`ArtifactMutationError::CrossFilesystem`] naming
    /// both paths; there is no copy fallback.
    pub fn publish_staged_artifact(&self, staged: StagedArtifact, destination: &str) -> Result<()> {
        ensure_stage_belongs_to_storage(self, &staged)?;
        let destination_path = self.physical_repo_path(destination)?;

        if fs::symlink_metadata(&destination_path).is_ok() {
            return Err(already_exists(&destination_path));
        }

        let destination_parent = destination_path.parent().ok_or_else(|| {
            anyhow::anyhow!(
                "artifact destination has no parent: {}",
                destination_path.display()
            )
        })?;
        fs::create_dir_all(destination_parent).with_context(|| {
            format!(
                "creating artifact destination directory {}",
                destination_parent.display()
            )
        })?;
        let destination_path = self.physical_repo_path(destination)?;

        // Preserve collision precedence after parent creation, before the
        // filesystem check. A pre-existing destination is always AlreadyExists.
        if fs::symlink_metadata(&destination_path).is_ok() {
            return Err(already_exists(&destination_path));
        }
        ensure_same_filesystem(&staged.path, &destination_path)?;

        match fs::hard_link(&staged.path, &destination_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(already_exists(&destination_path))
            }
            Err(error) if is_cross_filesystem(&error) => {
                Err(ArtifactMutationError::CrossFilesystem {
                    staged_path: staged.path.clone(),
                    destination_path,
                }
                .into())
            }
            Err(error) => Err(error).with_context(|| {
                format!(
                    "publishing staged artifact {} -> {}",
                    staged.path.display(),
                    destination_path.display()
                )
            }),
        }
    }

    /// Delete a caller-named source only if its current bytes still match the
    /// caller-supplied recorded identity.
    ///
    /// This primitive intentionally does not decide whether references make the
    /// source deletion-eligible. That command-layer precondition must be
    /// established before calling this method.
    pub fn delete_artifact_if_identity(
        &self,
        source: &str,
        expected: &ContentIdentity,
    ) -> Result<()> {
        let source_path = self.physical_repo_path(source)?;
        ensure_regular_file(&source_path)?;
        verify_path_identity(&source_path, expected)?;
        fs::remove_file(&source_path)
            .with_context(|| format!("deleting verified artifact {}", source_path.display()))
    }
}

fn ensure_regular_file(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("inspecting artifact file {}", path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(ArtifactMutationError::NotRegularFile {
            path: path.to_path_buf(),
        }
        .into());
    }
    Ok(())
}

fn ensure_stage_belongs_to_storage(
    storage: &JsonFileStorage,
    staged: &StagedArtifact,
) -> Result<()> {
    let repo_root = repository_root(storage)?;
    let relative = staged
        .path
        .strip_prefix(&repo_root)
        .with_context(|| format!("stage is outside repository: {}", staged.path.display()))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("stage path is not UTF-8: {}", staged.path.display()))?;
    let resolved = storage.physical_repo_path(relative)?;
    if resolved != staged.path {
        anyhow::bail!(
            "stage path changed during verification: {}",
            staged.path.display()
        );
    }
    ensure_regular_file(&staged.path)
}

fn repository_root(storage: &JsonFileStorage) -> Result<PathBuf> {
    let repo_root = IssueStore::root(storage)
        .parent()
        .ok_or_else(|| crate::errors::InvalidArgumentError::new("Invalid storage path"))?;
    if repo_root.as_os_str().is_empty() {
        std::env::current_dir().context("resolving repository root")
    } else if repo_root.is_absolute() {
        Ok(repo_root.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .context("resolving repository root")?
            .join(repo_root))
    }
}

fn verify_path_identity(path: &Path, expected: &ContentIdentity) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("reading artifact {}", path.display()))?;
    let actual = content_identity(&bytes);
    if actual.sha256 == expected.sha256() && actual.byte_size == expected.byte_size() {
        return Ok(());
    }
    Err(ArtifactMutationError::IdentityMismatch {
        path: path.to_path_buf(),
        expected_sha256: expected.sha256().to_string(),
        expected_byte_size: expected.byte_size(),
        actual_sha256: actual.sha256,
        actual_byte_size: actual.byte_size,
    }
    .into())
}

struct ComputedIdentity {
    sha256: String,
    byte_size: u64,
}

fn content_identity(bytes: &[u8]) -> ComputedIdentity {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    ComputedIdentity {
        sha256: format!("{:x}", hasher.finalize()),
        byte_size: bytes.len() as u64,
    }
}

fn already_exists(destination: &Path) -> anyhow::Error {
    AlreadyExistsError::new(format!(
        "artifact destination already exists: {}",
        destination.display()
    ))
    .into()
}

#[cfg(unix)]
fn ensure_same_filesystem(staged: &Path, destination: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    let destination_parent = destination.parent().ok_or_else(|| {
        anyhow::anyhow!(
            "artifact destination has no parent: {}",
            destination.display()
        )
    })?;
    let staged_device = fs::metadata(staged)
        .with_context(|| format!("inspecting staged artifact {}", staged.display()))?
        .dev();
    let destination_device = fs::metadata(destination_parent)
        .with_context(|| {
            format!(
                "inspecting artifact destination directory {}",
                destination_parent.display()
            )
        })?
        .dev();
    if staged_device != destination_device {
        return Err(ArtifactMutationError::CrossFilesystem {
            staged_path: staged.to_path_buf(),
            destination_path: destination.to_path_buf(),
        }
        .into());
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_same_filesystem(_staged: &Path, _destination: &Path) -> Result<()> {
    // std exposes no portable filesystem identity. `hard_link` remains the
    // authoritative check and its cross-volume error is mapped below.
    Ok(())
}

fn is_cross_filesystem(error: &std::io::Error) -> bool {
    #[cfg(unix)]
    {
        error.raw_os_error() == Some(nix::errno::Errno::EXDEV as i32)
    }
    #[cfg(windows)]
    {
        const ERROR_NOT_SAME_DEVICE: i32 = 17;
        error.raw_os_error() == Some(ERROR_NOT_SAME_DEVICE)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = error;
        false
    }
}
