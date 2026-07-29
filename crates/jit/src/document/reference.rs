//! Resolution of an issue's document references against captured evidence.
//!
//! A reference names a version: a commit pin names that commit, and an unpinned
//! reference names the current one. One rule serves every reader — whole-repository
//! validation and link checking classify a reference identically because they call
//! [`resolve_document_reference`] over the evidence [`DocumentReferenceRequests`]
//! declares, rather than each deciding for itself what a reference points at.
//!
//! Resolution is pure: every filesystem and Git read happens at the capture
//! boundary that closes the [`RepositoryImage`]. A pin can only be resolved
//! through Git, so where Git is unavailable the boundary records that as the
//! evidence's unavailable reason and the reference resolves to
//! [`UnresolvedDocumentReference::PinUnavailable`]; an unpinned reference is
//! unaffected, because the working tree answers it.

use crate::domain::DocumentReference;
use crate::repository_state::{
    CaptureError, RepositoryEntry, RepositoryImage, RepositoryLayoutError, VirtualPath,
};
use thiserror::Error;

/// Revision an unpinned reference falls back to when the working tree lacks its
/// file — the reference names the current version, which `HEAD` still carries
/// after a working-tree deletion.
const UNPINNED_REVISION: &str = "HEAD";

/// Failure to derive or resolve a document reference's evidence.
#[derive(Debug, Error)]
pub enum DocumentReferenceError {
    /// The reference's path is not a repository-relative worktree path.
    #[error(transparent)]
    Path(#[from] RepositoryLayoutError),
    /// The image does not carry the requested evidence.
    #[error(transparent)]
    Capture(#[from] CaptureError),
}

/// The evidence one document reference needs before it can be resolved.
///
/// A capturing caller enqueues every request; [`resolve_document_reference`]
/// then reads exactly those. Requesting less is a capture error at resolution
/// time rather than a silently different answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentReferenceRequests {
    worktree: Option<VirtualPath>,
    pinned: (String, String),
}

impl DocumentReferenceRequests {
    /// Derive the requests `document` resolves against.
    ///
    /// A pinned reference needs its `(commit, path)` evidence alone. An unpinned
    /// reference needs its working-tree entry and its `HEAD` evidence, because
    /// either one carries the current version.
    pub fn for_reference(document: &DocumentReference) -> Result<Self, DocumentReferenceError> {
        Ok(match document.commit.as_deref() {
            Some(commit) => Self {
                worktree: None,
                pinned: (commit.to_string(), document.path.clone()),
            },
            None => Self {
                worktree: Some(VirtualPath::worktree(&document.path)?),
                pinned: (UNPINNED_REVISION.to_string(), document.path.clone()),
            },
        })
    }

    /// The working-tree entry to capture, present for an unpinned reference.
    pub fn worktree(&self) -> Option<&VirtualPath> {
        self.worktree.as_ref()
    }

    /// The `(revision, path)` pinned-evidence request.
    pub fn pinned(&self) -> (&str, &str) {
        (&self.pinned.0, &self.pinned.1)
    }
}

/// Why a document reference does not resolve at the version it names.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnresolvedDocumentReference {
    /// The pinned `(commit, path)` pair carried no bytes at the capture
    /// boundary, either because the path is absent at that commit or because
    /// the commit itself could not be resolved.
    #[error("pinned document '{path}' at commit '{commit}' is unavailable: {reason}")]
    PinUnavailable {
        /// Repository-relative path the reference names.
        path: String,
        /// Commit the reference pins.
        commit: String,
        /// Stable boundary diagnostic for the missing evidence.
        reason: String,
    },
    /// The image was closed without the reference's pinned evidence.
    #[error("pinned document '{path}' at commit '{commit}' was not captured")]
    PinNotCaptured {
        /// Repository-relative path the reference names.
        path: String,
        /// Commit the reference pins.
        commit: String,
    },
    /// Neither the working tree nor `HEAD` carries an unpinned reference's file.
    #[error("file '{path}' not found in the working tree or at HEAD")]
    AbsentFromWorkingTreeAndHead {
        /// Repository-relative path the reference names.
        path: String,
    },
}

/// How a document reference resolves against captured evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentReferenceResolution {
    /// The reference's file exists at the version it names.
    Resolved,
    /// The reference's file does not exist at the version it names.
    Unresolved(UnresolvedDocumentReference),
}

