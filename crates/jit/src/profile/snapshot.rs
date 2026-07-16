use super::ProjectedFileMode;
use crate::validation::repository::RepositoryView;
use anyhow::{anyhow, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

/// One regular file captured in an immutable repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotFile {
    /// Exact file bytes.
    pub bytes: Vec<u8>,
    /// Platform-neutral mode observed by the snapshot loader.
    pub mode: ProjectedFileMode,
}

/// Captured filesystem entry relevant to profile planning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotEntry {
    /// Ordinary file bytes and mode.
    File(SnapshotFile),
    /// Symbolic link or equivalent redirecting entry.
    Symlink {
        /// Link payload retained for diagnostics only.
        target: PathBuf,
    },
    /// Directory occupying a path.
    Directory,
    /// Entry whose semantics cannot be safely planned on this filesystem.
    Unsupported {
        /// Stable diagnostic supplied by the snapshot boundary.
        reason: String,
    },
}

/// Immutable, path-sorted repository image used by the pure profile planner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositorySnapshot {
    root: PathBuf,
    entries: BTreeMap<PathBuf, SnapshotEntry>,
}

impl RepositorySnapshot {
    /// Build a snapshot from repository-relative entries.
    pub fn new(
        root: impl Into<PathBuf>,
        entries: impl IntoIterator<Item = (PathBuf, SnapshotEntry)>,
    ) -> Result<Self, SnapshotError> {
        let entries =
            entries
                .into_iter()
                .try_fold(BTreeMap::new(), |mut entries, (path, entry)| {
                    validate_snapshot_path(&path)?;
                    if entries.insert(path.clone(), entry).is_some() {
                        return Err(SnapshotError::DuplicatePath(
                            path.to_string_lossy().into_owned(),
                        ));
                    }
                    Ok(entries)
                })?;
        Ok(Self {
            root: root.into(),
            entries,
        })
    }

    /// Repository root used for diagnostics by overlay validation.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Captured entry at one repository-relative path.
    pub fn entry(&self, path: impl AsRef<Path>) -> Option<&SnapshotEntry> {
        self.entries.get(path.as_ref())
    }

    /// Captured ordinary file at one repository-relative path.
    pub fn file(&self, path: impl AsRef<Path>) -> Option<&SnapshotFile> {
        match self.entry(path) {
            Some(SnapshotEntry::File(file)) => Some(file),
            _ => None,
        }
    }

    /// Deterministically ordered captured entries.
    pub fn entries(&self) -> &BTreeMap<PathBuf, SnapshotEntry> {
        &self.entries
    }
}

impl RepositoryView for RepositorySnapshot {
    fn repository_root(&self) -> &Path {
        &self.root
    }

    fn read_file(&self, relative: &Path) -> Result<Option<Vec<u8>>> {
        validate_snapshot_path(relative).map_err(|error| anyhow!(error))?;
        match self.entries.get(relative) {
            Some(SnapshotEntry::File(file)) => Ok(Some(file.bytes.clone())),
            Some(SnapshotEntry::Directory) | None => Ok(None),
            Some(SnapshotEntry::Symlink { .. }) => Err(anyhow!(
                "snapshot path '{}' is a symlink",
                relative.display()
            )),
            Some(SnapshotEntry::Unsupported { reason }) => Err(anyhow!(
                "snapshot path '{}' is unsupported: {reason}",
                relative.display()
            )),
        }
    }

    fn list_files(&self, relative_dir: &Path) -> Result<Vec<PathBuf>> {
        validate_snapshot_path(relative_dir).map_err(|error| anyhow!(error))?;
        Ok(self
            .entries
            .iter()
            .filter_map(|(path, entry)| {
                (path.starts_with(relative_dir) && matches!(entry, SnapshotEntry::File(_)))
                    .then_some(path.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
    }
}

/// Invalid repository snapshot input.
#[derive(Debug, thiserror::Error)]
pub enum SnapshotError {
    /// Snapshot keys must be canonical repository-relative paths.
    #[error("unsafe repository snapshot path '{0}'")]
    UnsafePath(String),
    /// Snapshot keys must identify each captured entry exactly once.
    #[error("duplicate repository snapshot path '{0}'")]
    DuplicatePath(String),
}

fn validate_snapshot_path(path: &Path) -> Result<(), SnapshotError> {
    let text = path.to_string_lossy();
    let safe = !text.is_empty()
        && !text.contains('\\')
        && !text.contains(':')
        && !text.chars().any(char::is_control)
        && !path.is_absolute()
        && path.components().all(|component| match component {
            Component::Normal(value) => value != "." && value != "..",
            _ => false,
        });
    if safe {
        Ok(())
    } else {
        Err(SnapshotError::UnsafePath(text.into_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_snapshot_rejects_unsafe_paths_and_lists_files_deterministically() {
        assert!(RepositorySnapshot::new(
            "/repo",
            [(PathBuf::from("../escape"), SnapshotEntry::Directory)]
        )
        .is_err());
        assert!(matches!(
            RepositorySnapshot::new(
                "/repo",
                [
                    (PathBuf::from("same"), SnapshotEntry::Directory),
                    (PathBuf::from("same"), SnapshotEntry::Directory),
                ],
            ),
            Err(SnapshotError::DuplicatePath(path)) if path == "same"
        ));

        let snapshot = RepositorySnapshot::new(
            "/repo",
            [
                (
                    PathBuf::from(".jit/z.json"),
                    SnapshotEntry::File(SnapshotFile {
                        bytes: b"z".to_vec(),
                        mode: ProjectedFileMode::Regular,
                    }),
                ),
                (
                    PathBuf::from(".jit/a.json"),
                    SnapshotEntry::File(SnapshotFile {
                        bytes: b"a".to_vec(),
                        mode: ProjectedFileMode::Regular,
                    }),
                ),
            ],
        )
        .unwrap();
        assert_eq!(
            snapshot.list_files(Path::new(".jit")).unwrap(),
            vec![PathBuf::from(".jit/a.json"), PathBuf::from(".jit/z.json")]
        );
    }
}
