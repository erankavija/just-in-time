//! Pure codec and structural validation for the repository membership index.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// On-disk repository-format version understood by this binary.
pub(crate) const SUPPORTED_INDEX_SCHEMA_VERSION: u32 = 2;

/// Typed representation of `.jit/index.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepositoryIndex {
    pub(crate) schema_version: u32,
    pub(crate) all_ids: Vec<String>,
    #[serde(default)]
    pub(crate) deleted_ids: Vec<String>,
}

impl Default for RepositoryIndex {
    fn default() -> Self {
        Self {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            all_ids: Vec::new(),
            deleted_ids: Vec::new(),
        }
    }
}

/// Failure to decode or validate one captured membership index.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RepositoryIndexError {
    #[error("failed to parse index: {0}")]
    Parse(#[from] serde_json::Error),
    #[error(
        "repository format version {found} is newer than this binary supports (max {supported})"
    )]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("index.json contains duplicate active issue id '{0}'")]
    DuplicateActive(String),
    #[error("index.json contains duplicate deleted issue id '{0}'")]
    DuplicateDeleted(String),
    #[error("index.json issue id '{0}' is both active and deleted")]
    ActiveDeletedOverlap(String),
}

impl RepositoryIndex {
    /// Decode and structurally validate one captured index without ambient reads.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, RepositoryIndexError> {
        let index: Self = serde_json::from_slice(bytes)?;
        index.validate()
    }

    /// Validate version and membership-set invariants.
    pub(crate) fn validate(self) -> Result<Self, RepositoryIndexError> {
        if self.schema_version > SUPPORTED_INDEX_SCHEMA_VERSION {
            return Err(RepositoryIndexError::UnsupportedVersion {
                found: self.schema_version,
                supported: SUPPORTED_INDEX_SCHEMA_VERSION,
            });
        }

        let mut active = HashSet::new();
        for id in &self.all_ids {
            if !active.insert(id) {
                return Err(RepositoryIndexError::DuplicateActive(id.clone()));
            }
        }
        let mut deleted = HashSet::new();
        for id in &self.deleted_ids {
            if !deleted.insert(id) {
                return Err(RepositoryIndexError::DuplicateDeleted(id.clone()));
            }
            if active.contains(id) {
                return Err(RepositoryIndexError::ActiveDeletedOverlap(id.clone()));
            }
        }
        Ok(self)
    }

    /// Encode the canonical pretty-printed index used by every publisher.
    pub(crate) fn to_pretty_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        let fields = BTreeMap::from([
            ("all_ids", serde_json::to_value(&self.all_ids)?),
            ("deleted_ids", serde_json::to_value(&self.deleted_ids)?),
            ("schema_version", serde_json::to_value(self.schema_version)?),
        ]);
        serde_json::to_vec_pretty(&fields)
    }
}
