//! Recovered layout-aware repository capture and mutation sessions.
//!
//! Session orchestration lives here; durable publication belongs to the
//! capability-confined [`FileTransactionKernel`], and an opening session
//! applies the redo record a crash left.

use crate::repository_state::{
    CaptureError, CaptureSpec, CapturedBytes, EntryIdentity, ExpectedPreimage, FileMode,
    LinkedWorktreeEvidence, LinkedWorktreeSourceClass, ListingFingerprint, MaterializationPlan,
    PinnedDocumentEvidence, PinnedSourceClass, RepositoryAction, RepositoryDelta, RepositoryEntry,
    RepositoryImage, RepositoryLayout, RepositoryLayoutError, RepositoryRootClass,
    RepositoryRootEvidence, RootRelativePath, VirtualPath,
};
use crate::storage::atomic_write::sync_capability_directory;
use crate::storage::file_transaction::RepositoryRecoveryDisposition;
use crate::storage::memory::MemoryRepositoryState;
use crate::storage::{
    FileLocker, FileTransactionKernel, InMemoryStorage, IssueStore, JsonFileStorage,
    MemoryRedoApplier, MutationInterruptionInjector, MutationPoint, PutFile, RedoApplier,
    RedoApplyError, RedoRecord, RepoWriteGuard, SessionRedoInterruptions,
    TransactionControlLocation, TransactionFailurePoint, RECORD_FILE_NAME, RECORD_SCRATCH_DIR,
};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use cap_std::{ambient_authority, fs::MetadataExt as _};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Successful application of one exact repository delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryApplyOutcome {
    /// Complete semantic plan hash supplied unchanged to storage and recovery.
    pub transaction_hash: String,
    /// Number of normalized actions applied.
    pub actions_applied: usize,
}

/// Residue one session open carried forward.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryDispatchReport {
    /// External journals or orphan controls recovered in deterministic id order.
    pub external_transactions: Vec<String>,
    /// Internal data-root journals recovered in deterministic id order.
    pub internal_transactions: Vec<String>,
    /// Leftover redo records applied: one, or none, because a record is the
    /// whole of one mutation.
    pub applied_records: usize,
}

