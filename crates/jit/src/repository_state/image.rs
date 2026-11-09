//! Closed repository images, bounded capture declarations, and exact deltas.

use super::{RepositoryLayout, RepositoryLayoutError, VirtualPath};
use crate::domain::relative_path::RootRelativePath;
use crate::domain::vocabulary::wire_vocabulary;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

/// Platform-neutral file mode.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    schemars::JsonSchema,
)]
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
        let identity = Self::derive(object.into(), bytes);
        identity.validate()?;
        Ok(identity)
    }

    fn derive(object: String, bytes: &[u8]) -> Self {
        Self {
            object,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            byte_size: bytes.len() as u64,
        }
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
        Self::validate_object(&self.object)?;
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

    fn validate_object(object: &str) -> Result<(), CaptureError> {
        if object.is_empty() || object.chars().any(char::is_control) {
            return Err(CaptureError::InvalidEntryIdentity(
                "object identity is empty or contains control characters".into(),
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

/// Exact bytes paired with the content identity derived from them.
///
/// The identity is derived from the bytes on first use, so it can never
/// disagree with them and a capture nobody asks the identity of never hashes
/// its bytes. It serializes as its identity alone.
#[derive(Debug, Clone)]
pub struct CapturedBytes {
    object: String,
    bytes: Vec<u8>,
    identity: std::sync::OnceLock<EntryIdentity>,
}

impl CapturedBytes {
    /// `bytes` under the boundary-provided `object` identity.
    pub fn new(object: impl Into<String>, bytes: Vec<u8>) -> Result<Self, CaptureError> {
        let object = object.into();
        EntryIdentity::validate_object(&object)?;
        Ok(Self {
            object,
            bytes,
            identity: std::sync::OnceLock::new(),
        })
    }

    /// Identity derived from [`Self::bytes`].
    pub fn identity(&self) -> &EntryIdentity {
        self.identity
            .get_or_init(|| EntryIdentity::derive(self.object.clone(), &self.bytes))
    }

    /// Exact captured bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl PartialEq for CapturedBytes {
    fn eq(&self, other: &Self) -> bool {
        self.object == other.object && self.bytes == other.bytes
    }
}

impl Eq for CapturedBytes {}

impl Serialize for CapturedBytes {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.identity().serialize(serializer)
    }
}

/// Bytes at the end of a file a [`ReadMode::Tail`] capture keeps at least.
///
/// A byte window rather than the last complete line: the event log can end in an
/// uncommitted, unterminated line, so its end is not a line boundary.
pub const EVENT_LOG_TAIL_WINDOW: u64 = 4096;

/// A file's exact length with its last `min(length, window)` bytes, reaching
/// back to the newline before the final line when that line starts earlier.
///
/// `length` is the file's exact byte count and may include an uncommitted final
/// line. The tail holds that line whole, so [`final_line`](Self::final_line)
/// determines the log's committed length: the end of its last
/// newline-terminated line, where event appends are positioned. The tail is
/// bounded by the final line, never by the file's history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EventLogTail {
    length: u64,
    tail: Vec<u8>,
}

impl EventLogTail {
    /// The length and tail of `bytes`.
    pub fn of_bytes(bytes: &[u8]) -> Self {
        let length = bytes.len() as u64;
        let tail = Self::read_tail(length, |start, len| {
            let range = usize::try_from(start)
                .ok()
                .zip(usize::try_from(start.saturating_add(len)).ok());
            let read = range.and_then(|(start, end)| bytes.get(start..end));
            Ok::<_, std::convert::Infallible>(read.unwrap_or_default().to_vec())
        })
        .unwrap_or_else(|never| match never {});
        Self { length, tail }
    }

    /// The tail of a file `length` bytes long, through `read(start, len)`:
    /// the last window, then earlier windows until one holds the newline
    /// before the final line, kept from that newline on.
    pub fn read_tail<E>(
        length: u64,
        mut read: impl FnMut(u64, u64) -> Result<Vec<u8>, E>,
    ) -> Result<Vec<u8>, E> {
        let mut start = length.saturating_sub(EVENT_LOG_TAIL_WINDOW);
        let mut tail = read(start, length - start)?;
        while start > 0 && !tail.contains(&b'\n') {
            let from = start.saturating_sub(EVENT_LOG_TAIL_WINDOW);
            let mut earlier = read(from, start - from)?;
            if let Some(newline) = earlier.iter().rposition(|byte| *byte == b'\n') {
                earlier.drain(..newline);
            }
            earlier.extend(tail);
            (tail, start) = (earlier, from);
        }
        Ok(tail)
    }

    /// A boundary reading of a file `length` bytes long ending in `tail`.
    ///
    /// # Errors
    ///
    /// [`CaptureError::InvalidTail`] unless `tail` holds the last
    /// `min(length, window)` bytes, or a longer suffix that is the whole file
    /// or begins at the newline before a final line it holds whole.
    pub fn new(length: u64, tail: Vec<u8>) -> Result<Self, CaptureError> {
        let len = tail.len() as u64;
        let line_bounded = match tail.split_first() {
            Some((b'\n', line)) => !line.contains(&b'\n'),
            _ => len == length && !tail.contains(&b'\n'),
        };
        (len == length.min(EVENT_LOG_TAIL_WINDOW)
            || (EVENT_LOG_TAIL_WINDOW < len && len <= length && line_bounded))
            .then_some(Self { length, tail })
            .ok_or(CaptureError::InvalidTail { length })
    }

    /// The file's exact byte length.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// The file's last bytes, as [`EventLogTail`] bounds them.
    pub fn tail(&self) -> &[u8] {
        &self.tail
    }

    /// The bytes after the file's last newline, or `None` when the tail holds
    /// only part of that line.
    pub fn final_line(&self) -> Option<&[u8]> {
        match self.tail.iter().rposition(|byte| *byte == b'\n') {
            Some(newline) => self.tail.get(newline + 1..),
            None => (self.tail.len() as u64 == self.length).then_some(&self.tail[..]),
        }
    }
}

/// One exact captured path state, including typed absence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RepositoryEntry {
    /// The captured path was absent.
    Absent,
    /// An ordinary file and its exact bytes.
    File {
        /// Exact bytes and their identity.
        content: CapturedBytes,
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
    /// An ordinary file read as its length and tail ([`ReadMode::Tail`]).
    FileTail {
        /// Exact length and bounded tail.
        tail: EventLogTail,
        /// Normalized mode.
        mode: FileMode,
    },
    /// An ordinary file read as its kind alone ([`ReadMode::Kind`]).
    FileKind {
        /// Normalized mode.
        mode: FileMode,
    },
    /// A symbolic link, never followed.
    Symlink {
        /// Exact link payload and its identity.
        target: CapturedBytes,
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
    /// Exact identity when an occupant exists and was read whole.
    pub fn identity(&self) -> Option<&EntryIdentity> {
        match self {
            Self::Absent | Self::FileTail { .. } | Self::FileKind { .. } => None,
            Self::File { content, .. } => Some(content.identity()),
            Self::Symlink { target, .. } => Some(target.identity()),
            Self::Directory { identity, .. } | Self::Unsupported { identity, .. } => Some(identity),
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
            RepositoryEntry::File { .. }
            | RepositoryEntry::FileTail { .. }
            | RepositoryEntry::FileKind { .. } => Self::File,
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

/// Child names of one complete non-recursive directory listing; an absent
/// directory lists none.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DirectoryListing(BTreeSet<String>);

impl DirectoryListing {
    /// A listing of `names`, each of which must be one path component.
    pub fn new(names: impl IntoIterator<Item = String>) -> Result<Self, CaptureError> {
        names
            .into_iter()
            .map(|name| validate_listing_name(&name).map(|()| name))
            .collect::<Result<_, _>>()
            .map(Self)
    }

    /// Sorted child names.
    pub fn names(&self) -> &BTreeSet<String> {
        &self.0
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
    /// Exact bytes when present; serialization carries `identity` alone.
    #[serde(skip_serializing)]
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

    /// Git object OID when the target is present at the resolved commit.
    pub fn object_oid(&self) -> Option<&str> {
        self.object_oid.as_deref()
    }

    /// The stable Git-unavailable/not-found/read-failed reason, when the target
    /// could not be captured as a present typed object.
    pub fn unavailable_reason(&self) -> Option<&str> {
        self.unavailable_reason.as_deref()
    }

    /// Require that this evidence answers `request`. The payload needs no
    /// revalidation: [`new`](Self::new) is the only constructor and validates it.
    fn ensure_request(&self, request: &(String, String)) -> Result<(), CaptureError> {
        if (&self.requested_revision, &self.requested_path) != (&request.0, &request.1) {
            return Err(CaptureError::PinnedEvidenceRequestMismatch(request.clone()));
        }
        Ok(())
    }

    fn validate_request(&self, request: &(String, String)) -> Result<(), CaptureError> {
        self.ensure_request(request)?;
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
            // A kind-only request's file carries no bytes to identify.
            (RepositoryTargetKind::File, None, None, None) => {}
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

/// Explicit bounds for one capture.
///
/// Each dimension bounds something a capture can be asked for without anyone
/// having declared it. A capture grows on its own in exactly one place —
/// expanding a complete listing names children the declaration did not — and
/// both dimensions that bound it are here: `max_listings` bounds how many
/// listings may be requested, and `max_bytes` charges every listed child's name
/// alongside the bytes read, so a directory cannot expand a capture for free.
/// `max_depth` bounds the shape of every path, which bounds the ancestor
/// closure a caller building one path at a time can reach.
///
/// There is deliberately no bound on how many exact paths a caller may declare.
/// A declared path is not growth: it is one thing the caller decided to read,
/// and the number of them is bounded by whatever the caller derives its
/// declaration from — for the one declaration that arrives from outside the
/// repository, the profile package model's own file-count and byte bounds. A
/// budget here would restate that bound as a number this type cannot derive,
/// which is how a capture came to refuse a package the model accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CaptureBudget {
    /// Maximum complete listings.
    pub max_listings: usize,
    /// Maximum combined captured bytes, counting every listed child's name.
    pub max_bytes: u64,
    /// Maximum root-relative depth.
    pub max_depth: usize,
}

/// How much of an ordinary file a capture reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Every byte.
    Whole,
    /// The exact length and the last [`EVENT_LOG_TAIL_WINDOW`] bytes, reaching
    /// back to the final line's start, as an [`EventLogTail`] that determines
    /// the committed length where an event append is positioned.
    Tail,
    /// The target's kind alone: a working-tree file captures as
    /// [`RepositoryEntry::FileKind`], and pinned file evidence carries no bytes.
    Kind,
}

/// Normalized capture request: exact paths with their read modes, listings,
/// and evidence requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CaptureSpec {
    paths: BTreeSet<VirtualPath>,
    /// The paths read as [`ReadMode::Tail`].
    tails: BTreeSet<VirtualPath>,
    /// The paths read as [`ReadMode::Kind`]; every other path is read whole.
    kinds: BTreeSet<VirtualPath>,
    listings: BTreeSet<VirtualPath>,
    pinned: BTreeSet<(String, String)>,
    /// The pinned requests read as [`ReadMode::Kind`]; every other is read whole.
    pinned_kinds: BTreeSet<(String, String)>,
    budget: CaptureBudget,
}

impl CaptureSpec {
    /// Request `paths` whole, within `budget`; the event log requires
    /// [`discover_tail`](Self::discover_tail).
    pub fn new(
        paths: impl IntoIterator<Item = VirtualPath>,
        budget: CaptureBudget,
    ) -> Result<Self, CaptureError> {
        let mut spec = Self {
            paths: BTreeSet::new(),
            tails: BTreeSet::new(),
            kinds: BTreeSet::new(),
            listings: BTreeSet::new(),
            pinned: BTreeSet::new(),
            pinned_kinds: BTreeSet::new(),
            budget,
        };
        spec.discover_paths(paths)?;
        Ok(spec)
    }

    /// Add paths to read whole; a path requested as a tail or kind is read
    /// whole instead. The event log requires [`discover_tail`](Self::discover_tail).
    pub fn discover_paths(
        &mut self,
        paths: impl IntoIterator<Item = VirtualPath>,
    ) -> Result<(), CaptureError> {
        let paths = paths.into_iter().collect::<BTreeSet<_>>();
        if paths.contains(&VirtualPath::EVENTS) {
            return Err(CaptureError::WholeEventLogCapture);
        }
        for path in &paths {
            if !self.paths.contains(path) {
                self.validate_path(path)?;
            }
        }
        for path in paths {
            self.tails.remove(&path);
            self.kinds.remove(&path);
            self.paths.insert(path);
        }
        Ok(())
    }

    /// Request a path's length and tail.
    pub fn discover_tail(&mut self, path: VirtualPath) -> Result<(), CaptureError> {
        if !self.paths.contains(&path) {
            self.validate_path(&path)?;
        }
        self.paths.insert(path.clone());
        self.kinds.remove(&path);
        self.tails.insert(path);
        Ok(())
    }

    /// Add paths to read as their kind alone; an already requested path keeps
    /// its read mode.
    pub fn discover_kinds(
        &mut self,
        paths: impl IntoIterator<Item = VirtualPath>,
    ) -> Result<(), CaptureError> {
        let paths = paths.into_iter().collect::<BTreeSet<_>>();
        for path in &paths {
            if !self.paths.contains(path) {
                self.validate_path(path)?;
            }
        }
        for path in paths {
            if self.paths.insert(path.clone()) {
                self.kinds.insert(path);
            }
        }
        Ok(())
    }

    /// Require a complete non-recursive listing.
    pub fn discover_listing(&mut self, path: VirtualPath) -> Result<(), CaptureError> {
        self.discover_listings([path])
    }

    /// Require complete non-recursive listings, validating the declaration once
    /// for the whole batch.
    pub fn discover_listings(
        &mut self,
        paths: impl IntoIterator<Item = VirtualPath>,
    ) -> Result<(), CaptureError> {
        let paths = paths.into_iter().collect::<BTreeSet<_>>();
        let added = paths.difference(&self.listings).count();
        self.validate_listing_count(self.listings.len() + added)?;
        for path in paths.difference(&self.listings) {
            self.validate_path(path)?;
        }
        self.listings.extend(paths);
        Ok(())
    }

    /// Require immutable pinned evidence with its bytes; a request read as its
    /// kind is read whole instead.
    ///
    /// A pinned request constrains nothing else in the declaration, so only
    /// the new request is validated. Revalidating the whole declaration per
    /// request would make a capture quadratic in its pinned-request count.
    pub fn discover_pinned(
        &mut self,
        revision: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<(), CaptureError> {
        let request = (revision.into(), path.into());
        validate_pinned_request(&request.0, &request.1)?;
        self.pinned_kinds.remove(&request);
        self.pinned.insert(request);
        Ok(())
    }

    /// Require immutable pinned evidence of the target's kind alone, unless the
    /// request is already read whole.
    pub fn discover_pinned_kind(
        &mut self,
        revision: impl Into<String>,
        path: impl Into<String>,
    ) -> Result<(), CaptureError> {
        let request = (revision.into(), path.into());
        validate_pinned_request(&request.0, &request.1)?;
        if self.pinned.insert(request.clone()) {
            self.pinned_kinds.insert(request);
        }
        Ok(())
    }

    /// Every exact path in canonical order.
    pub fn paths(&self) -> impl Iterator<Item = &VirtualPath> {
        self.paths.iter()
    }

    /// How `path` is read, when it is requested.
    pub fn read_mode(&self, path: &VirtualPath) -> Option<ReadMode> {
        self.paths.contains(path).then(|| {
            if self.tails.contains(path) {
                ReadMode::Tail
            } else if self.kinds.contains(path) {
                ReadMode::Kind
            } else {
                ReadMode::Whole
            }
        })
    }

    /// Every complete-listing request.
    pub fn listings(&self) -> &BTreeSet<VirtualPath> {
        &self.listings
    }

    /// Every immutable pinned-document request.
    pub fn pinned(&self) -> &BTreeSet<(String, String)> {
        &self.pinned
    }

    /// How the pinned `request` is read, [`ReadMode::Whole`] or
    /// [`ReadMode::Kind`], when it is requested.
    pub fn pinned_read_mode(&self, request: &(String, String)) -> Option<ReadMode> {
        self.pinned.contains(request).then(|| {
            if self.pinned_kinds.contains(request) {
                ReadMode::Kind
            } else {
                ReadMode::Whole
            }
        })
    }

    /// Capture bounds.
    pub fn budget(&self) -> CaptureBudget {
        self.budget
    }

    /// Raise each bound to at least the one `budget` sets.
    pub fn widen_budget(&mut self, budget: CaptureBudget) {
        self.budget = CaptureBudget {
            max_listings: self.budget.max_listings.max(budget.max_listings),
            max_bytes: self.budget.max_bytes.max(budget.max_bytes),
            max_depth: self.budget.max_depth.max(budget.max_depth),
        };
    }

    fn validate_path(&self, path: &VirtualPath) -> Result<(), CaptureError> {
        if path.relative().depth() > self.budget.max_depth {
            return Err(CaptureError::DepthBudgetExceeded(path.clone()));
        }
        path.ensure_semantic()?;
        Ok(())
    }

    fn validate_listing_count(&self, count: usize) -> Result<(), CaptureError> {
        if count > self.budget.max_listings {
            return Err(CaptureError::ListingBudgetExceeded {
                actual: count,
                maximum: self.budget.max_listings,
            });
        }
        Ok(())
    }

    fn validate(&self) -> Result<(), CaptureError> {
        self.validate_listing_count(self.listings.len())?;
        for path in self.paths().chain(self.listings.iter()) {
            self.validate_path(path)?;
        }
        for (revision, path) in &self.pinned {
            validate_pinned_request(revision, path)?;
        }
        Ok(())
    }

    pub(crate) fn contains_path(&self, path: &VirtualPath) -> bool {
        self.paths.contains(path)
    }
}

/// Reads an earlier capture made, keyed as in a [`RepositoryImage`].
#[derive(Debug, Default)]
pub(crate) struct CapturedAnswers {
    pub(crate) entries: BTreeMap<VirtualPath, RepositoryEntry>,
    pub(crate) listings: BTreeMap<VirtualPath, DirectoryListing>,
    pub(crate) pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
}

/// Identity minted when a capture closes; clones of an image share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ImageToken(u64);

impl ImageToken {
    #[cfg(test)]
    pub(crate) fn id(self) -> usize {
        self.0 as usize
    }

    fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

#[derive(Debug, PartialEq, Eq, Serialize)]
struct ImageData {
    layout: RepositoryLayout,
    spec: CaptureSpec,
    entries: BTreeMap<VirtualPath, RepositoryEntry>,
    listings: BTreeMap<VirtualPath, DirectoryListing>,
    pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
}

/// Complete immutable repository image consumed by pure producers.
///
/// Clones share the captured data and the token. Equality compares the
/// captured data only; a session recognises its own image by `ImageToken`.
/// Declaration parses come from the memo of the command that captured the image.
#[derive(Clone)]
pub struct RepositoryImage {
    data: Arc<ImageData>,
    token: ImageToken,
    declarations: Arc<crate::declarations::DeclarationMemo>,
    /// The gate registry of `data`, resolved once for the image and its clones.
    gates: Arc<std::sync::OnceLock<crate::declarations::GateRegistry>>,
}

impl std::fmt::Debug for RepositoryImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RepositoryImage")
            .field("data", &self.data)
            .field("token", &self.token)
            .finish_non_exhaustive()
    }
}

impl PartialEq for RepositoryImage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data) || self.data == other.data
    }
}

impl Eq for RepositoryImage {}

impl Serialize for RepositoryImage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.data.serialize(serializer)
    }
}

impl RepositoryImage {
    /// Close a capture only when every requested path/listing/evidence is
    /// present; its declarations parse through `declarations`.
    pub fn close(
        layout: RepositoryLayout,
        spec: CaptureSpec,
        entries: BTreeMap<VirtualPath, RepositoryEntry>,
        listings: BTreeMap<VirtualPath, DirectoryListing>,
        pinned: BTreeMap<(String, String), PinnedDocumentEvidence>,
        declarations: &Arc<crate::declarations::DeclarationMemo>,
    ) -> Result<Self, CaptureError> {
        spec.validate()?;
        for (path, entry) in &entries {
            layout.ensure_canonical(path)?;
            if !spec.contains_path(path) {
                return Err(CaptureError::UnexpectedCapturedPath(path.clone()));
            }
            if matches!(entry, RepositoryEntry::FileTail { .. })
                && spec.read_mode(path) != Some(ReadMode::Tail)
            {
                return Err(CaptureError::InvalidCapturedEntry {
                    path: path.clone(),
                    reason: "a tail answers only a tail request".into(),
                });
            }
            let kind_request = spec.read_mode(path) == Some(ReadMode::Kind);
            match entry {
                RepositoryEntry::FileKind { .. } if !kind_request => {
                    return Err(CaptureError::InvalidCapturedEntry {
                        path: path.clone(),
                        reason: "a file's kind answers only a kind request".into(),
                    });
                }
                RepositoryEntry::File { .. } if kind_request => {
                    return Err(CaptureError::InvalidCapturedEntry {
                        path: path.clone(),
                        reason: "a kind request is answered without the file's bytes".into(),
                    });
                }
                _ => {}
            }
            validate_entry(path, entry)?;
        }
        for path in spec.paths() {
            layout.ensure_canonical(path)?;
            if !entries.contains_key(path) {
                return Err(CaptureError::IncompleteCapture(path.clone()));
            }
        }
        for path in listings.keys() {
            layout.ensure_canonical(path)?;
            if !spec.listings.contains(path) {
                return Err(CaptureError::UnexpectedListing(path.clone()));
            }
        }
        for path in spec.listings() {
            layout.ensure_canonical(path)?;
            if !listings.contains_key(path) {
                return Err(CaptureError::IncompleteListing(path.clone()));
            }
        }
        // Expanding a listing is the one way a capture grows beyond what was
        // declared, so every child it names is held to the depth budget and
        // charged its name to the byte budget.
        let listing_bytes = listings
            .iter()
            .flat_map(|(parent, listing)| {
                listing
                    .names()
                    .iter()
                    .map(move |name| (parent, name.as_str()))
            })
            .try_fold(0_u64, |bytes, (parent, name)| {
                let child = listing_child_path(&layout, parent, name)?;
                (child.relative().depth() <= spec.budget.max_depth)
                    .then(|| bytes.saturating_add(u64::try_from(name.len()).unwrap_or(u64::MAX)))
                    .ok_or(CaptureError::DepthBudgetExceeded(child))
            })?;
        for (request, evidence) in &pinned {
            if !spec.pinned.contains(request) {
                return Err(CaptureError::UnexpectedPinnedEvidence(request.clone()));
            }
            evidence.ensure_request(request)?;
            if evidence.target_kind.is_file()
                && evidence.bytes.is_some()
                    != (spec.pinned_read_mode(request) == Some(ReadMode::Whole))
            {
                return Err(CaptureError::InvalidPinnedEvidence(
                    "a pinned file carries its bytes exactly when read whole".into(),
                ));
            }
        }
        for request in &spec.pinned {
            if !pinned.contains_key(request) {
                return Err(CaptureError::IncompletePinnedEvidence(request.clone()));
            }
        }
        let byte_count = captured_byte_count(&entries, &pinned).saturating_add(listing_bytes);
        if byte_count > spec.budget.max_bytes {
            return Err(CaptureError::ByteBudgetExceeded {
                actual: byte_count,
                maximum: spec.budget.max_bytes,
            });
        }
        Ok(Self {
            data: Arc::new(ImageData {
                layout,
                spec,
                entries,
                listings,
                pinned,
            }),
            token: ImageToken::mint(),
            declarations: Arc::clone(declarations),
            gates: Arc::default(),
        })
    }

    pub(crate) fn resolved_gates(&self) -> &std::sync::OnceLock<crate::declarations::GateRegistry> {
        &self.gates
    }

    /// The memo this image's declaration parses come from.
    pub(crate) fn declaration_memo(&self) -> &Arc<crate::declarations::DeclarationMemo> {
        &self.declarations
    }

    pub(crate) fn token(&self) -> ImageToken {
        self.token
    }

    /// Whether both images hold the same captured data allocation.
    pub fn shares_data_with(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.data, &other.data)
    }

    /// Canonical layout captured by this image.
    pub fn layout(&self) -> &RepositoryLayout {
        &self.data.layout
    }

    /// Closed capture declaration used to build this image.
    pub fn capture_spec(&self) -> &CaptureSpec {
        &self.data.spec
    }

    /// Every exact captured path in canonical order.
    pub fn entries(&self) -> &BTreeMap<VirtualPath, RepositoryEntry> {
        &self.data.entries
    }

    /// Every complete directory listing in canonical order.
    pub fn directory_listings(&self) -> &BTreeMap<VirtualPath, DirectoryListing> {
        &self.data.listings
    }

    /// The `*.json` files `directory`'s captured listing names, in name order;
    /// an occupant other than a directory is refused as
    /// [`CaptureError::NotADirectory`], and an absent directory lists nothing.
    pub fn listed_json_files(
        &self,
        directory: &VirtualPath,
    ) -> Result<Vec<VirtualPath>, CaptureError> {
        if !matches!(
            self.entry(directory)?,
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. }
        ) {
            return Err(CaptureError::NotADirectory(directory.clone()));
        }
        self.data
            .listings
            .get(directory)
            .ok_or_else(|| CaptureError::IncompleteListing(directory.clone()))?
            .names()
            .iter()
            .filter(|name| name.ends_with(".json"))
            .map(|name| {
                Ok(VirtualPath::from_root(
                    directory.root_class(),
                    directory.relative().join(name),
                )?)
            })
            .collect()
    }

    /// Read an exactly captured entry; uncaptured reads are always errors.
    pub fn entry(&self, path: &VirtualPath) -> Result<&RepositoryEntry, CaptureError> {
        self.data.layout.ensure_canonical(path)?;
        if !self.data.spec.contains_path(path) {
            return Err(CaptureError::UndiscoveredRepositoryPath(path.clone()));
        }
        self.data
            .entries
            .get(path)
            .ok_or_else(|| CaptureError::IncompleteCapture(path.clone()))
    }

    /// Read an ordinary file's exact bytes.
    pub fn file_bytes(&self, path: &VirtualPath) -> Result<Option<&[u8]>, CaptureError> {
        match self.entry(path)? {
            RepositoryEntry::File { content, .. } => Ok(Some(content.bytes())),
            RepositoryEntry::FileTail { .. } => Err(CaptureError::TailOnlyCapture(path.clone())),
            RepositoryEntry::FileKind { .. } => Err(CaptureError::KindOnlyCapture(path.clone())),
            RepositoryEntry::Absent | RepositoryEntry::Directory { .. } => Ok(None),
            RepositoryEntry::Symlink { .. } | RepositoryEntry::Unsupported { .. } => {
                Err(CaptureError::UnsafeCapturedOccupant(path.clone()))
            }
        }
    }

    /// Every boundary-acquired pinned-document evidence, keyed by
    /// `(requested revision, requested path)`.
    pub fn pinned_evidence(&self) -> &BTreeMap<(String, String), PinnedDocumentEvidence> {
        &self.data.pinned
    }

    /// What this image read that `spec` requests the same way, for a capture
    /// of `spec` to keep rather than read again.
    pub(crate) fn into_answers(self, spec: &CaptureSpec) -> CapturedAnswers {
        let ImageData {
            spec: read,
            entries,
            listings,
            pinned,
            ..
        } = Arc::try_unwrap(self.data).unwrap_or_else(|shared| ImageData {
            layout: shared.layout.clone(),
            spec: shared.spec.clone(),
            entries: shared.entries.clone(),
            listings: shared.listings.clone(),
            pinned: shared.pinned.clone(),
        });
        CapturedAnswers {
            entries: entries
                .into_iter()
                .filter(|(path, _)| read.read_mode(path) == spec.read_mode(path))
                .collect(),
            listings,
            pinned: pinned
                .into_iter()
                .filter(|(request, _)| {
                    read.pinned_read_mode(request) == spec.pinned_read_mode(request)
                })
                .collect(),
        }
    }
}

