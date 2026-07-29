//! Closed repository images, bounded capture declarations, and exact deltas.

use super::{RepositoryLayout, RepositoryLayoutError, RepositoryRootClass, VirtualPath};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

/// Platform-neutral file mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileMode {
    /// Non-executable ordinary file.
    Regular,
    /// Executable ordinary file.
    Executable,
}

/// Exact identity of captured content or an occupant.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct EntryIdentity {
    /// Boundary-acquired no-follow object identity.
    object: String,
    /// SHA-256 of exact bytes or stable occupant metadata.
    sha256: String,
    /// Exact byte count where applicable.
    byte_size: u64,
}

impl EntryIdentity {
    /// Derive a content identity from boundary-provided object identity and bytes.
    pub fn for_bytes(object: impl Into<String>, bytes: &[u8]) -> Result<Self, CaptureError> {
        let identity = Self {
            object: object.into(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
        };
        identity.validate()?;
        Ok(identity)
    }

    /// Stable boundary object identity.
    pub fn object(&self) -> &str {
        &self.object
    }

    /// Lowercase SHA-256 digest.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    /// Exact captured byte size.
    pub fn byte_size(&self) -> u64 {
        self.byte_size
    }

    pub(crate) fn validate(&self) -> Result<(), CaptureError> {
        if self.object.is_empty() || self.object.chars().any(char::is_control) {
            return Err(CaptureError::InvalidEntryIdentity(
                "object identity is empty or contains control characters".into(),
            ));
        }
        if self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(CaptureError::InvalidEntryIdentity(
                "SHA-256 must be 64 lowercase hexadecimal characters".into(),
            ));
        }
        Ok(())
    }

    fn validate_bytes(&self, bytes: &[u8]) -> Result<(), CaptureError> {
        self.validate()?;
        let actual = Self::for_bytes(self.object.clone(), bytes)?;
        if self.sha256 != actual.sha256 || self.byte_size != actual.byte_size {
            return Err(CaptureError::EntryIdentityMismatch {
                object: self.object.clone(),
            });
        }
        Ok(())
    }
}

/// One exact captured path state, including typed absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RepositoryEntry {
    /// The captured path was absent.
    Absent,
    /// An ordinary file and its exact bytes.
    File {
        /// Exact entry identity.
        identity: EntryIdentity,
        /// Exact bytes.
        bytes: Vec<u8>,
        /// Normalized mode.
        mode: FileMode,
    },
    /// A directory occupant.
    Directory {
        /// Exact directory identity.
        identity: EntryIdentity,
        /// Normalized directory mode.
        mode: FileMode,
    },
    /// A symbolic link, never followed.
    Symlink {
        /// Exact link identity.
        identity: EntryIdentity,
        /// Exact link payload.
        target: Vec<u8>,
        /// Normalized link mode where the platform exposes one.
        mode: FileMode,
    },
    /// An occupant unsafe for semantic mutation.
    Unsupported {
        /// Exact occupant identity.
        identity: EntryIdentity,
        /// Stable diagnostic.
        reason: String,
        /// Normalized occupant mode where the platform exposes one.
        mode: FileMode,
    },
}

impl RepositoryEntry {
    /// Exact identity when an occupant exists.
    pub fn identity(&self) -> Option<&EntryIdentity> {
        match self {
            Self::Absent => None,
            Self::File { identity, .. }
            | Self::Directory { identity, .. }
            | Self::Symlink { identity, .. }
            | Self::Unsupported { identity, .. } => Some(identity),
        }
    }
}

/// Semantic kind of a repository target captured at a working-tree or pinned
/// revision boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryTargetKind {
    /// An ordinary file with readable bytes.
    File,
    /// A directory navigation target.
    Directory,
    /// No target exists at the named version.
    Missing,
    /// A present target which is not an ordinary file or directory.
    Unsupported,
}

impl RepositoryTargetKind {
    /// Classify a captured working-tree entry without filesystem I/O.
    pub fn from_entry(entry: &RepositoryEntry) -> Self {
        match entry {
            RepositoryEntry::File { .. } => Self::File,
            RepositoryEntry::Directory { .. } => Self::Directory,
            RepositoryEntry::Absent => Self::Missing,
            RepositoryEntry::Symlink { .. } | RepositoryEntry::Unsupported { .. } => {
                Self::Unsupported
            }
        }
    }

    /// Whether the target is an ordinary file.
    pub fn is_file(self) -> bool {
        matches!(self, Self::File)
    }
}

/// Fingerprint of one complete non-recursive directory listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ListingFingerprint {
    /// Exact identity of the listed directory; `None` represents absence.
    container: Option<EntryIdentity>,
    /// Sorted child name to exact child identity; `None` is not representable.
    children: BTreeMap<String, EntryIdentity>,
    /// SHA-256 of the boundary's canonical listing serialization.
    sha256: String,
}

impl ListingFingerprint {
    /// Construct a deterministic fingerprint.
    pub fn new(children: BTreeMap<String, EntryIdentity>) -> Result<Self, CaptureError> {
        Self::with_container(None, children)
    }

    /// Construct a fingerprint for one present directory and its complete children.
    pub fn for_directory(
        container: EntryIdentity,
        children: BTreeMap<String, EntryIdentity>,
    ) -> Result<Self, CaptureError> {
        Self::with_container(Some(container), children)
    }

    /// Construct a fingerprint for an absent listing root.
    pub fn for_absent() -> Result<Self, CaptureError> {
        Self::with_container(None, BTreeMap::new())
    }

    fn with_container(
        container: Option<EntryIdentity>,
        children: BTreeMap<String, EntryIdentity>,
    ) -> Result<Self, CaptureError> {
        if let Some(identity) = &container {
            identity.validate()?;
        }
        for (name, identity) in &children {
            validate_listing_name(name)?;
            identity.validate()?;
        }
        let mut hasher = Sha256::new();
        match &container {
            Some(identity) => {
                hash_field(&mut hasher, b"directory");
                hash_field(&mut hasher, identity.object.as_bytes());
                hash_field(&mut hasher, identity.sha256.as_bytes());
                hash_field(&mut hasher, &identity.byte_size.to_be_bytes());
            }
            None => hash_field(&mut hasher, b"absent"),
        }
        for (name, identity) in &children {
            hash_field(&mut hasher, name.as_bytes());
            hash_field(&mut hasher, identity.object.as_bytes());
            hash_field(&mut hasher, identity.sha256.as_bytes());
            hash_field(&mut hasher, &identity.byte_size.to_be_bytes());
        }
        Ok(Self {
            container,
            children,
            sha256: format!("{:x}", hasher.finalize()),
        })
    }

    /// Sorted child identities used to derive this fingerprint.
    pub fn children(&self) -> &BTreeMap<String, EntryIdentity> {
        &self.children
    }

    /// Exact listed-directory identity, or `None` when it was absent.
    pub fn container(&self) -> Option<&EntryIdentity> {
        self.container.as_ref()
    }

    /// Deterministic digest of the complete listing.
    pub fn sha256(&self) -> &str {
        &self.sha256
    }

    fn validate(&self) -> Result<(), CaptureError> {
        if let Some(identity) = &self.container {
            identity.validate()?;
        }
        for (name, identity) in &self.children {
            validate_listing_name(name)?;
            identity.validate()?;
        }
        if Self::with_container(self.container.clone(), self.children.clone())?.sha256
            != self.sha256
        {
            return Err(CaptureError::ListingFingerprintMismatch);
        }
        Ok(())
    }
}

/// Origin class for immutable pinned-document evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PinnedSourceClass {
    /// A Git object database entry.
    GitObject,
    /// Git was unavailable at the boundary.
    GitUnavailable,
}

/// Boundary-acquired pinned document or asset evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PinnedDocumentEvidence {
    /// Requested symbolic or object revision.
    requested_revision: String,
    /// Requested repository-relative path.
    requested_path: String,
    /// Evidence source class.
    source: PinnedSourceClass,
    /// Semantic kind of the target at the requested revision.
    target_kind: RepositoryTargetKind,
    /// Canonical commit OID when available.
    commit_oid: Option<String>,
    /// Git object OID when the target is present.
    object_oid: Option<String>,
    /// Exact object/content identity, including SHA-256 and byte size.
    identity: Option<EntryIdentity>,
    /// Exact bytes when present.
    bytes: Option<Vec<u8>>,
    /// Stable absence/unavailable reason.
    unavailable_reason: Option<String>,
}

