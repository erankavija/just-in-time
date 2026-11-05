//! Layout-aware repository capture and mutation sessions.
//!
//! Session orchestration lives here; an apply publishes through one redo record
//! and an opening session applies the record a crash left. A delta creating an
//! absent data root is the exception, published by `init_publish` because the
//! record's home is inside the root that does not exist yet.

use crate::domain::relative_path::RootRelativePath;
use crate::repository_state::{
    CaptureError, CaptureSpec, CapturedAnswers, CapturedBytes, DirectoryListing, EntryIdentity,
    EventLogTail, ExpectedPreimage, FileMode, ImageToken, MaterializationPlan,
    PinnedDocumentEvidence, PinnedSourceClass, ReadMode, RepositoryAction, RepositoryDelta,
    RepositoryEntry, RepositoryImage, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryRootEvidence, VirtualPath,
};
use crate::storage::capability_fs::{
    capability_metadata_identity, open_capability_parent, open_descendant_dir_nofollow,
    open_regular_file_nofollow, CapabilityFsError, CapabilityRoots,
};
use crate::storage::init_publish;
use crate::storage::memory::{append_events, MemoryRepositoryState};
use crate::storage::{
    commit_redo_record, EventAppend, FileLocker, InMemoryStorage, IssueStore, JsonFileStorage,
    MemoryRedoApplier, MutationInterruptionInjector, MutationPoint, PutFile, RedoApplier,
    RedoApplyError, RedoRecord, RepoWriteGuard, SessionRedoInterruptions, RECORD_FILE_NAME,
    RECORD_SCRATCH_DIR,
};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Successful application of one exact repository delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryApplyOutcome {
    /// Number of normalized actions applied.
    pub actions_applied: usize,
}

/// Residue one session open carried forward.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryDispatchReport {
    /// Leftover redo records applied: one, or none, because a record is the
    /// whole of one mutation.
    pub applied_records: usize,
}

impl RecoveryDispatchReport {
    /// Records applied.
    pub fn recovered_count(&self) -> usize {
        self.applied_records
    }
}

/// Storage/session failures with retryable conflicts kept distinguishable.
#[derive(Debug, thiserror::Error)]
pub enum RepositoryStateStoreError {
    #[error(transparent)]
    Layout(#[from] RepositoryLayoutError),
    #[error(transparent)]
    Capture(#[from] CaptureError),
    #[error("repository changed after capture: {path}")]
    RetryableConflict { path: String },
    #[error("selected data-root destination became occupied: {path}")]
    OccupiedDataRoot { path: PathBuf },
    /// A redo record lives inside the data root, so a mutation writing worktree
    /// targets while creating no root has no commit point; publishing it would
    /// leave partly written files nothing can replay.
    #[error("mutation writes worktree targets and creates no data root at {path}, so no redo record can commit it")]
    UncommittableWorktreeMutation { path: PathBuf },
    /// A redo record over a present data root appends no event, so the event
    /// log's length would not count its commit.
    #[error("redo record over a present data root appends no event")]
    RecordWithoutEventAppend,
    #[error("repository recovery or publication failed: {0}")]
    Publication(#[source] anyhow::Error),
    #[error("leftover redo record {path} is not readable: {source}")]
    UnreadableRedoRecord {
        path: String,
        #[source]
        source: crate::storage::RedoRecordError,
    },
    #[error("applying the leftover redo record failed: {0}")]
    RedoApply(#[source] Box<RedoApplyError>),
    #[error("unsafe repository mutation target: {0}")]
    UnsafeTarget(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<CapabilityFsError> for RepositoryStateStoreError {
    fn from(error: CapabilityFsError) -> Self {
        match error {
            CapabilityFsError::UnsafeTarget(target) => Self::UnsafeTarget(target),
            CapabilityFsError::RetryableConflict { path } => Self::RetryableConflict { path },
            CapabilityFsError::Io(error) => Self::Io(error),
        }
    }
}

/// Whether session open failed on the leftover redo record: it did not parse,
/// or applying it failed.
pub(crate) fn is_leftover_record_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<RepositoryStateStoreError>(),
            Some(
                RepositoryStateStoreError::UnreadableRedoRecord { .. }
                    | RepositoryStateStoreError::RedoApply(_)
            )
        )
    })
}

/// Whether this failure is a lock wait that expired rather than an outcome the
/// session decided.
///
/// A session opens behind the worktree-bootstrap, bootstrap and repository
/// locks, each of which reports a caller still queued as
/// [`LockTimeout`](crate::storage::lock::LockTimeout) carried through
/// [`RepositoryStateStoreError::Other`]. A caller that must tell "I have not
/// reached the critical section yet" from "the session refused this" reads that
/// distinction here; asking again is what puts the question back to the store.
pub(crate) fn is_lock_timeout(error: &RepositoryStateStoreError) -> bool {
    matches!(
        error,
        RepositoryStateStoreError::Other(error) if crate::storage::lock::is_lock_timeout(error)
    )
}

/// Whether an advisory read, such as the archive citation scan, may skip this
/// failure. Semantic failures and non-permission I/O stay hard.
pub(crate) fn is_advisory_permission_denied(error: &RepositoryStateStoreError) -> bool {
    matches!(
        error,
        RepositoryStateStoreError::Io(error)
            if error.kind() == ErrorKind::PermissionDenied
    )
}

/// Guards that a storage instance is reentered only for one canonical layout.
///
/// A retained CLI session holds the reentrant lock chain while dispatch opens a
/// second session; that reentry is legitimate only for the same selected roots.
/// A different worktree or data root while a session is live is a programming
/// error the session boundary rejects rather than silently serving stale roots.
/// Publishing a selected data root that was absent keeps those roots and
/// renews their evidence, so the publishing session
/// [rebinds](Self::rebind_published_root) what is tracked to the layout it
/// published.
#[derive(Default)]
pub(crate) struct ActiveLayoutTracker(Mutex<Option<(RepositoryLayout, usize)>>);

impl ActiveLayoutTracker {
    fn enter(&self, layout: &RepositoryLayout) -> Result<(), RepositoryStateStoreError> {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match slot.as_mut() {
            Some((active, depth)) if active == layout => {
                *depth += 1;
                Ok(())
            }
            Some((active, _)) => Err(RepositoryStateStoreError::RetryableConflict {
                path: format!(
                    "retained session holds layout at {} != requested {}",
                    active.data_root().display(),
                    layout.data_root().display()
                ),
            }),
            None => {
                *slot = Some((layout.clone(), 1));
                Ok(())
            }
        }
    }

    pub(crate) fn check_binding(
        &self,
        layout: &RepositoryLayout,
    ) -> Result<(), RepositoryStateStoreError> {
        let slot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((active, _)) = slot.as_ref() {
            if active.worktree_root() != layout.worktree_root()
                || active.data_root() != layout.data_root()
            {
                return Err(RepositoryStateStoreError::RetryableConflict {
                    path: layout.worktree_root().display().to_string(),
                });
            }
        }
        Ok(())
    }

    /// Track `published` as the layout for the roots it publishes.
    ///
    /// An absent data root is bound to its parent plus the name it will take,
    /// and the published root is bound to itself, so publication renews the
    /// evidence for roots nothing selected differently. Every holder of the
    /// pre-publication layout — a retained startup session outermost — would
    /// otherwise refuse the next session over the repository this one just
    /// created. Only the roots' evidence is renewed: a `published` naming other
    /// roots is not this transition and leaves what is tracked alone, so the
    /// guard still rejects a genuinely different selection.
    fn rebind_published_root(&self, published: &RepositoryLayout) {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((active, _)) = slot.as_mut() {
            if active.worktree_root() == published.worktree_root()
                && active.data_root() == published.data_root()
            {
                *active = published.clone();
            }
        }
    }

    fn leave(&self) {
        let mut slot = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, depth)) = slot.as_mut() {
            *depth -= 1;
            if *depth == 0 {
                *slot = None;
            }
        }
    }
}

/// Releases one reentry level on the owning store's [`ActiveLayoutTracker`].
struct LayoutReentry(Arc<ActiveLayoutTracker>);

impl Drop for LayoutReentry {
    fn drop(&mut self) {
        self.0.leave();
    }
}

