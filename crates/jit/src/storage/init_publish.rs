//! Publication of a data root that does not exist yet.
//!
//! The complete new root is written into a sibling stage directory, synced, and
//! published with a no-replace rename, so an occupied destination is refused
//! rather than overwritten (`@/inv/atomic-writes`, OD-6). This is the one
//! publication beside the redo record, and it exists because the record's home
//! is a scratch directory inside the root that does not exist yet: an
//! interruption before the rename therefore leaves no root at all, and the
//! orphan stage it left is removed by the next publication.

use crate::repository_state::RepositoryAction;
use crate::storage::atomic_write::{rename_noreplace_cap, sync_capability_directory};
use crate::storage::capability_fs::{open_child_dir_nofollow, DataRoot, DirectoryHandles};
use crate::storage::redo_apply::write_staged_file;
use crate::storage::repository_state_store::RepositoryStateStoreError;
use cap_std::fs::Dir;
use std::io::ErrorKind;
use std::path::Path;

/// The stage directory a publication of the root at `data_root` writes into.
#[cfg(test)]
pub(in crate::storage) fn stage_path(data_root: &Path) -> std::path::PathBuf {
    let leaf = data_root.file_name().unwrap_or_default().to_string_lossy();
    data_root.with_file_name(stage_name(&leaf))
}

/// The stage a publication of the root at `data_leaf` writes into.
///
/// One deterministic sibling, so the next publication reclaims the orphan an
/// interrupted one left; the session's bootstrap lock is what keeps two
/// publications from sharing it.
fn stage_name(data_leaf: &str) -> String {
    format!("{data_leaf}.stage")
}

/// Write the new root into a fresh stage beside the data root and return its
/// handle once its scaffold is durable.
///
/// Every action is a data-root action creating the root or something under it;
/// the stage is the root itself, so an action on the root's own path needs no
/// entry of its own.
///
/// # Errors
///
/// Returns an error when a non-directory occupies the stage name, when an
/// action neither creates a directory nor writes a file from its start — nothing exists yet
/// for a mode change or a deletion to name — or when the filesystem refuses an
/// operation.
pub(in crate::storage) fn stage_new_data_root<'a>(
    data_parent: &Dir,
    data_leaf: &str,
    actions: impl Iterator<Item = &'a RepositoryAction>,
) -> Result<DataRoot, RepositoryStateStoreError> {
    let stage = stage_name(data_leaf);
    remove_stage(data_parent, &stage)?;
    data_parent.create_dir(&stage)?;
    let root = open_child_dir_nofollow(data_parent, &stage)?;
    let mut handles = DirectoryHandles::new(&root);
    actions.into_iter().try_for_each(|action| {
        let relative = action.path().relative();
        let mut stage_file = |bytes: &[u8], mode| {
            let (parent, leaf) = handles.open_or_create_parent(relative.as_path())?;
            write_staged_file(parent, &leaf, bytes, mode)
                .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))
        };
        match action {
            RepositoryAction::CreateDirectory { .. } if relative.is_root() => Ok(()),
            RepositoryAction::CreateDirectory { .. } => {
                let (parent, leaf) = handles.open_or_create_parent(relative.as_path())?;
                match parent.create_dir(&leaf) {
                    Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
                    outcome => outcome.map_err(Into::into),
                }
            }
            RepositoryAction::WriteFile { bytes, mode, .. } => stage_file(bytes, *mode),
            RepositoryAction::AppendEvents {
                offset: 0, bytes, ..
            } => stage_file(bytes, crate::repository_state::FileMode::Regular),
            RepositoryAction::SetMode { .. }
            | RepositoryAction::DeleteFile { .. }
            | RepositoryAction::AppendEvents { .. } => Err(
                RepositoryStateStoreError::UnsafeTarget(format!("{:?}", action.path())),
            ),
        }
    })?;
    sync_tree(&root)?;
    Ok(DataRoot::from_dir(root))
}

/// Publish the stage as the data root at `data_root`.
///
/// The no-replace rename is itself the refusal, so a root another process
/// published since this session last looked is never overwritten. A refused
/// stage is removed: it holds a root no rename published.
///
/// # Errors
///
/// Returns [`RepositoryStateStoreError::OccupiedDataRoot`] when the
/// destination is taken, leaving what occupies it untouched.
pub(in crate::storage) fn publish_staged_data_root(
    data_parent: &Dir,
    data_leaf: &str,
    data_root: &Path,
) -> Result<(), RepositoryStateStoreError> {
    let stage = stage_name(data_leaf);
    match rename_noreplace_cap(data_parent, &stage, data_parent, data_leaf) {
        Ok(()) => sync_capability_directory(data_parent).map_err(Into::into),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            remove_stage(data_parent, &stage)?;
            Err(RepositoryStateStoreError::OccupiedDataRoot {
                path: data_root.to_path_buf(),
            })
        }
        Err(error) => Err(error.into()),
    }
}

/// Remove the stage directory, tolerating its absence.
///
/// What stands there is a root no rename ever published, so nothing outside it
/// refers to what it carries. A non-directory occupant is refused rather than
/// removed: the name was never this publication's to reclaim.
fn remove_stage(data_parent: &Dir, stage: &str) -> Result<(), RepositoryStateStoreError> {
    match data_parent.symlink_metadata(stage) {
        Ok(metadata) if metadata.is_dir() => data_parent.remove_dir_all(stage).map_err(Into::into),
        Ok(_) => Err(RepositoryStateStoreError::UnsafeTarget(stage.to_string())),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Fsync `directory` and every directory below it, so the scaffold's entries,
/// the ancestors staging created unsynced among them, are on the device before
/// the rename publishes them. A staged file's own bytes are synced as it is
/// written.
fn sync_tree(directory: &Dir) -> Result<(), RepositoryStateStoreError> {
    directory.entries()?.try_for_each(|entry| {
        let entry = entry?;
        match entry.file_type()?.is_dir() {
            true => sync_tree(&open_child_dir_nofollow(directory, entry.file_name())?),
            false => Ok(()),
        }
    })?;
    sync_capability_directory(directory).map_err(Into::into)
}
