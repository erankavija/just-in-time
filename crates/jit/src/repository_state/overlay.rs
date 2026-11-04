//! Proposed-state overlay over a captured repository image.
//!
//! This is a permanent `repository_state` primitive, not a migration shim: plan
//! §2 "One-way state pipeline" validates proposed state as an overlay of a delta
//! over a captured base image. [`apply_overlay`] takes a base [`RepositoryImage`]
//! and a set of canonical-[`VirtualPath`]-keyed proposed final bytes (or a typed
//! absence) — exactly the vocabulary a [`RepositoryDelta`](super::RepositoryDelta)'s
//! actions project to — and returns a new closed image. Only the PRODUCER of the
//! override map changes across the cutover (typed deltas replace the command-local
//! proposal machinery); this primitive is stable.
//!
//! The result is a [`RepositoryImage`], so the closed-read discipline is preserved:
//! a read outside the base's captured closure and the overridden paths still fails
//! typed as [`CaptureError::UndiscoveredRepositoryPath`], never a silent absence.
//! Every captured parent-directory listing reflects the overlay's additions and
//! removals so whole-repository validation sees one coherent proposed repository.

use super::RepositoryAction;
use super::{
    CaptureError, CapturedBytes, DirectoryListing, FileMode, RepositoryEntry, RepositoryImage,
    RepositoryLayoutError, RepositoryRootClass, VirtualPath,
};
use std::collections::BTreeMap;

/// A proposed-state overlay could not be closed into a coherent image.
#[derive(Debug, thiserror::Error)]
pub enum OverlayError {
    /// The overlaid image failed to close (uncanonical path, budget, identity).
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// An override path was not canonical for the base image's layout.
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
}

/// Object-identity marker for overlay-synthesized entries. Overlay bytes are
/// proposed, not boundary-captured, so they carry a stable synthetic object
/// identity distinct from a Git blob OID; the content SHA-256 still binds them.
const OVERLAY_OBJECT: &str = "overlay";

/// Overlay proposed final bytes/absence over `base`, returning a new closed image.
///
/// Each entry of `overrides` is a canonical [`VirtualPath`] mapped to
/// `Some(final bytes)` for a create/replace or `None` for a proposed deletion —
/// the projection of a [`RepositoryDelta`](super::RepositoryDelta)'s actions to
/// their final content. The base image's captured closure is extended by the
/// overridden paths so each override reads back through the returned image, while
/// any path outside `base ∪ overrides` still fails
/// [`CaptureError::UndiscoveredRepositoryPath`]. When an override's parent
/// directory has a captured listing, the listing's children are updated (added for
/// a create/replace, removed for a deletion) so directory-membership validation
/// observes the proposed final tree.
pub fn apply_overlay(
    base: &RepositoryImage,
    overrides: impl IntoIterator<Item = (VirtualPath, Option<Vec<u8>>)>,
) -> Result<RepositoryImage, OverlayError> {
    let layout = base.layout().clone();
    let mut spec = base.capture_spec().clone();
    let mut entries = base.entries().clone();
    let mut listings = base.directory_listings().clone();
    let pinned = base.pinned_evidence().clone();

    for (path, value) in overrides {
        layout.ensure_canonical(&path)?;
        // Extend the closed read-set so `entry(path)` resolves through the returned
        // image; a path outside base ∪ overrides stays undiscovered.
        spec.discover_paths([path.clone()])?;
        let present = value.is_some();
        let entry = match value {
            Some(bytes) => RepositoryEntry::File {
                content: CapturedBytes::new(OVERLAY_OBJECT, bytes)?,
                mode: FileMode::Regular,
            },
            None => RepositoryEntry::Absent,
        };
        entries.insert(path.clone(), entry);

        // Reflect the override in the captured parent listing, if the base captured
        // one. A directory validation never lists is simply left unchanged.
        if let Some((parent, name)) = parent_and_name(&path) {
            if let Some(listing) = listings.get(&parent) {
                let mut names = listing.names().clone();
                match present {
                    true => names.insert(name),
                    false => names.remove(&name),
                };
                listings.insert(parent, DirectoryListing::new(names)?);
            }
        }
    }

    Ok(RepositoryImage::close(
        layout, spec, entries, listings, pinned,
    )?)
}

/// The issue ids `base`'s listing states with `overrides` applied over it.
pub(crate) fn overlaid_issue_ids(
    base: &RepositoryImage,
    overrides: &BTreeMap<VirtualPath, Option<Vec<u8>>>,
) -> Result<Vec<crate::domain::identity::IssueId>, super::RepositoryIndexError> {
    let mut names = base
        .directory_listings()
        .get(&VirtualPath::ISSUES)
        .ok_or(super::RepositoryIndexError::IssuesListingNotCaptured)?
        .names()
        .clone();
    for (path, value) in overrides {
        match parent_and_name(path) {
            Some((parent, name)) if parent == VirtualPath::ISSUES => {
                match value {
                    Some(_) => names.insert(name),
                    None => names.remove(&name),
                };
            }
            _ => {}
        }
    }
    super::listed_issue_ids(&names)
}