impl PinnedDocumentEvidence {
    /// Construct and validate boundary evidence for one pinned request.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        requested_revision: impl Into<String>,
        requested_path: impl Into<String>,
        source: PinnedSourceClass,
        commit_oid: Option<String>,
        object_oid: Option<String>,
        identity: Option<EntryIdentity>,
        bytes: Option<Vec<u8>>,
        unavailable_reason: Option<String>,
    ) -> Result<Self, CaptureError> {
        let target_kind = if bytes.is_some() {
            RepositoryTargetKind::File
        } else {
            RepositoryTargetKind::Missing
        };
        Self::new_with_target_kind(
            requested_revision,
            requested_path,
            source,
            target_kind,
            commit_oid,
            object_oid,
            identity,
            bytes,
            unavailable_reason,
        )
    }

    /// Construct and validate boundary evidence with an explicit target kind.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_target_kind(
        requested_revision: impl Into<String>,
        requested_path: impl Into<String>,
        source: PinnedSourceClass,
        target_kind: RepositoryTargetKind,
        commit_oid: Option<String>,
        object_oid: Option<String>,
        identity: Option<EntryIdentity>,
        bytes: Option<Vec<u8>>,
        unavailable_reason: Option<String>,
    ) -> Result<Self, CaptureError> {
        let evidence = Self {
            requested_revision: requested_revision.into(),
            requested_path: requested_path.into(),
            source,
            target_kind,
            commit_oid,
            object_oid,
            identity,
            bytes,
            unavailable_reason,
        };
        evidence.validate_request(&(
            evidence.requested_revision.clone(),
            evidence.requested_path.clone(),
        ))?;
        Ok(evidence)
    }

    /// Whether the requested target is an ordinary file at its named revision.
    ///
    /// Directory and unsupported targets are present but intentionally return
    /// `false`; inspect [`target_kind`](Self::target_kind) when that distinction
    /// matters.
    pub fn exists(&self) -> bool {
        self.target_kind.is_file()
    }

    /// Semantic kind captured for the named target at its requested revision.
    pub fn target_kind(&self) -> RepositoryTargetKind {
        self.target_kind
    }

    /// The exact captured bytes at the requested revision, when present.
    pub fn bytes(&self) -> Option<&[u8]> {
        self.bytes.as_deref()
    }

    /// Canonical commit OID when Git resolved the requested revision.
    pub fn commit_oid(&self) -> Option<&str> {
        self.commit_oid.as_deref()
    }

    /// The stable Git-unavailable/not-found/read-failed reason, when the target
    /// could not be captured as a present typed object.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    fn validate_request(&self, request: &(String, String)) -> Result<(), CaptureError> {
        if (&self.requested_revision, &self.requested_path) != (&request.0, &request.1) {
            return Err(CaptureError::PinnedEvidenceRequestMismatch(request.clone()));
        }
        validate_pinned_request(&self.requested_revision, &self.requested_path)?;
        for oid in [&self.commit_oid, &self.object_oid].into_iter().flatten() {
            if oid.is_empty() || oid.chars().any(char::is_control) {
                return Err(CaptureError::InvalidPinnedEvidence(
                    "object identifiers must be non-empty and control-free".into(),
                ));
            }
        }
        match (
            self.target_kind,
            &self.identity,
            &self.bytes,
            &self.unavailable_reason,
        ) {
            (RepositoryTargetKind::File, Some(identity), Some(bytes), None) => {
                identity.validate_bytes(bytes)?
            }
            (
                RepositoryTargetKind::Directory | RepositoryTargetKind::Unsupported,
                None,
                None,
                None,
            ) => {}
            (RepositoryTargetKind::Missing, None, None, Some(reason))
                if !reason.is_empty() && !reason.chars().any(char::is_control) => {}
            _ => {
                return Err(CaptureError::InvalidPinnedEvidence(
                    "pinned target kind and evidence payload disagree".into(),
                ));
            }
        }
        if self.source == PinnedSourceClass::GitUnavailable
            && (self.commit_oid.is_some()
                || self.object_oid.is_some()
                || self.bytes.is_some()
                || self.target_kind != RepositoryTargetKind::Missing)
        {
            return Err(CaptureError::InvalidPinnedEvidence(
                "git-unavailable evidence cannot contain Git object data".into(),
            ));
        }
        Ok(())
    }
}

/// Source class for linked-worktree fallback evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkedWorktreeSourceClass {
    /// Selected data root.
    LocalData,
    /// Immutable Git HEAD blob.
    GitHead,
    /// Main worktree no-follow file.
    MainWorktree,
    /// No candidate existed.
    Absent,
}

/// Boundary-acquired linked-worktree fallback evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LinkedWorktreeEvidence {
    /// Logical Data path requested.
    path: VirtualPath,
    /// Selected fallback source.
    source: LinkedWorktreeSourceClass,
    /// Exact identity when present.
    identity: Option<EntryIdentity>,
    /// Exact bytes when present.
    bytes: Option<Vec<u8>>,
}

impl LinkedWorktreeEvidence {
    /// Construct and validate one linked-worktree fallback result.
    pub fn new(
        path: VirtualPath,
        source: LinkedWorktreeSourceClass,
        identity: Option<EntryIdentity>,
        bytes: Option<Vec<u8>>,
    ) -> Result<Self, CaptureError> {
        let evidence = Self {
            path,
            source,
            identity,
            bytes,
        };
        evidence.validate_request(&evidence.path)?;
        Ok(evidence)
    }

    fn validate_request(&self, request: &VirtualPath) -> Result<(), CaptureError> {
        if &self.path != request {
            return Err(CaptureError::LinkedEvidenceRequestMismatch(request.clone()));
        }
        if self.path.root_class() != super::RepositoryRootClass::Data {
            return Err(CaptureError::LinkedEvidenceRequiresDataPath(
                self.path.clone(),
            ));
        }
        match (&self.source, &self.identity, &self.bytes) {
            (LinkedWorktreeSourceClass::Absent, None, None) => Ok(()),
            (LinkedWorktreeSourceClass::Absent, _, _) => Err(CaptureError::InvalidLinkedEvidence(
                "absent evidence cannot contain identity or bytes".into(),
            )),
            (_, Some(identity), Some(bytes)) => identity.validate_bytes(bytes),
            _ => Err(CaptureError::InvalidLinkedEvidence(
                "present evidence requires matching identity and bytes".into(),
            )),
        }
    }
}

/// Explicit bounds for phase-two capture closure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CaptureBudget {
    /// Maximum exact paths.
    pub max_paths: usize,
    /// Maximum complete listings.
    pub max_listings: usize,
    /// Maximum combined captured bytes.
    pub max_bytes: u64,
    /// Maximum root-relative depth.
    pub max_depth: usize,
}

/// Normalized two-phase capture request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureSpec {
    fixed: BTreeSet<VirtualPath>,
    discovered: BTreeSet<VirtualPath>,
    listings: BTreeSet<VirtualPath>,
    pinned: BTreeSet<(String, String)>,
    linked_worktree: BTreeSet<VirtualPath>,
    budget: CaptureBudget,
}

impl CaptureSpec {
    /// Begin phase one with fixed declaration paths and explicit bounds.
    pub fn phase_one(
        fixed: impl IntoIterator<Item = VirtualPath>,
        budget: CaptureBudget,
    ) -> Result<Self, CaptureError> {
        let spec = Self {
            fixed: fixed.into_iter().collect(),
            discovered: BTreeSet::new(),
            listings: BTreeSet::new(),
            pinned: BTreeSet::new(),
            linked_worktree: BTreeSet::new(),
            budget,
        };
        spec.validate()?;
        Ok(spec)
    }

