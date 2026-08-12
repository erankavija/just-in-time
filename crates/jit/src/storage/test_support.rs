//! Semantic fixture setup for the in-memory aggregate store.
//!
//! Integration tests use this boundary instead of knowing repository JSON shapes.

use super::{atomic_write::rename_noreplace_cap, InMemoryStorage, IssueStore};
use crate::domain::Issue;
use crate::repository_state::{
    serialize_issue, EntryIdentity, FileMode, RepositoryEntry, RepositoryIndex, VirtualPath,
};
use anyhow::{bail, Context, Result};
use std::collections::BTreeMap;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
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
    /// Replace this store with an exact semantic image of `repository_root`.
    ///
    /// The adapter reads ordinary files and directories without following
    /// symbolic links, preserves normalized executable modes, and assigns the
    /// nested `.jit` tree to the data root. It is intentionally test-only: the
    /// source tree remains live beside the aggregate so profile records can
    /// resolve their worktree-relative package locations through the same real
    /// filesystem boundary as production commands.
    #[doc(hidden)]
    pub fn seed_repository_tree_fixture(&self, repository_root: &Path) -> Result<()> {
        if self.root() != repository_root {
            bail!(
                "memory fixture root {} does not match source repository {}",
                self.root().display(),
                repository_root.display()
            );
        }

        fn normalized_mode(metadata: &fs::Metadata) -> FileMode {
            #[cfg(unix)]
            {
                if metadata.permissions().mode() & 0o111 == 0 {
                    FileMode::Regular
                } else {
                    FileMode::Executable
                }
            }
            #[cfg(not(unix))]
            {
                let _ = metadata;
                FileMode::Regular
            }
        }

        fn visit(
            repository_root: &Path,
            current: &Path,
            entries: &mut BTreeMap<VirtualPath, RepositoryEntry>,
        ) -> Result<()> {
            fs::read_dir(current)?.try_for_each(|entry| {
                let entry = entry?;
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() {
                    bail!(
                        "profiled memory fixture contains symbolic link {}",
                        path.display()
                    );
                }
                let relative = path.strip_prefix(repository_root).with_context(|| {
                    format!("fixture path {} escaped its repository", path.display())
                })?;
                let virtual_path = if relative == Path::new(".jit") {
                    VirtualPath::data("")?
                } else if let Ok(data_relative) = relative.strip_prefix(".jit") {
                    VirtualPath::data(data_relative)?
                } else {
                    VirtualPath::worktree(relative)?
                };
                let object = format!(
                    "memory-fixture:{:?}:{}",
                    virtual_path.root_class(),
                    virtual_path.relative().as_str()
                );
                let mode = normalized_mode(&metadata);
                let semantic_entry = if metadata.is_dir() {
                    RepositoryEntry::Directory {
                        identity: EntryIdentity::for_bytes(object, b"directory")?,
                        mode,
                    }
                } else if metadata.is_file() {
                    let bytes = fs::read(&path)?;
                    RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes(object, &bytes)?,
                        bytes,
                        mode,
                    }
                } else {
                    bail!(
                        "profiled memory fixture contains irregular entry {}",
                        path.display()
                    );
                };
                entries.insert(virtual_path, semantic_entry);
                if metadata.is_dir() {
                    visit(repository_root, &path, entries)?;
                }
                Ok(())
            })
        }

        let mut entries = BTreeMap::new();
        visit(repository_root, repository_root, &mut entries)?;
        let data_root_exists = entries.contains_key(&VirtualPath::data("")?);
        *self.repository_state() = super::memory::MemoryRepositoryState {
            entries,
            data_root_exists,
            recovery: None,
        };
        Ok(())
    }

    /// Normalized mode of a present repository file in this fixture store.
    #[doc(hidden)]
    pub fn repository_file_mode_fixture(&self, path: &str) -> Result<FileMode> {
        let path = self
            .repository_layout()
            .classify_repository_relative(path)?;
        match self.repository_state().entries.get(&path) {
            Some(RepositoryEntry::File { mode, .. }) => Ok(*mode),
            Some(_) => bail!("fixture path {path:?} is not an ordinary file"),
            None => bail!("fixture path {path:?} is absent"),
        }
    }

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