/// Written bytes and deletions from `actions`; appends retain the captured log tail.
pub fn action_overrides<'a>(
    actions: impl IntoIterator<Item = &'a RepositoryAction>,
) -> BTreeMap<VirtualPath, Option<Vec<u8>>> {
    actions
        .into_iter()
        .filter_map(|action| match action {
            RepositoryAction::WriteFile { path, bytes, .. } => {
                Some((path.clone(), Some(bytes.clone())))
            }
            RepositoryAction::DeleteFile { path, .. } => Some((path.clone(), None)),
            RepositoryAction::AppendEvents { .. }
            | RepositoryAction::CreateDirectory { .. }
            | RepositoryAction::SetMode { .. } => None,
        })
        .collect()
}

/// The canonical parent-directory path and child name for a non-root path, or
/// `None` when `path` is a root itself (no parent listing to update).
fn parent_and_name(path: &VirtualPath) -> Option<(VirtualPath, String)> {
    let relative = path.relative().as_path();
    let name = relative.file_name()?.to_str()?.to_string();
    let parent_relative = relative.parent()?;
    let parent = match path.root_class() {
        RepositoryRootClass::Data => VirtualPath::data(parent_relative),
        RepositoryRootClass::Worktree => VirtualPath::worktree(parent_relative),
    }
    .ok()?;
    Some((parent, name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{ExpectedPreimage, RepositoryActionOwner};
    use crate::test_harness::image::ImageBuilder;

    #[test]
    fn test_apply_overlay_replaces_captured_file_bytes() {
        let base = ImageBuilder::new().file(VirtualPath::CONFIG, "old").build();
        let overlaid =
            apply_overlay(&base, [(VirtualPath::CONFIG, Some(b"new".to_vec()))]).unwrap();
        assert_eq!(
            overlaid.file_bytes(&VirtualPath::CONFIG).unwrap(),
            Some(&b"new"[..])
        );
    }

    #[test]
    fn test_action_overrides_event_append_produces_no_byte_override() {
        let base = ImageBuilder::new()
            .file(VirtualPath::EVENTS, "prior\nuncommitted")
            .build();
        let append = RepositoryAction::append_events(
            RepositoryActionOwner::RepositoryStateMutation,
            ExpectedPreimage::of(base.entry(&VirtualPath::EVENTS).unwrap()),
            6,
            b"next\n".to_vec(),
        );
        assert!(!action_overrides([&append]).contains_key(&VirtualPath::EVENTS));
    }

    #[test]
    fn test_apply_overlay_tombstone_reads_absent() {
        let base = ImageBuilder::new().file(VirtualPath::CONFIG, "old").build();
        let overlaid = apply_overlay(&base, [(VirtualPath::CONFIG, None)]).unwrap();
        assert_eq!(overlaid.file_bytes(&VirtualPath::CONFIG).unwrap(), None);
    }

    #[test]
    fn test_apply_overlay_adds_new_path_and_preserves_closed_read() {
        let base = ImageBuilder::new().file(VirtualPath::CONFIG, "cfg").build();
        let overlaid =
            apply_overlay(&base, [(VirtualPath::GATES, Some(b"gates".to_vec()))]).unwrap();
        assert_eq!(
            overlaid.file_bytes(&VirtualPath::GATES).unwrap(),
            Some(&b"gates"[..])
        );
        // A path outside base ∪ overrides is still an undiscovered read, never a
        // silent absence.
        let err = overlaid.entry(&VirtualPath::RULES).unwrap_err();
        assert!(matches!(err, CaptureError::UndiscoveredRepositoryPath(_)));
    }

    #[test]
    fn test_apply_overlay_updates_captured_issue_listing() {
        let first_path = VirtualPath::issue_record(&crate::test_utils::issue_id(1));
        let second_path = VirtualPath::issue_record(&crate::test_utils::issue_id(2));
        let first_name = first_path.file_name().unwrap().to_owned();
        let second_name = second_path.file_name().unwrap().to_owned();
        let base = ImageBuilder::new()
            .file(first_path.clone(), "{}")
            .listing(
                VirtualPath::ISSUES,
                DirectoryListing::new([first_name.clone()]).unwrap(),
            )
            .build();
        let overlaid = apply_overlay(
            &base,
            [(second_path, Some(b"{}".to_vec())), (first_path, None)],
        )
        .unwrap();
        let names = overlaid.directory_listings()[&VirtualPath::ISSUES].names();
        assert!(names.contains(&second_name));
        assert!(!names.contains(&first_name));
    }
}