fn validate_entry(path: &VirtualPath, entry: &RepositoryEntry) -> Result<(), CaptureError> {
    match entry {
        // Captured bytes carry the identity derived from them at construction.
        RepositoryEntry::Absent
        | RepositoryEntry::File { .. }
        | RepositoryEntry::FileTail { .. }
        | RepositoryEntry::FileKind { .. }
        | RepositoryEntry::Symlink { .. } => Ok(()),
        RepositoryEntry::Directory { identity, .. } => identity.validate(),
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
    let path = RootRelativePath::parse(path).map_err(RepositoryLayoutError::from)?;
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
) -> u64 {
    entries
        .values()
        .filter_map(|entry| match entry {
            RepositoryEntry::File { content, .. } => Some(content.bytes().len() as u64),
            RepositoryEntry::FileTail { tail, .. } => Some(tail.tail().len() as u64),
            RepositoryEntry::Symlink { target, .. } => Some(target.bytes().len() as u64),
            RepositoryEntry::Absent
            | RepositoryEntry::FileKind { .. }
            | RepositoryEntry::Directory { .. }
            | RepositoryEntry::Unsupported { .. } => None,
        })
        .chain(
            pinned
                .values()
                .filter_map(|evidence| evidence.bytes.as_ref().map(|bytes| bytes.len() as u64)),
        )
        .fold(0, u64::saturating_add)
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
        if owner.is_empty() || owner.chars().any(char::is_control) {
            return Err(DeltaError::InvalidClaimOwner);
        }
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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
    /// An ordinary file with exact length, tail, and mode.
    FileTail {
        /// Exact length and bounded tail.
        tail: EventLogTail,
        /// Normalized file mode.
        mode: FileMode,
    },
    /// An ordinary file known by its kind and mode alone, which no action
    /// may write over.
    FileKind {
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
            RepositoryEntry::File { content, mode } => Self::File {
                identity: content.identity().clone(),
                mode: *mode,
            },
            RepositoryEntry::FileTail { tail, mode } => Self::FileTail {
                tail: tail.clone(),
                mode: *mode,
            },
            RepositoryEntry::FileKind { mode } => Self::FileKind { mode: *mode },
            RepositoryEntry::Directory { identity, mode } => Self::Directory {
                identity: identity.clone(),
                mode: *mode,
            },
            RepositoryEntry::Symlink { target, mode } => Self::Symlink {
                identity: target.identity().clone(),
                target: target.bytes().to_vec(),
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

    /// Exact identity when the preimage expects an occupant read whole.
    pub fn identity(&self) -> Option<&EntryIdentity> {
        match self {
            Self::Absent | Self::FileTail { .. } | Self::FileKind { .. } => None,
            Self::File { identity, .. }
            | Self::Directory { identity, .. }
            | Self::Symlink { identity, .. }
            | Self::Unsupported { identity, .. } => Some(identity),
        }
    }
}

wire_vocabulary! {
    /// Producer of an exact repository-state action.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    pub enum RepositoryActionOwner {
        RepositoryStateMutation = "repository-state-mutation",
        RepositoryInit = "repository-init",
        ProfileApplication = "profile-application",
        GitattributesMergeDriver = "gitattributes-merge-driver",
        ArchiveExecution = "archive-execution",
        PackageCapture = "package-capture",
        RepositoryExport = "repository-export",
        ConfiguredProjection = "configured-projection",
        DataRootIgnore = "data-root-ignore",
        DefaultRuleMembership = "default-rule-membership",
        DefaultSchema = "default-schema",
        AuthoredConfig = "authored-config",
    }
}

static EVENT_LOG: VirtualPath = VirtualPath::EVENTS;

/// One exact repository-state action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum RepositoryAction {
    /// Create an absent directory.
    CreateDirectory {
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
    },
    /// Create or replace an ordinary file.
    WriteFile {
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
        /// Exact bytes to publish; serialization carries their digest.
        #[serde(serialize_with = "serialize_content_digest")]
        bytes: Vec<u8>,
        mode: FileMode,
    },
    /// Change only an ordinary file's mode.
    SetMode {
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
        mode: FileMode,
    },
    /// Delete one ordinary file.
    DeleteFile {
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
    },
    /// Write `bytes` into the event log at `offset`, the committed length its
    /// capture read.
    AppendEvents {
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
        offset: u64,
        #[serde(serialize_with = "serialize_content_digest")]
        bytes: Vec<u8>,
    },
}

impl RepositoryAction {
    #[cfg(test)]
    pub(crate) fn create_directory(
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
    ) -> Self {
        Self::CreateDirectory {
            path,
            owner,
            expected,
        }
    }

    #[cfg(test)]
    pub(crate) fn write_file(
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
        bytes: Vec<u8>,
        mode: FileMode,
    ) -> Self {
        Self::WriteFile {
            path,
            owner,
            expected,
            bytes,
            mode,
        }
    }

    #[cfg(test)]
    pub(crate) fn set_mode(
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
        mode: FileMode,
    ) -> Self {
        Self::SetMode {
            path,
            owner,
            expected,
            mode,
        }
    }

    #[cfg(test)]
    pub(crate) fn delete_file(
        path: VirtualPath,
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
    ) -> Self {
        Self::DeleteFile {
            path,
            owner,
            expected,
        }
    }

    #[cfg(test)]
    pub(crate) fn append_events(
        owner: RepositoryActionOwner,
        expected: ExpectedPreimage,
        offset: u64,
        bytes: Vec<u8>,
    ) -> Self {
        Self::AppendEvents {
            owner,
            expected,
            offset,
            bytes,
        }
    }

    /// Canonical target path.
    pub fn path(&self) -> &VirtualPath {
        match self {
            Self::CreateDirectory { path, .. }
            | Self::WriteFile { path, .. }
            | Self::SetMode { path, .. }
            | Self::DeleteFile { path, .. } => path,
            Self::AppendEvents { .. } => &EVENT_LOG,
        }
    }

    /// Stable producer or ownership identity.
    pub fn owner(&self) -> RepositoryActionOwner {
        match self {
            Self::CreateDirectory { owner, .. }
            | Self::WriteFile { owner, .. }
            | Self::SetMode { owner, .. }
            | Self::DeleteFile { owner, .. }
            | Self::AppendEvents { owner, .. } => *owner,
        }
    }

    /// Exact preimage required immediately before publication.
    pub fn expected(&self) -> &ExpectedPreimage {
        match self {
            Self::CreateDirectory { expected, .. }
            | Self::WriteFile { expected, .. }
            | Self::SetMode { expected, .. }
            | Self::DeleteFile { expected, .. }
            | Self::AppendEvents { expected, .. } => expected,
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
    /// pre-commit write-set check as a retryable conflict.
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

    /// The actions replacing, changing or removing a file: everything but
    /// directory creation and the event-log append, which keeps every prior line.
    pub(crate) fn file_writes(&self) -> impl Iterator<Item = &RepositoryAction> {
        self.actions.iter().filter(|action| {
            !matches!(
                action,
                RepositoryAction::CreateDirectory { .. } | RepositoryAction::AppendEvents { .. }
            )
        })
    }
}

fn validate_action(action: &RepositoryAction) -> Result<(), DeltaError> {
    let expected = action.expected();
    if *action.path() == EVENT_LOG && !matches!(action, RepositoryAction::AppendEvents { .. }) {
        return Err(DeltaError::InvalidAction(
            "the event log changes only by a positioned append".into(),
        ));
    }
    if super::path::is_bootstrap_lock(action.path()) {
        return Err(DeltaError::EngineControlTarget(action.path().clone()));
    }
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
        | RepositoryAction::AppendEvents {
            expected:
                ExpectedPreimage::Absent
                | ExpectedPreimage::File { .. }
                | ExpectedPreimage::FileTail { .. },
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
        RepositoryAction::WriteFile { .. } | RepositoryAction::AppendEvents { .. } => Err(
            DeltaError::InvalidAction("file write requires an absent or file preimage".into()),
        ),
        RepositoryAction::SetMode { .. } => Err(DeltaError::InvalidAction(
            "mode changes require a file preimage".into(),
        )),
        RepositoryAction::DeleteFile { .. } => Err(DeltaError::InvalidAction(
            "file deletion requires a file preimage".into(),
        )),
    }
}

/// Serialize bytes as their SHA-256 digest and length, so a serialized delta
/// commits to written content without encoding it byte by byte.
fn serialize_content_digest<S: serde::Serializer>(
    bytes: &[u8],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    (format!("{:x}", Sha256::digest(bytes)), bytes.len()).serialize(serializer)
}

/// Capture closure failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error("the event log can only be captured through discover_tail")]
    WholeEventLogCapture,
    #[error("capture requested {actual} listings but the maximum is {maximum}")]
    ListingBudgetExceeded { actual: usize, maximum: usize },
    #[error("captured {actual} bytes but the maximum is {maximum}")]
    ByteBudgetExceeded { actual: u64, maximum: u64 },
    #[error("capture path {0:?} exceeds the depth budget")]
    DepthBudgetExceeded(VirtualPath),
    #[error("capture did not provide requested path {0:?}")]
    IncompleteCapture(VirtualPath),
    #[error("capture read only the length and tail of {0:?}")]
    TailOnlyCapture(VirtualPath),
    #[error("capture read only the kind of {0:?}")]
    KindOnlyCapture(VirtualPath),
    #[error("a tail of a {length}-byte file must hold its last window bytes and its final line")]
    InvalidTail { length: u64 },
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
    #[error("entry identity is invalid: {0}")]
    InvalidEntryIdentity(String),
    #[error("entry identity for object '{object}' does not match captured bytes")]
    EntryIdentityMismatch { object: String },
    #[error("captured entry {path:?} is invalid: {reason}")]
    InvalidCapturedEntry { path: VirtualPath, reason: String },
    #[error("directory listing child name '{0}' is invalid")]
    InvalidListingName(String),
    #[error("pure producer requested undiscovered path {0:?}")]
    UndiscoveredRepositoryPath(VirtualPath),
    #[error("captured path {0:?} has a symlink or unsupported occupant")]
    UnsafeCapturedOccupant(VirtualPath),
    #[error("captured path {0:?} is not a directory")]
    NotADirectory(VirtualPath),
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
    #[error("target claim owner is empty or contains control characters")]
    InvalidClaimOwner,
    #[error("delta action targets machine-local engine control at {0:?}")]
    EngineControlTarget(VirtualPath),
    #[error("delta action is invalid: {0}")]
    InvalidAction(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    impl RepositoryDelta {
        /// The bytes the delta appends to the event log, if it appends any.
        pub(crate) fn appended_event_bytes(&self) -> Option<&[u8]> {
            self.actions.iter().find_map(|action| match action {
                RepositoryAction::AppendEvents { bytes, .. } => Some(bytes.as_slice()),
                _ => None,
            })
        }
    }
    use crate::repository_state::RepositoryRootEvidence;

    #[test]
    fn test_listed_json_files_lists_json_names_and_refuses_a_non_directory_occupant() {
        let directory = VirtualPath::GATE_PRESETS;
        let listing =
            DirectoryListing::new(["a.json".to_string(), "notes.txt".to_string()]).unwrap();
        let listed = crate::test_harness::image::ImageBuilder::new()
            .entry(directory.clone(), RepositoryEntry::Absent)
            .listing(directory.clone(), listing.clone())
            .build();
        let occupied = crate::test_harness::image::ImageBuilder::new()
            .file(directory.clone(), b"not a directory")
            .listing(directory.clone(), listing)
            .build();

        assert_eq!(
            listed.listed_json_files(&directory).unwrap(),
            vec![VirtualPath::data("config/gate-presets/a.json").unwrap()]
        );
        assert!(matches!(
            occupied.listed_json_files(&directory),
            Err(CaptureError::NotADirectory(path)) if path == directory
        ));
    }

    fn layout() -> RepositoryLayout {
        RepositoryLayout::new(
            RepositoryRootEvidence::new("/repo", "wt", true),
            RepositoryRootEvidence::new("/repo/.jit", "data", true),
        )
        .unwrap()
    }

    fn image() -> RepositoryImage {
        let path = VirtualPath::data("index.json").unwrap();
        let budget = CaptureBudget {
            max_listings: 0,
            max_bytes: 64,
            max_depth: 4,
        };
        RepositoryImage::close(
            layout(),
            CaptureSpec::new([path.clone()], budget).unwrap(),
            BTreeMap::from([(
                path,
                RepositoryEntry::File {
                    content: CapturedBytes::new("index", vec![1]).unwrap(),
                    mode: FileMode::Regular,
                },
            )]),
            BTreeMap::new(),
            BTreeMap::new(),
            &Default::default(),
        )
        .unwrap()
    }

    fn generated_data_path(depth: usize, id: usize) -> VirtualPath {
        let path = match depth {
            0 => String::new(),
            1 => format!("leaf-{id}"),
            depth => format!("{}{id}", "segment/".repeat(depth - 1)),
        };
        VirtualPath::data(path).unwrap()
    }

    fn grown_spec(mut spec: CaptureSpec, operation: u8, paths: &[VirtualPath]) -> CaptureSpec {
        match operation {
            0 => paths.iter().for_each(|path| {
                spec.tails.remove(path);
                spec.kinds.remove(path);
                spec.paths.insert(path.clone());
            }),
            1 => {
                let path = paths.first().unwrap();
                spec.paths.insert(path.clone());
                spec.kinds.remove(path);
                spec.tails.insert(path.clone());
            }
            2 => paths.iter().for_each(|path| {
                if spec.paths.insert(path.clone()) {
                    spec.kinds.insert(path.clone());
                }
            }),
            3 => spec.listings.extend(paths.iter().cloned()),
            _ => unreachable!(),
        }
        spec
    }

    fn assert_growth_matches_whole_validation(
        prior: &CaptureSpec,
        grown: CaptureSpec,
        actual: CaptureSpec,
        batch_accepted: bool,
    ) -> Result<(), proptest::test_runner::TestCaseError> {
        let whole_spec_accepts = grown.validate().is_ok();
        proptest::prop_assert_eq!(batch_accepted, whole_spec_accepts);
        proptest::prop_assert_eq!(
            actual,
            if whole_spec_accepts {
                grown
            } else {
                prior.clone()
            }
        );
        Ok(())
    }

    #[test]
    fn test_capture_rejects_undiscovered_repository_read() {
        let mut captured = image();
        let unrequested = VirtualPath::data("config.toml").unwrap();
        Arc::get_mut(&mut captured.data)
            .unwrap()
            .entries
            .insert(unrequested.clone(), RepositoryEntry::Absent);
        assert!(matches!(
            captured.entry(&unrequested),
            Err(CaptureError::UndiscoveredRepositoryPath(_))
        ));
    }

    #[test]
    fn test_close_rejects_every_unrequested_evidence_map() {
        let requested = VirtualPath::data("index.json").unwrap();
        let extra = VirtualPath::data("extra.json").unwrap();
        let spec = CaptureSpec::new(
            [requested.clone()],
            CaptureBudget {
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
                BTreeMap::new(), &Default::default(),
            ),
            Err(CaptureError::UnexpectedCapturedPath(path)) if path == extra
        ));

        assert!(matches!(
            RepositoryImage::close(
                layout(),
                spec.clone(),
                entries.clone(),
                BTreeMap::from([(extra.clone(), DirectoryListing::default())]),
                BTreeMap::new(), &Default::default(),
            ),
            Err(CaptureError::UnexpectedListing(path)) if path == extra
        ));

        let pinned_request = ("HEAD".to_owned(), "README.md".to_owned());
        let pinned = PinnedDocumentEvidence::new(
            &pinned_request.0,
            &pinned_request.1,
            PinnedSourceClass::GitUnavailable,
            RepositoryTargetKind::Missing,
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
                BTreeMap::from([(pinned_request.clone(), pinned)]), &Default::default(),
            ),
            Err(CaptureError::UnexpectedPinnedEvidence(request)) if request == pinned_request
        ));
    }

    fn close_listing(
        path: &str,
        children: &[&str],
        exact: &[&str],
        budget: CaptureBudget,
    ) -> Result<RepositoryImage, CaptureError> {
        let path = VirtualPath::data(path)?;
        let listing = DirectoryListing::new(children.iter().map(|name| (*name).to_owned()))?;
        let entries = exact
            .iter()
            .map(|name| {
                Ok((
                    VirtualPath::data(format!("{path}/{name}", path = path.relative().as_str()))?,
                    RepositoryEntry::File {
                        content: CapturedBytes::new(*name, Vec::new())?,
                        mode: FileMode::Regular,
                    },
                ))
            })
            .collect::<Result<BTreeMap<_, _>, CaptureError>>()?;
        let mut spec = CaptureSpec::new(entries.keys().cloned(), budget)?;
        spec.discover_listing(path.clone())?;
        RepositoryImage::close(
            layout(),
            spec,
            entries,
            BTreeMap::from([(path, listing)]),
            BTreeMap::new(),
            &Default::default(),
        )
    }

    fn listing_budget(max_bytes: u64, max_depth: usize) -> CaptureBudget {
        CaptureBudget {
            max_listings: 1,
            max_bytes,
            max_depth,
        }
    }

    /// The bytes a capture actually read are held to the byte budget, so a file
    /// larger than the declaration admits refuses the whole image rather than
    /// reaching a producer.
    #[test]
    fn test_close_refuses_captured_bytes_over_the_byte_budget() {
        let window = EVENT_LOG_TAIL_WINDOW as usize;
        let log = [vec![b'h'; window], vec![b'\n'], vec![b'x'; window * 3]].concat();
        let mode = FileMode::Regular;
        let payload = b"captured payload".to_vec();
        let read_whole = payload.len();
        let content = CapturedBytes::new("index", payload).unwrap();
        let tail = EventLogTail::of_bytes(&log);
        // A tail's read-back to its final line's start counts like any read.
        let cases = [
            (
                VirtualPath::INDEX,
                read_whole,
                RepositoryEntry::File { content, mode },
            ),
            (
                VirtualPath::EVENTS,
                window * 3 + 1,
                RepositoryEntry::FileTail { tail, mode },
            ),
        ];
        for (path, read, entry) in cases {
            let close = |max_bytes: u64| {
                let budget = CaptureBudget {
                    max_listings: 0,
                    max_bytes,
                    max_depth: 1,
                };
                let mut spec = CaptureSpec::new([], budget)?;
                match entry {
                    RepositoryEntry::FileTail { .. } => spec.discover_tail(path.clone())?,
                    _ => spec.discover_paths([path.clone()])?,
                }
                let entries = BTreeMap::from([(path.clone(), entry.clone())]);
                let (listings, pinned) = Default::default();
                RepositoryImage::close(
                    layout(),
                    spec,
                    entries,
                    listings,
                    pinned,
                    &Default::default(),
                )
            };
            let read = read as u64;

            assert!(close(read).is_ok());
            assert!(matches!(
                close(read - 1),
                Err(CaptureError::ByteBudgetExceeded { actual, maximum })
                    if actual == read && maximum == read - 1
            ));
        }
    }

    #[test]
    fn test_capture_spec_refuses_whole_event_log_reads() {
        let budget = image().capture_spec().budget();
        assert!(matches!(
            CaptureSpec::new([VirtualPath::EVENTS], budget),
            Err(CaptureError::WholeEventLogCapture)
        ));

        let mut spec = CaptureSpec::new([], budget).unwrap();
        spec.discover_tail(VirtualPath::EVENTS).unwrap();
        let prior = spec.clone();
        let too_deep = VirtualPath::data("a/b/c/d/e").unwrap();
        assert!(matches!(
            spec.discover_paths([too_deep, VirtualPath::EVENTS]),
            Err(CaptureError::WholeEventLogCapture)
        ));
        assert_eq!(spec, prior);
    }

    /// A kind request never narrows a request for content: whichever order they
    /// arrive in, the content read wins.
    #[test]
    fn test_capture_spec_kind_requests_yield_to_content_requests() {
        let path = VirtualPath::worktree("docs/a.md").unwrap();
        let request = ("HEAD".to_string(), "docs/a.md".to_string());
        let spec = || CaptureSpec::new([], image().capture_spec().budget()).unwrap();

        let mut kind_first = spec();
        kind_first.discover_kinds([path.clone()]).unwrap();
        assert_eq!(kind_first.read_mode(&path), Some(ReadMode::Kind));
        kind_first.discover_paths([path.clone()]).unwrap();
        assert_eq!(kind_first.read_mode(&path), Some(ReadMode::Whole));
        let mut whole_first = spec();
        whole_first.discover_paths([path.clone()]).unwrap();
        whole_first.discover_kinds([path.clone()]).unwrap();
        assert_eq!(whole_first.read_mode(&path), Some(ReadMode::Whole));
        let mut tail_over_kind = spec();
        tail_over_kind.discover_kinds([path.clone()]).unwrap();
        tail_over_kind.discover_tail(path.clone()).unwrap();
        tail_over_kind.discover_kinds([path.clone()]).unwrap();
        assert_eq!(tail_over_kind.read_mode(&path), Some(ReadMode::Tail));

        let mut pinned_kind_first = spec();
        pinned_kind_first
            .discover_pinned_kind(&request.0, &request.1)
            .unwrap();
        assert_eq!(
            pinned_kind_first.pinned_read_mode(&request),
            Some(ReadMode::Kind)
        );
        pinned_kind_first
            .discover_pinned(&request.0, &request.1)
            .unwrap();
        assert_eq!(
            pinned_kind_first.pinned_read_mode(&request),
            Some(ReadMode::Whole)
        );
        let mut pinned_whole_first = spec();
        pinned_whole_first
            .discover_pinned(&request.0, &request.1)
            .unwrap();
        pinned_whole_first
            .discover_pinned_kind(&request.0, &request.1)
            .unwrap();
        assert_eq!(
            pinned_whole_first.pinned_read_mode(&request),
            Some(ReadMode::Whole)
        );
    }

    /// A kind request is answered by a file's kind alone, which costs no bytes,
    /// and only a kind request is.
    #[test]
    fn test_close_answers_a_kind_request_exactly_with_a_bytes_free_kind() {
        let path = VirtualPath::worktree("docs/a.md").unwrap();
        let request = ("HEAD".to_string(), "docs/a.md".to_string());
        let mode = FileMode::Regular;
        let budget = CaptureBudget {
            max_listings: 0,
            max_bytes: 0,
            max_depth: 4,
        };
        let close = |kind: bool, entry: RepositoryEntry| {
            let mut spec = CaptureSpec::new([], budget)?;
            if kind {
                spec.discover_kinds([path.clone()])?;
            } else {
                spec.discover_paths([path.clone()])?;
            }
            let entries = BTreeMap::from([(path.clone(), entry)]);
            RepositoryImage::close(
                layout(),
                spec,
                entries,
                BTreeMap::new(),
                BTreeMap::new(),
                &Default::default(),
            )
        };
        let file = || RepositoryEntry::File {
            content: CapturedBytes::new("a", b"bytes".to_vec()).unwrap(),
            mode,
        };

        let image = close(true, RepositoryEntry::FileKind { mode }).unwrap();
        assert!(matches!(
            image.file_bytes(&path),
            Err(CaptureError::KindOnlyCapture(_))
        ));
        assert!(RepositoryTargetKind::from_entry(image.entry(&path).unwrap()).is_file());
        assert!(close(true, RepositoryEntry::Absent).is_ok());
        assert!(matches!(
            close(true, file()),
            Err(CaptureError::InvalidCapturedEntry { .. })
        ));
        assert!(matches!(
            close(false, RepositoryEntry::FileKind { mode }),
            Err(CaptureError::InvalidCapturedEntry { .. })
        ));

        let evidence = |bytes: Option<Vec<u8>>| {
            let identity = bytes
                .as_deref()
                .map(|bytes| EntryIdentity::for_bytes("git-blob:b", bytes).unwrap());
            PinnedDocumentEvidence::new(
                &request.0,
                &request.1,
                PinnedSourceClass::GitObject,
                RepositoryTargetKind::File,
                Some("a".repeat(40)),
                Some("b".repeat(40)),
                identity,
                bytes,
                None,
            )
            .unwrap()
        };
        let close_pinned = |kind: bool, evidence: PinnedDocumentEvidence| {
            let mut spec = CaptureSpec::new([], budget)?;
            if kind {
                spec.discover_pinned_kind(&request.0, &request.1)?;
            } else {
                spec.discover_pinned(&request.0, &request.1)?;
            }
            let pinned = BTreeMap::from([(request.clone(), evidence)]);
            RepositoryImage::close(
                layout(),
                spec,
                BTreeMap::new(),
                BTreeMap::new(),
                pinned,
                &Default::default(),
            )
        };
        let image = close_pinned(true, evidence(None)).unwrap();
        assert!(image.pinned_evidence()[&request].exists());
        assert!(matches!(
            close_pinned(true, evidence(Some(b"bytes".to_vec()))),
            Err(CaptureError::InvalidPinnedEvidence(_))
        ));
        assert!(matches!(
            close_pinned(false, evidence(None)),
            Err(CaptureError::InvalidPinnedEvidence(_))
        ));
    }

    /// Requesting a listing is what a caller is bounded on, and the refusal
    /// arrives at declaration time rather than after the reads it would drive.
    #[test]
    fn test_capture_spec_refuses_more_listings_than_the_budget_admits() {
        let budget = CaptureBudget {
            max_listings: 1,
            max_bytes: 1024,
            max_depth: 4,
        };
        let mut spec = CaptureSpec::new([], budget).unwrap();
        spec.discover_listing(VirtualPath::data("issues").unwrap())
            .unwrap();

        assert!(matches!(
            spec.discover_listing(VirtualPath::data("templates").unwrap()),
            Err(CaptureError::ListingBudgetExceeded {
                actual: 2,
                maximum: 1
            })
        ));
        assert_eq!(spec.listings().len(), 1);
    }

    /// A listing's children are bounded by what their names cost, not by how
    /// many of them there are: a directory whose children outnumber every path
    /// the declaration named closes when their names fit the byte budget, and is
    /// refused only once they do not.
    #[test]
    fn test_close_bounds_listing_children_by_their_names_rather_than_their_count() {
        let children = ["a", "b", "c", "d"];
        let names = children.iter().map(|name| name.len() as u64).sum::<u64>();

        assert!(close_listing("issues", &children, &[], listing_budget(names, 2)).is_ok());
        assert!(matches!(
            close_listing("issues", &children, &[], listing_budget(names - 1, 2)),
            Err(CaptureError::ByteBudgetExceeded { actual, maximum })
                if actual == names && maximum == names - 1
        ));
    }

    /// A listed child that is itself a requested listing root is admitted: one
    /// path reached two ways is still one path.
    #[test]
    fn test_close_admits_a_listed_child_that_is_a_requested_listing_root() {
        let root = VirtualPath::data("").unwrap();
        let issues = VirtualPath::data("issues").unwrap();
        let mut spec = CaptureSpec::new(
            [],
            CaptureBudget {
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
                (root, DirectoryListing::new(["issues".into()]).unwrap()),
                (issues, DirectoryListing::default()),
            ]),
            BTreeMap::new(),
            &Default::default(),
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_close_charges_multibyte_listing_names_to_byte_budget() {
        let result = close_listing("issues", &["é"], &[], listing_budget(1, 2));

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
        let issue_path = VirtualPath::issue_record(&crate::test_utils::issue_id(1));
        let name = issue_path.file_name().unwrap();
        let result = close_listing("issues", &[name], &[], listing_budget(64, 1));

        assert!(matches!(
            result,
            Err(CaptureError::DepthBudgetExceeded(path))
                if path == issue_path
        ));
    }

    #[test]
    fn test_close_empty_listing_costs_no_bytes() {
        let result = close_listing("issues", &[], &[], listing_budget(0, 1));

        assert!(result.is_ok());
    }

    #[test]
    fn test_capture_spec_failed_discovery_leaves_the_spec_unchanged() {
        let fixed = VirtualPath::data("index.json").unwrap();
        let mut spec = CaptureSpec::new(
            [fixed.clone()],
            CaptureBudget {
                max_listings: 0,
                max_bytes: 0,
                max_depth: 1,
            },
        )
        .unwrap();
        let prior = spec.clone();
        assert!(matches!(
            spec.discover_paths([
                VirtualPath::data("new.json").unwrap(),
                VirtualPath::data("nested/config.toml").unwrap(),
            ]),
            Err(CaptureError::DepthBudgetExceeded(_))
        ));
        assert_eq!(spec, prior);
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
                    owner: RepositoryActionOwner::RepositoryStateMutation,
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
                    owner: RepositoryActionOwner::RepositoryStateMutation,
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
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::File {
                    identity,
                    mode: FileMode::Regular,
                },
            }],
        )
        .is_ok());
    }

    #[test]
    fn test_delta_refuses_every_action_targeting_the_worktree_bootstrap_lock() {
        use crate::repository_state::BOOTSTRAP_LOCK_FILE;

        let identity = EntryIdentity::for_bytes("42:1", b"lock").unwrap();
        let file = ExpectedPreimage::File {
            identity: identity.clone(),
            mode: FileMode::Regular,
        };
        let lock = VirtualPath::worktree(BOOTSTRAP_LOCK_FILE).unwrap();
        let below = VirtualPath::worktree(format!("{BOOTSTRAP_LOCK_FILE}/stage")).unwrap();
        let actions = [
            RepositoryAction::WriteFile {
                path: lock.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: file.clone(),
                bytes: b"clobber".to_vec(),
                mode: FileMode::Regular,
            },
            RepositoryAction::DeleteFile {
                path: lock.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: file.clone(),
            },
            RepositoryAction::SetMode {
                path: lock.clone(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: file,
                mode: FileMode::Executable,
            },
            RepositoryAction::CreateDirectory {
                path: below,
                owner: RepositoryActionOwner::RepositoryInit,
                expected: ExpectedPreimage::Absent,
            },
        ];
        for action in actions {
            assert!(
                matches!(
                    RepositoryDelta::new(&layout(), vec![action.clone()]),
                    Err(DeltaError::EngineControlTarget(target)) if target == *action.path()
                ),
                "{action:?} must not reach the lock file",
            );
        }

        // The refusal covers that path alone: a worktree name merely prefixed by
        // it is ordinary content.
        assert!(RepositoryDelta::new(
            &layout(),
            vec![RepositoryAction::WriteFile {
                path: VirtualPath::worktree(format!("{BOOTSTRAP_LOCK_FILE}ed")).unwrap(),
                owner: RepositoryActionOwner::RepositoryStateMutation,
                expected: ExpectedPreimage::Absent,
                bytes: b"ordinary".to_vec(),
                mode: FileMode::Regular,
            }],
        )
        .is_ok());
    }

    #[test]
    fn test_delta_refuses_every_event_log_action_but_a_positioned_append() {
        let log = VirtualPath::EVENTS;
        let file = ExpectedPreimage::File {
            identity: EntryIdentity::for_bytes("42:2", b"log").unwrap(),
            mode: FileMode::Regular,
        };
        let refused = [
            RepositoryAction::write_file(
                log.clone(),
                RepositoryActionOwner::RepositoryStateMutation,
                file.clone(),
                b"rewritten".to_vec(),
                FileMode::Regular,
            ),
            RepositoryAction::delete_file(
                log.clone(),
                RepositoryActionOwner::RepositoryStateMutation,
                file.clone(),
            ),
            RepositoryAction::set_mode(
                log.clone(),
                RepositoryActionOwner::RepositoryStateMutation,
                file.clone(),
                FileMode::Executable,
            ),
            RepositoryAction::create_directory(
                log,
                RepositoryActionOwner::RepositoryInit,
                ExpectedPreimage::Absent,
            ),
        ];
        for action in refused {
            assert!(
                matches!(
                    RepositoryDelta::new(&layout(), vec![action.clone()]),
                    Err(DeltaError::InvalidAction(_))
                ),
                "{action:?} must not reach the event log",
            );
        }
        assert!(RepositoryDelta::new(
            &layout(),
            vec![RepositoryAction::append_events(
                RepositoryActionOwner::RepositoryStateMutation,
                file,
                3,
                b"line\n".to_vec()
            )],
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
        let set_mode =
            |path: VirtualPath, owner: RepositoryActionOwner, expected: ExpectedPreimage| {
                RepositoryAction::SetMode {
                    path,
                    owner,
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
                        RepositoryActionOwner::RepositoryStateMutation,
                        file(&shared)
                    ),
                    set_mode(
                        VirtualPath::data("shared").unwrap(),
                        RepositoryActionOwner::ProfileApplication,
                        file(&shared)
                    ),
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
                    set_mode(
                        VirtualPath::data("a").unwrap(),
                        RepositoryActionOwner::RepositoryStateMutation,
                        file(&shared)
                    ),
                    set_mode(
                        VirtualPath::data("b").unwrap(),
                        RepositoryActionOwner::ProfileApplication,
                        file(&shared)
                    ),
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
                    RepositoryActionOwner::RepositoryStateMutation,
                    file(&shared)
                ),
                set_mode(
                    VirtualPath::data("shared").unwrap(),
                    RepositoryActionOwner::ProfileApplication,
                    file(&other)
                ),
            ],
        )
        .is_ok());

        // Absent preimages carry no physical identity and never collide.
        assert!(RepositoryDelta::new(
            &layout(),
            vec![
                RepositoryAction::WriteFile {
                    path: VirtualPath::worktree("fresh-a").unwrap(),
                    owner: RepositoryActionOwner::RepositoryStateMutation,
                    expected: ExpectedPreimage::Absent,
                    bytes: b"a".to_vec(),
                    mode: FileMode::Regular,
                },
                RepositoryAction::WriteFile {
                    path: VirtualPath::data("fresh-b").unwrap(),
                    owner: RepositoryActionOwner::ProfileApplication,
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
            DirectoryListing::new(["../escape".into()]),
            Err(CaptureError::InvalidListingName(_))
        ));
        assert!(matches!(
            PinnedDocumentEvidence::new(
                "HEAD",
                "README.md",
                PinnedSourceClass::GitObject,
                RepositoryTargetKind::File,
                Some("commit".into()),
                Some("blob".into()),
                Some(EntryIdentity::for_bytes("blob", b"bytes").unwrap()),
                None,
                None,
            ),
            Err(CaptureError::InvalidPinnedEvidence(_))
        ));

        let path = VirtualPath::data("index.json").unwrap();
        assert!(matches!(
            TargetClaim::new(&layout(), path.clone(), ""),
            Err(DeltaError::InvalidClaimOwner)
        ));
        assert!(matches!(
            RepositoryDelta::new(
                &layout(),
                vec![RepositoryAction::CreateDirectory {
                    path,
                    owner: RepositoryActionOwner::RepositoryStateMutation,
                    expected: ExpectedPreimage::File {
                        identity: EntryIdentity::for_bytes("occupant", b"metadata").unwrap(),
                        mode: FileMode::Regular,
                    },
                }],
            ),
            Err(DeltaError::InvalidAction(_))
        ));
    }

    proptest::proptest! {
        /// Growth agrees with validating the complete resulting declaration.
        #[test]
        fn prop_capture_spec_growth_matches_whole_spec_validation(
            max_depth in 0usize..6,
            max_listings in 0usize..6,
            existing_paths in proptest::collection::vec((0usize..6, 0usize..16, 0u8..3), 0..8),
            existing_listings in proptest::collection::vec((0usize..6, 0usize..16), 0..8),
            whole_batch in proptest::collection::vec((0usize..8, 0usize..16), 0..8),
            tail_request in (0usize..8, 0usize..16),
            kind_batch in proptest::collection::vec((0usize..8, 0usize..16), 0..8),
            listing_batch in proptest::collection::vec((0usize..8, 0usize..16), 0..8),
        ) {
            let budget = CaptureBudget {
                max_listings,
                max_bytes: 0,
                max_depth,
            };
            let mut prior = CaptureSpec::new([], budget).unwrap();
            for (depth, id, mode) in existing_paths {
                if depth <= max_depth {
                    let path = generated_data_path(depth, id);
                    prior.paths.insert(path.clone());
                    match mode {
                        1 => { prior.tails.insert(path); }
                        2 => { prior.kinds.insert(path); }
                        _ => {}
                    }
                }
            }
            for (depth, id) in existing_listings.into_iter().take(max_listings) {
                if depth <= max_depth {
                    prior.listings.insert(generated_data_path(depth, id));
                }
            }
            proptest::prop_assert!(prior.validate().is_ok());

            let whole_batch = whole_batch
                .into_iter()
                .map(|(depth, id)| generated_data_path(depth, id))
                .collect::<Vec<_>>();
            let mut expected = grown_spec(prior.clone(), 0, &whole_batch);
            let mut actual = prior.clone();
            let accepted = actual.discover_paths(whole_batch).is_ok();
            assert_growth_matches_whole_validation(&prior, expected, actual, accepted)?;

            let tail_request = generated_data_path(tail_request.0, tail_request.1);
            expected = grown_spec(prior.clone(), 1, std::slice::from_ref(&tail_request));
            actual = prior.clone();
            let accepted = actual.discover_tail(tail_request).is_ok();
            assert_growth_matches_whole_validation(&prior, expected, actual, accepted)?;

            let kind_batch = kind_batch
                .into_iter()
                .map(|(depth, id)| generated_data_path(depth, id))
                .collect::<Vec<_>>();
            expected = grown_spec(prior.clone(), 2, &kind_batch);
            actual = prior.clone();
            let accepted = actual.discover_kinds(kind_batch).is_ok();
            assert_growth_matches_whole_validation(&prior, expected, actual, accepted)?;

            let listing_batch = listing_batch
                .into_iter()
                .map(|(depth, id)| generated_data_path(depth, id))
                .collect::<Vec<_>>();
            expected = grown_spec(prior.clone(), 3, &listing_batch);
            actual = prior.clone();
            let accepted = actual.discover_listings(listing_batch).is_ok();
            assert_growth_matches_whole_validation(&prior, expected, actual, accepted)?;
        }

        /// Captured bytes carry exactly the identity their bytes hash to.
        #[test]
        fn prop_captured_bytes_identity_matches_its_bytes(
            object in "[a-z][a-z0-9-]{0,15}",
            bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..256),
        ) {
            let expected = EntryIdentity::for_bytes(object.clone(), &bytes).unwrap();
            let captured = CapturedBytes::new(object, bytes.clone()).unwrap();
            proptest::prop_assert_eq!(captured.identity().sha256(), expected.sha256());
            proptest::prop_assert_eq!(captured.identity().byte_size(), expected.byte_size());
            proptest::prop_assert_eq!(captured.bytes(), bytes.as_slice());
        }
    }
}
