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

use super::{
    CaptureError, EntryIdentity, FileMode, ListingFingerprint, RepositoryEntry, RepositoryImage,
    RepositoryLayoutError, RepositoryRootClass, VirtualPath,
};

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
    let mut listings = base.listing_fingerprints().clone();
    let pinned = base.pinned_evidence().clone();
    let linked = base.linked_worktree_evidence().clone();

    for (path, value) in overrides {
        layout.ensure_canonical(&path)?;
        // Extend the closed read-set so `entry(path)` resolves through the returned
        // image; a path outside base ∪ overrides stays undiscovered.
        spec.discover_paths([path.clone()])?;
        let (entry, child_identity) = match value {
            Some(bytes) => {
                let identity = EntryIdentity::for_bytes(OVERLAY_OBJECT, &bytes)?;
                (
                    RepositoryEntry::File {
                        identity: identity.clone(),
                        bytes,
                        mode: FileMode::Regular,
                    },
                    Some(identity),
                )
            }
            None => (RepositoryEntry::Absent, None),
        };
        entries.insert(path.clone(), entry);

        // Reflect the override in the captured parent listing, if the base captured
        // one. A directory validation never lists is simply left unchanged.
        if let Some((parent, name)) = parent_and_name(&path) {
            if let Some(listing) = listings.get(&parent) {
                let mut children = listing.children().clone();
                match child_identity {
                    Some(identity) => {
                        children.insert(name, identity);
                    }
                    None => {
                        children.remove(&name);
                    }
                }
                let updated = match listing.container() {
                    Some(container) => {
                        ListingFingerprint::for_directory(container.clone(), children)?
                    }
                    None => ListingFingerprint::new(children)?,
                };
                listings.insert(parent, updated);
            }
        }
    }

    Ok(RepositoryImage::close(
        layout, spec, entries, listings, pinned, linked,
    )?)
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
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, RepositoryLayout, RepositoryRootEvidence,
    };
    use std::collections::BTreeMap;

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "worktree", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn vpath(repo_rel: &str) -> VirtualPath {
        layout().classify_repository_relative(repo_rel).unwrap()
    }

    fn file_entry(text: &str) -> RepositoryEntry {
        RepositoryEntry::File {
            identity: EntryIdentity::for_bytes("base", text.as_bytes()).unwrap(),
            bytes: text.as_bytes().to_vec(),
            mode: FileMode::Regular,
        }
    }

    /// Build a base image with the given data files plus an optional captured
    /// listing for `.jit/issues` over the supplied issue file names.
    fn base_image(files: &[(&str, &str)], issues_listing: Option<&[&str]>) -> RepositoryImage {
        let budget = CaptureBudget {
            max_paths: 64,
            max_listings: 8,
            max_bytes: 1 << 20,
            max_depth: 8,
        };
        let mut entries = BTreeMap::new();
        let mut paths = Vec::new();
        for (repo_rel, text) in files {
            let path = vpath(repo_rel);
            paths.push(path.clone());
            entries.insert(path, file_entry(text));
        }
        let mut spec = CaptureSpec::phase_one(paths, budget).unwrap();
        let mut listings = BTreeMap::new();
        if let Some(names) = issues_listing {
            let dir = VirtualPath::data("issues").unwrap();
            spec.discover_listing(dir.clone()).unwrap();
            let mut children = BTreeMap::new();
            for name in names {
                children.insert(
                    (*name).to_string(),
                    EntryIdentity::for_bytes("child", name.as_bytes()).unwrap(),
                );
            }
            let container = EntryIdentity::for_bytes("dir", b"issues").unwrap();
            listings.insert(
                dir,
                ListingFingerprint::for_directory(container, children).unwrap(),
            );
        }
        RepositoryImage::close(
            layout(),
            spec,
            entries,
            listings,
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_apply_overlay_replaces_captured_file_bytes() {
        let base = base_image(&[(".jit/config.toml", "old")], None);
        let overlaid =
            apply_overlay(&base, [(vpath(".jit/config.toml"), Some(b"new".to_vec()))]).unwrap();
        assert_eq!(
            overlaid.file_bytes(&vpath(".jit/config.toml")).unwrap(),
            Some(&b"new"[..])
        );
    }

    #[test]
    fn test_apply_overlay_tombstone_reads_absent() {
        let base = base_image(&[(".jit/config.toml", "old")], None);
        let overlaid = apply_overlay(&base, [(vpath(".jit/config.toml"), None)]).unwrap();
        assert_eq!(
            overlaid.file_bytes(&vpath(".jit/config.toml")).unwrap(),
            None
        );
    }

    #[test]
    fn test_apply_overlay_adds_new_path_and_preserves_closed_read() {
        let base = base_image(&[(".jit/config.toml", "cfg")], None);
        let overlaid =
            apply_overlay(&base, [(vpath(".jit/gates.toml"), Some(b"gates".to_vec()))]).unwrap();
        assert_eq!(
            overlaid.file_bytes(&vpath(".jit/gates.toml")).unwrap(),
            Some(&b"gates"[..])
        );
        // A path outside base ∪ overrides is still an undiscovered read, never a
        // silent absence.
        let err = overlaid.entry(&vpath(".jit/rules.toml")).unwrap_err();
        assert!(matches!(err, CaptureError::UndiscoveredRepositoryPath(_)));
    }

    #[test]
    fn test_apply_overlay_updates_captured_issue_listing() {
        let base = base_image(&[(".jit/issues/a.json", "{}")], Some(&["a.json"]));
        let overlaid = apply_overlay(
            &base,
            [
                (vpath(".jit/issues/b.json"), Some(b"{}".to_vec())),
                (vpath(".jit/issues/a.json"), None),
            ],
        )
        .unwrap();
        let dir = VirtualPath::data("issues").unwrap();
        let children = overlaid.listing_fingerprints()[&dir].children();
        assert!(children.contains_key("b.json"));
        assert!(!children.contains_key("a.json"));
    }
}
