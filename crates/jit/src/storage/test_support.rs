//! Semantic fixture setup for the in-memory aggregate store.
//!
//! Integration tests use this boundary instead of knowing repository JSON shapes.

use super::{InMemoryStorage, IssueStore};
use crate::domain::Issue;
use crate::repository_state::{serialize_issue, RepositoryIndex};

impl InMemoryStorage {
    /// Seed one exact issue preimage and canonical active-index membership.
    ///
    /// This deliberately bypasses product mutation behavior while preserving the
    /// repository codec and index invariants used by production publishers.
    #[doc(hidden)]
    pub fn seed_issue_fixture(&self, issue: &Issue) {
        let mut index = self
            .read_repo_file(".jit/index.json")
            .expect("fixture index path is valid")
            .map(|bytes| RepositoryIndex::parse(bytes.as_bytes()).expect("fixture index is valid"))
            .unwrap_or_default();
        index.upsert_active(issue.id.clone());

        let issue_bytes = serialize_issue(issue).expect("fixture issue serializes");
        let index_bytes = index.to_pretty_bytes().expect("fixture index serializes");
        self.add_repo_file(
            &format!(".jit/issues/{}.json", issue.id),
            std::str::from_utf8(&issue_bytes).expect("issue JSON is UTF-8"),
        );
        self.add_repo_file(
            ".jit/index.json",
            std::str::from_utf8(&index_bytes).expect("index JSON is UTF-8"),
        );
    }
}