/// The capture half of a session: what a read derives from.
pub trait RepositoryReadSession {
    /// Canonical layout held by this session.
    fn layout(&self) -> &RepositoryLayout;
    /// Capture one complete bounded image.
    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError>;
    /// Grow `image` to `spec`, reading only the requests `image` does not
    /// already answer the same way; what it read is kept, never re-read.
    fn extend(
        &mut self,
        image: RepositoryImage,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError>;
    /// The event log's whole bytes, read without following a link, or `None`
    /// when absent. Outside any [`CaptureSpec`], so no image holds the whole
    /// log; whole-repository record fidelity is its one reader.
    fn read_history_log(&mut self) -> Result<Option<Vec<u8>>, RepositoryStateStoreError>;
}

/// Opaque mutation session retaining serialization through capture and apply.
pub trait RepositoryMutationSession: RepositoryReadSession {
    /// Recovery completed before this session became available for capture.
    fn recovery_report(&self) -> &RecoveryDispatchReport;
    /// Publish one complete semantic plan, refused if the files it writes moved.
    fn apply(
        &mut self,
        plan: &MaterializationPlan,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError>;
}

/// The whole log from its entry: a link or any non-file is unsafe, not absent.
fn history_log_bytes(
    entry: Option<&RepositoryEntry>,
) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
    match entry {
        Some(RepositoryEntry::File { content, .. }) => Ok(Some(content.bytes().to_vec())),
        None | Some(RepositoryEntry::Absent) => Ok(None),
        _ => Err(RepositoryStateStoreError::UnsafeTarget(format!(
            "{:?}",
            VirtualPath::EVENTS
        ))),
    }
}

/// The whole JSON-backed log, through the session's capability roots.
fn json_history_log(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
    revalidate_layout_capabilities(layout, roots)?;
    history_log_bytes(Some(&inspect_capability_entry(
        layout,
        roots,
        &VirtualPath::EVENTS,
        ReadMode::Whole,
    )?))
}

/// The whole in-memory log.
fn memory_history_log(
    storage: &InMemoryStorage,
) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
    history_log_bytes(storage.repository_state().entries.get(&VirtualPath::EVENTS))
}

/// Whether a lock-free read's capture is one whole repository version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadAdmission {
    /// No commit crossed the capture.
    Granted,
    /// A redo record was present at open or at admission.
    Busy,
    /// The event log changed, the data root's presence changed, or a root was
    /// replaced.
    Moved,
}

/// A lock-free read session (`@/charter/D-22`): its capture counts only once
/// [`admit`](Self::admit) grants it.
pub trait OptimisticReadSession: RepositoryReadSession {
    /// Judge the captures made so far against the repository now.
    ///
    /// Sound because every record over a present data root appends an event
    /// and is committed before its first live write: a commit whose live write
    /// fell inside the capture either left its record in place or grew the log.
    fn admit(&self) -> Result<ReadAdmission, RepositoryStateStoreError>;
}

/// Storage backend capable of one capture/apply mutation boundary.
pub trait RepositoryStateStore {
    /// Open a mutation session for exactly `layout`, applying any leftover redo
    /// record first.
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError>;

    /// Open a read session for exactly `layout`, taking no lock and applying
    /// nothing.
    fn open_read_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn OptimisticReadSession>, RepositoryStateStoreError>;
}

/// What a read session observes of commits: whether a record is present, then
/// the event log's length and tail.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CommitObservation {
    data_root: bool,
    record: bool,
    log: Option<EventLogTail>,
}

impl CommitObservation {
    /// The admission of a read opened at `self` and observing `now`; a
    /// retryable conflict while observing is a move.
    fn admit(
        &self,
        now: Result<CommitObservation, RepositoryStateStoreError>,
    ) -> Result<ReadAdmission, RepositoryStateStoreError> {
        match now {
            _ if self.record => Ok(ReadAdmission::Busy),
            Ok(now) if now.record => Ok(ReadAdmission::Busy),
            Ok(now) if now == *self => Ok(ReadAdmission::Granted),
            Ok(_) | Err(RepositoryStateStoreError::RetryableConflict { .. }) => {
                Ok(ReadAdmission::Moved)
            }
            Err(error) => Err(error),
        }
    }

    /// Refuse a capture over a record the open saw: its admission is `Busy`.
    fn ensure_capturable(&self) -> Result<(), RepositoryStateStoreError> {
        match self.record {
            true => Err(RepositoryStateStoreError::RetryableConflict {
                path: format!("{RECORD_SCRATCH_DIR}/{RECORD_FILE_NAME}"),
            }),
            false => Ok(()),
        }
    }
}

/// The record [`read_leftover_record`] found, applied and cleared: one, or
/// none.
///
/// # Errors
///
/// A record this open cannot apply stays where it is, for the next one.
fn apply_leftover_record(
    roots: &CapabilityRoots,
    record: Option<RedoRecord>,
    interruptions: Arc<dyn MutationInterruptionInjector>,
) -> Result<usize, RepositoryStateStoreError> {
    let Some(record) = record else {
        return Ok(0);
    };
    RedoApplier::new(SessionRedoInterruptions::new(interruptions))
        .apply(roots, &record)
        .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))?;
    Ok(1)
}

/// Runs before the record is applied, so a record this open refuses is refused
/// over an untouched repository.
fn read_leftover_record(
    roots: &CapabilityRoots,
) -> Result<Option<RedoRecord>, RepositoryStateStoreError> {
    let Some(mut file) = open_leftover_record(roots)? else {
        return Ok(None);
    };
    let relative = Path::new(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME);
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    RedoRecord::parse(&bytes).map(Some).map_err(|source| {
        RepositoryStateStoreError::UnreadableRedoRecord {
            path: relative.display().to_string(),
            source,
        }
    })
}

/// The leftover record, opened by name. A repository holding none pays one look
/// at that name: the parent walk stops at an absent scratch directory and the
/// open at an absent record, and nothing there is enumerated or otherwise read.
fn open_leftover_record(
    roots: &CapabilityRoots,
) -> Result<Option<cap_std::fs::File>, RepositoryStateStoreError> {
    let Some(data) = roots.root(RepositoryRootClass::Data) else {
        return Ok(None);
    };
    let relative = Path::new(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME);
    let Some((scratch, leaf)) = open_capability_parent(data, &relative)? else {
        return Ok(None);
    };
    match open_regular_file_nofollow(&scratch, &leaf) {
        Ok(file) => Ok(Some(file)),
        Err(CapabilityFsError::Io(error)) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// [`apply_leftover_record`] over the record the in-memory backend holds.
fn apply_memory_leftover_record(
    storage: &InMemoryStorage,
    record: Option<RedoRecord>,
    interruptions: Arc<dyn MutationInterruptionInjector>,
) -> Result<usize, RepositoryStateStoreError> {
    let Some(record) = record else {
        return Ok(0);
    };
    MemoryRedoApplier::new(SessionRedoInterruptions::new(interruptions))
        .apply(storage, &record)
        .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))?;
    Ok(1)
}

/// Acquire no-follow root evidence and construct the canonical layout.
pub fn discover_repository_layout(
    worktree: impl AsRef<Path>,
    data: impl AsRef<Path>,
) -> Result<RepositoryLayout, RepositoryStateStoreError> {
    let worktree = lexical_absolute(worktree.as_ref())?;
    let data = lexical_absolute(data.as_ref())?;
    RepositoryLayout::new(
        discover_root_evidence(&worktree, false)?,
        discover_root_evidence(&data, true)?,
    )
    .map_err(Into::into)
}

struct JsonMutationSession {
    layout: RepositoryLayout,
    storage: JsonFileStorage,
    recovery_report: RecoveryDispatchReport,
    roots: CapabilityRoots,
    /// The data root's parent, with the leaf it is to be published at: the
    /// publisher of an absent root stages beside that leaf and renames onto it.
    data_parent: Dir,
    data_leaf: String,
    /// Worktree-root bootstrap lock, held outermost so the worktree-side
    /// bootstrap namespace is serialized across disjoint-data-root sessions.
    _worktree_bootstrap_guard: RepoWriteGuard,
    _bootstrap_guard: RepoWriteGuard,
    _repository_guard: Option<RepoWriteGuard>,
    events_lock: Option<(FileLocker, PathBuf)>,
    _reentry: LayoutReentry,
    _order_guard: crate::storage::guard_order::RepositoryOrderGuard,
    captured: Option<ImageToken>,
}

