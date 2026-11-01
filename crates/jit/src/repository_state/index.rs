//! Pure codec for `.jit/index.json` and the issue membership the issue files state.

use super::{RepositoryImage, VirtualPath};
use crate::domain::identity::IssueId;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// On-disk repository-format version understood by this binary.
pub(crate) const SUPPORTED_INDEX_SCHEMA_VERSION: u32 = 3;

/// Typed representation of `.jit/index.json`.
///
/// Membership is the issue files; `deleted_ids` lets a store's delete hide a lower
/// source's file. A format 2 index's `all_ids` is ignored and dropped at its next write.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepositoryIndex {
    pub(crate) schema_version: u32,
    #[serde(default)]
    pub(crate) deleted_ids: Vec<IssueId>,
}

impl Default for RepositoryIndex {
    fn default() -> Self {
        Self {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            deleted_ids: Vec::new(),
        }
    }
}

/// Failure to decode or validate one captured membership source.
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
    /// The deleted membership contains the same issue more than once.
    #[error("index.json contains duplicate deleted issue id '{0}'")]
    DuplicateDeleted(IssueId),
    /// A `.json` file under `.jit/issues` is not named for an issue id.
    #[error("issue file '{0}' is not named for an issue id")]
    IssueFileName(String),
    /// The image holds no listing of `.jit/issues`.
    #[error("captured image is missing the .jit/issues listing")]
    IssuesListingNotCaptured,
}

impl RepositoryIndex {
    /// Decode and structurally validate one captured index without ambient reads.
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self, RepositoryIndexError> {
        let index: Self = serde_json::from_slice(bytes)?;
        index.validate()
    }

    /// Validate version and tombstone-set invariants.
    pub(crate) fn validate(self) -> Result<Self, RepositoryIndexError> {
        if self.schema_version > SUPPORTED_INDEX_SCHEMA_VERSION {
            return Err(RepositoryIndexError::UnsupportedVersion {
                found: self.schema_version,
                supported: SUPPORTED_INDEX_SCHEMA_VERSION,
            });
        }
        let mut deleted = HashSet::new();
        match self.deleted_ids.iter().find(|id| !deleted.insert(*id)) {
            Some(id) => Err(RepositoryIndexError::DuplicateDeleted(id.clone())),
            None => Ok(self),
        }
    }

    /// Encode the canonical pretty-printed index used by every publisher.
    pub(crate) fn to_pretty_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        self.clone().validate().map_err(|error| {
            serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
        })?;
        let fields = BTreeMap::from([
            ("deleted_ids", serde_json::to_value(&self.deleted_ids)?),
            ("schema_version", serde_json::to_value(self.schema_version)?),
        ]);
        serde_json::to_vec_pretty(&fields)
    }

    /// Record one issue's tombstone idempotently.
    #[cfg(test)]
    pub(crate) fn mark_deleted(&mut self, id: IssueId) {
        if !self.deleted_ids.contains(&id) {
            self.deleted_ids.push(id);
            self.deleted_ids.sort();
        }
    }
}

/// The issue ids `names` of `.jit/issues` entries state, ascending as given.
/// Only `.json` names are records, which leaves out `*.lock`.
pub(crate) fn listed_issue_ids<'a>(
    names: impl IntoIterator<Item = &'a String>,
) -> Result<Vec<IssueId>, RepositoryIndexError> {
    names
        .into_iter()
        .filter_map(|name| name.strip_suffix(".json").map(|stem| (name, stem)))
        .map(|(name, stem)| {
            stem.parse()
                .map_err(|_| RepositoryIndexError::IssueFileName(name.clone()))
        })
        .collect()
}

/// The issue ids `image`'s captured `.jit/issues` listing states.
pub(crate) fn captured_issue_ids(
    image: &RepositoryImage,
) -> Result<Vec<IssueId>, RepositoryIndexError> {
    image
        .directory_listings()
        .get(&VirtualPath::ISSUES)
        .ok_or(RepositoryIndexError::IssuesListingNotCaptured)
        .and_then(|listing| listed_issue_ids(listing.names()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_to_pretty_bytes_rejects_duplicate_tombstones() {
        let id = crate::test_utils::issue_id(1);
        let index = RepositoryIndex {
            schema_version: SUPPORTED_INDEX_SCHEMA_VERSION,
            deleted_ids: vec![id.clone(), id.clone()],
        };

        assert!(index
            .to_pretty_bytes()
            .unwrap_err()
            .to_string()
            .contains(&format!("duplicate deleted issue id '{id}'")));
    }

    #[test]
    fn test_listed_issue_ids_skips_locks_and_rejects_other_json_names() {
        let id = crate::test_utils::issue_id(1);
        let lock = format!("{id}.lock");
        let record = format!("{id}.json");
        assert_eq!(listed_issue_ids([&lock, &record]).unwrap(), vec![id]);

        let stray = "notes.json".to_string();
        assert!(matches!(
            listed_issue_ids([&stray]),
            Err(RepositoryIndexError::IssueFileName(name)) if name == stray
        ));
    }

    #[test]
    fn test_parse_ignores_a_format_2_all_ids_and_keeps_its_marker() {
        let index = RepositoryIndex::parse(
            br#"{"schema_version": 2, "all_ids": ["stale"], "deleted_ids": []}"#,
        )
        .unwrap();
        assert_eq!(index.schema_version, 2);

        let rewritten: serde_json::Value =
            serde_json::from_slice(&index.to_pretty_bytes().unwrap()).unwrap();
        assert!(rewritten.get("all_ids").is_none());
        assert_eq!(rewritten["schema_version"], 2);
    }
}
