//! Semantic fixture setup for the in-memory aggregate store.
//!
//! Integration tests use this boundary instead of knowing repository JSON shapes.

use super::{atomic_write::rename_noreplace_cap, InMemoryStorage, IssueStore};
use crate::domain::Issue;
use crate::repository_state::{
    serialize_issue, CapturedBytes, EntryIdentity, FileMode, RepositoryEntry, RepositoryIndex,
    VirtualPath, RECORD_SCRATCH_DIR,
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
    let parent = super::capability_fs::open_absolute_dir_nofollow(source_parent)
        .with_context(|| format!("open fixture cache parent {}", source_parent.display()))?;
    rename_noreplace_cap(&parent, source_name, &parent, destination_name).map_err(Into::into)
}

impl InMemoryStorage {
    /// Replace this store with an exact semantic image of `repository_root`.
    ///
    /// The adapter reads ordinary files and directories without following
    /// symbolic links, preserves normalized executable modes, and assigns the
    /// nested `.jit` tree to the data root. It is intentionally test-only: the
    /// source tree remains live beside the aggregate so worktree-relative
    /// package locations resolve through the same real filesystem boundary as
    /// production commands.
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
                // The redo record's scratch directory, which no repository path
                // addresses: a published root carries it from the publication
                // that created it.
                if relative == Path::new(".jit").join(RECORD_SCRATCH_DIR) {
                    return Ok(());
                }
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
                    RepositoryEntry::File {
                        content: CapturedBytes::new(object, fs::read(&path)?)?,
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
            record: None,
            commits: BTreeMap::new(),
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

    /// Seed one exact issue preimage and an index when the fixture has none.
    ///
    /// This deliberately bypasses product mutation behavior while preserving the
    /// repository codec and index invariants used by production publishers.
    #[doc(hidden)]
    pub fn seed_issue_fixture(&self, issue: &Issue) {
        let index = self
            .read_repo_file(".jit/index.json")
            .expect("fixture index path is valid")
            .map(|bytes| RepositoryIndex::parse(bytes.as_bytes()).expect("fixture index is valid"))
            .unwrap_or_default();

        let issue_bytes = serialize_issue(issue).expect("fixture issue serializes");
        let index_bytes = index.to_pretty_bytes().expect("fixture index serializes");
        self.add_data_file(
            crate::repository_state::VirtualPath::issue_record(&issue.id)
                .relative()
                .as_path(),
            std::str::from_utf8(&issue_bytes).expect("issue JSON is UTF-8"),
        );
        self.add_data_file(
            "index.json",
            std::str::from_utf8(&index_bytes).expect("index JSON is UTF-8"),
        );
    }
}