    /// Add paths discovered by parsing already captured bytes.
    pub fn discover_paths(
        &mut self,
        paths: impl IntoIterator<Item = VirtualPath>,
    ) -> Result<(), CaptureError> {
        let mut candidate = self.clone();
        candidate.discovered.extend(
            paths
                .into_iter()
                .filter(|path| !candidate.fixed.contains(path)),
        );
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Require a complete non-recursive listing.
    pub fn discover_listing(&mut self, path: VirtualPath) -> Result<(), CaptureError> {
        let mut candidate = self.clone();
        candidate.listings.insert(path);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Require immutable pinned evidence.
    pub fn discover_pinned(
        &mut self,
        revision: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<(), CaptureError> {
        let mut candidate = self.clone();
        candidate.pinned.insert((revision.into(), path.into()));
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Require one linked-worktree fallback result for a logical Data path.
    pub fn discover_linked_worktree(&mut self, path: VirtualPath) -> Result<(), CaptureError> {
        let mut candidate = self.clone();
        candidate.linked_worktree.insert(path);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Every exact path in deterministic work-queue order.
    pub fn paths(&self) -> impl Iterator<Item = &VirtualPath> {
        self.fixed.iter().chain(self.discovered.iter())
    }

    /// Every complete-listing request.
    pub fn listings(&self) -> &BTreeSet<VirtualPath> {
        &self.listings
    }

    /// Every immutable pinned-document request.
    pub fn pinned(&self) -> &BTreeSet<(String, String)> {
        &self.pinned
    }

    /// Every linked-worktree fallback request.
    pub fn linked_worktree(&self) -> &BTreeSet<VirtualPath> {
        &self.linked_worktree
    }

    /// Capture bounds.
    pub fn budget(&self) -> CaptureBudget {
        self.budget
    }

    fn validate(&self) -> Result<(), CaptureError> {
        // Phase one reads only fixed `Data(...)` declaration roots; every Worktree
        // path (documents, projection targets, linked-worktree reads) is enqueued
        // in phase two, never in the fixed set.
        if let Some(path) = self
            .fixed
            .iter()
            .find(|path| path.root_class() != RepositoryRootClass::Data)
        {
            return Err(CaptureError::PhaseOneRequiresDataPath(path.clone()));
        }
        if self.discovered.iter().any(|path| self.fixed.contains(path)) {
            return Err(CaptureError::DuplicateCapturePath);
        }
        let path_count = self.fixed.len()
            + self.discovered.len()
            + self.pinned.len()
            + self.linked_worktree.len();
        if path_count > self.budget.max_paths {
            return Err(CaptureError::PathBudgetExceeded {
                actual: path_count,
                maximum: self.budget.max_paths,
            });
        }
        if self.listings.len() > self.budget.max_listings {
            return Err(CaptureError::ListingBudgetExceeded {
                actual: self.listings.len(),
                maximum: self.budget.max_listings,
            });
        }
        if let Some(path) = self
            .paths()
            .chain(self.listings.iter())
            .chain(self.linked_worktree.iter())
            .find(|path| path.relative().depth() > self.budget.max_depth)
        {
            return Err(CaptureError::DepthBudgetExceeded(path.clone()));
        }
        for path in self
            .paths()
            .chain(self.listings.iter())
            .chain(self.linked_worktree.iter())
        {
            path.ensure_semantic()?;
        }
        for path in &self.linked_worktree {
            if path.root_class() != super::RepositoryRootClass::Data {
                return Err(CaptureError::LinkedEvidenceRequiresDataPath(path.clone()));
            }
        }
        for (revision, path) in &self.pinned {
            validate_pinned_request(revision, path)?;
        }
        Ok(())
    }

    pub(crate) fn contains_path(&self, path: &VirtualPath) -> bool {
        self.fixed.contains(path) || self.discovered.contains(path)
    }
}

/// Complete immutable repository image consumed by pure producers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryImage {
    layout: RepositoryLayout,
    spec: CaptureSpec,
    entries: BTreeMap<VirtualPath, RepositoryEntry>,
    listings: BTreeMap<VirtualPath, ListingFingerprint>,
    pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
    linked_worktree: BTreeMap<VirtualPath, LinkedWorktreeEvidence>,
}

impl RepositoryImage {
    /// Close a capture only when every requested path/listing/evidence is present.
    pub fn close(
        layout: RepositoryLayout,
        spec: CaptureSpec,
        entries: BTreeMap<VirtualPath, RepositoryEntry>,
        listings: BTreeMap<VirtualPath, ListingFingerprint>,
        pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
        linked_worktree: BTreeMap<VirtualPath, LinkedWorktreeEvidence>,
    ) -> Result<Self, CaptureError> {
        spec.validate()?;
        for (path, entry) in &entries {
            layout.ensure_canonical(path)?;
            if !spec.contains_path(path) {
                return Err(CaptureError::UnexpectedCapturedPath(path.clone()));
            }
            validate_entry(path, entry)?;
        }
        for path in spec.paths() {
            layout.ensure_canonical(path)?;
            if !entries.contains_key(path) {
                return Err(CaptureError::IncompleteCapture(path.clone()));
            }
        }
        for (path, listing) in &listings {
            layout.ensure_canonical(path)?;
            if !spec.listings.contains(path) {
                return Err(CaptureError::UnexpectedListing(path.clone()));
            }
            listing.validate()?;
        }
        for path in spec.listings() {
            layout.ensure_canonical(path)?;
            if !listings.contains_key(path) {
                return Err(CaptureError::IncompleteListing(path.clone()));
            }
        }
        let mut listing_only_paths = BTreeSet::new();
        let mut listing_bytes = 0_u64;
        for (parent, listing) in &listings {
            for name in listing.children().keys() {
                listing_bytes =
                    listing_bytes.saturating_add(u64::try_from(name.len()).unwrap_or(u64::MAX));
                let child = listing_child_path(&layout, parent, name)?;
                if child.relative().depth() > spec.budget.max_depth {
                    return Err(CaptureError::DepthBudgetExceeded(child));
                }
                let already_requested = spec.contains_path(&child)
                    || spec.listings.contains(&child)
                    || spec.linked_worktree.contains(&child)
                    || (child.root_class() == RepositoryRootClass::Worktree
                        && spec
                            .pinned
                            .iter()
                            .any(|(_, path)| path == child.relative().as_str()));
                if !already_requested {
                    listing_only_paths.insert(child);
                }
            }
        }
        let path_count = spec
            .paths()
            .count()
            .saturating_add(spec.pinned.len())
            .saturating_add(spec.linked_worktree.len())
            .saturating_add(listing_only_paths.len());
        if path_count > spec.budget.max_paths {
            return Err(CaptureError::PathBudgetExceeded {
                actual: path_count,
                maximum: spec.budget.max_paths,
            });
        }
        for (request, evidence) in &pinned {
            if !spec.pinned.contains(request) {
                return Err(CaptureError::UnexpectedPinnedEvidence(request.clone()));
            }
            evidence.validate_request(request)?;
        }
        for request in &spec.pinned {
            if !pinned.contains_key(request) {
                return Err(CaptureError::IncompletePinnedEvidence(request.clone()));
            }
        }
        for (path, evidence) in &linked_worktree {
            layout.ensure_canonical(path)?;
            if !spec.linked_worktree.contains(path) {
                return Err(CaptureError::UnexpectedLinkedEvidence(path.clone()));
            }
            evidence.validate_request(path)?;
        }
        for path in &spec.linked_worktree {
            if !linked_worktree.contains_key(path) {
                return Err(CaptureError::IncompleteLinkedEvidence(path.clone()));
            }
        }
        let byte_count =
            captured_byte_count(&entries, &pinned, &linked_worktree).saturating_add(listing_bytes);
        if byte_count > spec.budget.max_bytes {
            return Err(CaptureError::ByteBudgetExceeded {
                actual: byte_count,
                maximum: spec.budget.max_bytes,
            });
        }
        Ok(Self {
            layout,
            spec,
            entries,
            listings,
            pinned,
            linked_worktree,
        })
    }

    /// Canonical layout captured by this image.
    pub fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    /// Closed capture declaration used to build this image.
    pub fn capture_spec(&self) -> &CaptureSpec {
        &self.spec
    }

    /// Every exact captured path in canonical order.
    pub fn entries(&self) -> &BTreeMap<VirtualPath, RepositoryEntry> {
        &self.entries
    }

    /// Every complete-listing fingerprint in canonical order.
    pub fn listing_fingerprints(&self) -> &BTreeMap<VirtualPath, ListingFingerprint> {
        &self.listings
    }

    /// Read an exactly captured entry; uncaptured reads are always errors.
    pub fn entry(&self, path: &VirtualPath) -> Result<&RepositoryEntry, CaptureError> {
        self.layout.ensure_canonical(path)?;
        if !self.spec.contains_path(path) {
            return Err(CaptureError::UndiscoveredRepositoryPath(path.clone()));
        }
        self.entries
            .get(path)
            .ok_or_else(|| CaptureError::IncompleteCapture(path.clone()))
    }

    /// Read an ordinary file's exact bytes.
    pub fn file_bytes(&self, path: &VirtualPath) -> Result<Option<&[u8]>, CaptureError> {
        match self.entry(path)? {
            RepositoryEntry::File { bytes, .. } => Ok(Some(bytes)),
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => Ok(None),
            RepositoryEntry::Symlink { .. } | RepositoryEntry::Unsupported { .. } => {
                Err(CaptureError::UnsafeCapturedOccupant(path.clone()))
            }
        }
    }

    /// Every boundary-acquired pinned-document evidence, keyed by
    /// `(requested revision, requested path)`.
    pub fn pinned_evidence(&self) -> &BTreeMap<(String, String), PinnedDocumentEvidence> {
        &self.pinned
    }

    /// Every boundary-acquired linked-worktree fallback evidence, keyed by the
    /// canonical logical `Data(...)` path it resolves.
    pub fn linked_worktree_evidence(&self) -> &BTreeMap<VirtualPath, LinkedWorktreeEvidence> {
        &self.linked_worktree
    }

    /// Whether a later capture preserves every fact already present in `prior`.
    ///
    /// Capture expansion may add paths and evidence, but it must not silently
    /// replace facts that pure derivation has already consumed. A changed root
    /// layout or any changed/removed entry, listing, pinned result, or linked
    /// worktree result requires the caller to restart derivation from the newer
    /// image.
    pub(crate) fn has_stable_overlap(&self, prior: &Self) -> bool {
        self.layout == prior.layout
            && prior
                .entries
                .iter()
                .all(|(path, evidence)| self.entries.get(path) == Some(evidence))
            && prior
                .listings
                .iter()
                .all(|(path, evidence)| self.listings.get(path) == Some(evidence))
            && prior
                .pinned
                .iter()
                .all(|(request, evidence)| self.pinned.get(request) == Some(evidence))
            && prior
                .linked_worktree
                .iter()
                .all(|(path, evidence)| self.linked_worktree.get(path) == Some(evidence))
    }
}

fn validate_entry(path: &VirtualPath, entry: &RepositoryEntry) -> Result<(), CaptureError> {
    match entry {
        RepositoryEntry::Absent => Ok(()),
        RepositoryEntry::File {
            identity, bytes, ..
        } => identity.validate_bytes(bytes),
        RepositoryEntry::Directory { identity, .. } => identity.validate(),
        RepositoryEntry::Symlink {
            identity, target, ..
        } => identity.validate_bytes(target),
        RepositoryEntry::Unsupported {
            identity, reason, ..
        } => {
            identity.validate()?;
            if reason.is_empty() || reason.chars().any(char::is_control) {
                return Err(CaptureError::InvalidCapturedEntry {
                    path: path.clone(),
                    reason: "unsupported occupant reason is empty or contains controls".into(),
                });
            }
            Ok(())
        }
    }
}

fn validate_listing_name(name: &str) -> Result<(), CaptureError> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains(['/', '\\', ':'])
        || name.chars().any(char::is_control)
    {
        return Err(CaptureError::InvalidListingName(name.into()));
    }
    Ok(())
}

fn listing_child_path(
    layout: &RepositoryLayout,
    parent: &VirtualPath,
    name: &str,
) -> Result<VirtualPath, CaptureError> {
    let physical = layout.resolve(parent)?.join(name);
    layout
        .classify_and_canonicalize(physical)
        .map_err(Into::into)
}

fn validate_pinned_request(revision: &str, path: &str) -> Result<(), CaptureError> {
    if revision.is_empty() || revision.chars().any(char::is_control) {
        return Err(CaptureError::InvalidPinnedRequest(
            revision.into(),
            path.into(),
        ));
    }
    let path = super::RootRelativePath::parse(path)?;
    if path.is_root() {
        return Err(CaptureError::InvalidPinnedRequest(
            revision.into(),
            String::new(),
        ));
    }
    Ok(())
}

fn captured_byte_count(
    entries: &BTreeMap<VirtualPath, RepositoryEntry>,
    pinned: &BTreeMap<(String, String), PinnedDocumentEvidence>,
    linked_worktree: &BTreeMap<VirtualPath, LinkedWorktreeEvidence>,
) -> u64 {
    entries
        .values()
        .filter_map(|entry| match entry {
            RepositoryEntry::File { bytes, .. } => Some(bytes.len() as u64),
            RepositoryEntry::Symlink { target, .. } => Some(target.len() as u64),
            RepositoryEntry::Absent
            | RepositoryEntry::Directory { .. }
            | RepositoryEntry::Unsupported { .. } => None,
        })
        .chain(
            pinned
                .values()
                .filter_map(|evidence| evidence.bytes.as_ref().map(|bytes| bytes.len() as u64)),
        )
        .chain(
            linked_worktree
                .values()
                .filter_map(|evidence| evidence.bytes.as_ref().map(|bytes| bytes.len() as u64)),
        )
        .fold(0, u64::saturating_add)
}

/// Typed command/profile input included in a semantic plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositorySeed {
    /// Closed seed source and semantic operation identity.
    kind: RepositorySeedKind,
    /// Sorted typed scalar facts.
    facts: BTreeMap<String, String>,
    /// Sorted exact immutable payloads.
    payloads: BTreeMap<String, Vec<u8>>,
}

impl RepositorySeed {
    /// Construct typed semantic input with non-empty stable names.
    pub fn new(
        kind: RepositorySeedKind,
        facts: BTreeMap<String, String>,
        payloads: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, SeedError> {
        kind.validate()?;
        for key in facts.keys().chain(payloads.keys()) {
            validate_seed_text("key", key)?;
        }
        Ok(Self {
            kind,
            facts,
            payloads,
        })
    }

    /// Semantic source class.
    pub fn kind(&self) -> &RepositorySeedKind {
        &self.kind
    }
}

/// Closed source class for semantic seed data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositorySeedKind {
    /// An ordinary command mutation.
    Command {
        /// Stable command semantic name.
        name: String,
    },
    /// A fresh repository initialization.
    Initialization,
    /// An embedded profile operation.
    Profile {
        /// Profile identity.
        name: String,
        /// Profile declaration version.
        version: String,
        /// Exact package identity.
        package_hash: String,
    },
}

impl RepositorySeedKind {
    fn validate(&self) -> Result<(), SeedError> {
        match self {
            Self::Command { name } => validate_seed_text("command name", name),
            Self::Initialization => Ok(()),
            Self::Profile {
                name,
                version,
                package_hash,
            } => {
                validate_seed_text("profile name", name)?;
                validate_seed_text("profile version", version)?;
                validate_seed_text("profile package hash", package_hash)
            }
        }
    }
}

fn validate_seed_text(field: &'static str, value: &str) -> Result<(), SeedError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        Err(SeedError::InvalidText { field })
    } else {
        Ok(())
    }
}

/// Constrained complete materialization operation.
///
/// The render variant carries a `selected` name set that scopes WHICH declared
/// projections are in scope for this operation, not WHICH producer families run.
/// This is declaration scope, not an à-la-carte producer switch: each selected
/// projection's output still derives completely from its own declared sources, so
/// a `jit project render <name>` renders exactly that projection's target from its
/// own closure and leaves sibling projections' targets untouched. Semantic-mutation
/// and repair intents remain complete over every declared projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterializationIntent {
    /// Full derived-state rebuild after a semantic mutation.
    SemanticMutation,
    /// Render the configured projections in scope. `None` selects every declared
    /// projection; `Some(set)` scopes the operation to exactly the named
    /// projections (each rendered completely from its own declared sources).
    RenderConfiguredProjections {
        /// The declaration-scoped projection names, or `None` for all.
        selected: Option<std::collections::BTreeSet<String>>,
    },
    /// Repair every explicitly owned derived-state drift finding.
    RepairDerivedState,
    /// Publish a fresh (or missing-file) repository scaffold, optionally with an
    /// embedded profile, as one complete delta.
    InitializeRepository,
    /// Apply an embedded profile to an existing repository as one complete delta.
    ApplyProfile,
    /// Publish one explicitly selected repository-contained export file.
    /// Finalized by
    /// [`finalize_repository_export`](crate::repository_state::finalize_repository_export),
    /// not the declaration-derived producer graph.
    RepositoryExport,
}

