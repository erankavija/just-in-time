//! Typed errors for path-based file reads in the storage layer.

use thiserror::Error;

/// Errors that can occur when reading file bytes from the storage backend.
///
/// Replaces stringly-typed `anyhow` errors for `IssueStore::read_path_bytes`
/// so callers (e.g. HTTP route handlers) can branch on the specific failure
/// without pattern-matching error message strings.
///
/// # Examples
///
/// ```
/// use jit::storage::PathReadError;
///
/// let err = PathReadError::NotFound("docs/spec.md".to_string());
/// assert!(matches!(err, PathReadError::NotFound(_)));
/// assert!(err.to_string().contains("not found"));
/// ```
#[derive(Error, Debug)]
pub enum PathReadError {
    /// The requested file or tree entry does not exist.
    #[error("File not found: {0}")]
    NotFound(String),
    /// The supplied git commit reference could not be resolved.
    #[error("Commit not found: {0}")]
    CommitNotFound(String),
    /// Any other storage or I/O error.
    #[error("Storage error: {0}")]
    Other(#[from] anyhow::Error),
}

impl From<std::io::Error> for PathReadError {
    fn from(e: std::io::Error) -> Self {
        // Preserve the NotFound kind so callers can map it to 404; everything
        // else becomes a generic storage error.
        if e.kind() == std::io::ErrorKind::NotFound {
            PathReadError::NotFound(e.to_string())
        } else {
            PathReadError::Other(anyhow::Error::from(e))
        }
    }
}
