//! Recovered layout-aware repository capture and mutation sessions.
//!
//! Session orchestration lives here; durable publication and recovery belong to
//! the capability-confined [`FileTransactionKernel`].

use crate::repository_state::{
    CaptureError, CaptureSpec, EntryIdentity, ExpectedPreimage, FileMode, LinkedWorktreeEvidence,
    LinkedWorktreeSourceClass, ListingFingerprint, PinnedDocumentEvidence, PinnedSourceClass,
    RepositoryAction, RepositoryDelta, RepositoryEntry, RepositoryImage, RepositoryLayout,
    RepositoryLayoutError, RepositoryRootClass, RepositoryRootEvidence, RootRelativePath,
    VirtualPath,
};
use crate::storage::memory::{MemoryRecoveryResidue, MemoryRepositoryState};
use crate::storage::{
    FileTransactionKernel, InMemoryStorage, IssueStore, JsonFileStorage, RepoWriteGuard,
    TransactionControlLocation, TransactionFailurePoint,
};
use cap_primitives::fs::FollowSymlinks;
use cap_std::fs::{Dir, OpenOptions};
use cap_std::{ambient_authority, fs::MetadataExt as _};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Successful application of one exact repository delta.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryApplyOutcome {
    /// Backend-independent hash of normalized action semantics.
    pub transaction_hash: String,
    /// Number of normalized actions applied.
    pub actions_applied: usize,
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

/// Guards that a storage instance is reentered only for one canonical layout.
///
/// A retained CLI session holds the reentrant lock chain while dispatch opens a
/// second session; that reentry is legitimate only for the same selected roots.
/// A different worktree or data root while a session is live is a programming
/// error the session boundary rejects rather than silently serving stale roots.
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
    /// Capture one complete bounded image after recovery has converged.
    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError>;
    /// Revalidate the complete image and publish one normalized exact delta.
    fn apply(
        &mut self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError>;
}

/// Storage backend capable of one recovered capture/apply mutation boundary.
pub trait RepositoryStateStore {
    /// Open a recovered session for exactly `layout`.
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession + '_>, RepositoryStateStoreError>;
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
    roots: CapabilityRoots,
    kernel: FileTransactionKernel,
    /// Worktree-root bootstrap lock, held outermost so the worktree-side
    /// `.jit-bootstrap` namespace is serialized across disjoint-data-root sessions.
    _worktree_bootstrap_guard: RepoWriteGuard,
    _bootstrap_guard: RepoWriteGuard,
    _repository_guard: Option<RepoWriteGuard>,
    _events_guard: Option<crate::storage::lock::LockGuard>,
    _reentry: LayoutReentry,
    captured: Option<RepositoryImage>,
}

struct MemoryMutationSession<'a> {
    layout: RepositoryLayout,
    storage: &'a InMemoryStorage,
    _guard: RepoWriteGuard,
    _reentry: LayoutReentry,
    captured: Option<RepositoryImage>,
}

impl RepositoryStateStore for JsonFileStorage {
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession + '_>, RepositoryStateStoreError> {
        if lexical_absolute(self.root())? != layout.data_root() {
            return Err(
                RepositoryLayoutError::OutsideRepositoryRoots(self.root().to_path_buf()).into(),
            );
        }
        // A retained session may reenter the reentrant lock chain only for the
        // same canonical layout.
        self.active_mutation_layout().enter(&layout)?;
        let reentry = LayoutReentry(self.active_mutation_layout());
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
        recover_location(
            &recovery_kernel,
            &bootstrap_guard,
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

        let (repository_guard, events_guard) = if roots.data.is_some() {
            let repository_guard = self.acquire_repo_write_lock_raw()?;
            let events_guard = self.acquire_events_write_lock()?;
            injector.check(&TransactionFailurePoint::RepositoryRecoveryInternal)?;
            recover_location(
                &kernel,
                &repository_guard,
                TransactionControlLocation::InternalRepository,
            )?;
            // After internal-journal recovery, reclaim worktree-side companions
            // left orphaned by a crash whose internal transaction is already gone.
            kernel.sweep_orphan_companions(&repository_guard)?;
            (Some(repository_guard), Some(events_guard))
        } else {
            (None, None)
        };

        Ok(Box::new(JsonMutationSession {
            layout,
            roots,
            kernel,
            _worktree_bootstrap_guard: worktree_bootstrap_guard,
            _bootstrap_guard: bootstrap_guard,
            _repository_guard: repository_guard,
            _events_guard: events_guard,
            _reentry: reentry,
            captured: None,
        }))
    }
}

impl RepositoryStateStore for InMemoryStorage {
    fn open_mutation_session(
        &self,
        layout: RepositoryLayout,
    ) -> Result<Box<dyn RepositoryMutationSession + '_>, RepositoryStateStoreError> {
        self.active_mutation_layout().enter(&layout)?;
        let reentry = LayoutReentry(self.active_mutation_layout());
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
            storage: self,
            _guard: guard,
            _reentry: reentry,
            captured: None,
        }))
    }
}

impl RepositoryMutationSession for JsonMutationSession {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        revalidate_layout_capabilities(&self.layout, &self.roots)?;
        let image = capture_capability_image(&self.layout, &self.roots, spec)?;
        self.captured = Some(image.clone());
        Ok(image)
    }

    fn apply(
        &mut self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
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
        let guard = self
            ._repository_guard
            .as_ref()
            .unwrap_or(&self._bootstrap_guard);
        let transaction_id = uuid::Uuid::new_v4().simple().to_string();
        let outcome = self
            .kernel
            .execute_repository_delta(guard, &transaction_id, delta)
            .map_err(|error| map_transaction_error(error, &self.layout))?;
        self.captured = None;
        Ok(RepositoryApplyOutcome {
            transaction_hash: outcome.plan_hash,
            actions_applied: delta.actions().len(),
        })
    }
}