/// Stable ownership identity for a target claim.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct TargetClaim {
    /// Canonical target.
    target: VirtualPath,
    /// Stable producer/owner identity.
    owner: String,
}

impl TargetClaim {
    /// Construct one canonical, non-anonymous ownership claim.
    pub fn new(
        layout: &RepositoryLayout,
        target: VirtualPath,
        owner: impl Into<String>,
    ) -> Result<Self, DeltaError> {
        layout.ensure_canonical(&target)?;
        let owner = owner.into();
        validate_owner(&owner)?;
        Ok(Self { target, owner })
    }

    /// Canonical target path.
    pub fn target(&self) -> &VirtualPath {
        &self.target
    }

    /// Stable producer owner.
    pub fn owner(&self) -> &str {
        &self.owner
    }
}

/// Exact expected preimage for a delta action.
///
/// Mirrors the captured [`RepositoryEntry`] kinds so a preimage distinguishes an
/// absent path, an ordinary file (identity + mode), a directory, a symlink
/// (identity + payload), and an occupant unsafe for mutation — the kinds the plan
/// delta vocabulary enumerates. A later publication boundary verifies this
/// preimage against the live occupant before applying the action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpectedPreimage {
    /// Path must remain absent.
    Absent,
    /// An ordinary file with exact identity and mode.
    File {
        /// Exact entry identity (object, content hash, byte size).
        identity: EntryIdentity,
        /// Normalized file mode.
        mode: FileMode,
    },
    /// A directory occupant.
    Directory {
        /// Exact directory identity.
        identity: EntryIdentity,
        /// Normalized directory mode.
        mode: FileMode,
    },
    /// A symbolic link with exact payload, never followed.
    Symlink {
        /// Exact link identity.
        identity: EntryIdentity,
        /// Exact link payload.
        target: Vec<u8>,
        /// Normalized link mode where the platform exposes one.
        mode: FileMode,
    },
    /// An occupant unsafe for semantic mutation.
    Unsupported {
        /// Exact occupant identity.
        identity: EntryIdentity,
        /// Stable diagnostic.
        reason: String,
        /// Normalized occupant mode where the platform exposes one.
        mode: FileMode,
    },
}

impl ExpectedPreimage {
    /// The expected preimage matching a captured occupant, kind for kind.
    pub fn of(entry: &RepositoryEntry) -> Self {
        match entry {
            RepositoryEntry::Absent => Self::Absent,
            RepositoryEntry::File { identity, mode, .. } => Self::File {
                identity: identity.clone(),
                mode: *mode,
            },
            RepositoryEntry::Directory { identity, mode } => Self::Directory {
                identity: identity.clone(),
                mode: *mode,
            },
            RepositoryEntry::Symlink {
                identity,
                target,
                mode,
            } => Self::Symlink {
                identity: identity.clone(),
                target: target.clone(),
                mode: *mode,
            },
            RepositoryEntry::Unsupported {
                identity,
                reason,
                mode,
            } => Self::Unsupported {
                identity: identity.clone(),
                reason: reason.clone(),
                mode: *mode,
            },
        }
    }