struct MemoryMutationSession {
    layout: RepositoryLayout,
    storage: InMemoryStorage,
    recovery_report: RecoveryDispatchReport,
    _guard: RepoWriteGuard,
    _reentry: LayoutReentry,
    _order_guard: crate::storage::guard_order::RepositoryOrderGuard,
    captured: Option<ImageToken>,
}

/// A lock-free read over the filesystem, holding no guard.
struct JsonReadSession {
    layout: RepositoryLayout,
    storage: JsonFileStorage,
    roots: CapabilityRoots,
    opened: CommitObservation,
}

/// A lock-free read over the in-memory backend's entry map.
struct MemoryReadSession {
    layout: RepositoryLayout,
    storage: InMemoryStorage,
    opened: CommitObservation,
}

impl JsonFileStorage {
    /// The layout a session opens: the one this storage is bound to when it
    /// names the same roots, refused when it names others.
    fn session_layout(
        &self,
        layout: RepositoryLayout,
    ) -> Result<RepositoryLayout, RepositoryStateStoreError> {
        let layout = match self.configured_layout() {
            Ok(configured)
                if configured.worktree_root() == layout.worktree_root()
                    && configured.data_root() == layout.data_root() =>
            {
                configured
            }
            Ok(configured) => {
                return Err(RepositoryStateStoreError::RetryableConflict {
                    path: format!(
                        "storage is bound to {} and cannot open {}",
                        configured.worktree_root().display(),
                        layout.worktree_root().display()
                    ),
                })
            }
            Err(_) => layout,
        };
        if lexical_absolute(self.root())? != layout.data_root() {
            return Err(
                RepositoryLayoutError::OutsideRepositoryRoots(self.root().to_path_buf()).into(),
            );
        }
        Ok(layout)
    }
}

/// The record probe, then the event log's tail: a commit removes its record
/// only after its append.
fn observe_json_commits(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
) -> Result<CommitObservation, RepositoryStateStoreError> {
    let record = open_leftover_record(roots)?.is_some();
    Ok(CommitObservation {
        data_root: roots.root(RepositoryRootClass::Data).is_some(),
        record,
        log: live_event_log_tail(layout, roots)?,
    })
}

/// [`observe_json_commits`] over the in-memory backend, read under one lock
/// of its state.
fn observe_memory_commits(storage: &InMemoryStorage) -> CommitObservation {
    let state = storage.repository_state();
    CommitObservation {
        data_root: state.data_root_exists,
        record: state.record.is_some(),
        log: match state.entries.get(&VirtualPath::EVENTS) {
            Some(RepositoryEntry::File { content, .. }) => {
                Some(EventLogTail::of_bytes(content.bytes()))
            }
            _ => None,
        },
    }
}

impl RepositoryStateStore for JsonFileStorage {
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        let layout = self.session_layout(layout)?;
        // A retained session may reenter the reentrant lock chain only for the
        // same canonical layout.
        self.active_mutation_layout().enter(&layout)?;
        let reentry = LayoutReentry(self.active_mutation_layout());
        self.session_layout(layout.clone())?;
        // Mark a repository mutation session as held on this thread so a later
        // attempt to enter claim coordination (reverse order) is rejected.
        let order_guard = crate::storage::guard_order::RepositoryOrderGuard::enter();
        let interruptions = self.mutation_interruptions();

        // Serialize the worktree-side bootstrap namespace at the WORKTREE root,
        // outermost, so two sessions sharing this worktree with data roots under
        // different parents cannot concurrently publish into it. For a nested
        // repo this is the same reentrant lock as the bootstrap lock below; for
        // a disjoint root it is a distinct outer lock. Acquired before any
        // capability discovery.
        let worktree_bootstrap_guard =
            self.acquire_worktree_bootstrap_lock(layout.worktree_root())?;
        // Fixed order: bootstrap serialization is acquired BEFORE any capability
        // discovery, so a concurrent creator cannot publish the selected data
        // root between discovery and the lock and leave the session bound to a
        // root it never serialized against.
        let bootstrap_guard = self.acquire_bootstrap_write_lock()?;
        revalidate_layout(&layout)?;

        let (roots, data_parent, data_leaf) = CapabilityRoots::open(&layout)?;
        let leftover_record = read_leftover_record(&roots)?;
        interruptions.check(&MutationPoint::SessionOpen)?;

        let (repository_guard, applied_records) = if roots.root(RepositoryRootClass::Data).is_some()
        {
            let repository_guard = self.acquire_repo_write_lock_raw()?;
            let _events_guard = self.acquire_events_write_lock()?;
            // Inside both locks, because the record writes repository state.
            // The record lives inside the data root, so the absent-root branch
            // has none to apply.
            let applied_records =
                apply_leftover_record(&roots, leftover_record, Arc::clone(&interruptions))?;
            (Some(repository_guard), applied_records)
        } else {
            (None, 0)
        };
        let events_lock = roots
            .root(RepositoryRootClass::Data)
            .map(|_| self.events_lock_spec());

        self.configure_repository_layout(&layout)?;
        Ok(Box::new(JsonMutationSession {
            layout,
            storage: self.clone(),
            recovery_report: RecoveryDispatchReport { applied_records },
            roots,
            data_parent,
            data_leaf,
            _worktree_bootstrap_guard: worktree_bootstrap_guard,
            _bootstrap_guard: bootstrap_guard,
            _repository_guard: repository_guard,
            events_lock,
            _reentry: reentry,
            _order_guard: order_guard,
            captured: None,
        }))
    }

    fn open_read_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn OptimisticReadSession>, RepositoryStateStoreError> {
        let layout = self.session_layout(layout)?;
        revalidate_layout(&layout)?;
        let (roots, _, _) = CapabilityRoots::open(&layout)?;
        self.mutation_interruptions()
            .check(&MutationPoint::ReadSessionOpen)?;
        let opened = observe_json_commits(&layout, &roots)?;
        Ok(Box::new(JsonReadSession {
            layout,
            storage: self.clone(),
            roots,
            opened,
        }))
    }
}

impl RepositoryStateStore for InMemoryStorage {
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
        self.active_mutation_layout().enter(&layout)?;
        let reentry = LayoutReentry(self.active_mutation_layout());
        let order_guard = crate::storage::guard_order::RepositoryOrderGuard::enter();
        let guard = self.acquire_repo_write_lock()?;
        let interruptions = self.mutation_interruptions();
        interruptions.check(&MutationPoint::SessionOpen)?;
        let (data_exists, leftover_record) = {
            let state = self.repository_state();
            (state.data_root_exists, state.record.clone())
        };
        // Where the JSON open applies it, gated on the data root for the same
        // reason: a record's home is inside it.
        let applied_records = match data_exists {
            true => apply_memory_leftover_record(self, leftover_record, interruptions)?,
            false => 0,
        };
        Ok(Box::new(MemoryMutationSession {
            layout,
            storage: self.clone(),
            recovery_report: RecoveryDispatchReport { applied_records },
            _guard: guard,
            _reentry: reentry,
            _order_guard: order_guard,
            captured: None,
        }))
    }

    fn open_read_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn OptimisticReadSession>, RepositoryStateStoreError> {
        self.mutation_interruptions()
            .check(&MutationPoint::ReadSessionOpen)?;
        Ok(Box::new(MemoryReadSession {
            layout,
            storage: self.clone(),
            opened: observe_memory_commits(self),
        }))
    }
}

impl RepositoryReadSession for JsonReadSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.capture_over(CapturedAnswers::default(), spec)
    }

    fn extend(
        &mut self,
        image: RepositoryImage,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.capture_over(image.into_answers(&spec), spec)
    }

    fn read_history_log(&mut self) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
        self.opened.ensure_capturable()?;
        json_history_log(&self.layout, &self.roots)
    }
}