impl RecoveryDispatchReport {
    /// Journals recovered plus records applied.
    pub fn recovered_count(&self) -> usize {
        self.external_transactions.len() + self.internal_transactions.len() + self.applied_records
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
    #[error("repository recovery or publication failed: {0}")]
    Transaction(#[source] anyhow::Error),
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

/// Whether an advisory capture may replace this failure with unreadable
/// evidence. Semantic capture failures and non-permission I/O stay hard.
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

/// Opaque recovered session retaining serialization through capture and apply.
pub trait RepositoryMutationSession {
    /// Canonical layout held by this session.
    fn layout(&self) -> &RepositoryLayout;
    /// Recovery completed before this session became available for capture.
    fn recovery_report(&self) -> &RecoveryDispatchReport;
    /// Capture one complete bounded image after recovery has converged.
    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError>;
    /// Revalidate the complete image and publish one complete semantic plan.
    fn apply(
        &mut self,
        plan: &MaterializationPlan,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError>;
}

/// Storage backend capable of one recovered capture/apply mutation boundary.
pub trait RepositoryStateStore {
    /// Open a recovered session for exactly `layout`.
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError>;
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

/// Runs before any recovery pass, so a record it refuses is refused over an
/// untouched repository. A repository holding no record pays one look at the
/// record's own name: the parent walk stops at an absent scratch directory and
/// the open at an absent record, and nothing there is enumerated or otherwise
/// read.
fn read_leftover_record(
    roots: &CapabilityRoots,
) -> Result<Option<RedoRecord>, RepositoryStateStoreError> {
    let Some(data) = roots.root(RepositoryRootClass::Data) else {
        return Ok(None);
    };
    let relative = Path::new(RECORD_SCRATCH_DIR).join(RECORD_FILE_NAME);
    let Some((scratch, leaf)) = open_capability_parent(data, &relative)? else {
        return Ok(None);
    };
    let mut file = match open_regular_file_nofollow(&scratch, &leaf) {
        Ok(file) => file,
        Err(RepositoryStateStoreError::Io(error)) if error.kind() == ErrorKind::NotFound => {
            return Ok(None)
        }
        Err(error) => return Err(error),
    };
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    RedoRecord::parse(&bytes).map(Some).map_err(|source| {
        RepositoryStateStoreError::UnreadableRedoRecord {
            path: relative.display().to_string(),
            source,
        }
    })
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

/// Open no-follow handles for the selected roots, constructed only inside the
/// session boundary that verified their identity.
pub struct CapabilityRoots {
    worktree: Dir,
    data: Option<Dir>,
}

impl CapabilityRoots {
    pub(in crate::storage) fn open(
        layout: &RepositoryLayout,
    ) -> Result<(Self, Dir, String), RepositoryStateStoreError> {
        let worktree = open_absolute_dir_nofollow(layout.worktree_root())?;
        ensure_capability_identity(
            &worktree,
            layout.worktree_identity(),
            layout.worktree_root(),
        )?;
        let data_parent_path = layout.data_root().parent().ok_or_else(|| {
            RepositoryStateStoreError::UnsafeTarget(layout.data_root().display().to_string())
        })?;
        let data_parent = open_absolute_dir_nofollow(data_parent_path)?;
        let data_leaf = layout
            .data_root()
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                RepositoryStateStoreError::UnsafeTarget(layout.data_root().display().to_string())
            })?
            .to_string();
        let data = match data_parent.symlink_metadata(&data_leaf) {
            Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
                let data = open_child_dir_nofollow(&data_parent, &data_leaf)?;
                ensure_capability_identity(&data, layout.data_identity(), layout.data_root())?;
                Some(data)
            }
            Ok(_) => {
                return Err(RepositoryStateStoreError::UnsafeTarget(
                    layout.data_root().display().to_string(),
                ))
            }
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        // Bind the held parent capability to the absent-root parent recorded in
        // the layout. The fd already resists path redirection; this additionally
        // rejects a parent swapped out from under the layout before capture, so a
        // staged root is never published beneath a directory the layout never saw.
        if data.is_none() {
            if let Some(parent_identity) = capability_dir_identity(&data_parent) {
                let expected = format!("absent:{parent_identity}:{data_leaf}");
                if expected != layout.data_identity() {
                    return Err(RepositoryStateStoreError::RetryableConflict {
                        path: layout.data_root().display().to_string(),
                    });
                }
            }
        }
        Ok((Self { worktree, data }, data_parent, data_leaf))
    }

    pub(in crate::storage) fn root(&self, class: RepositoryRootClass) -> Option<&Dir> {
        match class {
            RepositoryRootClass::Worktree => Some(&self.worktree),
            RepositoryRootClass::Data => self.data.as_ref(),
        }
    }
}

struct JsonMutationSession {
    layout: RepositoryLayout,
    storage: JsonFileStorage,
    recovery_report: RecoveryDispatchReport,
    roots: CapabilityRoots,
    kernel: FileTransactionKernel,
    /// Worktree-root bootstrap lock, held outermost so the worktree-side
    /// `.jit-bootstrap` namespace is serialized across disjoint-data-root sessions.
    _worktree_bootstrap_guard: RepoWriteGuard,
    _bootstrap_guard: RepoWriteGuard,
    _repository_guard: Option<RepoWriteGuard>,
    events_lock: Option<(FileLocker, PathBuf)>,
    _reentry: LayoutReentry,
    _order_guard: crate::storage::guard_order::RepositoryOrderGuard,
    captured: Option<RepositoryImage>,
}

struct MemoryMutationSession {
    layout: RepositoryLayout,
    storage: InMemoryStorage,
    recovery_report: RecoveryDispatchReport,
    _guard: RepoWriteGuard,
    _reentry: LayoutReentry,
    _order_guard: crate::storage::guard_order::RepositoryOrderGuard,
    captured: Option<RepositoryImage>,
}

impl RepositoryStateStore for JsonFileStorage {
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession>, RepositoryStateStoreError> {
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
        // A retained session may reenter the reentrant lock chain only for the
        // same canonical layout.
        self.active_mutation_layout().enter(&layout)?;
        let reentry = LayoutReentry(self.active_mutation_layout());
        // Mark a repository mutation session as held on this thread so a later
        // attempt to enter claim coordination (reverse order) is rejected.
        let order_guard = crate::storage::guard_order::RepositoryOrderGuard::enter();
        let injector = self.repository_state_failures();
        let interruptions = self.mutation_interruptions();

        // Serialize the worktree-side `.jit-bootstrap` namespace at the WORKTREE
        // root, outermost, so two sessions sharing this worktree with data roots
        // under different parents cannot concurrently mutate its companions and
        // external journals. For a nested repo this is the same reentrant lock as
        // the bootstrap lock below; for a disjoint root it is a distinct outer
        // lock. Acquired before any capability discovery.
        let worktree_bootstrap_guard =
            self.acquire_worktree_bootstrap_lock(layout.worktree_root())?;
        // Fixed order: bootstrap serialization is acquired BEFORE any capability
        // discovery, so a concurrent creator cannot publish the selected data
        // root between discovery and the lock and leave the session bound to a
        // root it never serialized against.
        let bootstrap_guard = self.acquire_bootstrap_write_lock()?;
        revalidate_layout(&layout)?;

        // External recovery needs the data capability exactly as it exists now:
        // a still-unpublished prepared root has none and rolls back, while an
        // already-published absent root exists and completes forward, verifying
        // its final data actions. Recovery never itself newly publishes a root,
        // so this pre-recovery view is authoritative for the external pass; the
        // main kernel is rebuilt from a fresh view afterward.
        let (pre_roots, pre_parent, pre_leaf) = CapabilityRoots::open(&layout)?;
        // Read before the first recovery pass, apply after the last: a record
        // this session cannot read is refused while the repository is still
        // exactly as it was found, and a readable one is carried to the apply
        // below rather than read a second time, so a repository holding none
        // still pays one by-name look. The bytes are safe to carry because the
        // bootstrap lock above is held until this session ends, and no other
        // session can open — and so write a record — while it is.
        let leftover_record = read_leftover_record(&pre_roots)?;
        let recovery_kernel = FileTransactionKernel::for_repository_layout(
            layout.clone(),
            pre_roots.worktree.try_clone()?,
            pre_roots.data.as_ref().map(Dir::try_clone).transpose()?,
            pre_parent,
            pre_leaf,
            Arc::clone(&injector),
        )?;
        interruptions.check(&MutationPoint::SessionOpen)?;
        injector.check(&TransactionFailurePoint::RepositoryRecoveryExternal)?;
        let external_transactions = recover_location(
            &recovery_kernel,
            TransactionControlLocation::ExternalBootstrap,
        )?;
        drop(recovery_kernel);
        drop(pre_roots);

        // Rediscover capabilities AFTER external recovery: absent-root recovery
        // may have published the selected data root, and prepared-root recovery
        // may have removed it. The kernel is pinned to these fresh, identity-
        // bound handles for the remainder of the session.
        let (roots, data_parent, data_leaf) = CapabilityRoots::open(&layout)?;
        let kernel = FileTransactionKernel::for_repository_layout(
            layout.clone(),
            roots.worktree.try_clone()?,
            roots.data.as_ref().map(Dir::try_clone).transpose()?,
            data_parent,
            data_leaf,
            Arc::clone(&injector),
        )?;

        let (repository_guard, internal_transactions, external_transactions, applied_records) =
            if roots.data.is_some() {
                let repository_guard = self.acquire_repo_write_lock_raw()?;
                let _events_guard = self.acquire_events_write_lock()?;
                injector.check(&TransactionFailurePoint::RepositoryRecoveryInternal)?;
                let internal_transactions =
                    recover_location(&kernel, TransactionControlLocation::InternalRepository)?;
                // After internal-journal recovery, reclaim worktree-side companions
                // left orphaned by a crash whose internal transaction is already gone.
                let swept_companions = kernel.sweep_orphan_companions(&repository_guard)?;
                let mut external_transactions = external_transactions;
                external_transactions.extend(swept_companions);
                external_transactions.sort();
                external_transactions.dedup();
                // Inside both locks, because the record writes repository
                // state, and after journal recovery, whose rollback would undo
                // what the record published. The record lives inside the data
                // root, so the absent-root branch has none to apply.
                let applied_records =
                    apply_leftover_record(&roots, leftover_record, Arc::clone(&interruptions))?;
                (
                    Some(repository_guard),
                    internal_transactions,
                    external_transactions,
                    applied_records,
                )
            } else {
                (None, Vec::new(), external_transactions, 0)
            };
        let events_lock = roots.data.as_ref().map(|_| self.events_lock_spec());

        self.configure_repository_layout(&layout);
        Ok(Box::new(JsonMutationSession {
            layout,
            storage: self.clone(),
            recovery_report: RecoveryDispatchReport {
                external_transactions,
                internal_transactions,
                applied_records,
            },
            roots,
            kernel,
            _worktree_bootstrap_guard: worktree_bootstrap_guard,
            _bootstrap_guard: bootstrap_guard,
            _repository_guard: repository_guard,
            events_lock,
            _reentry: reentry,
            _order_guard: order_guard,
            captured: None,
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
        // Model the kernel's recovery boundaries so both backends fail identically
        // when one is injected: external recovery always runs; the internal
        // recovery and orphan-companion sweep run only when the data root already
        // exists, matching the JSON session-open guard sequence.
        let failures = self.repository_state_failures();
        let interruptions = self.mutation_interruptions();
        interruptions.check(&MutationPoint::SessionOpen)?;
        failures.check(&TransactionFailurePoint::RepositoryRecoveryExternal)?;
        let (data_exists, leftover_record) = {
            let state = self.repository_state();
            (state.data_root_exists, state.record.clone())
        };
        if data_exists {
            failures.check(&TransactionFailurePoint::RepositoryRecoveryInternal)?;
            failures.check(&TransactionFailurePoint::RepositorySweepCompanions)?;
        }
        // Where the JSON open applies it, gated on the data root for the same
        // reason: a record's home is inside it.
        let applied_records = match data_exists {
            true => apply_memory_leftover_record(self, leftover_record, interruptions)?,
            false => 0,
        };
        Ok(Box::new(MemoryMutationSession {
            layout,
            storage: self.clone(),
            recovery_report: RecoveryDispatchReport {
                applied_records,
                ..Default::default()
            },
            _guard: guard,
            _reentry: reentry,
            _order_guard: order_guard,
            captured: None,
        }))
    }
}

impl RepositoryMutationSession for JsonMutationSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn recovery_report(&self) -> &RecoveryDispatchReport {
        &self.recovery_report
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        revalidate_layout_capabilities(&self.layout, &self.roots)?;
        let worktree_paths = self.storage.configured_worktree_paths()?;
        let image = capture_capability_image(
            &self.layout,
            &self.roots,
            &worktree_paths,
            &self.storage,
            spec,
        )?;
        self.captured = Some(image.clone());
        Ok(image)
    }

    fn apply(
        &mut self,
        plan: &MaterializationPlan,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
        let image = plan.image();
        let delta = plan.delta();
        ensure_session_image(&self.layout, self.captured.as_ref(), image)?;
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
            transaction_hash: plan.hash().to_string(),
            actions_applied: delta.actions().len(),
        };
        match (self.roots.data.is_some(), delta.actions().is_empty()) {
            // A record's home is inside the data root, so a delta over an absent
            // one still publishes through the transaction kernel; the case leaves
            // it when initialization takes over (jit:b0ee2dd6).
            (false, _) => self.publish_through_kernel(plan)?,
            (true, false) => self.publish_through_record(image, delta, interruptions)?,
            (true, true) => {}
        }
        self.captured = None;
        Ok(outcome)
    }
}

impl JsonMutationSession {
    /// Publish `delta` as one redo record: derived from the delta, refused if
    /// its write set moved, made durable, applied, and removed.
    fn publish_through_record(
        &self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
        interruptions: Arc<dyn MutationInterruptionInjector>,
    ) -> Result<(), RepositoryStateStoreError> {
        let record = redo_record_for(image, delta)?;
        ensure_write_set_unchanged(&self.roots, image, &record)?;
        create_delta_directories(&self.roots, delta)?;
        record
            .commit(self.layout.data_root())
            .map_err(RepositoryStateStoreError::Transaction)?;
        RedoApplier::new(SessionRedoInterruptions::new(interruptions))
            .apply(&self.roots, &record)
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))
    }

    /// Publish a delta whose data root does not exist yet, rebinding the layout
    /// when the publication created it.
    fn publish_through_kernel(
        &self,
        plan: &MaterializationPlan,
    ) -> Result<(), RepositoryStateStoreError> {
        let delta = plan.delta();
        let guard = self
            ._repository_guard
            .as_ref()
            .unwrap_or(&self._bootstrap_guard);
        let transaction_id = uuid::Uuid::new_v4().simple().to_string();
        self.kernel
            .execute_repository_delta(guard, &transaction_id, delta, plan.hash())
            .map_err(|error| map_transaction_error(error, &self.layout))?;
        if !delta
            .actions()
            .iter()
            .any(|action| action.path().root_class() == RepositoryRootClass::Data)
        {
            return Ok(());
        }
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
        self.storage.configure_repository_layout(&refreshed);
        Ok(())
    }
}

impl RepositoryMutationSession for MemoryMutationSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn recovery_report(&self) -> &RecoveryDispatchReport {
        &self.recovery_report
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        let state = self.storage.repository_state();
        let image = capture_memory_image(&self.layout, &state, spec)?;
        self.captured = Some(image.clone());
        Ok(image)
    }

    fn apply(
        &mut self,
        plan: &MaterializationPlan,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
        let image = plan.image();
        let delta = plan.delta();
        ensure_session_image(&self.layout, self.captured.as_ref(), image)?;
        ensure_delta_is_captured(image, delta)?;
        #[cfg(feature = "test-support")]
        if self.storage.consume_repository_state_apply_conflict() {
            return Err(RepositoryStateStoreError::RetryableConflict {
                path: "injected memory read-set conflict".into(),
            });
        }
        let interruptions = self.storage.mutation_interruptions();
        interruptions.check(&MutationPoint::BeforeCommitCheck)?;
        let outcome = RepositoryApplyOutcome {
            transaction_hash: plan.hash().to_string(),
            actions_applied: delta.actions().len(),
        };
        let data_root_exists = self.storage.repository_state().data_root_exists;
        match (data_root_exists, delta.actions().is_empty()) {
            (false, _) => publish_absent_memory_root(&self.layout, &self.storage, delta)?,
            (true, false) => self.publish_through_record(image, delta, interruptions)?,
            (true, true) => {}
        }
        self.captured = None;
        Ok(outcome)
    }
}

impl MemoryMutationSession {
    /// The in-memory counterpart of [`JsonMutationSession::publish_through_record`],
    /// over the entry map.
    fn publish_through_record(
        &self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
        interruptions: Arc<dyn MutationInterruptionInjector>,
    ) -> Result<(), RepositoryStateStoreError> {
        let record = redo_record_for(image, delta)?;
        {
            let mut state = self.storage.repository_state();
            ensure_memory_write_set_unchanged(&state, image, &record)?;
            create_memory_delta_directories(&mut state, delta);
        }
        self.storage
            .commit_record(&record)
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))?;
        MemoryRedoApplier::new(SessionRedoInterruptions::new(interruptions))
            .apply(&self.storage, &record)
            .map_err(|source| RepositoryStateStoreError::RedoApply(Box::new(source)))
    }
}