    /// Exact identity when the preimage expects an occupant (`None` for absent).
    pub fn identity(&self) -> Option<&EntryIdentity> {
        match self {
            Self::Absent => None,
            Self::File { identity, .. }
            | Self::Directory { identity, .. }
            | Self::Symlink { identity, .. }
            | Self::Unsupported { identity, .. } => Some(identity),
        }
    }
}

/// One exact repository-state action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RepositoryAction {
    /// Create an absent directory.
    CreateDirectory {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
    },
    /// Create or replace an ordinary file.
    WriteFile {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
        bytes: Vec<u8>,
        mode: FileMode,
    },
    /// Change only an ordinary file's mode.
    SetMode {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
        mode: FileMode,
    },
    /// Delete one ordinary file.
    DeleteFile {
        path: VirtualPath,
        owner: String,
        expected: ExpectedPreimage,
    },
}

impl RepositoryAction {
    #[cfg(test)]
    pub(crate) fn create_directory(
        path: VirtualPath,
        owner: impl Into<String>,
        expected: ExpectedPreimage,
    ) -> Self {
        Self::CreateDirectory {
            path,
            owner: owner.into(),
            expected,
        }
    }

    #[cfg(test)]
    pub(crate) fn write_file(
        path: VirtualPath,
        owner: impl Into<String>,
        expected: ExpectedPreimage,
        bytes: Vec<u8>,
        mode: FileMode,
    ) -> Self {
        Self::WriteFile {
            path,
            owner: owner.into(),
            expected,
            bytes,
            mode,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_mode(
        path: VirtualPath,
        owner: impl Into<String>,
        expected: ExpectedPreimage,
        mode: FileMode,
    ) -> Self {
        Self::SetMode {
            path,
            owner: owner.into(),
            expected,
            mode,
        }
    }

    #[cfg(test)]
    pub(crate) fn delete_file(
        path: VirtualPath,
        owner: impl Into<String>,
        expected: ExpectedPreimage,
    ) -> Self {
        Self::DeleteFile {
            path,
            owner: owner.into(),
            expected,
        }
    }

    /// Canonical target path.
    pub fn path(&self) -> &VirtualPath {
        match self {
            Self::CreateDirectory { path, .. }
            | Self::WriteFile { path, .. }
            | Self::SetMode { path, .. }
            | Self::DeleteFile { path, .. } => path,
        }
    }

    /// Stable producer or ownership identity.
    pub fn owner(&self) -> &str {
        match self {
            Self::CreateDirectory { owner, .. }
            | Self::WriteFile { owner, .. }
            | Self::SetMode { owner, .. }
            | Self::DeleteFile { owner, .. } => owner,
        }
    }

    /// Exact preimage required immediately before publication.
    pub fn expected(&self) -> &ExpectedPreimage {
        match self {
            Self::CreateDirectory { expected, .. }
            | Self::WriteFile { expected, .. }
            | Self::SetMode { expected, .. }
            | Self::DeleteFile { expected, .. } => expected,
        }
    }
}

/// Sorted, duplicate-free exact repository delta.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RepositoryDelta {
    actions: Vec<RepositoryAction>,
}

impl RepositoryDelta {
    /// Normalize actions and reject duplicate canonical targets.
    pub fn new(
        layout: &RepositoryLayout,
        mut actions: Vec<RepositoryAction>,
    ) -> Result<Self, DeltaError> {
        for action in &actions {
            layout.ensure_canonical(action.path())?;
            validate_action(action)?;
        }
        actions.sort_by(|left, right| left.path().cmp(right.path()));
        if let Some(pair) = actions
            .windows(2)
            .find(|pair| pair[0].path() == pair[1].path())
        {
            return Err(DeltaError::DuplicateTarget(pair[0].path().clone()));
        }
        Self::reject_physical_aliases(&actions)?;
        Ok(Self { actions })
    }

    /// Reject any two actions whose captured preimages carry one physical identity
    /// at distinct canonical paths — a hard-link alias. Mutating either target
    /// would silently mutate the other, breaking virtual-to-physical injectivity;
    /// canonical-path uniqueness alone does not catch it, since two distinct paths
    /// (across roots, or within one root) can name one inode.
    ///
    /// Pure: it compares only identities already captured at the boundary (the
    /// no-follow object identity — `dev:ino` on unix), never touching the
    /// filesystem. On unix, equal captured identities mean one inode, so a
    /// distinct file with identical content — a distinct inode, distinct object —
    /// is correctly not flagged. An `Absent` preimage carries no identity and thus
    /// cannot collide: two not-yet-created targets cannot share physical storage.
    /// A hard link introduced only after capture is a distinct, second-layer
    /// concern: it changes a captured identity and is rejected by the held-session
    /// pre-journal revalidation as a retryable conflict.
    fn reject_physical_aliases(actions: &[RepositoryAction]) -> Result<(), DeltaError> {
        let mut by_identity: BTreeMap<&EntryIdentity, &VirtualPath> = BTreeMap::new();
        for action in actions {
            if let Some(identity) = action.expected().identity() {
                if let Some(previous) = by_identity.insert(identity, action.path()) {
                    return Err(DeltaError::PhysicalAlias {
                        first: previous.clone(),
                        second: action.path().clone(),
                    });
                }
            }
        }
        Ok(())
    }

    /// Exact normalized actions.
    pub fn actions(&self) -> &[RepositoryAction] {
        &self.actions
    }
}

fn validate_action(action: &RepositoryAction) -> Result<(), DeltaError> {
    let (owner, expected) = match action {
        RepositoryAction::CreateDirectory {
            owner, expected, ..
        }
        | RepositoryAction::WriteFile {
            owner, expected, ..
        }
        | RepositoryAction::SetMode {
            owner, expected, ..
        }
        | RepositoryAction::DeleteFile {
            owner, expected, ..
        } => (owner, expected),
    };
    validate_owner(owner)?;
    if let Some(identity) = expected.identity() {
        identity
            .validate()
            .map_err(|error| DeltaError::InvalidAction(error.to_string()))?;
    }
    match action {
        RepositoryAction::CreateDirectory {
            expected: ExpectedPreimage::Absent,
            ..
        }
        | RepositoryAction::WriteFile {
            expected: ExpectedPreimage::Absent | ExpectedPreimage::File { .. },
            ..
        }
        | RepositoryAction::SetMode {
            expected: ExpectedPreimage::File { .. },
            ..
        }
        | RepositoryAction::DeleteFile {
            expected: ExpectedPreimage::File { .. },
            ..
        } => Ok(()),
        RepositoryAction::CreateDirectory { .. } => Err(DeltaError::InvalidAction(
            "directory creation requires an absent preimage".into(),
        )),
        RepositoryAction::WriteFile { .. } => Err(DeltaError::InvalidAction(
            "file write requires an absent or file preimage".into(),
        )),
        RepositoryAction::SetMode { .. } => Err(DeltaError::InvalidAction(
            "mode changes require a file preimage".into(),
        )),
        RepositoryAction::DeleteFile { .. } => Err(DeltaError::InvalidAction(
            "file deletion requires a file preimage".into(),
        )),
    }
}

fn validate_owner(owner: &str) -> Result<(), DeltaError> {
    if owner.is_empty() || owner.chars().any(char::is_control) {
        Err(DeltaError::InvalidOwner)
    } else {
        Ok(())
    }
}

/// Deterministic plan hash over layout, capture, evidence, seed, intent, and delta.
pub fn plan_hash(
    image: &RepositoryImage,
    seed: &RepositorySeed,
    intent: &MaterializationIntent,
    delta: &RepositoryDelta,
) -> Result<String, PlanHashError> {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"jit-repository-plan-v1");
    hash_serialized(&mut hasher, &image.layout)?;
    hash_serialized(&mut hasher, &image.spec)?;
    hash_serialized(&mut hasher, &image.entries.iter().collect::<Vec<_>>())?;
    hash_serialized(&mut hasher, &image.listings.iter().collect::<Vec<_>>())?;
    hash_serialized(&mut hasher, &image.pinned.iter().collect::<Vec<_>>())?;
    hash_serialized(
        &mut hasher,
        &image.linked_worktree.iter().collect::<Vec<_>>(),
    )?;
    hash_serialized(&mut hasher, seed)?;
    hash_serialized(&mut hasher, intent)?;
    hash_serialized(&mut hasher, delta)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
}

fn hash_serialized(hasher: &mut Sha256, value: &impl Serialize) -> Result<(), PlanHashError> {
    let mut counter = ByteCounter::default();
    serde_json::to_writer(&mut counter, value)?;
    hasher.update(counter.0.to_be_bytes());
    serde_json::to_writer(HashWriter(hasher), value)?;
    Ok(())
}

