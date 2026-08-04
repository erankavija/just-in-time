//! Recovered layout-aware repository capture and mutation sessions.
//!
//! Session orchestration lives here; durable publication and recovery belong to
//! the capability-confined [`FileTransactionKernel`].

use crate::repository_state::{
    CaptureError, CaptureSpec, EntryIdentity, ExpectedPreimage, FileMode, LinkedWorktreeEvidence,
    LinkedWorktreeSourceClass, ListingFingerprint, MaterializationPlan, PinnedDocumentEvidence,
    PinnedSourceClass, RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage,
    RepositoryLayout, RepositoryLayoutError, RepositoryRootClass, RepositoryRootEvidence,
    RootRelativePath, VirtualPath,
};
use crate::storage::file_transaction::RepositoryRecoveryDisposition;
use crate::storage::memory::{MemoryRecoveryResidue, MemoryRepositoryState};
use crate::storage::{
    FileLocker, FileTransactionKernel, InMemoryStorage, IssueStore, JsonFileStorage,
    RepoWriteGuard, TransactionControlLocation, TransactionFailurePoint,
};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use cap_std::{ambient_authority, fs::MetadataExt as _};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Successful application of one exact repository delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryApplyOutcome {
    /// Complete semantic plan hash supplied unchanged to storage and recovery.
    pub transaction_hash: String,
    /// Number of normalized actions applied.
    pub actions_applied: usize,
}

/// Transaction journals recovered while opening one repository mutation session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecoveryDispatchReport {
    /// External journals or orphan controls recovered in deterministic id order.
    pub external_transactions: Vec<String>,
    /// Internal data-root journals recovered in deterministic id order.
    pub internal_transactions: Vec<String>,
}