/// Publish a delta whose data root does not exist yet, as the transaction
/// kernel publishes one on the filesystem: the record's home is inside the root
/// the delta creates, so this one delta reaches the entry map directly.
fn publish_absent_memory_root(
    layout: &RepositoryLayout,
    storage: &InMemoryStorage,
    delta: &RepositoryDelta,
) -> Result<(), RepositoryStateStoreError> {
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

fn recover_location(
    kernel: &FileTransactionKernel,
    location: TransactionControlLocation,
) -> Result<Vec<String>, RepositoryStateStoreError> {
    let transactions = kernel.pending_repository_transactions(location)?;
    let recovered = transactions
        .into_iter()
        .filter_map(
            |id| match kernel.recover_repository_transaction(location, &id) {
                Ok(RepositoryRecoveryDisposition::Recovered) => Some(Ok(id)),
                Ok(
                    RepositoryRecoveryDisposition::SkippedCompanion
                    | RepositoryRecoveryDisposition::SkippedForeignOwner,
                ) => None,
                Err(error) => Some(Err(RepositoryStateStoreError::Transaction(error))),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    kernel.cleanup_empty_repository_control(location)?;
    Ok(recovered)
}

fn map_transaction_error(
    error: anyhow::Error,
    layout: &RepositoryLayout,
) -> RepositoryStateStoreError {
    if matches!(
        error.downcast_ref::<crate::storage::FileTransactionError>(),
        Some(crate::storage::FileTransactionError::OccupiedDataRoot { .. })
    ) {
        RepositoryStateStoreError::OccupiedDataRoot {
            path: layout.data_root().to_path_buf(),
        }
    } else {
        RepositoryStateStoreError::Transaction(error)
    }
}

fn ensure_session_image(
    layout: &RepositoryLayout,
    captured: Option<&RepositoryImage>,
    image: &RepositoryImage,
) -> Result<(), RepositoryStateStoreError> {
    if image.layout() != layout || captured != Some(image) {
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

/// Bytes at the end of the event log the pre-commit check compares.
///
/// A byte window rather than the last complete line: the torn-tail protocol
/// admits an incomplete final line, so the log's end is not a line boundary.
pub const EVENT_LOG_TAIL_WINDOW: u64 = 4096;

/// Refuse a mutation whose write set moved after the mutation read it.
///
/// Compares the live occupant of every path `record` puts or deletes with the
/// bytes `captured` holds for it, and the event log as its length together with
/// its last [`EVENT_LOG_TAIL_WINDOW`] bytes — the one answer for that path, so
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
        &|path| inspect_capability_entry(captured.layout(), roots, path),
        &|| live_event_log_tail(roots, &VirtualPath::EVENTS),
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
        .puts()
        .iter()
        .map(|put| &put.path)
        .chain(record.deletes().iter())
        .filter(|path| **path != VirtualPath::EVENTS)
        .try_for_each(|path| ensure_target_unchanged(captured, path, live))?;
    let touches_event_log = record.event_append().is_some()
        || record
            .puts()
            .iter()
            .any(|put| put.path == VirtualPath::EVENTS)
        || record.deletes().contains(&VirtualPath::EVENTS);
    if !touches_event_log {
        return Ok(());
    }
    match captured.entry(&VirtualPath::EVENTS)? {
        // An ordinary captured log is the only state carrying a tail to compare;
        // any other occupant is compared like any other write target.
        RepositoryEntry::File { content, .. } => (live_log_tail()?
            == Some(EventLogTail::of_bytes(content.bytes())))
        .then_some(())
        .ok_or_else(|| retryable_conflict(&VirtualPath::EVENTS)),
        _ => ensure_target_unchanged(captured, &VirtualPath::EVENTS, live),
    }
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

/// Create the directories `delta` names, ahead of the record's commit.
///
/// A directory holds no content, so it is not one of the files a record
/// publishes (`@/charter/D-17`), and a put creates only the directories its own
/// path needs. A directory the mutation creates and writes nothing beneath —
/// initialization's issues directory is the one the engine composes — is made
/// here. Creation is additive and idempotent, so an interruption before the
/// commit point leaves an empty directory the next attempt reuses.
fn create_delta_directories(
    roots: &CapabilityRoots,
    delta: &RepositoryDelta,
) -> Result<(), RepositoryStateStoreError> {
    created_directories(delta).try_for_each(|path| {
        let root = roots
            .root(path.root_class())
            .ok_or_else(|| RepositoryStateStoreError::UnsafeTarget(format!("{path:?}")))?;
        let (parent, leaf) = open_or_create_capability_parent(root, path.relative().as_path())?;
        match parent.create_dir(&leaf) {
            Ok(()) => sync_capability_directory(&parent).map_err(Into::into),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => Ok(()),
            Err(error) => Err(error.into()),
        }
    })
}

/// [`create_delta_directories`] over the in-memory backend's entry map.
fn create_memory_delta_directories(state: &mut MemoryRepositoryState, delta: &RepositoryDelta) {
    created_directories(delta)
        .for_each(|path| crate::storage::memory::ensure_directory(state, path));
}

fn created_directories(delta: &RepositoryDelta) -> impl Iterator<Item = &VirtualPath> {
    delta.actions().iter().filter_map(|action| match action {
        RepositoryAction::CreateDirectory { path, .. } => Some(path),
        _ => None,
    })
}

/// The redo record that publishes `delta` over the repository `captured` read.
///
/// A directory the delta creates is no operation of its own: a put creates the
/// directories its path needs (OD-7). A directory no put lands beneath is
/// therefore not created, and is made by the first mutation that writes into it.
/// A mode change republishes the captured file's own bytes under the new mode,
/// which the write-set comparison has just confirmed are still the live ones.
fn redo_record_for(
    captured: &RepositoryImage,
    delta: &RepositoryDelta,
) -> Result<RedoRecord, RepositoryStateStoreError> {
    let puts = delta
        .actions()
        .iter()
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
    let deletes = delta
        .actions()
        .iter()
        .filter_map(|action| match action {
            RepositoryAction::DeleteFile { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    RedoRecord::new(puts, deletes, None)
        .map_err(|source| RepositoryStateStoreError::Other(source.into()))
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

/// An event log's exact length with its last `min(length, window)` bytes.
#[derive(Debug, PartialEq, Eq)]
struct EventLogTail {
    length: u64,
    tail: Vec<u8>,
}

impl EventLogTail {
    fn of_bytes(bytes: &[u8]) -> Self {
        let window = usize::try_from(EVENT_LOG_TAIL_WINDOW).unwrap_or(usize::MAX);
        Self {
            length: bytes.len() as u64,
            tail: bytes[bytes.len().saturating_sub(window)..].to_vec(),
        }
    }
}

/// Read the live log's length and tail, or `None` when no ordinary file is there.
fn live_event_log_tail(
    roots: &CapabilityRoots,
    path: &VirtualPath,
) -> Result<Option<EventLogTail>, RepositoryStateStoreError> {
    let Some(root) = roots.root(path.root_class()) else {
        return Ok(None);
    };
    let Some((parent, leaf)) = open_capability_parent(root, path.relative().as_path())? else {
        return Ok(None);
    };
    match parent.symlink_metadata(&leaf) {
        Ok(metadata) if metadata.is_file() => (),
        Ok(_) => return Ok(None),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(&leaf, &options)?;
    let length = file.metadata()?.len();
    let mut tail = Vec::new();
    file.seek(SeekFrom::Start(length - length.min(EVENT_LOG_TAIL_WINDOW)))?;
    file.read_to_end(&mut tail)?;
    Ok(Some(EventLogTail { length, tail }))
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
    worktree_paths: &crate::storage::WorktreePaths,
    storage: &JsonFileStorage,
    spec: CaptureSpec,
) -> Result<RepositoryImage, RepositoryStateStoreError> {
    let entries = spec
        .paths()
        .map(|path| {
            let entry = match inspect_capability_entry(layout, roots, path) {
                Ok(entry) => entry,
                Err(error) if spec.is_advisory(path) && is_advisory_permission_denied(&error) => {
                    advisory_unreadable_entry(path)?
                }
                Err(error) => return Err(error),
            };
            Ok((path.clone(), entry))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let listings = spec
        .listings()
        .iter()
        .map(|path| {
            Ok((
                path.clone(),
                inspect_capability_listing(layout, roots, path, spec.is_advisory(path), &entries)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let pinned_reads = storage.inspect_pinned_targets(layout.worktree_root(), spec.pinned());
    let mut evidence_cache = storage.pinned_evidence_cache();
    let pinned = spec
        .pinned()
        .iter()
        .map(|(revision, path)| {
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
    let linked = spec
        .linked_worktree()
        .iter()
        .map(|path| {
            Ok((
                path.clone(),
                capture_linked_evidence(layout, roots, worktree_paths, path)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    RepositoryImage::close(layout.clone(), spec, entries, listings, pinned, linked)
        .map_err(Into::into)
}

fn capture_memory_image(
    layout: &RepositoryLayout,
    state: &MemoryRepositoryState,
    spec: CaptureSpec,
) -> Result<RepositoryImage, RepositoryStateStoreError> {
    let entries = spec
        .paths()
        .map(|path| {
            (
                path.clone(),
                state
                    .entries
                    .get(path)
                    .cloned()
                    .unwrap_or(RepositoryEntry::Absent),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let listings = spec
        .listings()
        .iter()
        .map(|path| memory_listing(layout, state, path).map(|listing| (path.clone(), listing)))
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
    let linked = spec
        .linked_worktree()
        .iter()
        .map(|path| {
            let evidence = match state.entries.get(path) {
                Some(RepositoryEntry::File { content, .. }) => LinkedWorktreeEvidence::new(
                    path.clone(),
                    LinkedWorktreeSourceClass::LocalData,
                    Some(content.identity().clone()),
                    Some(content.bytes().to_vec()),
                )?,
                None | Some(RepositoryEntry::Absent) => LinkedWorktreeEvidence::new(
                    path.clone(),
                    LinkedWorktreeSourceClass::Absent,
                    None,
                    None,
                )?,
                _ => {
                    return Err(RepositoryStateStoreError::UnsafeTarget(format!(
                        "linked-worktree evidence at {path:?} is not a regular file"
                    )))
                }
            };
            Ok((path.clone(), evidence))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    RepositoryImage::close(layout.clone(), spec, entries, listings, pinned, linked)
        .map_err(Into::into)
}

fn memory_listing(
    layout: &RepositoryLayout,
    state: &MemoryRepositoryState,
    path: &VirtualPath,
) -> Result<ListingFingerprint, RepositoryStateStoreError> {
    let physical_parent = layout.resolve(path)?;
    let children = state
        .entries
        .iter()
        .filter(|(candidate, entry)| {
            entry.identity().is_some()
                && layout
                    .resolve(candidate)
                    .ok()
                    .and_then(|physical| physical.parent().map(Path::to_path_buf))
                    .as_ref()
                    == Some(&physical_parent)
        })
        .filter_map(|(candidate, entry)| {
            let physical = layout.resolve(candidate).ok()?;
            Some((
                physical.file_name()?.to_str()?.to_string(),
                entry.identity()?.clone(),
            ))
        })
        .collect();
    // Mirror inspect_capability_listing: a directory yields its fingerprint, an
    // absent path yields the absent fingerprint, and any other kind (file, symlink,
    // unsupported) is an error rather than a fabricated directory listing.
    match state.entries.get(path) {
        Some(RepositoryEntry::Directory { identity, .. }) => {
            ListingFingerprint::for_directory(identity.clone(), children).map_err(Into::into)
        }
        None | Some(RepositoryEntry::Absent) => {
            ListingFingerprint::for_absent().map_err(Into::into)
        }
        Some(_) => Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}"))),
    }
}

/// Require that an action's parent directory already exists, mirroring the JSON
/// kernel's `open_parent` (which walks every intermediate component under the root
/// capability and fails with `MissingParent` when one is absent). Only intermediate
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
    let parent_path = VirtualPath::from_root(path.root_class(), RootRelativePath::parse(parent)?)?;
    if matches!(
        state.entries.get(&parent_path),
        Some(RepositoryEntry::Directory { .. })
    ) {
        Ok(())
    } else {
        Err(RepositoryStateStoreError::Transaction(anyhow::anyhow!(
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
    }
    state.data_root_exists |= action.path().root_class() == RepositoryRootClass::Data;
    Ok(())
}

fn inspect_capability_entry(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    path: &VirtualPath,
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
    inspect_capability_leaf(&parent, &leaf)
}

fn advisory_unreadable_entry(
    path: &VirtualPath,
) -> Result<RepositoryEntry, RepositoryStateStoreError> {
    Ok(RepositoryEntry::Unsupported {
        identity: EntryIdentity::for_bytes(
            format!("advisory-unreadable:{}", path.relative().as_str()),
            b"",
        )?,
        reason: "advisory citation scan could not read this entry".into(),
        mode: FileMode::Regular,
    })
}

/// Fingerprint the directory at `path`. A child this capture already inspected
/// as an exact path in `captured` keeps that identity, so each captured file is
/// read once and the image states one identity for it.
fn inspect_capability_listing(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    path: &VirtualPath,
    advisory: bool,
    captured: &BTreeMap<VirtualPath, RepositoryEntry>,
) -> Result<ListingFingerprint, RepositoryStateStoreError> {
    let entry = inspect_capability_entry(layout, roots, path)?;
    let RepositoryEntry::Directory { identity, .. } = entry else {
        return match entry {
            RepositoryEntry::Absent => ListingFingerprint::for_absent().map_err(Into::into),
            _ => Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}"))),
        };
    };
    let root = roots.root(path.root_class()).expect("directory is present");
    let directory = if path.relative().is_root() {
        root.try_clone()?
    } else {
        match open_descendant_dir_nofollow(root, path.relative().as_path()) {
            Ok(directory) => directory,
            Err(error) if advisory && is_advisory_permission_denied(&error) => {
                return ListingFingerprint::for_advisory_unreadable(identity).map_err(Into::into)
            }
            Err(error) => return Err(error),
        }
    };
    let physical_directory = layout.resolve(path)?;
    let mut children = BTreeMap::new();
    let entries = match directory.entries() {
        Ok(entries) => entries,
        Err(error) if advisory && error.kind() == ErrorKind::PermissionDenied => {
            return ListingFingerprint::for_advisory_unreadable(identity).map_err(Into::into)
        }
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) if advisory && error.kind() == ErrorKind::PermissionDenied => {
                return ListingFingerprint::for_advisory_unreadable(identity).map_err(Into::into)
            }
            Err(error) => return Err(error.into()),
        };
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| RepositoryStateStoreError::UnsafeTarget("non-UTF-8 entry".into()))?;
        let child = layout.classify_and_canonicalize(physical_directory.join(&name))?;
        let identity = match captured.get(&child) {
            Some(entry) => entry.identity().cloned(),
            None => match inspect_capability_entry(layout, roots, &child) {
                Ok(entry) => entry.identity().cloned(),
                Err(error) if advisory && is_advisory_permission_denied(&error) => {
                    advisory_unreadable_entry(&child)?.identity().cloned()
                }
                Err(error) => return Err(error),
            },
        }
        .ok_or_else(|| RepositoryStateStoreError::RetryableConflict { path: name.clone() })?;
        children.insert(name, identity);
    }
    ListingFingerprint::for_directory(identity, children).map_err(Into::into)
}

fn inspect_capability_leaf(
    parent: &Dir,
    leaf: &str,
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
    if metadata.is_file() {
        let mut options = OpenOptions::new();
        options.read(true);
        options._cap_fs_ext_follow(FollowSymlinks::No);
        let mut file = parent.open_with(leaf, &options)?;
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

fn capture_linked_evidence(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    worktree_paths: &crate::storage::WorktreePaths,
    path: &VirtualPath,
) -> Result<LinkedWorktreeEvidence, RepositoryStateStoreError> {
    match inspect_capability_entry(layout, roots, path)? {
        RepositoryEntry::File { content, .. } => {
            return LinkedWorktreeEvidence::new(
                path.clone(),
                LinkedWorktreeSourceClass::LocalData,
                Some(content.identity().clone()),
                Some(content.into_bytes()),
            )
            .map_err(Into::into)
        }
        RepositoryEntry::Absent => {}
        _ => {
            return Err(RepositoryStateStoreError::UnsafeTarget(format!(
                "linked-worktree evidence at {path:?} is not a regular file"
            )))
        }
    }
    let Ok(data_prefix) = layout.data_root().strip_prefix(layout.worktree_root()) else {
        return LinkedWorktreeEvidence::new(
            path.clone(),
            LinkedWorktreeSourceClass::Absent,
            None,
            None,
        )
        .map_err(Into::into);
    };
    let git_path = data_prefix.join(path.relative().as_path());
    let Some(git_path) = git_path.to_str() else {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            git_path.display().to_string(),
        ));
    };
    if let Ok(read) = crate::storage::GitRevisionResolver::new(layout.worktree_root())
        .read_pinned_path("HEAD", git_path)
    {
        let bytes = read.bytes().to_vec();
        return LinkedWorktreeEvidence::new(
            path.clone(),
            LinkedWorktreeSourceClass::GitHead,
            Some(EntryIdentity::for_bytes(
                format!("git-blob:{}", read.blob_oid()),
                &bytes,
            )?),
            Some(bytes),
        )
        .map_err(Into::into);
    }
    if let (Some(main_root), Some(main_data)) = (
        worktree_paths.primary_worktree_root()?,
        worktree_paths.primary_data_root()?,
    ) {
        if let Ok(main_layout) = discover_repository_layout(main_root, &main_data) {
            let (main_roots, _, _) = CapabilityRoots::open(&main_layout)?;
            match inspect_capability_entry(&main_layout, &main_roots, path)? {
                RepositoryEntry::File { content, .. } => {
                    return LinkedWorktreeEvidence::new(
                        path.clone(),
                        LinkedWorktreeSourceClass::MainWorktree,
                        Some(content.identity().clone()),
                        Some(content.into_bytes()),
                    )
                    .map_err(Into::into)
                }
                RepositoryEntry::Absent => {}
                _ => {
                    return Err(RepositoryStateStoreError::UnsafeTarget(format!(
                        "main-worktree evidence at {path:?} is not a regular file"
                    )))
                }
            }
        }
    }
    LinkedWorktreeEvidence::new(path.clone(), LinkedWorktreeSourceClass::Absent, None, None)
        .map_err(Into::into)
}

/// Open the parent directory of a repository-relative target for capture.
///
/// `Ok(None)` means an ancestor directory does not exist, so the target captures
/// as absence. A symlinked or non-directory ancestor stays a hard error: it is an
/// unsafe target, not a clean absence.
pub(in crate::storage) fn open_capability_parent(
    root: &Dir,
    path: &Path,
) -> Result<Option<(Dir, String)>, RepositoryStateStoreError> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) = components
        .split_last()
        .ok_or_else(|| RepositoryStateStoreError::UnsafeTarget(path.display().to_string()))?;
    let mut current = root.try_clone()?;
    for component in parents {
        match current.symlink_metadata(component) {
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                return Err(RepositoryStateStoreError::UnsafeTarget(component.clone()))
            }
            Ok(_) => current = open_child_dir_nofollow(&current, component)?,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some((current, leaf.clone())))
}

/// Open the parent directory of a repository-relative target, creating the
/// ancestors it lacks.
///
/// Each directory it creates is synced into the parent that now names it before
/// the walk descends, so a file published underneath is reachable after a crash
/// rather than hanging off an entry the filesystem never wrote
/// (`@/inv/atomic-writes`). An ancestor that already exists costs no sync.
pub(in crate::storage) fn open_or_create_capability_parent(
    root: &Dir,
    path: &Path,
) -> Result<(Dir, String), RepositoryStateStoreError> {
    let components = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) = components
        .split_last()
        .ok_or_else(|| RepositoryStateStoreError::UnsafeTarget(path.display().to_string()))?;
    parents
        .iter()
        .try_fold(root.try_clone()?, |current, component| {
            match current.symlink_metadata(component) {
                Err(error) if error.kind() == ErrorKind::NotFound => {
                    current.create_dir(component)?;
                    sync_capability_directory(&current)?;
                }
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
            open_child_dir_nofollow(&current, component)
        })
        .map(|parent| (parent, leaf.clone()))
}

fn open_descendant_dir_nofollow(root: &Dir, path: &Path) -> Result<Dir, RepositoryStateStoreError> {
    let mut current = root.try_clone()?;
    for component in path.components() {
        current = open_child_dir_nofollow(&current, Path::new(component.as_os_str()))?;
    }
    Ok(current)
}

pub(crate) fn open_child_dir_nofollow(
    parent: &Dir,
    name: impl AsRef<Path>,
) -> Result<Dir, RepositoryStateStoreError> {
    let name = name.as_ref();
    let metadata = parent.symlink_metadata(name)?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            name.display().to_string(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    options._cap_fs_ext_maybe_dir(true);
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            name.display().to_string(),
        ));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

/// Open `leaf` under `parent` as an ordinary file, refusing a symlink or any
/// non-regular target without following it.
pub(crate) fn open_regular_file_nofollow(
    parent: &Dir,
    leaf: &str,
) -> Result<cap_std::fs::File, RepositoryStateStoreError> {
    let metadata = parent.symlink_metadata(leaf)?;
    if metadata.is_symlink() || !metadata.is_file() {
        return Err(RepositoryStateStoreError::UnsafeTarget(leaf.to_string()));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let file = parent.open_with(leaf, &options)?;
    if !file.metadata()?.is_file() {
        return Err(RepositoryStateStoreError::UnsafeTarget(leaf.to_string()));
    }
    Ok(file)
}

pub(crate) fn open_absolute_dir_nofollow(path: &Path) -> Result<Dir, RepositoryStateStoreError> {
    if !path.is_absolute() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            path.display().to_string(),
        ));
    }
    let mut anchor = PathBuf::new();
    let mut descendants = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => anchor.push(component.as_os_str()),
            Component::Normal(name) => descendants.push(name),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(RepositoryStateStoreError::UnsafeTarget(
                    path.display().to_string(),
                ))
            }
        }
    }
    let mut current = Dir::open_ambient_dir(&anchor, ambient_authority())?;
    for name in descendants {
        current = open_child_dir_nofollow(&current, name)?;
    }
    Ok(current)
}

/// Read one validated root-relative ordinary file without following links.
///
/// `Ok(None)` means the leaf or one of its parents is absent. Symlinked
/// ancestors, symlink leaves, directories, and special files are unsafe rather
/// than absence.
pub(crate) fn read_repository_file_nofollow(
    root: &Path,
    relative: &RootRelativePath,
) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
    if relative.is_root() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            root.display().to_string(),
        ));
    }
    let root = open_absolute_dir_nofollow(root)?;
    let Some((parent, leaf)) = open_capability_parent(&root, relative.as_path())? else {
        return Ok(None);
    };
    let metadata = match parent.symlink_metadata(&leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() || !metadata.is_file() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            relative.as_path().display().to_string(),
        ));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(&leaf, &options)?;
    if !file.metadata()?.is_file() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            relative.as_path().display().to_string(),
        ));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(Some(bytes))
}

/// Read one validated root-relative ordinary file through a confined root handle.
///
/// Symlinks may be followed, but capability resolution keeps every traversal
/// beneath `root`; a raced path cannot redirect the final open outside it.
pub(crate) fn read_repository_file_confined(
    root: &Path,
    relative: &RootRelativePath,
) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
    if relative.is_root() {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            root.display().to_string(),
        ));
    }
    let root_dir = open_absolute_dir_nofollow(root)?;
    read_confined_relative(root, &root_dir, relative.as_path(), 0)
}

fn read_confined_relative(
    root_path: &Path,
    root: &Dir,
    relative: &Path,
    symlink_depth: usize,
) -> Result<Option<Vec<u8>>, RepositoryStateStoreError> {
    if symlink_depth >= 40 {
        return Err(RepositoryStateStoreError::UnsafeTarget(
            relative.display().to_string(),
        ));
    }
    let components = relative
        .components()
        .map(|component| match component {
            Component::Normal(name) => Ok(name.to_os_string()),
            _ => Err(RepositoryStateStoreError::UnsafeTarget(
                relative.display().to_string(),
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut current = root.try_clone()?;
    let mut traversed = PathBuf::new();
    for (index, component) in components.iter().enumerate() {
        let metadata = match current.symlink_metadata(component) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.is_symlink() {
            let target = current.read_link_contents(component)?;
            let resolved =
                resolve_confined_symlink(root_path, &traversed, &target, &components[index + 1..])?;
            return read_confined_relative(root_path, root, &resolved, symlink_depth + 1);
        }
        if index + 1 == components.len() {
            if !metadata.is_file() {
                return Err(RepositoryStateStoreError::UnsafeTarget(
                    relative.display().to_string(),
                ));
            }
            let mut options = OpenOptions::new();
            options.read(true);
            options._cap_fs_ext_follow(FollowSymlinks::No);
            let mut file = current.open_with(component, &options)?;
            if !file.metadata()?.is_file() {
                return Err(RepositoryStateStoreError::UnsafeTarget(
                    relative.display().to_string(),
                ));
            }
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            return Ok(Some(bytes));
        }
        if !metadata.is_dir() {
            return Err(RepositoryStateStoreError::UnsafeTarget(
                relative.display().to_string(),
            ));
        }
        current = open_child_dir_nofollow(&current, component)?;
        traversed.push(component);
    }
    Err(RepositoryStateStoreError::UnsafeTarget(
        relative.display().to_string(),
    ))
}

fn resolve_confined_symlink(
    root: &Path,
    parent: &Path,
    target: &Path,
    tail: &[std::ffi::OsString],
) -> Result<PathBuf, RepositoryStateStoreError> {
    let is_absolute = target.is_absolute();
    let target = if is_absolute {
        target
            .strip_prefix(root)
            .map_err(|_| RepositoryStateStoreError::UnsafeTarget(target.display().to_string()))?
    } else {
        target
    };
    let mut resolved = if is_absolute {
        PathBuf::new()
    } else {
        parent.to_path_buf()
    };
    for component in target.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(name) => resolved.push(name),
            Component::ParentDir if resolved.pop() => {}
            _ => {
                return Err(RepositoryStateStoreError::UnsafeTarget(
                    target.display().to_string(),
                ))
            }
        }
    }
    resolved.extend(tail);
    Ok(resolved)
}

fn ensure_capability_identity(
    directory: &Dir,
    expected: &str,
    path: &Path,
) -> Result<(), RepositoryStateStoreError> {
    let actual = capability_metadata_identity(&directory.dir_metadata()?)?;
    if actual == expected {
        Ok(())
    } else {
        Err(RepositoryStateStoreError::RetryableConflict {
            path: path.display().to_string(),
        })
    }
}

/// Directory-capability identity in the same `dev:ino` form the layout records
/// for the absent-root parent. `None` on platforms without stable inode identity,
/// where the absent-root parent binding is skipped.
pub(crate) fn capability_dir_identity(directory: &Dir) -> Option<String> {
    #[cfg(any(unix, windows))]
    {
        let metadata = directory.dir_metadata().ok()?;
        capability_metadata_identity(&metadata).ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = directory;
        None
    }
}

#[cfg(unix)]
fn capability_metadata_identity(
    metadata: &cap_std::fs::Metadata,
) -> Result<String, RepositoryStateStoreError> {
    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn capability_metadata_identity(
    metadata: &cap_std::fs::Metadata,
) -> Result<String, RepositoryStateStoreError> {
    let volume =
        cap_primitives::fs::_WindowsByHandle::volume_serial_number(metadata).ok_or_else(|| {
            RepositoryStateStoreError::UnsafeTarget("Windows volume identity unavailable".into())
        })?;
    let index = cap_primitives::fs::_WindowsByHandle::file_index(metadata).ok_or_else(|| {
        RepositoryStateStoreError::UnsafeTarget("Windows file identity unavailable".into())
    })?;
    Ok(format!("{volume}:{index}"))
}

#[cfg(not(any(unix, windows)))]
fn capability_metadata_identity(
    metadata: &cap_std::fs::Metadata,
) -> Result<String, RepositoryStateStoreError> {
    Ok(format!(
        "{}:{}",
        metadata.len(),
        metadata.permissions().readonly()
    ))
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
    match (&roots.data, layout.data_root().exists()) {
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
#[path = "repository_state_store_tests.rs"]
mod tests;