#[derive(Default)]
struct ByteCounter(u64);

impl Write for ByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct HashWriter<'a>(&'a mut Sha256);

impl Write for HashWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Capture closure failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error("capture requested {actual} paths but the maximum is {maximum}")]
    PathBudgetExceeded { actual: usize, maximum: usize },
    #[error("capture requested {actual} listings but the maximum is {maximum}")]
    ListingBudgetExceeded { actual: usize, maximum: usize },
    #[error("captured {actual} bytes but the maximum is {maximum}")]
    ByteBudgetExceeded { actual: u64, maximum: u64 },
    #[error("capture path {0:?} exceeds the depth budget")]
    DepthBudgetExceeded(VirtualPath),
    #[error("capture spec contains the same path in fixed and discovered sets")]
    DuplicateCapturePath,
    #[error("phase-one fixed capture requires a Data path, got {0:?}")]
    PhaseOneRequiresDataPath(VirtualPath),
    #[error("capture did not provide requested path {0:?}")]
    IncompleteCapture(VirtualPath),
    #[error("capture provided unrequested path {0:?}")]
    UnexpectedCapturedPath(VirtualPath),
    #[error("capture did not provide requested listing {0:?}")]
    IncompleteListing(VirtualPath),
    #[error("capture provided unrequested listing {0:?}")]
    UnexpectedListing(VirtualPath),
    #[error("capture did not provide pinned evidence {0:?}")]
    IncompletePinnedEvidence((String, String)),
    #[error("capture provided unrequested pinned evidence {0:?}")]
    UnexpectedPinnedEvidence((String, String)),
    #[error("pinned evidence does not match request {0:?}")]
    PinnedEvidenceRequestMismatch((String, String)),
    #[error("pinned request revision '{0}' or path '{1}' is invalid")]
    InvalidPinnedRequest(String, String),
    #[error("pinned evidence is invalid: {0}")]
    InvalidPinnedEvidence(String),
    #[error("capture did not provide linked-worktree evidence {0:?}")]
    IncompleteLinkedEvidence(VirtualPath),
    #[error("capture provided unrequested linked-worktree evidence {0:?}")]
    UnexpectedLinkedEvidence(VirtualPath),
    #[error("linked-worktree evidence does not match request {0:?}")]
    LinkedEvidenceRequestMismatch(VirtualPath),
    #[error("linked-worktree evidence requires a Data path, got {0:?}")]
    LinkedEvidenceRequiresDataPath(VirtualPath),
    #[error("linked-worktree evidence is invalid: {0}")]
    InvalidLinkedEvidence(String),
    #[error("entry identity is invalid: {0}")]
    InvalidEntryIdentity(String),
    #[error("entry identity for object '{object}' does not match captured bytes")]
    EntryIdentityMismatch { object: String },
    #[error("captured entry {path:?} is invalid: {reason}")]
    InvalidCapturedEntry { path: VirtualPath, reason: String },
    #[error("directory listing child name '{0}' is invalid")]
    InvalidListingName(String),
    #[error("directory listing fingerprint does not match its children")]
    ListingFingerprintMismatch,
    #[error("pure producer requested undiscovered path {0:?}")]
    UndiscoveredRepositoryPath(VirtualPath),
    #[error("captured path {0:?} has a symlink or unsupported occupant")]
    UnsafeCapturedOccupant(VirtualPath),
}

/// Invalid exact delta.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeltaError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error("delta contains duplicate canonical target {0:?}")]
    DuplicateTarget(VirtualPath),
    #[error(
        "delta actions {first:?} and {second:?} resolve to one physical identity (a hard-link alias)"
    )]
    PhysicalAlias {
        first: VirtualPath,
        second: VirtualPath,
    },
    #[error("delta action owner is empty or contains control characters")]
    InvalidOwner,
    #[error("delta action is invalid: {0}")]
    InvalidAction(String),
}

/// Invalid typed semantic seed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SeedError {
    /// A stable semantic identifier was empty or contained controls.
    #[error("repository seed {field} is empty or contains control characters")]
    InvalidText { field: &'static str },
}