impl RecoveryDispatchReport {
    /// Total number of transaction journals recovered.
    pub fn recovered_count(&self) -> usize {
        self.external_transactions.len() + self.internal_transactions.len()
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
    #[error("unsafe repository mutation target: {0}")]
    UnsafeTarget(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
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

struct CapabilityRoots {
    worktree: Dir,
    data: Option<Dir>,
}

impl CapabilityRoots {
    fn open(layout: &RepositoryLayout) -> Result<(Self, Dir, String), RepositoryStateStoreError> {
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

    fn root(&self, class: RepositoryRootClass) -> Option<&Dir> {
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
        let recovery_kernel = FileTransactionKernel::for_repository_layout(
            layout.clone(),
            pre_roots.worktree.try_clone()?,
            pre_roots.data.as_ref().map(Dir::try_clone).transpose()?,
            pre_parent,
            pre_leaf,
            Arc::clone(&injector),
        )?;
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

        let (repository_guard, internal_transactions, external_transactions) =
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
                (
                    Some(repository_guard),
                    internal_transactions,
                    external_transactions,
                )
            } else {
                (None, Vec::new(), external_transactions)
            };
        let events_lock = roots.data.as_ref().map(|_| self.events_lock_spec());

        self.configure_repository_layout(&layout);
        Ok(Box::new(JsonMutationSession {
            layout,
            storage: self.clone(),
            recovery_report: RecoveryDispatchReport {
                external_transactions,
                internal_transactions,
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
        failures.check(&TransactionFailurePoint::RepositoryRecoveryExternal)?;
        let mut state = self.repository_state();
        let data_exists = state.data_root_exists;
        if data_exists {
            failures.check(&TransactionFailurePoint::RepositoryRecoveryInternal)?;
        }
        recover_memory_state(&mut state);
        drop(state);
        if data_exists {
            failures.check(&TransactionFailurePoint::RepositorySweepCompanions)?;
        }
        Ok(Box::new(MemoryMutationSession {
            layout,
            storage: self.clone(),
            recovery_report: RecoveryDispatchReport::default(),
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
        let image = capture_capability_image(&self.layout, &self.roots, spec)?;
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
        // Immediately before journal preparation, revalidate the entire read set
        // and the selected-root identities while the session is held. Any content,
        // listing, root, or alias change yields a typed retryable conflict before
        // anything durable is written.
        revalidate_layout_capabilities(&self.layout, &self.roots)?;
        let current =
            capture_capability_image(&self.layout, &self.roots, image.capture_spec().clone())?;
        if &current != image {
            return Err(RepositoryStateStoreError::RetryableConflict {
                path: first_image_difference(image, &current),
            });
        }
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
        let guard = self
            ._repository_guard
            .as_ref()
            .unwrap_or(&self._bootstrap_guard);
        let transaction_id = uuid::Uuid::new_v4().simple().to_string();
        let published_absent_data_root = self.roots.data.is_none()
            && delta
                .actions()
                .iter()
                .any(|action| action.path().root_class() == RepositoryRootClass::Data);
        let outcome = self
            .kernel
            .execute_repository_delta(guard, &transaction_id, delta, plan.hash())
            .map_err(|error| map_transaction_error(error, &self.layout))?;
        if published_absent_data_root {
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
        }
        self.captured = None;
        Ok(RepositoryApplyOutcome {
            transaction_hash: outcome.plan_hash,
            actions_applied: delta.actions().len(),
        })
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
        let mut state = self.storage.repository_state();
        let current = capture_memory_image(&self.layout, &state, image.capture_spec().clone())?;
        if &current != image {
            return Err(RepositoryStateStoreError::RetryableConflict {
                path: "memory read set".into(),
            });
        }
        let original = state.clone_without_recovery();
        let mut candidate = original.clone();
        let plan_hash = plan.hash().to_string();
        // Mirror the kernel short-circuit: an empty delta is a no-op that creates
        // no control, residue, or injector boundary; return the clean zero-action
        // outcome before any failure check or residue write.
        if delta.actions().is_empty() {
            self.captured = None;
            return Ok(RepositoryApplyOutcome {
                transaction_hash: plan_hash,
                actions_applied: 0,
            });
        }
        state.recovery = Some(MemoryRecoveryResidue::Prepared {
            original: Box::new(original.clone()),
            final_state: Box::new(candidate.clone()),
            _plan_hash: plan_hash.clone(),
        });
        let failures = self.storage.repository_state_failures();
        failures.check(&TransactionFailurePoint::RepositoryBeforeControlCreation)?;
        failures.check(&TransactionFailurePoint::RepositoryCreateControl)?;
        failures.check(&TransactionFailurePoint::RepositoryBeforeInitialJournal)?;
        failures.check(&TransactionFailurePoint::RepositorySyncInitialJournal)?;
        failures.check(&TransactionFailurePoint::RepositoryPrepareIntent)?;
        // Model the kernel's create-companion boundary: an internal transaction
        // (data root already present) publishing a Worktree action creates a
        // worktree-side companion. Failure there converges to the old state.
        let has_worktree_action = delta
            .actions()
            .iter()
            .any(|action| action.path().root_class() == RepositoryRootClass::Worktree);
        if original.data_root_exists && has_worktree_action {
            failures.check(&TransactionFailurePoint::RepositoryCreateCompanion)?;
        }
        if !original.data_root_exists
            && delta
                .actions()
                .iter()
                .any(|action| action.path().root_class() == RepositoryRootClass::Data)
        {
            failures.check(&TransactionFailurePoint::RepositoryBeforeDataStageJournal)?;
        }
        // Model the kernel's per-action prepare boundaries. The memory backend
        // stages nothing, but a failure here must still converge to the old state
        // exactly as the JSON kernel's prepared-journal rollback does.
        for index in 0..delta.actions().len() {
            failures.check(&TransactionFailurePoint::RepositoryPrepareAction { action: index })?;
            failures.check(&TransactionFailurePoint::RepositoryStageAction { action: index })?;
            failures.check(&TransactionFailurePoint::RepositorySyncStage { action: index })?;
            if matches!(
                delta.actions()[index].expected(),
                ExpectedPreimage::File { .. }
            ) {
                failures.check(&TransactionFailurePoint::RepositorySyncBackup { action: index })?;
            }
            failures
                .check(&TransactionFailurePoint::RepositorySyncPreparedAction { action: index })?;
            failures.check(&TransactionFailurePoint::RepositoryBeforePreparedJournal {
                action: index,
            })?;
        }
        for (index, action) in delta.actions().iter().enumerate() {
            // Staged Data actions of an absent-root delta have no per-action
            // publication boundary in the kernel (they land inside the stage and
            // are committed by the single root rename), so skip their Before/After
            // checks to keep the failure-point set identical across backends.
            let staged_absent_data = !original.data_root_exists
                && action.path().root_class() == RepositoryRootClass::Data;
            if !staged_absent_data {
                failures
                    .check(&TransactionFailurePoint::RepositoryBeforeAction { action: index })?;
                failures.check(&TransactionFailurePoint::RepositoryBeforeTargetMutation {
                    action: index,
                })?;
                failures.check(&TransactionFailurePoint::RepositoryAfterRootBindingCheck {
                    action: index,
                })?;
                if matches!(action, RepositoryAction::DeleteFile { .. }) {
                    failures.check(&TransactionFailurePoint::RepositoryBeforeDeleteRename {
                        action: index,
                    })?;
                }
            }
            apply_memory_action(&self.layout, &mut candidate, action)?;
            if let Some(MemoryRecoveryResidue::Prepared { final_state, .. }) = &mut state.recovery {
                **final_state = candidate.clone();
            }
            if !staged_absent_data {
                failures.check(&TransactionFailurePoint::RepositorySyncTargetParent {
                    action: index,
                })?;
                failures.check(&TransactionFailurePoint::RepositoryVerifyFinalIdentity {
                    action: index,
                })?;
                failures.check(&TransactionFailurePoint::RepositoryBeforePublishedJournal {
                    action: index,
                })?;
                failures
                    .check(&TransactionFailurePoint::RepositoryAfterAction { action: index })?;
            }
        }
        let absent_root = !original.data_root_exists && candidate.data_root_exists;
        if absent_root {
            failures.check(&TransactionFailurePoint::RepositoryBeforeDataRootPublication)?;
            failures.check(&TransactionFailurePoint::RepositoryAfterDataParentBindingCheck)?;
            // Publish the staged root: it becomes a real Directory entry, so a
            // later capture of Data("") is a Directory on both backends. Idempotent
            // with an explicit CreateDirectory Data("") (which the kernel does not
            // require — the stage dir is the root). A worktree-only absent-root
            // delta materializes nothing (candidate.data_root_exists stays false).
            let published_root = RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("memory-directory:data-root", b"directory")?,
                mode: FileMode::Executable,
            };
            candidate
                .entries
                .entry(VirtualPath::data("")?)
                .or_insert(published_root);
            // Once the root rename is modeled, prepared recovery converges
            // forward just like the file kernel's published-root detection.
            state.entries = candidate.entries.clone();
            state.data_root_exists = candidate.data_root_exists;
            state.recovery = Some(MemoryRecoveryResidue::Committed {
                final_state: Box::new(candidate.clone()),
                _plan_hash: plan_hash.clone(),
            });
            failures.check(&TransactionFailurePoint::RepositoryAfterDataRootPublication)?;
        }
        failures.check(&TransactionFailurePoint::RepositoryBeforeCommitDecision)?;
        state.entries = candidate.entries.clone();
        state.data_root_exists = candidate.data_root_exists;
        state.recovery = Some(MemoryRecoveryResidue::Committed {
            final_state: Box::new(candidate),
            _plan_hash: plan_hash.clone(),
        });
        failures.check(&TransactionFailurePoint::RepositoryAfterCommit)?;
        // Model the kernel's post-commit cleanup boundary: past the commit point a
        // failure leaves committed residue and converges forward to the new state.
        failures.check(&TransactionFailurePoint::RepositoryCleanup)?;
        if !absent_root {
            failures.check(&TransactionFailurePoint::RepositoryBeforeStageCleanup)?;
        }
        if original.data_root_exists && has_worktree_action {
            failures.check(&TransactionFailurePoint::RepositoryBeforeCompanionCleanup)?;
        }
        failures.check(&TransactionFailurePoint::RepositoryBeforeControlCleanup)?;
        state.recovery = None;
        self.captured = None;
        Ok(RepositoryApplyOutcome {
            transaction_hash: plan_hash,
            actions_applied: delta.actions().len(),
        })
    }
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
                inspect_capability_listing(layout, roots, path, spec.is_advisory(path))?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let resolver = crate::storage::GitRevisionResolver::new(layout.worktree_root());
    let pinned = spec
        .pinned()
        .iter()
        .map(|(revision, path)| {
            let evidence = match resolver.inspect_pinned_target(revision, path) {
                Ok(read) => {
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
    let linked = spec
        .linked_worktree()
        .iter()
        .map(|path| Ok((path.clone(), capture_linked_evidence(layout, roots, path)?)))
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
                Some(RepositoryEntry::File {
                    identity, bytes, ..
                }) => LinkedWorktreeEvidence::new(
                    path.clone(),
                    LinkedWorktreeSourceClass::LocalData,
                    Some(identity.clone()),
                    Some(bytes.clone()),
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

fn recover_memory_state(state: &mut MemoryRepositoryState) {
    match state.recovery.take() {
        Some(MemoryRecoveryResidue::Prepared { original, .. }) => {
            state.entries = original.entries;
            state.data_root_exists = original.data_root_exists;
        }
        Some(MemoryRecoveryResidue::Committed { final_state, .. }) => {
            state.entries = final_state.entries;
            state.data_root_exists = final_state.data_root_exists;
        }
        None => {}
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
                    identity: EntryIdentity::for_bytes(format!("memory-file:{path:?}"), bytes)?,
                    bytes: bytes.clone(),
                    mode: *mode,
                },
            );
        }
        RepositoryAction::SetMode { path, mode, .. } => {
            let RepositoryEntry::File {
                identity, bytes, ..
            } = current
            else {
                return Err(RepositoryStateStoreError::UnsafeTarget(format!("{path:?}")));
            };
            state.entries.insert(
                path.clone(),
                RepositoryEntry::File {
                    identity,
                    bytes,
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

fn inspect_capability_listing(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    path: &VirtualPath,
    advisory: bool,
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
        let child_entry = match inspect_capability_entry(layout, roots, &child) {
            Ok(entry) => entry,
            Err(error) if advisory && is_advisory_permission_denied(&error) => {
                advisory_unreadable_entry(&child)?
            }
            Err(error) => return Err(error),
        };
        let identity = child_entry
            .identity()
            .cloned()
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
            identity: entry_identity(&metadata, &bytes)?,
            target: bytes,
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
            identity: entry_identity(&metadata, &bytes)?,
            bytes,
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
    path: &VirtualPath,
) -> Result<LinkedWorktreeEvidence, RepositoryStateStoreError> {
    match inspect_capability_entry(layout, roots, path)? {
        RepositoryEntry::File {
            identity, bytes, ..
        } => {
            return LinkedWorktreeEvidence::new(
                path.clone(),
                LinkedWorktreeSourceClass::LocalData,
                Some(identity),
                Some(bytes),
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
    if let Some(main_root) = discover_main_worktree(layout.worktree_root())? {
        let main_data = main_root.join(data_prefix);
        if let Ok(main_layout) = discover_repository_layout(&main_root, &main_data) {
            let (main_roots, _, _) = CapabilityRoots::open(&main_layout)?;
            match inspect_capability_entry(&main_layout, &main_roots, path)? {
                RepositoryEntry::File {
                    identity, bytes, ..
                } => {
                    return LinkedWorktreeEvidence::new(
                        path.clone(),
                        LinkedWorktreeSourceClass::MainWorktree,
                        Some(identity),
                        Some(bytes),
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

fn discover_main_worktree(worktree: &Path) -> Result<Option<PathBuf>, RepositoryStateStoreError> {
    let common = Command::new("git")
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .current_dir(worktree)
        .output();
    let Ok(common) = common else { return Ok(None) };
    if !common.status.success() {
        return Ok(None);
    }
    let common = PathBuf::from(String::from_utf8_lossy(&common.stdout).trim());
    let Some(main) = common.parent() else {
        return Ok(None);
    };
    let main = lexical_absolute(main)?;
    if main == worktree {
        Ok(None)
    } else {
        Ok(Some(main))
    }
}

/// Open the parent directory of a repository-relative target for capture.
///
/// `Ok(None)` means an ancestor directory does not exist, so the target captures
/// as absence. A symlinked or non-directory ancestor stays a hard error: it is an
/// unsafe target, not a clean absence.
fn open_capability_parent(
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

fn first_image_difference(expected: &RepositoryImage, actual: &RepositoryImage) -> String {
    expected
        .entries()
        .iter()
        .find(|(path, entry)| actual.entries().get(*path) != Some(*entry))
        .map(|(path, _)| format!("{path:?}"))
        .or_else(|| {
            expected
                .listing_fingerprints()
                .iter()
                .find_map(|(path, listing)| {
                    (actual.listing_fingerprints().get(path) != Some(listing))
                        .then(|| format!("listing {path:?}"))
                })
        })
        .unwrap_or_else(|| "captured evidence".into())
}

#[cfg(test)]
#[path = "repository_state_store_contention_tests.rs"]
mod repository_state_store_contention_tests;
#[cfg(test)]
#[path = "repository_state_store_tests.rs"]
mod tests;