impl DocumentReferenceResolution {
    /// Whether the reference resolves.
    pub fn is_resolved(&self) -> bool {
        matches!(self, Self::Resolved)
    }

    /// The reason the reference does not resolve, when it does not.
    pub fn unresolved(&self) -> Option<&UnresolvedDocumentReference> {
        match self {
            Self::Resolved => None,
            Self::Unresolved(reason) => Some(reason),
        }
    }
}

/// Resolve `document` against `image`.
///
/// A pinned reference resolves exactly when the image's evidence for its
/// `(commit, path)` request carries bytes, so a pin keeps naming its version
/// after the working tree moves past it, and a pin whose file is absent at that
/// commit stays unresolved even while the working tree holds the path. An
/// unpinned reference resolves from its working-tree entry, falling back to
/// `HEAD`.
///
/// # Errors
///
/// Returns [`DocumentReferenceError::Capture`] when `image` was closed without
/// the evidence [`DocumentReferenceRequests::for_reference`] declares, and
/// [`DocumentReferenceError::Path`] when the reference's path is not
/// repository-relative.
pub fn resolve_document_reference(
    image: &RepositoryImage,
    document: &DocumentReference,
) -> Result<DocumentReferenceResolution, DocumentReferenceError> {
    let requests = DocumentReferenceRequests::for_reference(document)?;
    let (revision, path) = requests.pinned();
    let evidence = image
        .pinned_evidence()
        .get(&(revision.to_string(), path.to_string()));

    let Some(worktree) = requests.worktree() else {
        return Ok(match evidence {
            Some(evidence) if evidence.exists() => DocumentReferenceResolution::Resolved,
            Some(evidence) => DocumentReferenceResolution::Unresolved(
                UnresolvedDocumentReference::PinUnavailable {
                    path: path.to_string(),
                    commit: revision.to_string(),
                    reason: evidence
                        .unavailable_reason()
                        .unwrap_or("not found")
                        .to_string(),
                },
            ),
            None => DocumentReferenceResolution::Unresolved(
                UnresolvedDocumentReference::PinNotCaptured {
                    path: path.to_string(),
                    commit: revision.to_string(),
                },
            ),
        });
    };

    let in_worktree = matches!(image.entry(worktree)?, RepositoryEntry::File { .. });
    let in_head = evidence.is_some_and(crate::repository_state::PinnedDocumentEvidence::exists);
    Ok(if in_worktree || in_head {
        DocumentReferenceResolution::Resolved
    } else {
        DocumentReferenceResolution::Unresolved(
            UnresolvedDocumentReference::AbsentFromWorkingTreeAndHead {
                path: path.to_string(),
            },
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::{
        CaptureBudget, CaptureSpec, EntryIdentity, FileMode, PinnedDocumentEvidence,
        PinnedSourceClass, RepositoryLayout, RepositoryRootEvidence,
    };
    use std::collections::BTreeMap;

    const BUDGET: CaptureBudget = CaptureBudget {
        max_paths: 8,
        max_listings: 0,
        max_bytes: 4096,
        max_depth: 8,
    };

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "wt", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn reference(path: &str, commit: Option<&str>) -> DocumentReference {
        DocumentReference {
            path: path.to_string(),
            commit: commit.map(String::from),
            label: None,
            doc_type: None,
            format: None,
            assets: Vec::new(),
        }
    }

    fn present_evidence(revision: &str, path: &str) -> PinnedDocumentEvidence {
        let bytes = b"captured".to_vec();
        PinnedDocumentEvidence::new(
            revision,
            path,
            PinnedSourceClass::GitObject,
            Some("a".repeat(40)),
            Some("b".repeat(40)),
            Some(EntryIdentity::for_bytes("git-blob:b", &bytes).unwrap()),
            Some(bytes),
            None,
        )
        .unwrap()
    }

    fn absent_evidence(revision: &str, path: &str, reason: &str) -> PinnedDocumentEvidence {
        PinnedDocumentEvidence::new(
            revision,
            path,
            PinnedSourceClass::GitUnavailable,
            None,
            None,
            None,
            None,
            Some(reason.to_string()),
        )
        .unwrap()
    }

    /// Close an image over exactly the requests `document` declares, with the
    /// supplied working-tree presence and pinned evidence.
    fn image(
        document: &DocumentReference,
        in_worktree: bool,
        evidence: Option<PinnedDocumentEvidence>,
    ) -> RepositoryImage {
        let requests = DocumentReferenceRequests::for_reference(document).unwrap();
        let (revision, path) = requests.pinned();
        let mut spec = CaptureSpec::phase_one(Vec::new(), BUDGET).unwrap();
        let mut entries = BTreeMap::new();
        if let Some(worktree) = requests.worktree() {
            spec.discover_paths([worktree.clone()]).unwrap();
            let bytes = b"working tree".to_vec();
            entries.insert(
                worktree.clone(),
                if in_worktree {
                    RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes("wt", &bytes).unwrap(),
                        bytes,
                        mode: FileMode::Regular,
                    }
                } else {
                    RepositoryEntry::Absent
                },
            );
        }
        let mut pinned = BTreeMap::new();
        if let Some(evidence) = evidence {
            spec.discover_pinned(revision, path).unwrap();
            pinned.insert((revision.to_string(), path.to_string()), evidence);
        }
        RepositoryImage::close(
            layout(),
            spec,
            entries,
            BTreeMap::new(),
            pinned,
            BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_for_reference_requests_only_the_pin_for_a_pinned_reference() {
        let pinned = DocumentReferenceRequests::for_reference(&reference(
            "docs/design.md",
            Some("c".repeat(40).as_str()),
        ))
        .unwrap();
        assert!(pinned.worktree().is_none());
        assert_eq!(pinned.pinned(), ("c".repeat(40).as_str(), "docs/design.md"));

        let unpinned =
            DocumentReferenceRequests::for_reference(&reference("docs/design.md", None)).unwrap();
        assert_eq!(
            unpinned.worktree(),
            Some(&VirtualPath::worktree("docs/design.md").unwrap())
        );
        assert_eq!(unpinned.pinned(), (UNPINNED_REVISION, "docs/design.md"));
    }

    #[test]
    fn test_resolve_document_reference_honours_the_pin_over_the_working_tree() {
        let commit = "d".repeat(40);
        let document = reference("docs/design.md", Some(&commit));

        let present = image(
            &document,
            false,
            Some(present_evidence(&commit, "docs/design.md")),
        );
        assert!(resolve_document_reference(&present, &document)
            .unwrap()
            .is_resolved());

        let absent = image(
            &document,
            true,
            Some(absent_evidence(
                &commit,
                "docs/design.md",
                "path not in tree",
            )),
        );
        assert!(matches!(
            resolve_document_reference(&absent, &document)
                .unwrap()
                .unresolved(),
            Some(UnresolvedDocumentReference::PinUnavailable { reason, .. })
                if reason.contains("path not in tree")
        ));
    }

    #[test]
    fn test_resolve_document_reference_reports_an_uncaptured_pin() {
        let commit = "e".repeat(40);
        let document = reference("docs/design.md", Some(&commit));
        assert!(matches!(
            resolve_document_reference(&image(&document, true, None), &document)
                .unwrap()
                .unresolved(),
            Some(UnresolvedDocumentReference::PinNotCaptured { .. })
        ));
    }

    #[test]
    fn test_resolve_document_reference_falls_back_to_head_for_an_unpinned_reference() {
        let document = reference("docs/design.md", None);

        assert!(resolve_document_reference(
            &image(
                &document,
                true,
                Some(absent_evidence(
                    UNPINNED_REVISION,
                    "docs/design.md",
                    "deleted"
                ))
            ),
            &document
        )
        .unwrap()
        .is_resolved());
        assert!(resolve_document_reference(
            &image(
                &document,
                false,
                Some(present_evidence(UNPINNED_REVISION, "docs/design.md"))
            ),
            &document
        )
        .unwrap()
        .is_resolved());
        assert!(matches!(
            resolve_document_reference(
                &image(
                    &document,
                    false,
                    Some(absent_evidence(
                        UNPINNED_REVISION,
                        "docs/design.md",
                        "deleted"
                    ))
                ),
                &document
            )
            .unwrap()
            .unresolved(),
            Some(UnresolvedDocumentReference::AbsentFromWorkingTreeAndHead { .. })
        ));
    }
}