/// Failure to serialize the closed typed plan vocabulary.
#[derive(Debug, thiserror::Error)]
#[error("failed to serialize repository plan identity: {0}")]
pub struct PlanHashError(#[from] serde_json::Error);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository_state::RepositoryRootEvidence;

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "wt", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn image(seed_byte: u8, listing_byte: u8) -> RepositoryImage {
        let path = VirtualPath::data("index.json").unwrap();
        let dir = VirtualPath::data("issues").unwrap();
        let spec = {
            let mut spec = CaptureSpec::phase_one(
                [path.clone()],
                CaptureBudget {
                    max_paths: 8,
                    max_listings: 2,
                    max_bytes: 64,
                    max_depth: 4,
                },
            )
            .unwrap();
            spec.discover_listing(dir.clone()).unwrap();
            spec
        };
        let bytes = vec![seed_byte];
        RepositoryImage::close(
            layout(),
            spec,
            BTreeMap::from([(
                path,
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes("index", &bytes).unwrap(),
                    bytes,
                    mode: FileMode::Regular,
                },
            )]),
            BTreeMap::from([(
                dir,
                ListingFingerprint::new(BTreeMap::from([(
                    "one.json".into(),
                    EntryIdentity::for_bytes("issue", &[listing_byte]).unwrap(),
                )]))
                .unwrap(),
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap()
    }

    #[test]
    fn test_capture_rejects_undiscovered_repository_read() {
        let mut captured = image(1, 2);
        let unrequested = VirtualPath::data("config.toml").unwrap();
        captured
            .entries
            .insert(unrequested.clone(), RepositoryEntry::Absent);
        assert!(matches!(
            captured.entry(&unrequested),
            Err(CaptureError::UndiscoveredRepositoryPath(_))
        ));
    }

    #[test]
    fn test_capture_overlap_rejects_changed_prior_evidence() {
        let baseline = image(1, 2);
        assert!(baseline.has_stable_overlap(&baseline));
        assert!(!image(2, 2).has_stable_overlap(&baseline));
        assert!(!image(1, 3).has_stable_overlap(&baseline));

        let request = ("HEAD".to_owned(), "README.md".to_owned());
        let mut with_external = baseline.clone();
        with_external
            .spec
            .discover_pinned(request.0.clone(), request.1.clone())
            .unwrap();
        with_external.pinned.insert(
            request.clone(),
            PinnedDocumentEvidence::new(
                &request.0,
                &request.1,
                PinnedSourceClass::GitUnavailable,
                None,
                None,
                None,
                None,
                Some("first failure".into()),
            )
            .unwrap(),
        );
        let linked = VirtualPath::data("linked.toml").unwrap();
        with_external
            .spec
            .discover_linked_worktree(linked.clone())
            .unwrap();
        with_external.linked_worktree.insert(
            linked.clone(),
            LinkedWorktreeEvidence::new(
                linked.clone(),
                LinkedWorktreeSourceClass::Absent,
                None,
                None,
            )
            .unwrap(),
        );

        let mut changed_pinned = with_external.clone();
        changed_pinned.pinned.insert(
            request.clone(),
            PinnedDocumentEvidence::new(
                &request.0,
                &request.1,
                PinnedSourceClass::GitUnavailable,
                None,
                None,
                None,
                None,
                Some("second failure".into()),
            )
            .unwrap(),
        );
        assert!(!changed_pinned.has_stable_overlap(&with_external));

        let mut changed_linked = with_external.clone();
        let bytes = b"changed".to_vec();
        changed_linked.linked_worktree.insert(
            linked.clone(),
            LinkedWorktreeEvidence::new(
                linked,
                LinkedWorktreeSourceClass::MainWorktree,
                Some(EntryIdentity::for_bytes("linked", &bytes).unwrap()),
                Some(bytes),
            )
            .unwrap(),
        );
        assert!(!changed_linked.has_stable_overlap(&with_external));
    }

    #[test]
    fn test_close_rejects_every_unrequested_evidence_map() {
        let requested = VirtualPath::data("index.json").unwrap();
        let extra = VirtualPath::data("extra.json").unwrap();
        let spec = CaptureSpec::phase_one(
            [requested.clone()],
            CaptureBudget {
                max_paths: 8,
                max_listings: 2,
                max_bytes: 64,
                max_depth: 4,
            },
        )
        .unwrap();
        let entries = BTreeMap::from([(requested, RepositoryEntry::Absent)]);

        let mut extra_entries = entries.clone();
        extra_entries.insert(extra.clone(), RepositoryEntry::Absent);
        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec.clone(),
                extra_entries,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            Err(CaptureError::UnexpectedCapturedPath(path)) if path == extra
        ));

        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec.clone(),
                entries.clone(),
                BTreeMap::from([(extra.clone(), ListingFingerprint::new(BTreeMap::new()).unwrap())]),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            Err(CaptureError::UnexpectedListing(path)) if path == extra
        ));

        let pinned_request = ("HEAD".to_owned(), "README.md".to_owned());
        let pinned = PinnedDocumentEvidence::new(
            &pinned_request.0,
            &pinned_request.1,
            PinnedSourceClass::GitUnavailable,
            None,
            None,
            None,
            None,
            Some("git unavailable".into()),
        )
        .unwrap();
        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec.clone(),
                entries.clone(),
                BTreeMap::new(),
                BTreeMap::from([(pinned_request.clone(), pinned)]),
                BTreeMap::new(),
            ),
            Err(CaptureError::UnexpectedPinnedEvidence(request)) if request == pinned_request
        ));

        let linked = LinkedWorktreeEvidence::new(
            extra.clone(),
            LinkedWorktreeSourceClass::Absent,
            None,
            None,
        )
        .unwrap();
        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec,
                entries,
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::from([(extra.clone(), linked)]),
            ),
            Err(CaptureError::UnexpectedLinkedEvidence(path)) if path == extra
        ));
    }

    #[test]
    fn test_close_requires_requested_linked_evidence() {
        let requested = VirtualPath::data("index.json").unwrap();
        let linked_path = VirtualPath::data("config.toml").unwrap();
        let mut spec = CaptureSpec::phase_one(
            [requested.clone()],
            CaptureBudget {
                max_paths: 4,
                max_listings: 0,
                max_bytes: 64,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_linked_worktree(linked_path.clone()).unwrap();
        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec,
                BTreeMap::from([(requested, RepositoryEntry::Absent)]),
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            Err(CaptureError::IncompleteLinkedEvidence(path)) if path == linked_path
        ));
    }

    #[test]
    fn test_close_revalidates_entry_identity_and_listing_fingerprint() {
        let path = VirtualPath::data("index.json").unwrap();
        let spec = CaptureSpec::phase_one(
            [path.clone()],
            CaptureBudget {
                max_paths: 2,
                max_listings: 1,
                max_bytes: 64,
                max_depth: 4,
            },
        )
        .unwrap();
        let bytes = b"captured".to_vec();
        let mut identity = EntryIdentity::for_bytes("index", &bytes).unwrap();
        identity.byte_size += 1;
        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec,
                BTreeMap::from([(
                    path,
                    RepositoryEntry::File {
                        identity,
                        bytes,
                        mode: FileMode::Regular,
                    },
                )]),
                BTreeMap::new(),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            Err(CaptureError::EntryIdentityMismatch { .. })
        ));

        let directory = VirtualPath::data("issues").unwrap();
        let mut listing = ListingFingerprint::new(BTreeMap::new()).unwrap();
        listing.sha256 = "0".repeat(64);
        let mut listing_spec = CaptureSpec::phase_one(
            [],
            CaptureBudget {
                max_paths: 1,
                max_listings: 1,
                max_bytes: 0,
                max_depth: 4,
            },
        )
        .unwrap();
        listing_spec.discover_listing(directory.clone()).unwrap();
        assert!(matches!(
            RepositoryImage::close(
                layout(),
                listing_spec,
                BTreeMap::new(),
                BTreeMap::from([(directory, listing)]),
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            Err(CaptureError::ListingFingerprintMismatch)
        ));
    }

    fn close_listing(
        path: &str,
        children: &[&str],
        exact: &[&str],
        budget: CaptureBudget,
    ) -> Result<RepositoryImage, CaptureError> {
        let path = VirtualPath::data(path)?;
        let identities = children
            .iter()
            .map(|name| Ok(((*name).to_owned(), EntryIdentity::for_bytes(*name, b"")?)))
            .collect::<Result<BTreeMap<_, _>, CaptureError>>()?;
        let entries = exact
            .iter()
            .map(|name| {
                let identity = identities.get(*name).unwrap().clone();
                Ok((
                    VirtualPath::data(format!("{path}/{name}", path = path.relative().as_str()))?,
                    RepositoryEntry::File {
                        identity,
                        bytes: Vec::new(),
                        mode: FileMode::Regular,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, CaptureError>>()?;
        let mut spec = CaptureSpec::phase_one(entries.keys().cloned(), budget)?;
        spec.discover_listing(path.clone())?;
        RepositoryImage::close(
            layout(),
            spec,
            entries,
            BTreeMap::from([(path, ListingFingerprint::new(identities)?)]),
            BTreeMap::new(),
            BTreeMap::new(),
        )
    }

    fn listing_budget(max_paths: usize, max_bytes: u64, max_depth: usize) -> CaptureBudget {
        CaptureBudget {
            max_paths,
            max_listings: 1,
            max_bytes,
            max_depth,
        }
    }

    #[test]
    fn test_close_charges_listing_only_children_to_path_budget() {
        let result = close_listing(
            "issues",
            &["one.json", "two.json"],
            &[],
            listing_budget(1, 64, 2),
        );

        assert!(matches!(
            result,
            Err(CaptureError::PathBudgetExceeded {
                actual: 2,
                maximum: 1
            })
        ));
    }

    #[test]
    fn test_close_counts_exact_listed_child_once() {
        let result = close_listing(
            "issues",
            &["one.json"],
            &["one.json"],
            listing_budget(1, 64, 2),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_close_does_not_charge_listed_child_that_is_a_listing_root() {
        let root = VirtualPath::data("").unwrap();
        let issues = VirtualPath::data("issues").unwrap();
        let identity = EntryIdentity::for_bytes("issues", b"directory").unwrap();
        let mut spec = CaptureSpec::phase_one(
            [],
            CaptureBudget {
                max_paths: 0,
                max_listings: 2,
                max_bytes: 6,
                max_depth: 1,
            },
        )
        .unwrap();
        spec.discover_listing(root.clone()).unwrap();
        spec.discover_listing(issues.clone()).unwrap();

        let result = RepositoryImage::close(
            layout(),
            spec,
            BTreeMap::new(),
            BTreeMap::from([
                (
                    root,
                    ListingFingerprint::new(BTreeMap::from([("issues".into(), identity.clone())]))
                        .unwrap(),
                ),
                (
                    issues,
                    ListingFingerprint::for_directory(identity, BTreeMap::new()).unwrap(),
                ),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_close_charges_multibyte_listing_names_to_byte_budget() {
        let result = close_listing("issues", &["é"], &[], listing_budget(1, 1, 2));

        assert!(matches!(
            result,
            Err(CaptureError::ByteBudgetExceeded {
                actual: 2,
                maximum: 1
            })
        ));
    }

    #[test]
    fn test_close_enforces_depth_for_listed_children() {
        let result = close_listing("issues", &["one.json"], &[], listing_budget(1, 64, 1));

        assert!(matches!(
            result,
            Err(CaptureError::DepthBudgetExceeded(path))
                if path == VirtualPath::data("issues/one.json").unwrap()
        ));
    }

    #[test]
    fn test_close_empty_listing_costs_no_path_or_bytes() {
        let result = close_listing("issues", &[], &[], listing_budget(0, 0, 1));

        assert!(result.is_ok());
    }

    #[test]
    fn test_capture_spec_failed_discovery_is_transactional() {
        let fixed = VirtualPath::data("index.json").unwrap();
        let mut spec = CaptureSpec::phase_one(
            [fixed.clone()],
            CaptureBudget {
                max_paths: 1,
                max_listings: 0,
                max_bytes: 0,
                max_depth: 2,
            },
        )
        .unwrap();
        assert!(matches!(
            spec.discover_paths([VirtualPath::data("config.toml").unwrap()]),
            Err(CaptureError::PathBudgetExceeded { .. })
        ));
        assert_eq!(spec.paths().collect::<Vec<_>>(), vec![&fixed]);
    }

    #[test]
    fn test_phase_one_requires_data_fixed_paths() {
        let budget = CaptureBudget {
            max_paths: 8,
            max_listings: 2,
            max_bytes: 64,
            max_depth: 4,
        };
        // A Worktree path in the phase-one fixed set is rejected: only Data
        // declaration roots are read in phase one; Worktree paths enter phase two.
        assert!(matches!(
            CaptureSpec::phase_one([VirtualPath::worktree("docs/plan.md").unwrap()], budget),
            Err(CaptureError::PhaseOneRequiresDataPath(_))
        ));
        // A Data fixed path is accepted.
        assert!(
            CaptureSpec::phase_one([VirtualPath::data("config.toml").unwrap()], budget).is_ok()
        );
    }

    #[test]
    fn test_expected_preimage_distinguishes_kinds_and_gates_actions() {
        let identity = EntryIdentity::for_bytes("occupant", b"bytes").unwrap();
        // The preimage mirrors the captured entry kind for kind.
        assert_eq!(
            ExpectedPreimage::of(&RepositoryEntry::Directory {
                identity: identity.clone(),
                mode: FileMode::Executable,
            }),
            ExpectedPreimage::Directory {
                identity: identity.clone(),
                mode: FileMode::Executable,
            }
        );
        assert!(ExpectedPreimage::Absent.identity().is_none());
        assert!(ExpectedPreimage::Directory {
            identity: identity.clone(),
            mode: FileMode::Executable,
        }
        .identity()
        .is_some());

        // SetMode and DeleteFile require a File preimage; a Directory preimage is
        // rejected, so the delta vocabulary distinguishes the kinds.
        let target = VirtualPath::data("occupant").unwrap();
        let directory = ExpectedPreimage::Directory {
            identity: identity.clone(),
            mode: FileMode::Executable,
        };
        assert!(matches!(
            RepositoryDelta::new(
                &layout(),
                vec![RepositoryAction::DeleteFile {
                    path: target.clone(),
                    owner: "delete".into(),
                    expected: directory.clone(),
                }],
            ),
            Err(DeltaError::InvalidAction(_))
        ));
        assert!(matches!(
            RepositoryDelta::new(
                &layout(),
                vec![RepositoryAction::SetMode {
                    path: target.clone(),
                    owner: "mode".into(),
                    expected: directory,
                    mode: FileMode::Executable,
                }],
            ),
            Err(DeltaError::InvalidAction(_))
        ));
        // A File preimage is accepted.
        assert!(RepositoryDelta::new(
            &layout(),
            vec![RepositoryAction::DeleteFile {
                path: target,
                owner: "delete".into(),
                expected: ExpectedPreimage::File {
                    identity,
                    mode: FileMode::Regular,
                },
            }],
        )
        .is_ok());
    }

    #[test]
    fn test_delta_rejects_physical_identity_aliases() {
        let shared = EntryIdentity::for_bytes("42:100", b"linked").unwrap();
        let file = |identity: &EntryIdentity| ExpectedPreimage::File {
            identity: identity.clone(),
            mode: FileMode::Regular,
        };
        let set_mode = |path: VirtualPath, owner: &str, expected: ExpectedPreimage| {
            RepositoryAction::SetMode {
                path,
                owner: owner.into(),
                expected,
                mode: FileMode::Executable,
            }
        };

        // A file hard-linked into both roots captures ONE physical identity at two
        // canonical paths; the cross-root pair is rejected.
        assert!(matches!(
            RepositoryDelta::new(
                &layout(),
                vec![
                    set_mode(
                        VirtualPath::worktree("shared").unwrap(),
                        "one",
                        file(&shared)
                    ),
                    set_mode(VirtualPath::data("shared").unwrap(), "two", file(&shared)),
                ],
            ),
            Err(DeltaError::PhysicalAlias { .. })
        ));

        // Two distinct canonical paths in ONE root can also name one inode; the
        // same-root case is reachable (path uniqueness does not preclude a hard
        // link) and is equally rejected.
        assert!(matches!(
            RepositoryDelta::new(
                &layout(),
                vec![
                    set_mode(VirtualPath::data("a").unwrap(), "one", file(&shared)),
                    set_mode(VirtualPath::data("b").unwrap(), "two", file(&shared)),
                ],
            ),
            Err(DeltaError::PhysicalAlias { .. })
        ));

        // Distinct physical identities (distinct inodes) pass even across roots.
        let other = EntryIdentity::for_bytes("42:200", b"distinct").unwrap();
        assert!(RepositoryDelta::new(
            &layout(),
            vec![
                set_mode(
                    VirtualPath::worktree("shared").unwrap(),
                    "one",
                    file(&shared)
                ),
                set_mode(VirtualPath::data("shared").unwrap(), "two", file(&other)),
            ],
        )
        .is_ok());

        // Absent preimages carry no physical identity and never collide.
        assert!(RepositoryDelta::new(
            &layout(),
            vec![
                RepositoryAction::WriteFile {
                    path: VirtualPath::worktree("fresh-a").unwrap(),
                    owner: "one".into(),
                    expected: ExpectedPreimage::Absent,
                    bytes: b"a".to_vec(),
                    mode: FileMode::Regular,
                },
                RepositoryAction::WriteFile {
                    path: VirtualPath::data("fresh-b").unwrap(),
                    owner: "two".into(),
                    expected: ExpectedPreimage::Absent,
                    bytes: b"b".to_vec(),
                    mode: FileMode::Regular,
                },
            ],
        )
        .is_ok());
    }

    #[test]
    fn test_evidence_seed_and_delta_constructors_reject_invalid_state() {
        assert!(matches!(
            EntryIdentity::for_bytes("", b"bytes"),
            Err(CaptureError::InvalidEntryIdentity(_))
        ));
        assert!(matches!(
            ListingFingerprint::new(BTreeMap::from([(
                "../escape".into(),
                EntryIdentity::for_bytes("entry", b"bytes").unwrap(),
            )])),
            Err(CaptureError::InvalidListingName(_))
        ));
        assert!(matches!(
            PinnedDocumentEvidence::new(
                "HEAD",
                "README.md",
                PinnedSourceClass::GitObject,
                Some("commit".into()),
                Some("blob".into()),
                Some(EntryIdentity::for_bytes("blob", b"bytes").unwrap()),
                None,
                None,
            ),
            Err(CaptureError::InvalidPinnedEvidence(_))
        ));
        assert!(matches!(
            LinkedWorktreeEvidence::new(
                VirtualPath::worktree("config.toml").unwrap(),
                LinkedWorktreeSourceClass::Absent,
                None,
                None,
            ),
            Err(CaptureError::LinkedEvidenceRequiresDataPath(_))
        ));
        assert!(matches!(
            RepositorySeed::new(
                RepositorySeedKind::Command {
                    name: String::new(),
                },
                BTreeMap::new(),
                BTreeMap::new(),
            ),
            Err(SeedError::InvalidText { .. })
        ));

        let path = VirtualPath::data("index.json").unwrap();
        assert!(matches!(
            TargetClaim::new(&layout(), path.clone(), ""),
            Err(DeltaError::InvalidOwner)
        ));
        assert!(matches!(
            RepositoryDelta::new(
                &layout(),
                vec![RepositoryAction::CreateDirectory {
                    path,
                    owner: "producer".into(),
                    expected: ExpectedPreimage::File {
                        identity: EntryIdentity::for_bytes("occupant", b"metadata").unwrap(),
                        mode: FileMode::Regular,
                    },
                }],
            ),
            Err(DeltaError::InvalidAction(_))
        ));
    }

    #[test]
    fn test_capture_spec_phase_two_closes_discovered_paths() {
        let fixed = VirtualPath::data("config.toml").unwrap();
        let discovered = VirtualPath::worktree("docs/plan.md").unwrap();
        let mut spec = CaptureSpec::phase_one(
            [fixed.clone()],
            CaptureBudget {
                max_paths: 4,
                max_listings: 1,
                max_bytes: 64,
                max_depth: 4,
            },
        )
        .unwrap();
        spec.discover_paths([discovered.clone()]).unwrap();
        let bytes = b"plan".to_vec();
        let captured = RepositoryImage::close(
            layout(),
            spec,
            BTreeMap::from([
                (fixed, RepositoryEntry::Absent),
                (
                    discovered.clone(),
                    RepositoryEntry::File {
                        identity: EntryIdentity::for_bytes("plan", &bytes).unwrap(),
                        bytes: bytes.clone(),
                        mode: FileMode::Regular,
                    },
                ),
            ]),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(captured.file_bytes(&discovered).unwrap(), Some(&bytes[..]));
    }

    #[test]
    fn test_hash_serialized_matches_buffered_length_prefixed_json_contract() {
        let captured = image(1, 2);
        let value = captured.entries.iter().collect::<Vec<_>>();
        let mut streamed = Sha256::new();
        hash_serialized(&mut streamed, &value).unwrap();

        let mut buffered = Sha256::new();
        let bytes = serde_json::to_vec(&value).unwrap();
        hash_field(&mut buffered, &bytes);

        assert_eq!(streamed.finalize(), buffered.finalize());
    }

    #[test]
    fn test_plan_hash_is_deterministic_and_covers_entry_listing_and_seed() {
        let captured = image(1, 2);
        let seed = RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "test".into(),
            },
            BTreeMap::from([("key".into(), "value".into())]),
            BTreeMap::new(),
        )
        .unwrap();
        let delta = RepositoryDelta::new(&layout(), Vec::new()).unwrap();
        let hash = plan_hash(
            &captured,
            &seed,
            &MaterializationIntent::SemanticMutation,
            &delta,
        )
        .unwrap();
        assert_eq!(
            hash,
            plan_hash(
                &captured,
                &seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
        assert_ne!(
            hash,
            plan_hash(
                &image(9, 2),
                &seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
        assert_ne!(
            hash,
            plan_hash(
                &image(1, 9),
                &seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
        let changed_seed = RepositorySeed::new(
            RepositorySeedKind::Command {
                name: "changed".into(),
            },
            seed.facts.clone(),
            seed.payloads.clone(),
        )
        .unwrap();
        assert_ne!(
            hash,
            plan_hash(
                &captured,
                &changed_seed,
                &MaterializationIntent::SemanticMutation,
                &delta
            )
            .unwrap()
        );
    }
}

// Include property-based tests
#[cfg(test)]
#[path = "image_plan_hash_property_tests.rs"]
mod plan_hash_property_tests;