impl JsonReadSession {
    fn capture_over(
        &mut self,
        prior: CapturedAnswers,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.opened.ensure_capturable()?;
        capture_capability_image(&self.layout, &self.roots, &self.storage, spec, prior)
    }
}

impl OptimisticReadSession for JsonReadSession {
    fn admit(&self) -> Result<ReadAdmission, RepositoryStateStoreError> {
        self.opened.admit(
            revalidate_layout_capabilities(&self.layout, &self.roots)
                .and_then(|()| observe_json_commits(&self.layout, &self.roots)),
        )
    }
}

impl RepositoryReadSession for MemoryReadSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.opened.ensure_capturable()?;
        capture_memory_image(
            &self.layout,
            &self.storage.repository_state(),
            spec,
            CapturedAnswers::default(),
        )
    }

    fn extend(
        &mut self,
        image: RepositoryImage,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.opened.ensure_capturable()?;
        let prior = image.into_answers(&spec);
        capture_memory_image(&self.layout, &self.storage.repository_state(), spec, prior)
    }

    fn read_history_log(&mut self) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
        self.opened.ensure_capturable()?;
        memory_history_log(&self.storage)
    }
}

impl OptimisticReadSession for MemoryReadSession {
    fn admit(&self) -> Result<ReadAdmission, RepositoryStateStoreError> {
        self.opened.admit(Ok(observe_memory_commits(&self.storage)))
    }
}

impl RepositoryReadSession for JsonMutationSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.capture_over(CapturedAnswers::default(), spec)
    }

    fn extend(
        &mut self,
        image: RepositoryImage,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.capture_over(image.into_answers(&spec), spec)
    }

    fn read_history_log(&mut self) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
        json_history_log(&self.layout, &self.roots)
    }
}

impl RepositoryMutationSession for JsonMutationSession {
    fn recovery_report(&self) -> &RecoveryDispatchReport {
        &self.recovery_report
    }

    fn apply(
        &mut self,
        plan: &MaterializationPlan,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
        let image = plan.image();
        let delta = plan.publication();
        ensure_session_image(&self.layout, self.captured, image)?;
        ensure_delta_is_captured(image, delta)?;
        let interruptions = self.storage.mutation_interruptions();
        interruptions.check(&MutationPoint::BeforeCommitCheck)?;
        // Not part of the write set, and not covered by comparing it: a whole
        // root atomically replaced since capture leaves this session's handles
        // on the unlinked inode, which every comparison read through them still
        // agrees with.
        revalidate_layout_capabilities(&self.layout, &self.roots)?;
        // Recovery takes this finer lock only while it edits the event log.
        // Acquire it again for publication, rather than retaining it for the
        // whole session: ordinary command dispatch legitimately reenters the
        // repository guard and may append events while the startup session is
        // retained.
        let _events_guard = self
            .events_lock
            .as_ref()
            .map(|(locker, path)| locker.lock_exclusive(path))
            .transpose()?;
        let outcome = RepositoryApplyOutcome {
            actions_applied: delta.actions().len(),
        };
        match (
            self.roots.root(RepositoryRootClass::Data).is_some(),
            delta.actions().is_empty(),
        ) {
            (false, false) => self.publish_absent_data_root(image, delta, interruptions)?,
            (true, false) => self.publish_through_record(image, delta, interruptions)?,
            (_, true) => {}
        }
        self.captured = None;
        Ok(outcome)
    }
}

impl JsonMutationSession {
    fn capture_over(
        &mut self,
        prior: CapturedAnswers,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        revalidate_layout_capabilities(&self.layout, &self.roots)?;
        let image =
            capture_capability_image(&self.layout, &self.roots, &self.storage, spec, prior)?;
        self.captured = Some(image.token());
        Ok(image)
    }

    /// Publish `delta` as one redo record: derived from the delta, refused if
    /// its write set moved, made durable, applied, and removed.
    fn publish_through_record(
        &self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
        interruptions: Arc<dyn MutationInterruptionInjector>,
    ) -> Result<(), RepositoryStateStoreError> {
        let record = appending_record(image, delta)?;
        ensure_write_set_unchanged(&self.roots, image, &record)?;
        let root = self.roots.data_root().ok_or_else(|| {
            RepositoryStateStoreError::UnsafeTarget(self.layout.data_root().display().to_string())
        })?;
        commit_redo_record(root, &record).map_err(RepositoryStateStoreError::Publication)?;
        RedoApplier::new(SessionRedoInterruptions::new(interruptions))
            .apply(&self.roots, &record)
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))
    }

    /// Publish a delta whose data root does not exist yet: the new root through
    /// the init publisher, and its worktree targets through a redo record the
    /// stage carries into the published root, so an interruption after the
    /// rename is replayed by the next session that opens.
    ///
    /// A delta publishing no root is refused by
    /// [`ensure_committable_absent_root_delta`], so every delta that reaches the
    /// stage creates the root its record is committed into.
    fn publish_absent_data_root(
        &self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
        interruptions: Arc<dyn MutationInterruptionInjector>,
    ) -> Result<(), RepositoryStateStoreError> {
        ensure_committable_absent_root_delta(&self.layout, delta)?;
        let of_class = |class| {
            delta
                .actions()
                .iter()
                .filter(move |action| action.path().root_class() == class)
        };
        let record = redo_record_for(image, of_class(RepositoryRootClass::Worktree))?;
        ensure_write_set_unchanged(&self.roots, image, &record)?;
        let applier = RedoApplier::new(SessionRedoInterruptions::new(Arc::clone(&interruptions)));
        let redo = |outcome: Result<(), RedoApplyError>| {
            outcome.map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))
        };
        let stage = init_publish::stage_new_data_root(
            &self.data_parent,
            &self.data_leaf,
            of_class(RepositoryRootClass::Data),
        )?;
        let carries_worktree = of_class(RepositoryRootClass::Worktree).next().is_some();
        if carries_worktree {
            // Durable inside the stage, so publishing the root and committing
            // its worktree targets are one act: the rename carries the record
            // into the root, and an interruption between them leaves it there.
            commit_redo_record(&stage, &record).map_err(RepositoryStateStoreError::Publication)?;
        }
        interruptions.check(&MutationPoint::BeforeRootPublication)?;
        init_publish::publish_staged_data_root(
            &self.data_parent,
            &self.data_leaf,
            self.layout.data_root(),
        )?;
        let refreshed =
            discover_repository_layout(self.layout.worktree_root(), self.layout.data_root())?;
        // The published root is the layout for these roots from here on, for
        // every holder of the one this session opened with: the storage every
        // later session reads its layout from, and the reentry guard a
        // retained startup session left the pre-publication layout in. A
        // session opened after this one — the second package of a profile
        // closure applied to the repository initialization just created — is
        // otherwise refused for roots nothing changed.
        self.storage
            .active_mutation_layout()
            .rebind_published_root(&refreshed);
        self.storage.configure_repository_layout(&refreshed)?;
        match carries_worktree {
            // Through the published root's capabilities: the record the rename
            // carried in is removed from the scratch directory it now has.
            true => redo(applier.apply(&CapabilityRoots::open(&refreshed)?.0, &record)),
            false => Ok(()),
        }
    }
}

impl RepositoryReadSession for MemoryMutationSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.capture_over(CapturedAnswers::default(), spec)
    }

    fn extend(
        &mut self,
        image: RepositoryImage,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        self.capture_over(image.into_answers(&spec), spec)
    }

    fn read_history_log(&mut self) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
        memory_history_log(&self.storage)
    }
}

impl RepositoryMutationSession for MemoryMutationSession {
    fn recovery_report(&self) -> &RecoveryDispatchReport {
        &self.recovery_report
    }

