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
pub enum RepositoryIndexError {
    /// The index is not valid JSON or does not match the index schema.
    #[error("failed to parse index: {0}")]
    Parse(#[from] serde_json::Error),
    /// The index was written by a repository format newer than this binary.
    #[error(
        "repository format version {found} is newer than this binary supports (max {supported})"
    )]
    UnsupportedVersion { found: u32, supported: u32 },
    /// The active membership contains the same issue more than once.
    #[error("index.json contains duplicate active issue id '{0}'")]
    DuplicateActive(String),
    /// The deleted membership contains the same issue more than once.
    #[error("index.json contains duplicate deleted issue id '{0}'")]
    DuplicateDeleted(String),
    /// One issue is simultaneously classified as active and deleted.
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
        self.clone().validate().map_err(|error| {
            serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?;
        let fields = BTreeMap::from([
            ("all_ids", serde_json::to_value(&self.all_ids)?),
            ("deleted_ids", serde_json::to_value(&self.deleted_ids)?),
            ("schema_version", serde_json::to_value(self.schema_version)?),
        ]);
        serde_json::to_vec_pretty(&fields)
    }

    /// Add one issue to active membership and remove it from deleted membership.
    ///
    /// The canonical active order is lexical and repeated insertion is a no-op.
    #[allow(dead_code)] // Only called from the test-support-gated fixture seam.
    pub(crate) fn upsert_active(&mut self, id: String) {
        if !self.all_ids.contains(&id) {
            self.all_ids.push(id.clone());
            self.all_ids.sort();
        }
        self.deleted_ids.retain(|deleted| deleted != &id);
    }

    /// Move one issue from active to deleted membership idempotently.
    ///
    /// Reachable under `feature = "test-support"` in addition to `cfg(test)`:
    /// `commands::test_helpers::with_open_race`'s delete-race fixture calls it,
    /// and `test_helpers` itself is reachable independent of `cfg(test)` when
    /// the feature is enabled without a test build (e.g. `cargo clippy
    /// --features test-support`), a configuration where this method is itself
    /// unreached.
    #[cfg(any(test, feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn mark_deleted(&mut self, id: String) {
        self.all_ids.retain(|active| active != &id);
        if !self.deleted_ids.contains(&id) {
            self.deleted_ids.push(id);
            self.deleted_ids.sort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pretty_bytes_rejects_invalid_membership() {
        let index = RepositoryIndex {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            all_ids: vec!["duplicate".into(), "duplicate".into()],
            deleted_ids: Vec::new(),
        };

        assert!(index
            .to_pretty_bytes()
            .unwrap_err()
            .to_string()
            .contains("duplicate active issue id 'duplicate'"));
    }
}