impl RepositoryMutationSession for MemoryMutationSession<'_> {
    fn layout(&self) -> &RepositoryLayout {
        &self.layout
    }

    fn capture(&mut self, spec: CaptureSpec) -> Result<RepositoryImage, RepositoryStateStoreError> {
        let state = self.storage.repository_state();
        let image = capture_memory_image(&self.layout, &state, spec)?;
        self.captured = Some(image.clone());
        Ok(image)
    }

    fn apply(
        &mut self,
        image: &RepositoryImage,
        delta: &RepositoryDelta,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
        ensure_session_image(&self.layout, self.captured.as_ref(), image)?;
        ensure_delta_is_captured(image, delta)?;
        let mut state = self.storage.repository_state();
        let current = capture_memory_image(&self.layout, &state, image.capture_spec().clone())?;
        if &current != image {
            return Err(RepositoryStateStoreError::RetryableConflict {
                path: "memory read set".into(),
            });
        }
        let original = state.clone_without_recovery();
        let mut candidate = original.clone();
        let plan_hash = semantic_delta_hash(delta)?;
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
        // Model the kernel's per-action prepare boundaries. The memory backend
        // stages nothing, but a failure here must still converge to the old state
        // exactly as the JSON kernel's prepared-journal rollback does.
        for index in 0..delta.actions().len() {
            failures.check(&TransactionFailurePoint::RepositoryPrepareAction { action: index })?;
            failures
                .check(&TransactionFailurePoint::RepositorySyncPreparedAction { action: index })?;
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
            }
            apply_memory_action(&self.layout, &mut candidate, action)?;
            if let Some(MemoryRecoveryResidue::Prepared { final_state, .. }) = &mut state.recovery {
                **final_state = candidate.clone();
            }
            if !staged_absent_data {
                failures
                    .check(&TransactionFailurePoint::RepositoryAfterAction { action: index })?;
            }
        }
        let absent_root = !original.data_root_exists && candidate.data_root_exists;
        if absent_root {
            failures.check(&TransactionFailurePoint::RepositoryBeforeDataRootPublication)?;
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
        }
        state.entries = candidate.entries.clone();
        state.data_root_exists = candidate.data_root_exists;
        state.recovery = Some(MemoryRecoveryResidue::Committed {
            final_state: Box::new(candidate),
            _plan_hash: plan_hash.clone(),
        });
        if absent_root {
            failures.check(&TransactionFailurePoint::RepositoryAfterDataRootPublication)?;
        }
        failures.check(&TransactionFailurePoint::RepositoryAfterCommit)?;
        // Model the kernel's post-commit cleanup boundary: past the commit point a
        // failure leaves committed residue and converges forward to the new state.
        failures.check(&TransactionFailurePoint::RepositoryCleanup)?;
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
    guard: &RepoWriteGuard,
    location: TransactionControlLocation,
) -> Result<(), RepositoryStateStoreError> {
    for id in kernel.pending_repository_transactions(location)? {
        kernel.recover_repository_transaction(guard, location, &id)?;
    }
    Ok(())
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

fn capture_capability_image(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    spec: CaptureSpec,
) -> Result<RepositoryImage, RepositoryStateStoreError> {
    let entries = spec
        .paths()
        .map(|path| Ok((path.clone(), inspect_capability_entry(layout, roots, path)?)))
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let listings = spec
        .listings()
        .iter()
        .map(|path| {
            Ok((
                path.clone(),
                inspect_capability_listing(layout, roots, path)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, RepositoryStateStoreError>>()?;
    let resolver = crate::storage::GitRevisionResolver::new(layout.worktree_root());
    let pinned = spec
        .pinned()
        .iter()
        .map(|(revision, path)| {
            let evidence = match resolver.read_pinned_path(revision, path) {
                Ok(read) => {
                    let bytes = read.bytes().to_vec();
                    let commit = read.version().as_str().to_string();
                    let blob_oid = read.blob_oid().to_string();
                    PinnedDocumentEvidence::new(
                        revision.clone(),
                        path.clone(),
                        PinnedSourceClass::GitObject,
                        Some(commit.clone()),
                        Some(blob_oid.clone()),
                        Some(EntryIdentity::for_bytes(
                            format!("git-blob:{blob_oid}"),
                            &bytes,
                        )?),
                        Some(bytes),
                        None,
                    )?
                }
                Err(error) => PinnedDocumentEvidence::new(
                    revision.clone(),
                    path.clone(),
                    PinnedSourceClass::GitUnavailable,
                    None,
                    None,
                    None,
                    None,
                    Some(error.to_string()),
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
        .map(|path| memory_listing(state, path).map(|listing| (path.clone(), listing)))
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
    state: &MemoryRepositoryState,
    path: &VirtualPath,
) -> Result<ListingFingerprint, RepositoryStateStoreError> {
    let children = state
        .entries
        .iter()
        .filter(|(candidate, entry)| {
            candidate.root_class() == path.root_class()
                && entry.identity().is_some()
                && candidate.relative().as_path().parent() == Some(path.relative().as_path())
        })
        .filter_map(|(candidate, entry)| {
            Some((
                candidate
                    .relative()
                    .as_path()
                    .file_name()?
                    .to_str()?
                    .to_string(),
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

fn semantic_delta_hash(delta: &RepositoryDelta) -> Result<String, RepositoryStateStoreError> {
    let bytes = serde_json::to_vec(delta).map_err(anyhow::Error::from)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
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

fn inspect_capability_listing(
    layout: &RepositoryLayout,
    roots: &CapabilityRoots,
    path: &VirtualPath,
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
        open_descendant_dir_nofollow(root, path.relative().as_path())?
    };
    let mut children = BTreeMap::new();
    for entry in directory.entries()? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| RepositoryStateStoreError::UnsafeTarget("non-UTF-8 entry".into()))?;
        let relative = if path.relative().is_root() {
            RootRelativePath::parse(&name)?
        } else {
            RootRelativePath::parse(format!("{}/{}", path.relative().as_path().display(), name))?
        };
        let child = VirtualPath::from_root(path.root_class(), relative)?;
        let identity = inspect_capability_entry(layout, roots, &child)?
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
    #[cfg(unix)]
    let object = format!("{}:{}", metadata.dev(), metadata.ino());
    #[cfg(not(unix))]
    let object = format!("{}:{}", metadata.len(), metadata.permissions().readonly());
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
        current = open_child_dir_nofollow(&current, &component.as_os_str().to_string_lossy())?;
    }
    Ok(current)
}

fn open_child_dir_nofollow(parent: &Dir, name: &str) -> Result<Dir, RepositoryStateStoreError> {
    let metadata = parent.symlink_metadata(name)?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err(RepositoryStateStoreError::UnsafeTarget(name.to_string()));
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(RepositoryStateStoreError::UnsafeTarget(name.to_string()));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

fn open_absolute_dir_nofollow(path: &Path) -> Result<Dir, RepositoryStateStoreError> {
    if path.parent().is_none() {
        return Dir::open_ambient_dir(path, ambient_authority()).map_err(Into::into);
    }
    let parent = path.parent().expect("checked above");
    let leaf = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| RepositoryStateStoreError::UnsafeTarget(path.display().to_string()))?;
    let parent = Dir::open_ambient_dir(parent, ambient_authority())?;
    open_child_dir_nofollow(&parent, leaf)
}

fn ensure_capability_identity(
    directory: &Dir,
    expected: &str,
    path: &Path,
) -> Result<(), RepositoryStateStoreError> {
    let metadata = directory.dir_metadata()?;
    #[cfg(unix)]
    let actual = format!("{}:{}", metadata.dev(), metadata.ino());
    #[cfg(not(unix))]
    let actual = format!("{}:{}", metadata.len(), metadata.permissions().readonly());
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
fn capability_dir_identity(directory: &Dir) -> Option<String> {
    #[cfg(unix)]
    {
        let metadata = directory.dir_metadata().ok()?;
        Some(format!("{}:{}", metadata.dev(), metadata.ino()))
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        None
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
            std_metadata_identity(&metadata),
            true,
        )),
        Ok(_) => Err(RepositoryStateStoreError::UnsafeTarget(
            path.display().to_string(),
        )),
        Err(error) if allow_absent && error.kind() == ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                RepositoryStateStoreError::UnsafeTarget(path.display().to_string())
            })?;
            let metadata = std::fs::symlink_metadata(parent)?;
            let leaf = path.file_name().ok_or_else(|| {
                RepositoryStateStoreError::UnsafeTarget(path.display().to_string())
            })?;
            Ok(RepositoryRootEvidence::new(
                path,
                format!(
                    "absent:{}:{}",
                    std_metadata_identity(&metadata),
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
fn std_metadata_identity(metadata: &std::fs::Metadata) -> String {
    use std::os::unix::fs::MetadataExt as _;
    format!("{}:{}", metadata.dev(), metadata.ino())
}

#[cfg(not(unix))]
fn std_metadata_identity(metadata: &std::fs::Metadata) -> String {
    format!("{}:{}", metadata.len(), metadata.permissions().readonly())
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
mod tests {
    use super::*;
    use crate::repository_state::{CaptureBudget, RepositoryAction};
    use std::collections::HashSet;
    use std::sync::Mutex;
    use tempfile::TempDir;

    fn budget() -> CaptureBudget {
        CaptureBudget {
            max_paths: 32,
            max_listings: 8,
            max_bytes: 1024 * 1024,
            max_depth: 8,
        }
    }

    fn initial_spec() -> CaptureSpec {
        let mut spec = CaptureSpec::phase_one(
            [
                VirtualPath::data("").unwrap(),
                VirtualPath::data("index.json").unwrap(),
            ],
            budget(),
        )
        .unwrap();
        spec.discover_paths([VirtualPath::worktree("note.txt").unwrap()])
            .unwrap();
        spec
    }

    fn initialization_delta(layout: &RepositoryLayout) -> RepositoryDelta {
        RepositoryDelta::new(
            layout,
            vec![
                RepositoryAction::create_directory(
                    VirtualPath::data("").unwrap(),
                    "init",
                    ExpectedPreimage::Absent,
                ),
                RepositoryAction::write_file(
                    VirtualPath::data("index.json").unwrap(),
                    "init",
                    ExpectedPreimage::Absent,
                    b"{}".to_vec(),
                    FileMode::Regular,
                ),
                RepositoryAction::write_file(
                    VirtualPath::worktree("note.txt").unwrap(),
                    "init",
                    ExpectedPreimage::Absent,
                    b"note".to_vec(),
                    FileMode::Regular,
                ),
            ],
        )
        .unwrap()
    }

    #[derive(Default)]
    struct SelectedFailures(Mutex<HashSet<TransactionFailurePoint>>);

    impl SelectedFailures {
        fn one(point: TransactionFailurePoint) -> Arc<Self> {
            Arc::new(Self(Mutex::new(HashSet::from([point]))))
        }
    }

    impl crate::storage::TransactionFailureInjector for SelectedFailures {
        fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
            // Fire once, then clear, so a later recovery on the same in-memory
            // instance (whose state lives with the storage, not on disk) proceeds
            // with a clean boundary instead of re-tripping the injected point.
            if self.0.lock().unwrap().remove(point) {
                Err(std::io::Error::other(format!("injected {point:?}")))
            } else {
                Ok(())
            }
        }
    }

    struct OccupyDataRoot {
        path: PathBuf,
        fired: std::sync::atomic::AtomicBool,
    }

    impl crate::storage::TransactionFailureInjector for OccupyDataRoot {
        fn check(&self, point: &TransactionFailurePoint) -> std::io::Result<()> {
            if point == &TransactionFailurePoint::RepositoryBeforeDataRootPublication
                && !self.fired.swap(true, std::sync::atomic::Ordering::SeqCst)
            {
                std::fs::create_dir(&self.path)?;
            }
            Ok(())
        }
    }

    #[test]
    fn test_json_and_memory_share_absent_root_actions_and_hash() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        let delta = initialization_delta(&layout);

        let memory = InMemoryStorage::new();
        let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
        let memory_image = memory_session.capture(initial_spec()).unwrap();
        let memory_outcome = memory_session.apply(&memory_image, &delta).unwrap();

        let json = JsonFileStorage::new(&data);
        let mut json_session = json.open_mutation_session(layout).unwrap();
        let json_image = json_session.capture(initial_spec()).unwrap();
        let json_outcome = json_session.apply(&json_image, &delta).unwrap();

        assert_eq!(memory_outcome, json_outcome);
        assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
        assert_eq!(
            std::fs::read(temp.path().join("note.txt")).unwrap(),
            b"note"
        );
    }

    #[test]
    fn test_interruption_boundaries_recover_complete_old_or_new_state() {
        let points = [
            TransactionFailurePoint::RepositoryPrepareIntent,
            TransactionFailurePoint::RepositoryPrepareAction { action: 0 },
            TransactionFailurePoint::RepositorySyncPreparedAction { action: 0 },
            TransactionFailurePoint::RepositoryBeforeAction { action: 0 },
            TransactionFailurePoint::RepositoryAfterAction { action: 0 },
            TransactionFailurePoint::RepositoryBeforeDataRootPublication,
            TransactionFailurePoint::RepositoryAfterDataRootPublication,
            TransactionFailurePoint::RepositoryAfterCommit,
            TransactionFailurePoint::RepositoryCleanup,
        ];
        for point in points {
            let temp = TempDir::new().unwrap();
            let data = temp.path().join(".jit");
            let layout = discover_repository_layout(temp.path(), &data).unwrap();
            let storage = JsonFileStorage::with_repository_state_failures(
                &data,
                SelectedFailures::one(point.clone()),
            );
            let mut session = storage.open_mutation_session(layout.clone()).unwrap();
            let image = session.capture(initial_spec()).unwrap();
            assert!(session
                .apply(&image, &initialization_delta(&layout))
                .is_err());
            drop(session);

            let recovered_layout = discover_repository_layout(temp.path(), &data).unwrap();
            let clean = JsonFileStorage::new(&data);
            let _recovered = clean
                .open_mutation_session(recovered_layout)
                .unwrap_or_else(|error| panic!("recovery failed at {point:?}: {error:#}"));
            let root_exists = data.exists();
            assert_eq!(
                temp.path().join("note.txt").exists(),
                root_exists,
                "{point:?}"
            );
            assert_eq!(data.join("index.json").exists(), root_exists, "{point:?}");
        }
    }

    #[test]
    fn test_delete_recovery_refuses_post_crash_occupant() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("victim"), b"old").unwrap();
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let spec =
            CaptureSpec::phase_one([VirtualPath::data("victim").unwrap()], budget()).unwrap();
        let image = session.capture(spec).unwrap();
        let expected =
            ExpectedPreimage::of(image.entry(&VirtualPath::data("victim").unwrap()).unwrap());
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::delete_file(
                VirtualPath::data("victim").unwrap(),
                "delete",
                expected,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);
        std::fs::write(data.join("victim"), b"new occupant").unwrap();

        let clean_storage = JsonFileStorage::new(&data);
        let recovery = clean_storage.open_mutation_session(layout);
        assert!(recovery.is_err());
        assert_eq!(std::fs::read(data.join("victim")).unwrap(), b"new occupant");
    }

    #[test]
    fn test_no_replace_root_race_preserves_occupant_and_rolls_back_worktree() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            Arc::new(OccupyDataRoot {
                path: data.clone(),
                fired: std::sync::atomic::AtomicBool::new(false),
            }),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(initial_spec()).unwrap();
        // The occupied destination fails with the typed occupied-data-root error,
        // not merely some error.
        assert!(matches!(
            session.apply(&image, &initialization_delta(&layout)),
            Err(RepositoryStateStoreError::OccupiedDataRoot { .. })
        ));
        assert!(data.is_dir());
        assert!(!data.join("index.json").exists());
        assert!(!temp.path().join("note.txt").exists());
    }

    #[test]
    fn test_journal_rejects_traversal_control_name() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        let transaction = data.join("tmp/transactions/bad");
        std::fs::create_dir_all(transaction.join("stages")).unwrap();
        std::fs::create_dir(transaction.join("backups")).unwrap();
        std::fs::write(
            transaction.join("journal.json"),
            br#"{"version":2,"transaction_id":"bad","layout_digest":"x","plan_hash":"x","data_root_was_absent":false,"data_stage":"../escape","data_stage_identity":null,"decision":"prepared","actions":[]}"#,
        )
        .unwrap();
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        assert!(JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .is_err());
        assert!(transaction.exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_control_path_symlink_is_never_followed() {
        use std::os::unix::fs::symlink;
        let temp = TempDir::new().unwrap();
        let outside = temp.path().join("outside");
        let data = temp.path().join(".jit");
        std::fs::create_dir(&outside).unwrap();
        std::fs::create_dir(&data).unwrap();
        symlink(&outside, data.join("tmp")).unwrap();
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        assert!(JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .is_err());
        assert!(std::fs::read_dir(outside).unwrap().next().is_none());
    }

    #[test]
    fn test_shared_retained_lock_chain_reenters_same_layout() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(temp.path(), &data).unwrap();
        let storage = JsonFileStorage::new(&data);
        let _bootstrap = storage.acquire_bootstrap_write_lock().unwrap();
        let _repository = storage.acquire_repo_write_lock_raw().unwrap();
        let session = storage.open_mutation_session(layout).unwrap();
        assert_eq!(session.layout().data_root(), data);
    }

    #[test]
    fn test_memory_prepared_residue_recovers_original_aggregate_state() {
        let temp = TempDir::new().unwrap();
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new(temp.path(), "memory-worktree", true),
            RepositoryRootEvidence::new(temp.path().join(".jit"), "memory-data", true),
        )
        .unwrap();
        let storage = InMemoryStorage::with_repository_state_failures(SelectedFailures::one(
            TransactionFailurePoint::RepositoryAfterAction { action: 0 },
        ));
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(initial_spec()).unwrap();
        assert!(session
            .apply(&image, &initialization_delta(&layout))
            .is_err());
        drop(session);

        let mut recovered = storage.open_mutation_session(layout).unwrap();
        let image = recovered.capture(initial_spec()).unwrap();
        assert!(matches!(
            image.entry(&VirtualPath::worktree("note.txt").unwrap()),
            Ok(RepositoryEntry::Absent)
        ));
        assert!(matches!(
            image.entry(&VirtualPath::data("index.json").unwrap()),
            Ok(RepositoryEntry::Absent)
        ));
    }

    #[test]
    fn test_memory_set_mode_uses_exact_file_preimage() {
        let temp = TempDir::new().unwrap();
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new(temp.path(), "memory-worktree", true),
            RepositoryRootEvidence::new(temp.path().join(".jit"), "memory-data", true),
        )
        .unwrap();
        let storage = InMemoryStorage::new();
        {
            let mut state = storage.repository_state();
            let path = VirtualPath::data("tool").unwrap();
            state.data_root_exists = true;
            state.entries.insert(
                path,
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes("memory-tool", b"tool").unwrap(),
                    bytes: b"tool".to_vec(),
                    mode: FileMode::Regular,
                },
            );
        }
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let spec = CaptureSpec::phase_one([VirtualPath::data("tool").unwrap()], budget()).unwrap();
        let image = session.capture(spec).unwrap();
        let path = VirtualPath::data("tool").unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::set_mode(
                path.clone(),
                "mode",
                ExpectedPreimage::of(image.entry(&path).unwrap()),
                FileMode::Executable,
            )],
        )
        .unwrap();
        session.apply(&image, &delta).unwrap();
        drop(session);
        let state = storage.repository_state();
        assert!(matches!(
            state.entries.get(&path),
            Some(RepositoryEntry::File {
                mode: FileMode::Executable,
                ..
            })
        ));
    }

    // --- Cross-backend conformance matrix ------------------------------------
    //
    // One suite exercises identical canonical semantics on the JSON and memory
    // backends: identical captured images, identical result hashes, every action
    // kind, nested and disjoint existing/absent roots, aliases, and convergence
    // to a complete old/new state at every declared failure edge. Object identity
    // differs by construction (device/inode versus a synthesized memory id), so
    // parity is asserted over the identity-independent semantic projection.

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum SemanticEntry {
        Absent,
        Directory(FileMode),
        File(Vec<u8>, FileMode),
        Symlink(Vec<u8>),
        Unsupported,
    }

    fn semantic_entry(entry: &RepositoryEntry) -> SemanticEntry {
        match entry {
            RepositoryEntry::Absent => SemanticEntry::Absent,
            RepositoryEntry::Directory { mode, .. } => SemanticEntry::Directory(*mode),
            RepositoryEntry::File { bytes, mode, .. } => SemanticEntry::File(bytes.clone(), *mode),
            RepositoryEntry::Symlink { target, .. } => SemanticEntry::Symlink(target.clone()),
            RepositoryEntry::Unsupported { .. } => SemanticEntry::Unsupported,
        }
    }

    fn semantic_view(image: &RepositoryImage) -> BTreeMap<VirtualPath, SemanticEntry> {
        image
            .entries()
            .iter()
            .map(|(path, entry)| (path.clone(), semantic_entry(entry)))
            .collect()
    }

    #[test]
    fn test_conformance_absent_root_publishes_identically_across_topologies() {
        for disjoint in [false, true] {
            let worktree = TempDir::new().unwrap();
            let elsewhere = TempDir::new().unwrap();
            let data = if disjoint {
                elsewhere.path().join("store")
            } else {
                worktree.path().join(".jit")
            };
            let layout = discover_repository_layout(worktree.path(), &data).unwrap();
            let delta = initialization_delta(&layout);

            let memory = InMemoryStorage::new();
            let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
            let memory_image = memory_session.capture(initial_spec()).unwrap();
            let memory_outcome = memory_session.apply(&memory_image, &delta).unwrap();
            drop(memory_session);

            let json = JsonFileStorage::new(&data);
            let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
            let json_image = json_session.capture(initial_spec()).unwrap();
            let json_outcome = json_session.apply(&json_image, &delta).unwrap();
            drop(json_session);

            // Identical all-absent capture and identical result hash (both hash
            // the same delta), for nested and disjoint absent roots alike.
            assert_eq!(
                semantic_view(&memory_image),
                semantic_view(&json_image),
                "capture disjoint={disjoint}"
            );
            assert_eq!(memory_outcome, json_outcome, "outcome disjoint={disjoint}");
            assert_eq!(std::fs::read(data.join("index.json")).unwrap(), b"{}");
            assert_eq!(
                std::fs::read(worktree.path().join("note.txt")).unwrap(),
                b"note"
            );
        }
    }

    #[cfg(unix)]
    fn existing_conformance_spec() -> CaptureSpec {
        let mut spec = CaptureSpec::phase_one(
            [
                VirtualPath::data("").unwrap(),
                VirtualPath::data("keep.txt").unwrap(),
                VirtualPath::data("replace.txt").unwrap(),
                VirtualPath::data("remove.txt").unwrap(),
                VirtualPath::data("chmod.txt").unwrap(),
                VirtualPath::data("generated").unwrap(),
                VirtualPath::data("generated/new.txt").unwrap(),
            ],
            budget(),
        )
        .unwrap();
        spec.discover_paths([VirtualPath::worktree("out.txt").unwrap()])
            .unwrap();
        spec
    }

    fn seed_memory_existing(memory: &InMemoryStorage, files: &[(VirtualPath, &[u8])]) {
        let mut state = memory.repository_state();
        state.data_root_exists = true;
        state.entries.insert(
            VirtualPath::data("").unwrap(),
            RepositoryEntry::Directory {
                identity: EntryIdentity::for_bytes("mem-data-root", b"directory").unwrap(),
                mode: FileMode::Executable,
            },
        );
        for (path, bytes) in files {
            state.entries.insert(
                path.clone(),
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes(format!("mem:{path:?}"), bytes).unwrap(),
                    bytes: bytes.to_vec(),
                    mode: FileMode::Regular,
                },
            );
        }
    }

    #[cfg(unix)]
    fn apply_all_action_kinds<S: RepositoryStateStore>(
        store: &S,
        layout: &RepositoryLayout,
    ) -> (RepositoryApplyOutcome, BTreeMap<VirtualPath, SemanticEntry>) {
        let mut session = store.open_mutation_session(layout.clone()).unwrap();
        let image = session.capture(existing_conformance_spec()).unwrap();
        let preimage = |name: &str| {
            ExpectedPreimage::of(image.entry(&VirtualPath::data(name).unwrap()).unwrap())
        };
        let delta = RepositoryDelta::new(
            layout,
            vec![
                RepositoryAction::create_directory(
                    VirtualPath::data("generated").unwrap(),
                    "conformance",
                    ExpectedPreimage::Absent,
                ),
                RepositoryAction::write_file(
                    VirtualPath::data("generated/new.txt").unwrap(),
                    "conformance",
                    ExpectedPreimage::Absent,
                    b"created".to_vec(),
                    FileMode::Regular,
                ),
                RepositoryAction::write_file(
                    VirtualPath::data("replace.txt").unwrap(),
                    "conformance",
                    preimage("replace.txt"),
                    b"replaced".to_vec(),
                    FileMode::Regular,
                ),
                RepositoryAction::set_mode(
                    VirtualPath::data("chmod.txt").unwrap(),
                    "conformance",
                    preimage("chmod.txt"),
                    FileMode::Executable,
                ),
                RepositoryAction::delete_file(
                    VirtualPath::data("remove.txt").unwrap(),
                    "conformance",
                    preimage("remove.txt"),
                ),
                RepositoryAction::write_file(
                    VirtualPath::worktree("out.txt").unwrap(),
                    "conformance",
                    ExpectedPreimage::Absent,
                    b"worktree".to_vec(),
                    FileMode::Regular,
                ),
            ],
        )
        .unwrap();
        let outcome = session.apply(&image, &delta).unwrap();
        drop(session);

        let mut after = store.open_mutation_session(layout.clone()).unwrap();
        let post = after.capture(existing_conformance_spec()).unwrap();
        (outcome, semantic_view(&post))
    }

    #[cfg(unix)]
    #[test]
    fn test_conformance_existing_root_all_action_kinds_match_across_backends() {
        for disjoint in [false, true] {
            let worktree = TempDir::new().unwrap();
            let elsewhere = TempDir::new().unwrap();
            let data = if disjoint {
                elsewhere.path().join("store")
            } else {
                worktree.path().join(".jit")
            };
            std::fs::create_dir_all(&data).unwrap();
            std::fs::write(data.join("keep.txt"), b"keep").unwrap();
            std::fs::write(data.join("replace.txt"), b"old").unwrap();
            std::fs::write(data.join("remove.txt"), b"gone").unwrap();
            std::fs::write(data.join("chmod.txt"), b"exec").unwrap();
            let layout = discover_repository_layout(worktree.path(), &data).unwrap();

            let memory = InMemoryStorage::new();
            seed_memory_existing(
                &memory,
                &[
                    (VirtualPath::data("keep.txt").unwrap(), b"keep"),
                    (VirtualPath::data("replace.txt").unwrap(), b"old"),
                    (VirtualPath::data("remove.txt").unwrap(), b"gone"),
                    (VirtualPath::data("chmod.txt").unwrap(), b"exec"),
                ],
            );
            let json = JsonFileStorage::new(&data);

            let (json_outcome, json_view) = apply_all_action_kinds(&json, &layout);
            let (memory_outcome, memory_view) = apply_all_action_kinds(&memory, &layout);

            assert_eq!(
                json_outcome.actions_applied, memory_outcome.actions_applied,
                "action count disjoint={disjoint}"
            );
            assert_eq!(json_view, memory_view, "post-state disjoint={disjoint}");
            // Every action kind reached its expected terminal state.
            assert_eq!(
                json_view[&VirtualPath::data("replace.txt").unwrap()],
                SemanticEntry::File(b"replaced".to_vec(), FileMode::Regular)
            );
            assert_eq!(
                json_view[&VirtualPath::data("chmod.txt").unwrap()],
                SemanticEntry::File(b"exec".to_vec(), FileMode::Executable)
            );
            assert_eq!(
                json_view[&VirtualPath::data("remove.txt").unwrap()],
                SemanticEntry::Absent
            );
        }
    }

    #[test]
    fn test_conformance_worktree_only_absent_root_materializes_no_data_root() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit"); // absent
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let make_spec = || {
            let mut spec =
                CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
            spec.discover_paths([VirtualPath::worktree("note.txt").unwrap()])
                .unwrap();
            spec
        };
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::worktree("note.txt").unwrap(),
                "wt",
                ExpectedPreimage::Absent,
                b"note".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();

        let memory = InMemoryStorage::new();
        let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
        let memory_image = memory_session.capture(make_spec()).unwrap();
        let memory_outcome = memory_session.apply(&memory_image, &delta).unwrap();
        drop(memory_session);

        let json = JsonFileStorage::new(&data);
        let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
        let json_image = json_session.capture(make_spec()).unwrap();
        let json_outcome = json_session.apply(&json_image, &delta).unwrap();
        drop(json_session);

        // Identical result hash/action count, and neither backend materialized a
        // data root: a worktree-only delta over an absent root leaves it absent.
        assert_eq!(memory_outcome, json_outcome);
        assert!(!data.exists(), "json must not publish an empty data root");
        assert!(!worktree.path().join(".jit-bootstrap").exists());
        assert_eq!(
            std::fs::read(worktree.path().join("note.txt")).unwrap(),
            b"note"
        );

        // Post-apply capture agrees on both backends: data root absent, note present.
        let mut memory_after = memory.open_mutation_session(layout).unwrap();
        let memory_view = semantic_view(&memory_after.capture(make_spec()).unwrap());
        let json_after = JsonFileStorage::new(&data);
        let mut json_after_session = json_after
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap();
        let json_view = semantic_view(&json_after_session.capture(make_spec()).unwrap());
        assert_eq!(memory_view, json_view);
        assert_eq!(
            memory_view[&VirtualPath::data("").unwrap()],
            SemanticEntry::Absent
        );
    }

    #[test]
    fn test_conformance_empty_delta_is_a_noop_on_both_backends() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let spec = || CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        let delta = RepositoryDelta::new(&layout, vec![]).unwrap();

        let memory = InMemoryStorage::new();
        seed_memory_existing(&memory, &[]);
        let mut memory_session = memory.open_mutation_session(layout.clone()).unwrap();
        let memory_image = memory_session.capture(spec()).unwrap();
        let memory_outcome = memory_session.apply(&memory_image, &delta).unwrap();

        let json = JsonFileStorage::new(&data);
        let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
        let json_image = json_session.capture(spec()).unwrap();
        let json_outcome = json_session.apply(&json_image, &delta).unwrap();

        // An empty delta is a no-op with an identical zero-action outcome; no
        // control, stage, or companion residue is created on either backend.
        assert_eq!(memory_outcome, json_outcome);
        assert_eq!(json_outcome.actions_applied, 0);
        assert!(!worktree.path().join(".jit-bootstrap").exists());
        assert!(!data.join("tmp/transactions").exists());
    }

    fn apply_once<S: RepositoryStateStore>(
        store: &S,
        layout: &RepositoryLayout,
        spec: CaptureSpec,
        delta: &RepositoryDelta,
    ) -> Result<RepositoryApplyOutcome, RepositoryStateStoreError> {
        let mut session = store.open_mutation_session(layout.clone())?;
        let image = session.capture(spec)?;
        session.apply(&image, delta)
    }

    #[test]
    fn test_conformance_parent_existence_enforced_on_both_backends() {
        for existing_root in [false, true] {
            let worktree = TempDir::new().unwrap();
            let data = worktree.path().join(".jit");
            if existing_root {
                std::fs::create_dir(&data).unwrap();
            }
            let layout = discover_repository_layout(worktree.path(), &data).unwrap();
            let spec = || {
                CaptureSpec::phase_one(
                    [
                        VirtualPath::data("").unwrap(),
                        VirtualPath::data("nested").unwrap(),
                        VirtualPath::data("nested/file.txt").unwrap(),
                    ],
                    budget(),
                )
                .unwrap()
            };

            // A nested WriteFile whose parent directory is not created fails on both
            // backends (JSON: open_parent MissingParent; memory: parent-existence
            // rule), leaving the old state untouched.
            let missing_parent = RepositoryDelta::new(
                &layout,
                vec![RepositoryAction::write_file(
                    VirtualPath::data("nested/file.txt").unwrap(),
                    "p0",
                    ExpectedPreimage::Absent,
                    b"x".to_vec(),
                    FileMode::Regular,
                )],
            )
            .unwrap();
            let json = JsonFileStorage::new(&data);
            assert!(apply_once(&json, &layout, spec(), &missing_parent).is_err());
            assert!(
                !data.join("nested").exists(),
                "existing_root={existing_root}"
            );
            let memory = InMemoryStorage::new();
            if existing_root {
                seed_memory_existing(&memory, &[]);
            }
            assert!(apply_once(&memory, &layout, spec(), &missing_parent).is_err());

            // The same delta with the explicit CreateDirectory parent succeeds
            // identically on both backends (fresh trees to avoid carrying residue).
            let worktree2 = TempDir::new().unwrap();
            let data2 = worktree2.path().join(".jit");
            if existing_root {
                std::fs::create_dir(&data2).unwrap();
            }
            let layout2 = discover_repository_layout(worktree2.path(), &data2).unwrap();
            let with_parent = RepositoryDelta::new(
                &layout2,
                vec![
                    RepositoryAction::create_directory(
                        VirtualPath::data("nested").unwrap(),
                        "p0",
                        ExpectedPreimage::Absent,
                    ),
                    RepositoryAction::write_file(
                        VirtualPath::data("nested/file.txt").unwrap(),
                        "p0",
                        ExpectedPreimage::Absent,
                        b"x".to_vec(),
                        FileMode::Regular,
                    ),
                ],
            )
            .unwrap();
            let json2 = JsonFileStorage::new(&data2);
            let json_outcome = apply_once(&json2, &layout2, spec(), &with_parent).unwrap();
            let memory2 = InMemoryStorage::new();
            if existing_root {
                seed_memory_existing(&memory2, &[]);
            }
            let memory_outcome = apply_once(&memory2, &layout2, spec(), &with_parent).unwrap();
            assert_eq!(
                json_outcome, memory_outcome,
                "existing_root={existing_root}"
            );
        }
    }

    #[test]
    fn test_conformance_listing_over_file_errors_on_both_backends() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("file.txt"), b"content").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        let spec = || {
            let mut spec =
                CaptureSpec::phase_one([VirtualPath::data("file.txt").unwrap()], budget()).unwrap();
            spec.discover_listing(VirtualPath::data("file.txt").unwrap())
                .unwrap();
            spec
        };

        // A complete-listing request over a regular file is an error on both
        // backends, never a fabricated directory fingerprint.
        let json = JsonFileStorage::new(&data);
        let mut json_session = json.open_mutation_session(layout.clone()).unwrap();
        assert!(json_session.capture(spec()).is_err());

        let memory = InMemoryStorage::new();
        seed_memory_existing(
            &memory,
            &[(VirtualPath::data("file.txt").unwrap(), b"content")],
        );
        let mut memory_session = memory.open_mutation_session(layout).unwrap();
        assert!(memory_session.capture(spec()).is_err());
    }

    #[test]
    fn test_conformance_empty_delta_ignores_armed_injectors() {
        for point in [
            TransactionFailurePoint::RepositoryPrepareIntent,
            TransactionFailurePoint::RepositoryAfterCommit,
            TransactionFailurePoint::RepositoryCleanup,
        ] {
            let worktree = TempDir::new().unwrap();
            let data = worktree.path().join(".jit");
            std::fs::create_dir(&data).unwrap();
            let layout = discover_repository_layout(worktree.path(), &data).unwrap();
            let spec =
                || CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
            let delta = RepositoryDelta::new(&layout, vec![]).unwrap();

            // An empty delta short-circuits before any apply-phase injector on both
            // backends, so an armed apply-phase failure never fires.
            let json = JsonFileStorage::with_repository_state_failures(
                &data,
                SelectedFailures::one(point.clone()),
            );
            let json_outcome = apply_once(&json, &layout, spec(), &delta).unwrap();

            let memory = InMemoryStorage::with_repository_state_failures(SelectedFailures::one(
                point.clone(),
            ));
            seed_memory_existing(&memory, &[]);
            let memory_outcome = apply_once(&memory, &layout, spec(), &delta).unwrap();

            assert_eq!(json_outcome, memory_outcome, "{point:?}");
            assert_eq!(json_outcome.actions_applied, 0);
        }
    }

    #[test]
    fn test_conformance_absent_root_data_only_materializes_data_root_dir() {
        // JSON publishes the staged root WITHOUT requiring an explicit
        // CreateDirectory Data("") (staging creates the stage dir, which becomes the
        // published root); memory materializes the Data("") Directory to match. Both
        // spellings — with and without an explicit root-dir action — must produce
        // identical post-apply captures on both backends.
        for explicit_root_dir in [false, true] {
            let worktree = TempDir::new().unwrap();
            let data = worktree.path().join(".jit");
            let layout = discover_repository_layout(worktree.path(), &data).unwrap();
            let spec = || {
                CaptureSpec::phase_one(
                    [
                        VirtualPath::data("").unwrap(),
                        VirtualPath::data("index.json").unwrap(),
                    ],
                    budget(),
                )
                .unwrap()
            };
            let mut actions = Vec::new();
            if explicit_root_dir {
                actions.push(RepositoryAction::create_directory(
                    VirtualPath::data("").unwrap(),
                    "g",
                    ExpectedPreimage::Absent,
                ));
            }
            actions.push(RepositoryAction::write_file(
                VirtualPath::data("index.json").unwrap(),
                "g",
                ExpectedPreimage::Absent,
                b"{}".to_vec(),
                FileMode::Regular,
            ));
            let delta = RepositoryDelta::new(&layout, actions).unwrap();

            let memory = InMemoryStorage::new();
            apply_once(&memory, &layout, spec(), &delta).unwrap();
            let mut memory_after = memory.open_mutation_session(layout.clone()).unwrap();
            let memory_view = semantic_view(&memory_after.capture(spec()).unwrap());

            let json = JsonFileStorage::new(&data);
            apply_once(&json, &layout, spec(), &delta).unwrap();
            let json_after_store = JsonFileStorage::new(&data);
            let mut json_after = json_after_store
                .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
                .unwrap();
            let json_view = semantic_view(&json_after.capture(spec()).unwrap());

            assert_eq!(
                memory_view, json_view,
                "explicit_root_dir={explicit_root_dir}"
            );
            assert!(matches!(
                memory_view[&VirtualPath::data("").unwrap()],
                SemanticEntry::Directory(_)
            ));
            assert_eq!(
                memory_view[&VirtualPath::data("index.json").unwrap()],
                SemanticEntry::File(b"{}".to_vec(), FileMode::Regular)
            );
        }
    }

    #[test]
    fn test_conformance_rejects_worktree_data_alias() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();
        // A worktree-classed path physically inside the nested data root is a
        // cross-root alias; the layout rejects it wherever a canonical path is
        // required, so neither backend can capture or mutate through it.
        if let Ok(alias) = VirtualPath::worktree(".jit/index.json") {
            assert!(layout.ensure_canonical(&alias).is_err());
        }
    }

    /// Every repository failure point the kernel injects, so the conformance
    /// matrix proves both backends fire — and converge at — identical boundaries.
    fn all_repository_failure_points() -> Vec<TransactionFailurePoint> {
        use TransactionFailurePoint::*;
        vec![
            RepositoryRecoveryExternal,
            RepositoryRecoveryInternal,
            RepositoryPrepareIntent,
            RepositoryCreateCompanion,
            RepositoryPrepareAction { action: 0 },
            RepositorySyncPreparedAction { action: 0 },
            RepositoryBeforeAction { action: 0 },
            RepositoryAfterAction { action: 0 },
            RepositoryBeforeDataRootPublication,
            RepositoryAfterDataRootPublication,
            RepositoryAfterCommit,
            RepositoryCleanup,
            RepositorySweepCompanions,
        ]
    }

    #[derive(Debug, PartialEq, Eq)]
    enum EdgeOutcome {
        OpenFailed,
        ApplyFailed,
        Applied,
    }

    fn drive_edge<S: RepositoryStateStore>(
        store: &S,
        layout: &RepositoryLayout,
        spec: CaptureSpec,
        delta: &RepositoryDelta,
    ) -> EdgeOutcome {
        let mut session = match store.open_mutation_session(layout.clone()) {
            Ok(session) => session,
            Err(_) => return EdgeOutcome::OpenFailed,
        };
        let image = session.capture(spec).unwrap();
        match session.apply(&image, delta) {
            Ok(_) => EdgeOutcome::Applied,
            Err(_) => EdgeOutcome::ApplyFailed,
        }
    }

    #[derive(Debug, Clone, Copy)]
    enum EdgeScenario {
        /// Absent root, worktree action at index 0 (sort puts Worktree < Data).
        AbsentInit,
        /// Existing root, worktree + data action (companion + recovery/sweep fire).
        ExistingMixed,
        /// Absent root, DATA action at index 0 — exercises the absent-root
        /// Data-action publication boundary that AbsentInit hides behind its
        /// index-0 worktree action.
        AbsentDataOnly,
    }

    #[test]
    fn test_conformance_failure_edges_converge_on_both_backends() {
        for point in all_repository_failure_points() {
            for scenario in [
                EdgeScenario::AbsentInit,
                EdgeScenario::ExistingMixed,
                EdgeScenario::AbsentDataOnly,
            ] {
                converge_failure_edge(&point, scenario);
            }
        }
    }

    fn converge_failure_edge(point: &TransactionFailurePoint, scenario: EdgeScenario) {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        if matches!(scenario, EdgeScenario::ExistingMixed) {
            std::fs::create_dir(&data).unwrap();
        }
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // Every scenario uses only absent-preimage actions so one shared delta is
        // valid on both backends.
        let make_spec = || match scenario {
            EdgeScenario::ExistingMixed => {
                let mut spec =
                    CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
                spec.discover_paths([
                    VirtualPath::worktree("out.txt").unwrap(),
                    VirtualPath::data("gen.txt").unwrap(),
                ])
                .unwrap();
                spec
            }
            EdgeScenario::AbsentInit => initial_spec(),
            EdgeScenario::AbsentDataOnly => CaptureSpec::phase_one(
                [
                    VirtualPath::data("").unwrap(),
                    VirtualPath::data("index.json").unwrap(),
                ],
                budget(),
            )
            .unwrap(),
        };
        let delta = match scenario {
            EdgeScenario::ExistingMixed => RepositoryDelta::new(
                &layout,
                vec![
                    RepositoryAction::write_file(
                        VirtualPath::worktree("out.txt").unwrap(),
                        "edge",
                        ExpectedPreimage::Absent,
                        b"worktree".to_vec(),
                        FileMode::Regular,
                    ),
                    RepositoryAction::write_file(
                        VirtualPath::data("gen.txt").unwrap(),
                        "edge",
                        ExpectedPreimage::Absent,
                        b"data".to_vec(),
                        FileMode::Regular,
                    ),
                ],
            )
            .unwrap(),
            EdgeScenario::AbsentInit => initialization_delta(&layout),
            EdgeScenario::AbsentDataOnly => RepositoryDelta::new(
                &layout,
                vec![RepositoryAction::write_file(
                    VirtualPath::data("index.json").unwrap(),
                    "edge",
                    ExpectedPreimage::Absent,
                    b"{}".to_vec(),
                    FileMode::Regular,
                )],
            )
            .unwrap(),
        };

        let json = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(point.clone()),
        );
        let json_outcome = drive_edge(&json, &layout, make_spec(), &delta);
        let json_post = {
            let recovered = JsonFileStorage::new(&data);
            let mut session = recovered
                .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
                .unwrap_or_else(|error| panic!("json recovery {point:?} {scenario:?}: {error:#}"));
            semantic_view(&session.capture(make_spec()).unwrap())
        };

        let memory =
            InMemoryStorage::with_repository_state_failures(SelectedFailures::one(point.clone()));
        if matches!(scenario, EdgeScenario::ExistingMixed) {
            seed_memory_existing(&memory, &[]);
        }
        let memory_outcome = drive_edge(&memory, &layout, make_spec(), &delta);
        let memory_post = {
            let recovered = memory.without_repository_state_failures();
            let mut session = recovered
                .open_mutation_session(layout.clone())
                .unwrap_or_else(|error| {
                    panic!("memory recovery {point:?} {scenario:?}: {error:#}")
                });
            semantic_view(&session.capture(make_spec()).unwrap())
        };

        // Both backends fire the injected point at the same phase (open vs apply)
        // with the same outcome, and recover to the same complete old-or-new state.
        assert_eq!(
            json_outcome, memory_outcome,
            "outcome parity at {point:?} {scenario:?}"
        );
        assert_eq!(
            json_post, memory_post,
            "convergence parity at {point:?} {scenario:?}"
        );
    }

    // --- Finding 2: cross-filesystem per-root staging (worktree companion) ----
    //
    // An internal transaction publishing Worktree actions gets a worktree-side
    // companion control area so those actions stage and back up on the worktree
    // filesystem. True cross-filesystem (a data root on a different mount) cannot
    // be reproduced in CI; these tests prove the companion is worktree-colocated
    // by construction (device-id equality) and that its full lifecycle — create,
    // route, rollback, cleanup, orphan sweep — is crash-recoverable. InMemoryStorage
    // has no filesystem and stages nothing, so it needs no companion; its mixed
    // Worktree+Data parity is already covered by the conformance matrix above.

    #[cfg(unix)]
    #[test]
    fn test_finding2_worktree_companion_is_worktree_colocated_and_cleaned() {
        use std::os::unix::fs::MetadataExt as _;
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // Fail after commit but before cleanup so the companion is observable.
        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryAfterCommit),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("out.txt").unwrap()])
            .unwrap();
        let image = session.capture(spec).unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::worktree("out.txt").unwrap(),
                "f2",
                ExpectedPreimage::Absent,
                b"worktree".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);

        // The companion lives under the worktree, same device as worktree targets
        // by construction — the CI-reproducible proof that its staging/backup
        // authority is same-filesystem with the Worktree actions it publishes.
        let transactions = worktree.path().join(".jit-bootstrap/transactions");
        let companion = std::fs::read_dir(&transactions)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(companion.join("companion").is_file());
        assert!(companion.join("stages").is_dir());
        assert!(companion.join("backups").is_dir());
        assert_eq!(
            std::fs::metadata(companion.join("stages")).unwrap().dev(),
            std::fs::metadata(worktree.path()).unwrap().dev()
        );

        // Recovery reclaims the companion and converges to the published state.
        JsonFileStorage::new(&data)
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap();
        assert!(!worktree.path().join(".jit-bootstrap").exists());
        assert_eq!(
            std::fs::read(worktree.path().join("out.txt")).unwrap(),
            b"worktree"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_finding2_rollback_restores_worktree_action_from_companion_backup() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(worktree.path().join("out.txt"), b"old").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("out.txt").unwrap()])
            .unwrap();
        let image = session.capture(spec).unwrap();
        let expected = ExpectedPreimage::of(
            image
                .entry(&VirtualPath::worktree("out.txt").unwrap())
                .unwrap(),
        );
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::worktree("out.txt").unwrap(),
                "f2",
                expected,
                b"new".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);

        // Recovery rolls the Worktree replace back to the backup held in the
        // companion (worktree filesystem), then reclaims the companion.
        JsonFileStorage::new(&data)
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap();
        assert_eq!(
            std::fs::read(worktree.path().join("out.txt")).unwrap(),
            b"old"
        );
        assert!(!worktree.path().join(".jit-bootstrap").exists());
    }

    #[test]
    fn test_finding2_companion_creation_failure_recovers_clean() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(worktree.path().join("keep.txt"), b"keep").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryCreateCompanion),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("new.txt").unwrap()])
            .unwrap();
        let image = session.capture(spec).unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::worktree("new.txt").unwrap(),
                "f2",
                ExpectedPreimage::Absent,
                b"new".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);

        JsonFileStorage::new(&data)
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap();
        assert!(!worktree.path().join("new.txt").exists());
        assert!(!worktree.path().join(".jit-bootstrap").exists());
        assert_eq!(
            std::fs::read(worktree.path().join("keep.txt")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn test_finding2_orphan_companion_is_swept_on_open() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // Fabricate a companion OWNED BY THIS data root whose internal transaction
        // never existed. Its marker records the owner digest the running session
        // recomputes, so the sweep recognizes it as its own orphan.
        fabricate_companion(worktree.path(), "orphan-id", &owner_digest_of(&layout));

        // Opening a session skips the companion in external recovery, then the
        // orphan sweep (data-root guards held) removes it.
        JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .unwrap();
        assert!(!worktree.path().join(".jit-bootstrap").exists());
    }

    // Owner digest recomputed exactly as the kernel does (worktree/data paths).
    fn owner_digest_of(layout: &RepositoryLayout) -> String {
        use sha2::Digest;
        let mut hasher = Sha256::new();
        hasher.update(layout.worktree_root().to_string_lossy().as_bytes());
        hasher.update([0u8]);
        hasher.update(layout.data_root().to_string_lossy().as_bytes());
        format!("{:x}", hasher.finalize())
    }

    fn fabricate_companion(worktree: &std::path::Path, id: &str, owner: &str) {
        let companion = worktree.join(".jit-bootstrap/transactions").join(id);
        std::fs::create_dir_all(companion.join("stages")).unwrap();
        std::fs::create_dir(companion.join("backups")).unwrap();
        std::fs::write(companion.join("companion"), owner.as_bytes()).unwrap();
        std::fs::write(
            worktree.join(".jit-bootstrap/transaction-protocol-v1"),
            b"1\n",
        )
        .unwrap();
    }

    #[test]
    fn test_finding1_foreign_owner_companion_is_not_swept() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // A companion owned by a DIFFERENT data root sharing this worktree. Its
        // owning session may be mid-flight or crashed; only that owner may reap it.
        fabricate_companion(
            worktree.path(),
            "foreign-id",
            "a-different-data-root-owner-digest",
        );

        JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .unwrap();

        // The foreign companion survives — this session never reaps another data
        // root's transaction, so it cannot destroy a live companion's backup.
        assert!(worktree
            .path()
            .join(".jit-bootstrap/transactions/foreign-id/companion")
            .is_file());
    }

    #[test]
    fn test_finding1_foreign_owner_external_journal_is_skipped_not_failed() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // A committed absent-root external journal left by a DIFFERENT data root.
        // Recovering it against our layout would be wrong; our open must skip it,
        // not fail. Its foreign owner digest keeps it out of our recovery.
        let txn = worktree
            .path()
            .join(".jit-bootstrap/transactions/foreign-journal");
        std::fs::create_dir_all(txn.join("stages")).unwrap();
        std::fs::create_dir(txn.join("backups")).unwrap();
        std::fs::write(
            txn.join("journal.json"),
            br#"{"version":2,"transaction_id":"foreign-journal","layout_digest":"x","owner_digest":"a-different-owner","plan_hash":"x","data_root_was_absent":true,"data_stage":null,"data_stage_identity":null,"decision":"committed","actions":[]}"#,
        )
        .unwrap();
        std::fs::write(
            worktree
                .path()
                .join(".jit-bootstrap/transaction-protocol-v1"),
            b"1\n",
        )
        .unwrap();

        // Open succeeds (does not fail against the foreign journal) and leaves the
        // foreign residue intact for its owning data root's session.
        JsonFileStorage::new(&data)
            .open_mutation_session(layout)
            .unwrap();
        assert!(worktree
            .path()
            .join(".jit-bootstrap/transactions/foreign-journal/journal.json")
            .is_file());
    }

    #[test]
    fn test_finding1_disjoint_sessions_serialize_on_worktree_bootstrap() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Barrier;

        // Two data roots under DIFFERENT parents, both sharing one worktree. Their
        // data-root-parent bootstrap locks differ, so only the worktree-root
        // bootstrap lock serializes their shared `.jit-bootstrap` namespace.
        let worktree = TempDir::new().unwrap();
        let parent_one = TempDir::new().unwrap();
        let parent_two = TempDir::new().unwrap();
        let data_one = parent_one.path().join("store");
        let data_two = parent_two.path().join("store");
        std::fs::create_dir(&data_one).unwrap();
        std::fs::create_dir(&data_two).unwrap();
        let layout_one = discover_repository_layout(worktree.path(), &data_one).unwrap();
        let layout_two = discover_repository_layout(worktree.path(), &data_two).unwrap();

        let first = JsonFileStorage::new(&data_one);
        let session_one = first.open_mutation_session(layout_one).unwrap();

        let started = Arc::new(Barrier::new(2));
        let holder_released = Arc::new(AtomicBool::new(false));
        let waiter = {
            let started = Arc::clone(&started);
            let holder_released = Arc::clone(&holder_released);
            std::thread::spawn(move || {
                started.wait();
                let second = JsonFileStorage::new(&data_two);
                let _session_two = second.open_mutation_session(layout_two).unwrap();
                // The second session opened only after the first released the
                // worktree bootstrap lock — they never ran concurrently.
                assert!(
                    holder_released.load(Ordering::SeqCst),
                    "a disjoint-data-root session opened one worktree's bootstrap concurrently"
                );
            })
        };

        started.wait();
        std::thread::sleep(std::time::Duration::from_millis(150));
        holder_released.store(true, Ordering::SeqCst);
        drop(session_one);
        waiter.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn test_f2_committed_recovery_tolerates_edited_worktree_target() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // Internal transaction with a Worktree target; fail after commit so the
        // committed journal and companion residue survive the crash.
        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryAfterCommit),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("attributes").unwrap()])
            .unwrap();
        let image = session.capture(spec).unwrap();
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::worktree("attributes").unwrap(),
                "f2",
                ExpectedPreimage::Absent,
                b"committed".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);

        // The user legitimately edits the published worktree target before the
        // interrupted cleanup ever runs.
        std::fs::write(worktree.path().join("attributes"), b"user-edited").unwrap();

        // A later open must SUCCEED: past the commit point recovery converges
        // forward, tolerating the diverged worktree target rather than wedging.
        JsonFileStorage::new(&data)
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .expect("committed-journal recovery must not wedge on an edited worktree target");
        // Residue is cleaned (companion gone, internal transaction id removed) and
        // the user edit survives untouched.
        assert!(!worktree.path().join(".jit-bootstrap").exists());
        let internal = data.join("tmp/transactions");
        assert!(
            std::fs::read_dir(&internal)
                .map(|mut entries| entries.next().is_none())
                .unwrap_or(true),
            "internal transaction residue not cleaned"
        );
        assert_eq!(
            std::fs::read(worktree.path().join("attributes")).unwrap(),
            b"user-edited"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_f4_external_data_root_replacement_between_capture_and_apply_conflicts() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("keep.txt"), b"keep").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        let storage = JsonFileStorage::new(&data);
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let spec = CaptureSpec::phase_one(
            [
                VirtualPath::data("").unwrap(),
                VirtualPath::data("keep.txt").unwrap(),
                VirtualPath::data("new.txt").unwrap(),
            ],
            budget(),
        )
        .unwrap();
        let image = session.capture(spec).unwrap();

        // Externally replace the WHOLE data-root directory with a fresh inode
        // between capture and apply. The held capability still points to the old,
        // now-unlinked inode; a tautological FD stat would miss this.
        let replacement = worktree.path().join(".jit-replacement");
        std::fs::create_dir(&replacement).unwrap();
        std::fs::write(replacement.join("keep.txt"), b"keep").unwrap();
        std::fs::remove_dir_all(&data).unwrap();
        std::fs::rename(&replacement, &data).unwrap();

        // Applying re-resolves the root path no-follow, sees the new identity, and
        // aborts with a retryable conflict rather than writing into the old inode.
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::data("new.txt").unwrap(),
                "f4",
                ExpectedPreimage::Absent,
                b"new".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        assert!(matches!(
            session.apply(&image, &delta),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_delta_rejects_captured_cross_root_hard_link_alias() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(worktree.path().join("shared"), b"linked").unwrap();
        // One inode reachable through both roots: a cross-root hard-link alias.
        std::fs::hard_link(worktree.path().join("shared"), data.join("shared")).unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        let storage = JsonFileStorage::new(&data);
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let mut spec =
            CaptureSpec::phase_one([VirtualPath::data("shared").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("shared").unwrap()])
            .unwrap();
        let image = session.capture(spec).unwrap();

        // Capture recorded ONE physical identity (dev:ino) at both virtual paths,
        // so building the delta rejects the pair with the typed alias error.
        // (InMemoryStorage cannot represent this: each memory entry's object id is
        // derived from its path, so two distinct paths never share one identity.)
        let worktree_pre = ExpectedPreimage::of(
            image
                .entry(&VirtualPath::worktree("shared").unwrap())
                .unwrap(),
        );
        let data_pre =
            ExpectedPreimage::of(image.entry(&VirtualPath::data("shared").unwrap()).unwrap());
        let delta = RepositoryDelta::new(
            &layout,
            vec![
                RepositoryAction::set_mode(
                    VirtualPath::worktree("shared").unwrap(),
                    "one",
                    worktree_pre,
                    FileMode::Executable,
                ),
                RepositoryAction::set_mode(
                    VirtualPath::data("shared").unwrap(),
                    "two",
                    data_pre,
                    FileMode::Executable,
                ),
            ],
        );
        assert!(matches!(
            delta,
            Err(crate::repository_state::DeltaError::PhysicalAlias { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn test_a1_rolledback_recovery_tolerates_edited_worktree_target() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(worktree.path().join("attributes"), b"old").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // 1) A worktree replace interrupted right after publication → Prepared.
        let staged = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
        );
        let mut session = staged.open_mutation_session(layout.clone()).unwrap();
        let mut spec = CaptureSpec::phase_one([VirtualPath::data("").unwrap()], budget()).unwrap();
        spec.discover_paths([VirtualPath::worktree("attributes").unwrap()])
            .unwrap();
        let image = session.capture(spec).unwrap();
        let expected = ExpectedPreimage::of(
            image
                .entry(&VirtualPath::worktree("attributes").unwrap())
                .unwrap(),
        );
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::worktree("attributes").unwrap(),
                "a1",
                expected,
                b"new".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);

        // 2) Recovery rolls back (restoring "old" and writing the RolledBack
        //    decision) but its cleanup is interrupted → RolledBack residue remains.
        let interrupted_cleanup = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryCleanup),
        );
        assert!(interrupted_cleanup
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .is_err());
        assert_eq!(
            std::fs::read(worktree.path().join("attributes")).unwrap(),
            b"old"
        );

        // 3) The user edits the restored worktree target.
        std::fs::write(worktree.path().join("attributes"), b"user-edited").unwrap();

        // 4) A later open recovers the RolledBack residue WITHOUT re-asserting the
        //    worktree preimage — open succeeds, residue cleaned, edit survives.
        JsonFileStorage::new(&data)
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .expect("rolledback recovery must not wedge on an edited worktree target");
        assert!(!worktree.path().join(".jit-bootstrap").exists());
        assert_eq!(
            std::fs::read(worktree.path().join("attributes")).unwrap(),
            b"user-edited"
        );
    }

    #[test]
    fn test_a4_memory_stale_preimage_is_a_retryable_conflict() {
        let temp = TempDir::new().unwrap();
        let layout = RepositoryLayout::new(
            RepositoryRootEvidence::new(temp.path(), "wt", true),
            RepositoryRootEvidence::new(temp.path().join(".jit"), "data", true),
        )
        .unwrap();
        let memory = InMemoryStorage::new();
        seed_memory_existing(&memory, &[(VirtualPath::data("x").unwrap(), b"v1")]);

        let mut session = memory.open_mutation_session(layout.clone()).unwrap();
        let spec = CaptureSpec::phase_one([VirtualPath::data("x").unwrap()], budget()).unwrap();
        let image = session.capture(spec).unwrap();
        let expected = ExpectedPreimage::of(image.entry(&VirtualPath::data("x").unwrap()).unwrap());
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::write_file(
                VirtualPath::data("x").unwrap(),
                "a4",
                expected,
                b"v2".to_vec(),
                FileMode::Regular,
            )],
        )
        .unwrap();

        // A concurrent mutation changes the read set between capture and apply, so
        // apply must reject with the same typed retryable conflict the JSON kernel
        // returns from its pre-journal revalidation.
        {
            let mut state = memory.repository_state();
            state.entries.insert(
                VirtualPath::data("x").unwrap(),
                RepositoryEntry::File {
                    identity: EntryIdentity::for_bytes("mem:x", b"changed").unwrap(),
                    bytes: b"changed".to_vec(),
                    mode: FileMode::Regular,
                },
            );
        }
        assert!(matches!(
            session.apply(&image, &delta),
            Err(RepositoryStateStoreError::RetryableConflict { .. })
        ));
    }

    #[test]
    fn test_a5_prepared_delete_recovery_restores_original() {
        let worktree = TempDir::new().unwrap();
        let data = worktree.path().join(".jit");
        std::fs::create_dir(&data).unwrap();
        std::fs::write(data.join("victim"), b"old").unwrap();
        let layout = discover_repository_layout(worktree.path(), &data).unwrap();

        // Publish the DeleteFile, then interrupt before commit → Prepared residue.
        let storage = JsonFileStorage::with_repository_state_failures(
            &data,
            SelectedFailures::one(TransactionFailurePoint::RepositoryAfterAction { action: 0 }),
        );
        let mut session = storage.open_mutation_session(layout.clone()).unwrap();
        let spec =
            CaptureSpec::phase_one([VirtualPath::data("victim").unwrap()], budget()).unwrap();
        let image = session.capture(spec).unwrap();
        let expected =
            ExpectedPreimage::of(image.entry(&VirtualPath::data("victim").unwrap()).unwrap());
        let delta = RepositoryDelta::new(
            &layout,
            vec![RepositoryAction::delete_file(
                VirtualPath::data("victim").unwrap(),
                "a5",
                expected,
            )],
        )
        .unwrap();
        assert!(session.apply(&image, &delta).is_err());
        drop(session);

        // Prepared-journal recovery restores the original file from its backup.
        JsonFileStorage::new(&data)
            .open_mutation_session(discover_repository_layout(worktree.path(), &data).unwrap())
            .unwrap();
        assert_eq!(std::fs::read(data.join("victim")).unwrap(), b"old");
        assert!(!worktree.path().join(".jit-bootstrap").exists());
    }
}