    fn apply(
        &mut self,
        plan: &MaterializationPlan,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
        let image = plan.image();
        let delta = plan.publication();
        ensure_session_image(&self.layout, self.captured, image)?;
        ensure_delta_is_captured(image, delta)?;
        #[cfg(feature = "test-support")]
        if self.storage.consume_repository_state_apply_conflict() {
            return Err(RepositoryStateStoreError::RetryableConflict {
                path: "injected memory write-set conflict".into(),
            });
        }
        let interruptions = self.storage.mutation_interruptions();
        interruptions.check(&MutationPoint::BeforeCommitCheck)?;
        let outcome = RepositoryApplyOutcome {
            actions_applied: delta.actions().len(),
        };
        let data_root_exists = self.storage.repository_state().data_root_exists;
        match (data_root_exists, delta.actions().is_empty()) {
            (false, false) => publish_absent_memory_root(&self.layout, &self.storage, delta)?,
            (true, false) => self.publish_through_record(image, delta, interruptions)?,
            (_, true) => {}
        }
        self.captured = None;
        Ok(outcome)
    }
}

impl MemoryMutationSession {
    fn capture_over(
        &mut self,
        prior: CapturedAnswers,
        spec: CaptureSpec,
    ) -> Result<RepositoryImage, RepositoryStateStoreError> {
        let state = self.storage.repository_state();
        let image = capture_memory_image(&self.layout, &state, spec, prior)?;
        drop(state);
        self.captured = Some(image.token());
        Ok(image)
    }

    /// The in-memory counterpart of [`JsonMutationSession::publish_through_record`],
    /// over the entry map.
    fn publish_through_record(
        &self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
        interruptions: Arc<dyn MutationInterruptionInjector>,
    ) -> Result<(), RepositoryStateStoreError> {
        let record = appending_record(image, delta)?;
        ensure_memory_write_set_unchanged(&self.storage.repository_state(), image, &record)?;
        self.storage
            .commit_record(&record)
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))?;
        MemoryRedoApplier::new(SessionRedoInterruptions::new(interruptions))
            .apply(&self.storage, &record)
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))
    }
}

/// Refuse a mutation over an absent data root that writes worktree targets and
/// creates no root.
///
/// The redo record's home is inside the root such a delta does not create, so
/// the mutation has no commit point: publishing it would leave partly written
/// worktree files with nothing to replay them. Both backends raise this, so a
/// caller driving either reads one refusal.
fn ensure_committable_absent_root_delta(
    layout: &RepositoryLayout,
    delta: &RepositoryDelta,
) -> Result<(), RepositoryStateStoreError> {
    let writes = |class| {
        delta
            .actions()
            .iter()
            .any(|action| action.path().root_class() == class)
    };
    match (
        writes(RepositoryRootClass::Data),
        writes(RepositoryRootClass::Worktree),
    ) {
        (false, true) => Err(RepositoryStateStoreError::UncommittableWorktreeMutation {
            path: layout.data_root().to_path_buf(),
        }),
        _ => Ok(()),
    }
}

/// Publish a delta whose data root does not exist yet, as the init publisher
/// publishes one on the filesystem: the record's home is inside the root the
/// delta creates, so this one delta reaches the entry map directly.
fn publish_absent_memory_root(
    layout: &RepositoryLayout,
    storage: &InMemoryStorage,
    delta: &RepositoryDelta,
) -> Result<(), RepositoryStateStoreError> {
    ensure_committable_absent_root_delta(layout, delta)?;
    let mut state = storage.repository_state();
    let mut candidate = state.clone();
    delta
        .actions()
        .iter()
        .try_for_each(|action| apply_memory_action(layout, &mut candidate, action))?;
    if candidate.data_root_exists {
        // The published root is a real entry, so a later capture of Data("")
        // reads a directory as it does through the filesystem's published root.
        candidate
            .entries
            .entry(VirtualPath::data("")?)
            .or_insert(RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("memory-directory:data-root", b"directory")?,
                mode: FileMode::Executable,
            });
    }
    state.entries = candidate.entries;
    state.data_root_exists = candidate.data_root_exists;
    Ok(())
}

fn ensure_session_image(
    layout: &RepositoryLayout,
    captured: Option<ImageToken>,
    image: &RepositoryImage,
) -> Result<(), RepositoryStateStoreError> {
    if image.layout() != layout || captured != Some(image.token()) {
        return Err(RepositoryStateStoreError::RetryableConflict {
            path: "image does not belong to this mutation session".into(),
        });
    }
    Ok(())
}

fn ensure_delta_is_captured(
    image: &RepositoryImage,
    delta: &RepositoryDelta,
) -> Result<(), RepositoryStateStoreError> {
    for action in delta.actions() {
        let captured = image.entry(action.path())?;
        if ExpectedPreimage::of(captured) != *action.expected() {
            return Err(RepositoryStateStoreError::RetryableConflict {
                path: format!("{:?}", action.path()),
            });
        }
    }
    Ok(())
}

/// Refuse a mutation whose write set moved after the mutation read it.
///
/// Compares the live occupant of every path `record` makes a directory at, puts
/// or deletes with what `captured` holds for it, and the event log as its length
/// together with
/// its last [`crate::repository_state::EVENT_LOG_TAIL_WINDOW`] bytes — the one answer for that path, so
/// the log's whole contents are never reread (`@/charter/D-17`). A path the
/// mutation read but does not write is left out, so a change there leaves the
/// mutation admissible; the repository write lock keeps concurrent writers of
/// repository state out of the window.
///
/// # Errors
///
/// [`RepositoryStateStoreError::RetryableConflict`] names the first write target
/// whose live state differs from the captured one, raised before anything
/// durable is written. A path the mutation never read surfaces as a capture
/// error, and an unreadable occupant as its I/O failure.
pub fn ensure_write_set_unchanged(
    roots: &CapabilityRoots,
    captured: &RepositoryImage,
    record: &RedoRecord,
) -> Result<(), RepositoryStateStoreError> {
    ensure_write_set_unchanged_against(
        captured,
        record,
        &|path| inspect_capability_entry(captured.layout(), roots, path, ReadMode::Whole),
        &|| live_event_log_tail(captured.layout(), roots),
    )
}

/// [`ensure_write_set_unchanged`] over the in-memory backend's entry map.
fn ensure_memory_write_set_unchanged(
    state: &MemoryRepositoryState,
    captured: &RepositoryImage,
    record: &RedoRecord,
) -> Result<(), RepositoryStateStoreError> {
    let entry = |path: &VirtualPath| {
        Ok(state
            .entries
            .get(path)
            .cloned()
            .unwrap_or(RepositoryEntry::Absent))
    };
    ensure_write_set_unchanged_against(captured, record, &entry, &|| {
        Ok(match state.entries.get(&VirtualPath::EVENTS) {
            Some(RepositoryEntry::File { content, .. }) => {
                Some(EventLogTail::of_bytes(content.bytes()))
            }
            _ => None,
        })
    })
}

/// The comparison both backends run, over the live readings each one supplies:
/// `live` for an ordinary write target and `live_log_tail` for the event log,
/// whose contents are never read whole.
fn ensure_write_set_unchanged_against(
    captured: &RepositoryImage,
    record: &RedoRecord,
    live: &dyn Fn(&VirtualPath) -> Result<RepositoryEntry, RepositoryStateStoreError>,
    live_log_tail: &dyn Fn() -> Result<Option<EventLogTail>, RepositoryStateStoreError>,
) -> Result<(), RepositoryStateStoreError> {
    record
        .make_directories()
        .iter()
        .chain(record.puts().iter().map(|put| &put.path))
        .chain(record.deletes().iter())
        .try_for_each(|path| ensure_target_unchanged(captured, path, live))?;
    if record.event_append().is_none() {
        return Ok(());
    }
    // An ordinary captured log is the only state carrying a tail to compare;
    // any other occupant is compared like any other write target.
    let tail = match captured.entry(&VirtualPath::EVENTS)? {
        RepositoryEntry::FileTail { tail, .. } => tail.clone(),
        _ => return ensure_target_unchanged(captured, &VirtualPath::EVENTS, live),
    };
    (live_log_tail()? == Some(tail))
        .then_some(())
        .ok_or_else(|| retryable_conflict(&VirtualPath::EVENTS))
}

/// Compare one write target's live occupant with the captured one, byte for byte.
fn ensure_target_unchanged(
    captured: &RepositoryImage,
    path: &VirtualPath,
    live: &dyn Fn(&VirtualPath) -> Result<RepositoryEntry, RepositoryStateStoreError>,
) -> Result<(), RepositoryStateStoreError> {
    (live(path)? == *captured.entry(path)?)
        .then_some(())
        .ok_or_else(|| retryable_conflict(path))
}

