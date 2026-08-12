//! Semantic fixture setup for the in-memory aggregate store.
//!
//! Integration tests use this boundary instead of knowing repository JSON shapes.

use super::{atomic_write::rename_noreplace_cap, InMemoryStorage, IssueStore};
use crate::domain::Issue;
use crate::repository_state::{serialize_issue, RepositoryIndex};
use anyhow::{bail, Context, Result};
use std::path::Path;

/// Atomically publish a staged test-fixture directory without replacing an
/// occupied destination.
///
/// This test-support seam deliberately reuses the storage layer's production
/// no-replace rename primitive. It exists so shared cross-process fixture
/// caches do not grow a weaker parallel publication convention.
#[doc(hidden)]
pub fn publish_fixture_directory_noreplace(source: &Path, destination: &Path) -> Result<()> {
    let source_parent = source
        .parent()
        .context("fixture staging directory has no parent")?;
    let destination_parent = destination
        .parent()
        .context("fixture destination has no parent")?;
    if source_parent != destination_parent {
        bail!("fixture staging and destination must share one parent directory");
    }
    let source_name = source
        .file_name()
        .context("fixture staging directory has no file name")?;
    let destination_name = destination
        .file_name()
        .context("fixture destination has no file name")?;
    let parent = super::repository_state_store::open_absolute_dir_nofollow(source_parent)
        .with_context(|| format!("open fixture cache parent {}", source_parent.display()))?;
    rename_noreplace_cap(&parent, source_name, &parent, destination_name).map_err(Into::into)
}

/// Publish a staged fixture receipt as one no-replace hard link.
#[doc(hidden)]
pub fn publish_fixture_file_noreplace(source: &Path, destination: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(source)?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        bail!("fixture receipt staging path is not an ordinary file");
    }
    std::fs::hard_link(source, destination).with_context(|| {
        format!(
            "atomically publish fixture receipt {}",
            destination.display()
        )
    })?;
    Ok(())
}

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
        self.add_data_file(
            format!("issues/{}.json", issue.id),
            std::str::from_utf8(&issue_bytes).expect("issue JSON is UTF-8"),
        );
        self.add_data_file(
            "index.json",
            std::str::from_utf8(&index_bytes).expect("index JSON is UTF-8"),
        );
    }
}