/// Every directory `delta` creates that no other action's path lies beneath.
///
/// The rest are created by the operation that lands under them, so carrying them
/// too would put an ancestor and its descendant in one record — the overlap
/// [`RedoRecord::new`] refuses (OD-7).
fn made_directories<'a>(
    actions: impl Iterator<Item = &'a RepositoryAction> + Clone,
) -> impl Iterator<Item = &'a VirtualPath> {
    let covering = actions.clone();
    actions
        .filter_map(|action| match action {
            RepositoryAction::CreateDirectory { path, .. } => Some(path),
            _ => None,
        })
        .filter(move |directory| {
            !covering
                .clone()
                .any(|action| lies_beneath(action.path(), directory))
        })
}

/// Whether `path` lies strictly beneath `directory`: the same root, and every
/// component of `directory` a prefix of `path`'s.
fn lies_beneath(path: &VirtualPath, directory: &VirtualPath) -> bool {
    path != directory
        && path.root_class() == directory.root_class()
        && path
            .relative()
            .as_path()
            .starts_with(directory.relative().as_path())
}

/// The redo record that publishes `delta` over the repository `captured` read.
///
/// A directory another action's path lies beneath is created by that action, so
/// only the rest reach the record as make-directory operations (OD-7); their own
/// ancestors are created on apply. A mode change republishes the captured file's
/// own bytes under the new mode, which the write-set comparison has just
/// confirmed are still the live ones.
fn redo_record_for<'a>(
    captured: &RepositoryImage,
    actions: impl Iterator<Item = &'a RepositoryAction> + Clone,
) -> Result<RedoRecord, RepositoryStateStoreError> {
    let make_directories = made_directories(actions.clone()).cloned().collect();
    let puts = actions
        .clone()
        .filter_map(|action| match action {
            RepositoryAction::WriteFile {
                path, bytes, mode, ..
            } => Some(Ok(PutFile {
                path: path.clone(),
                bytes: bytes.clone(),
                mode: *mode,
            })),
            RepositoryAction::SetMode { path, mode, .. } => {
                Some(captured_put(captured, path, *mode))
            }
            _ => None,
        })
        .collect::<Result<Vec<_>, RepositoryStateStoreError>>()?;
    let event_append = actions.clone().find_map(event_append_of);
    let deletes = actions
        .filter_map(|action| match action {
            RepositoryAction::DeleteFile { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    RedoRecord::new(make_directories, puts, deletes, event_append)
        .map_err(|source| RepositoryStateStoreError::Other(source.into()))
}

/// The redo record publishing `delta` over a present data root, refused
/// when it appends no event: the event log's length counts every commit.
fn appending_record(
    image: &RepositoryImage,
    delta: &RepositoryDelta,
) -> Result<RedoRecord, RepositoryStateStoreError> {
    let record = redo_record_for(image, delta.actions().iter())?;
    match record.event_append() {
        Some(_) => Ok(record),
        None => Err(RepositoryStateStoreError::RecordWithoutEventAppend),
    }
}

fn event_append_of(action: &RepositoryAction) -> Option<EventAppend> {
    match action {
        RepositoryAction::AppendEvents { offset, bytes, .. } => Some(EventAppend {
            offset: *offset,
            bytes: bytes.clone(),
        }),
        _ => None,
    }
}

/// The put that carries `path`'s captured bytes under `mode`.
fn captured_put(
    captured: &RepositoryImage,
    path: &VirtualPath,
    mode: FileMode,
) -> Result<PutFile, RepositoryStateStoreError> {
    match captured.entry(path)? {
        RepositoryEntry::File { content, .. } => Ok(PutFile {
            path: path.clone(),
            bytes: content.bytes().to_vec(),
            mode,
        }),
        _ => Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}"))),
    }
}

fn retryable_conflict(path: &VirtualPath) -> RepositoryStateStoreError {
    RepositoryStateStoreError::RetryableConflict {
        path: format!("{path:?}"),
    }
}

/// Read the live log's length and tail, or `None` when no ordinary file is there.
fn live_event_log_tail(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
) -> Result<Option<EventLogTail>, RepositoryStateStoreError> {
    Ok(
        match inspect_capability_entry(layout, roots, &VirtualPath::EVENTS, ReadMode::Tail)? {
            RepositoryEntry::FileTail { tail, .. } => Some(tail),
            _ => None,
        },
    )
}

/// Normalize a Git diagnostic into a stable, control-free unavailable reason.
///
/// `PinnedDocumentEvidence` requires the unavailable reason to be non-empty and
/// control-character-free (the identity contract). A raw `git` diagnostic is
/// often multi-line (e.g. "fatal: not a git repository\nStopping at filesystem
/// boundary"), so each control character (newlines included) collapses to a
/// single space and the result is trimmed; an all-control diagnostic degrades to
/// a fixed fallback so the reason is never empty.
fn stable_unavailable_reason(diagnostic: &str) -> String {
    let normalized: String = diagnostic
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = normalized.split_whitespace().collect::<Vec<_>>().join(" ");
    if trimmed.is_empty() {
        "Git evidence is unavailable".to_string()
    } else {
        trimmed
    }
}

fn capture_capability_image(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    storage: &JsonFileStorage,
    spec: CaptureSpec,
    mut prior: CapturedAnswers,
) -> Result<RepositoryImage, RepositoryStateStoreError> {
    let entries = spec
        .paths()
        .map(|path| {
            if let Some(entry) = prior.entries.remove(path) {
                return Ok((path.clone(), entry));
            }
            let mode = spec.read_mode(path).unwrap_or(ReadMode::Whole);
            Ok((
                path.clone(),
                inspect_capability_entry(layout, roots, path, mode)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let listings = spec
        .listings()
        .iter()
        .map(|path| {
            let listing = match prior.listings.remove(path) {
                Some(listing) => listing,
                None => inspect_capability_listing(layout, roots, path)?,
            };
            Ok((path.clone(), listing))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let unanswered = spec
        .pinned()
        .iter()
        .filter(|request| !prior.pinned.contains_key(*request))
        .cloned()
        .collect::<BTreeSet<_>>();
    let kind_only = unanswered
        .iter()
        .filter(|request| spec.pinned_read_mode(request) == Some(ReadMode::Kind))
        .cloned()
        .collect();
    let pinned_reads =
        storage.inspect_pinned_targets(layout.worktree_root(), &unanswered, &kind_only);
    let mut evidence_cache = storage.pinned_evidence_cache();
    let pinned = spec
        .pinned()
        .iter()
        .map(|(revision, path)| {
            let request = (revision.clone(), path.clone());
            if let Some(evidence) = prior.pinned.remove(&request) {
                return Ok((request, evidence));
            }
            let read = pinned_reads
                .get(&(revision.clone(), path.clone()))
                .ok_or_else(|| {
                    RepositoryStateStoreError::Other(anyhow::anyhow!(
                        "batch inspection omitted pinned target {revision}:{path}"
                    ))
                })?;
            let evidence = match read {
                Ok(read) => {
                    let cached = evidence_cache
                        .get(&(revision.clone(), path.clone()))
                        .filter(|evidence| {
                            evidence.commit_oid() == Some(read.version().as_str())
                                && evidence.object_oid() == read.object_oid()
                                && evidence.bytes().is_some() == read.bytes().is_some()
                        });
                    if let Some(evidence) = cached {
                        return Ok(((revision.clone(), path.clone()), evidence.clone()));
                    }
                    let commit = read.version().as_str().to_string();
                    let object_oid = read.object_oid().map(str::to_string);
                    let bytes = read.bytes().map(<[u8]>::to_vec);
                    let identity = bytes
                        .as_ref()
                        .zip(object_oid.as_deref())
                        .map(|(bytes, oid)| {
                            EntryIdentity::for_bytes(format!("git-blob:{oid}"), bytes)
                        })
                        .transpose()?;
                    PinnedDocumentEvidence::new(
                        revision.clone(),
                        path.clone(),
                        PinnedSourceClass::GitObject,
                        read.target_kind(),
                        Some(commit.clone()),
                        object_oid,
                        identity,
                        bytes,
                        read.unavailable_reason().map(stable_unavailable_reason),
                    )?
                }
                Err(error) => PinnedDocumentEvidence::new(
                    revision.clone(),
                    path.clone(),
                    PinnedSourceClass::GitUnavailable,
                    crate::repository_state::RepositoryTargetKind::Missing,
                    None,
                    None,
                    None,
                    None,
                    Some(stable_unavailable_reason(&error.to_string())),
                )?,
            };
            Ok(((revision.clone(), path.clone()), evidence))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    // Mirror the target cache: a capture that pins nothing keeps the evidence
    // a later capture reuses, and Git-unavailable evidence is never retained.
    if !pinned.is_empty() {
        *evidence_cache = pinned
            .iter()
            .filter(|(_, evidence)| evidence.commit_oid().is_some())
            .map(|(request, evidence)| (request.clone(), evidence.clone()))
            .collect();
    }
    drop(evidence_cache);
    RepositoryImage::close(layout.clone(), spec, entries, listings, pinned).map_err(Into::into)
}

fn capture_memory_image(
    layout: &RepositoryLayout,
    state: &MemoryRepositoryState,
    spec: CaptureSpec,
    mut prior: CapturedAnswers,
) -> Result<RepositoryImage, RepositoryStateStoreError> {
    let read = |path: &VirtualPath| match (state.entries.get(path), spec.read_mode(path)) {
        (Some(RepositoryEntry::File { content, mode }), Some(ReadMode::Tail)) => {
            RepositoryEntry::FileTail {
                tail: EventLogTail::of_bytes(content.bytes()),
                mode: *mode,
            }
        }
        (Some(RepositoryEntry::File { mode, .. }), Some(ReadMode::Kind)) => {
            RepositoryEntry::FileKind { mode: *mode }
        }
        (entry, _) => entry.cloned().unwrap_or(RepositoryEntry::Absent),
    };
    let entries = spec
        .paths()
        .map(|path| {
            let entry = prior.entries.remove(path);
            (path.clone(), entry.unwrap_or_else(|| read(path)))
        })
        .collect::<BTreeMap<_, _>>();
    let listings = spec
        .listings()
        .iter()
        .map(|path| match prior.listings.remove(path) {
            Some(listing) => Ok((path.clone(), listing)),
            None => memory_listing(layout, state, path).map(|listing| (path.clone(), listing)),
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let pinned = spec
        .pinned()
        .iter()
        .map(|(revision, path)| {
            Ok((
                (revision.clone(), path.clone()),
                PinnedDocumentEvidence::new(
                    revision.clone(),
                    path.clone(),
                    PinnedSourceClass::GitUnavailable,
                    crate::repository_state::RepositoryTargetKind::Missing,
                    None,
                    None,
                    None,
                    None,
                    Some("Git evidence is unavailable in memory storage".into()),
                )?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    RepositoryImage::close(layout.clone(), spec, entries, listings, pinned).map_err(Into::into)
}

fn memory_listing(
    layout: &RepositoryLayout,
    state: &MemoryRepositoryState,
    path: &VirtualPath,
) -> Result<DirectoryListing, RepositoryStateStoreError> {
    let physical_parent = layout.resolve(path)?;
    let names = state
        .entries
        .iter()
        .filter(|(_, entry)| entry.identity().is_some())
        .filter_map(|(candidate, _)| {
            let physical = layout.resolve(candidate).ok()?;
            (physical.parent() == Some(physical_parent.as_path()))
                .then(|| Some(physical.file_name()?.to_str()?.to_string()))
                .flatten()
        });
    // Mirror inspect_capability_listing: a directory lists its children, an
    // absent path lists none, and any other kind (file, symlink, unsupported) is
    // an error rather than a fabricated directory listing.
    match state.entries.get(path) {
        Some(RepositoryEntry::Directory { .. }) => DirectoryListing::new(names).map_err(Into::into),
        None | Some(RepositoryEntry::Absent) => Ok(DirectoryListing::default()),
        Some(_) => Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}"))),
    }
}

/// Require that an action's parent directory already exists, mirroring the JSON
/// backend's parent walk (every intermediate component under the root
/// capability, failing when one is absent). Only intermediate
/// directories are checked; the root capability itself (empty relative) always
/// exists and is never a parent to verify. Because every `CreateDirectory` goes
/// through this same check, a present `Directory` entry implies all its ancestors
/// exist, so verifying the immediate parent is sufficient.
fn ensure_memory_parent_exists(
    state: &MemoryRepositoryState,
    path: &VirtualPath,
) -> Result<(), RepositoryStateStoreError> {
    let Some(parent) = path.relative().as_path().parent() else {
        return Ok(());
    };
    if parent.as_os_str().is_empty() {
        return Ok(());
    }
    let parent_path = VirtualPath::from_root(
        path.root_class(),
        RootRelativePath::parse(parent).map_err(RepositoryLayoutError::from)?,
    )?;
    if matches!(
        state.entries.get(&parent_path),
        Some(RepositoryEntry::Directory { .. })
    ) {
        Ok(())
    } else {
        Err(RepositoryStateStoreError::Publication(anyhow::anyhow!(
            "missing target parent: {parent_path:?}"
        )))
    }
}

fn apply_memory_action(
    layout: &RepositoryLayout,
    state: &mut MemoryRepositoryState,
    action: &RepositoryAction,
) -> Result<(), RepositoryStateStoreError> {
    layout.ensure_canonical(action.path())?;
    ensure_memory_parent_exists(state, action.path())?;
    let current = state
        .entries
        .get(action.path())
        .cloned()
        .unwrap_or(RepositoryEntry::Absent);
    if ExpectedPreimage::of(&current) != *action.expected() {
        return Err(RepositoryStateStoreError::RetryableConflict {
            path: format!("{:?}", action.path()),
        });
    }
    match action {
        RepositoryAction::CreateDirectory { path, .. } => {
            state.entries.insert(
                path.clone(),
                RepositoryEntry::Directory {
                    identity: EntryIdentity::for_bytes(
                        format!("memory-directory:{path:?}"),
                        b"directory",
                    )?,
                    mode: FileMode::Executable,
                },
            );
        }
        RepositoryAction::WriteFile {
            path, bytes, mode, ..
        } => {
            state.entries.insert(
                path.clone(),
                RepositoryEntry::File {
                    content: CapturedBytes::new(
                        crate::storage::memory::memory_file_object(path),
                        bytes.clone(),
                    )?,
                    mode: *mode,
                },
            );
        }
        RepositoryAction::SetMode { path, mode, .. } => {
            let RepositoryEntry::File { content, .. } = current else {
                return Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}")));
            };
            state.entries.insert(
                path.clone(),
                RepositoryEntry::File {
                    content,
                    mode: *mode,
                },
            );
        }
        RepositoryAction::DeleteFile { path, .. } => {
            state.entries.remove(path);
        }
        RepositoryAction::AppendEvents { .. } => event_append_of(action)
            .map_or(Ok(()), |append| append_events(state, &append))
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))?,
    }
    state.data_root_exists |= action.path().root_class() == RepositoryRootClass::Data;
    Ok(())
}

fn inspect_capability_entry(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    path: &VirtualPath,
    mode: ReadMode,
) -> Result<RepositoryEntry, RepositoryStateStoreError> {
    layout.ensure_canonical(path)?;
    let Some(root) = roots.root(path.root_class()) else {
        return Ok(RepositoryEntry::Absent);
    };
    if path.relative().is_root() {
        return directory_entry(root.dir_metadata()?);
    }
    let relative = path.relative().as_path();
    let Some((parent, leaf)) = open_capability_parent(root, relative)? else {
        // A missing ancestor means the target itself is absent, not a capture
        // failure: a not-yet-created nested path captures as a clean absence.
        return Ok(RepositoryEntry::Absent);
    };
    inspect_capability_leaf(&parent, &leaf, mode)
}

/// List the directory at `path`.
fn inspect_capability_listing(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    path: &VirtualPath,
) -> Result<DirectoryListing, RepositoryStateStoreError> {
    match inspect_capability_entry(layout, roots, path, ReadMode::Whole)? {
        RepositoryEntry::Directory { .. } => {}
        RepositoryEntry::Absent => return Ok(DirectoryListing::default()),
        _ => return Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}"))),
    }
    let root = roots.root(path.root_class()).expect("directory is present");
    let directory = if path.relative().is_root() {
        root.try_clone()?
    } else {
        open_descendant_dir_nofollow(root, path.relative().as_path())?
    };
    let names = match directory.entries() {
        Ok(entries) => entries
            .map(|entry| {
                entry?
                    .file_name()
                    .into_string()
                    .map_err(|_| RepositoryStateStoreError::UnsafeTarget("non-UTF-8 entry".into()))
            })
            .collect::<Result<Vec<_>, _>>(),
        Err(error) => Err(error.into()),
    };
    DirectoryListing::new(names?).map_err(Into::into)
}

fn inspect_capability_leaf(
    parent: &Dir,
    leaf: &str,
    read: ReadMode,
) -> Result<RepositoryEntry, RepositoryStateStoreError> {
    let metadata = match parent.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RepositoryEntry::Absent),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() {
        let target = parent.read_link(leaf)?;
        let bytes = target.as_os_str().as_encoded_bytes().to_vec();
        return Ok(RepositoryEntry::Symlink {
            target: CapturedBytes::new(capability_metadata_identity(&metadata)?, bytes)?,
            mode: normalized_mode(&metadata),
        });
    }
    if metadata.is_dir() {
        return directory_entry(metadata);
    }
    if metadata.is_file() && read == ReadMode::Kind {
        return Ok(RepositoryEntry::FileKind {
            mode: normalized_mode(&metadata),
        });
    }
    if metadata.is_file() {
        let mut options = OpenOptions::new();
        options.read(true);
        options._cap_fs_ext_follow(FollowSymlinks::No);
        let mut file = parent.open_with(leaf, &options)?;
        if read == ReadMode::Tail {
            let length = file.metadata()?.len();
            let tail = EventLogTail::read_tail(length, |start, len| {
                let mut bytes = Vec::new();
                file.seek(SeekFrom::Start(start))?;
                (&mut file).take(len).read_to_end(&mut bytes)?;
                Ok::<_, std::io::Error>(bytes)
            })?;
            // A file that changed length under the read leaves a short tail.
            let tail = EventLogTail::new(length, tail)
                .map_err(|_| RepositoryStateStoreError::RetryableConflict { path: leaf.into() })?;
            return Ok(RepositoryEntry::FileTail {
                tail,
                mode: normalized_mode(&metadata),
            });
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let metadata = file.metadata()?;
        return Ok(RepositoryEntry::File {
            content: CapturedBytes::new(capability_metadata_identity(&metadata)?, bytes)?,
            mode: normalized_mode(&metadata),
        });
    }
    Ok(RepositoryEntry::Unsupported {
        identity: entry_identity(&metadata, b"unsupported")?,
        reason: "unsupported filesystem object".into(),
        mode: normalized_mode(&metadata),
    })
}

fn directory_entry(
    metadata: cap_std::fs::Metadata,
) -> Result<RepositoryEntry, RepositoryStateStoreError> {
    Ok(RepositoryEntry::Directory {
        identity: entry_identity(&metadata, b"directory")?,
        mode: normalized_mode(&metadata),
    })
}

fn entry_identity(
    metadata: &cap_std::fs::Metadata,
    bytes: &[u8],
) -> Result<EntryIdentity, RepositoryStateStoreError> {
    let object = capability_metadata_identity(metadata)?;
    EntryIdentity::for_bytes(object, bytes).map_err(Into::into)
}

fn normalized_mode(metadata: &cap_std::fs::Metadata) -> FileMode {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;
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

fn revalidate_layout_capabilities(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
) -> Result<(), RepositoryStateStoreError> {
    // Re-resolve both roots by path (no-follow) and compare the fresh identity to
    // the one the held capability was opened against. Statting the held FDs would
    // be tautological — an FD's dev:ino never changes — and would miss an external
    // atomic replacement of a whole root directory between capture and apply,
    // which otherwise lets the delta publish into the now-unlinked old inode (a
    // silent lost update). A fresh path resolution sees the new inode and aborts.
    revalidate_layout(layout)?;
    match (
        roots.root(RepositoryRootClass::Data),
        layout.data_root().exists(),
    ) {
        (Some(_), true) | (None, false) => Ok(()),
        _ => Err(RepositoryStateStoreError::RetryableConflict {
            path: layout.data_root().display().to_string(),
        }),
    }
}

fn revalidate_layout(layout: &RepositoryLayout) -> Result<(), RepositoryStateStoreError> {
    let worktree = discover_root_evidence(layout.worktree_root(), false)?;
    let data = discover_root_evidence(layout.data_root(), true)?;
    if worktree.identity() == layout.worktree_identity()
        && data.identity() == layout.data_identity()
    {
        Ok(())
    } else {
        Err(RepositoryStateStoreError::RetryableConflict {
            path: "repository root identity".into(),
        })
    }
}

fn discover_root_evidence(
    path: &Path,
    allow_absent: bool,
) -> Result<RepositoryRootEvidence, RepositoryStateStoreError> {
    ensure_symlink_free_ancestry(path, allow_absent)?;
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => Ok(RepositoryRootEvidence::new(
            path,
            std_metadata_identity(path)?,
            true,
        )),
        Ok(_) => Err(RepositoryStateStoreError::UnsafeTarget(
            path.display().to_string(),
        )),
        Err(error) if allow_absent && error.kind() == ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                RepositoryStateStoreError::UnsafeTarget(path.display().to_string())
            })?;
            let leaf = path.file_name().ok_or_else(|| {
                RepositoryStateStoreError::UnsafeTarget(path.display().to_string())
            })?;
            Ok(RepositoryRootEvidence::new(
                path,
                format!(
                    "absent:{}:{}",
                    std_metadata_identity(parent)?,
                    leaf.to_string_lossy()
                ),
                true,
            ))
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_symlink_free_ancestry(
    path: &Path,
    allow_absent_leaf: bool,
) -> Result<(), RepositoryStateStoreError> {
    let mut current = PathBuf::new();
    let components = path.components().collect::<Vec<_>>();
    for (index, component) in components.iter().enumerate() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(RepositoryLayoutError::SymlinkedRoot(path.to_path_buf()).into())
            }
            Ok(_) => {}
            Err(error)
                if error.kind() == ErrorKind::NotFound
                    && allow_absent_leaf
                    && index + 1 == components.len() => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[cfg(unix)]
fn std_metadata_identity(path: &Path) -> Result<String, RepositoryStateStoreError> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

/// A `std::fs::Metadata`-only conversion leaves `volume_serial_number`/
/// `file_index` `None` on Windows (they come from the open handle, not the
/// stat), so this opens the path itself: `FILE_FLAG_OPEN_REPARSE_POINT`
/// preserves the no-follow semantics the callers rely on, and
/// `FILE_FLAG_BACKUP_SEMANTICS` is required to open a directory handle at
/// all. Once a live handle is in hand, `capability_metadata_identity`
/// already knows how to format the resulting Windows identity.
#[cfg(windows)]
fn std_metadata_identity(path: &Path) -> Result<String, RepositoryStateStoreError> {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = cap_primitives::fs::Metadata::from_file(&file)?;
    capability_metadata_identity(&metadata)
}

#[cfg(not(any(unix, windows)))]
fn std_metadata_identity(path: &Path) -> Result<String, RepositoryStateStoreError> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(format!(
        "{}:{}",
        metadata.len(),
        metadata.permissions().readonly()
    ))
}

fn lexical_absolute(path: &Path) -> Result<PathBuf, RepositoryStateStoreError> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(Path::new("/")),
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(
                        RepositoryLayoutError::LexicalEscape(path.display().to_string()).into(),
                    );
                }
            }
        }
    }
    Ok(normalized)
}

#[cfg(test)]
#[path = "repository_state_store_contention_tests.rs"]
mod repository_state_store_contention_tests;
#[cfg(test)]
#[path = "repository_state_store_fixtures.rs"]
pub(crate) mod test_fixtures;
#[cfg(test)]
#[path = "repository_state_store_tests.rs"]
mod tests;
