//! Durable, capability-confined publication of a set of files and directories.
//!
//! The caller owns serialization and passes a held repository write guard. This
//! module owns only storage mechanics: synchronized stages, a versioned journal,
//! identity-checked forward/reverse actions, durable decisions, and cleanup.

use super::repo_lock::RepoWriteGuard;
use super::transaction_action::{
    FileIdentity, JournalActionKind, TargetIdentity, TransactionAction,
};
use super::transaction_journal::{
    ControlName, JournalAction, RepositoryActionProgress, RepositoryFinalIdentity,
    RepositoryJournalAction, RepositoryJournalActionKind, RepositoryJournalPath,
    RepositoryTransactionJournal, RollbackActionState, TransactionDecision, TransactionJournal,
    JOURNAL_FILE, JOURNAL_VERSION, REPOSITORY_JOURNAL_VERSION,
};
use super::transaction_recovery::{
    FailurePoint, FileTransactionError, NoTransactionFailures, RecoveryRequiredError,
    RecoveryState, TransactionFailureInjector,
};
use super::transaction_staging::{stage_bytes, sync_directory};
use crate::repository_state::{
    EntryIdentity, ExpectedPreimage, FileMode, RepositoryAction, RepositoryDelta, RepositoryEntry,
    RepositoryLayout, RepositoryRootClass, VirtualPath,
};
use anyhow::{Context, Result};
use cap_primitives::fs::FollowSymlinks;
#[cfg(unix)]
use cap_std::fs::MetadataExt as _;
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const BOOTSTRAP_DIR: &str = ".jit-bootstrap";
const PROTOCOL_MARKER: &str = "transaction-protocol-v1";
/// Marker file distinguishing a worktree-side companion control directory (a
/// same-filesystem staging/backup area for the Worktree actions of an internal
/// transaction) from a genuine external transaction control. It carries the
/// owning internal transaction id. The companion lives under the already
/// permitted `.jit-bootstrap/transactions/{id}` literal and holds no journal.
const COMPANION_MARKER: &str = "companion";

/// A deterministic set of repository-relative storage actions.
#[derive(Debug, Clone)]
pub struct FileTransactionPlan {
    /// Stable transaction identifier, used only for machine-local recovery state.
    pub transaction_id: String,
    /// Actions whose final bytes and modes are published together.
    pub actions: Vec<TransactionAction>,
}

/// Successful publication and cleanup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileTransactionOutcome {
    /// Hash of the normalized action set recorded by the journal.
    pub plan_hash: String,
    /// Clean after ordinary success; a terminal state is returned by recovery.
    pub recovery_state: RecoveryState,
}

/// Machine-local control location containing pending transaction journals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionControlLocation {
    /// Worktree-side control at `Worktree(.jit-bootstrap/transactions)`, used while
    /// the selected data root does not yet exist.
    ExternalBootstrap,
    /// Data-root-relative control at `Data(tmp/transactions)`, used once the
    /// selected data root exists (its physical prefix follows the data root, which
    /// need not be a literal `.jit`).
    InternalRepository,
}

/// Capability-based transaction service rooted at an already-open repository
/// parent directory. No publication operation accepts an ambient path.
pub struct FileTransactionKernel {
    root: Dir,
    injector: Arc<dyn TransactionFailureInjector>,
    repository: Option<RepositoryKernelRoots>,
}

struct RepositoryKernelRoots {
    layout: RepositoryLayout,
    worktree: Dir,
    data: Option<Dir>,
    data_parent: Dir,
    data_leaf: String,
}

impl FileTransactionKernel {
    /// Construct a production kernel from a caller-opened repository capability.
    pub fn new(root: Dir) -> Result<Self> {
        Self::with_injector(root, Arc::new(NoTransactionFailures))
    }

    /// Construct a kernel with deterministic failure/race injection.
    pub fn with_injector(root: Dir, injector: Arc<dyn TransactionFailureInjector>) -> Result<Self> {
        let root = sync_capable_directory(&root).map_err(|error| {
            FileTransactionError::UnsupportedFilesystem {
                operation: format!("opening a synchronized repository root handle: {error}"),
            }
        })?;
        Ok(Self {
            root,
            injector,
            repository: None,
        })
    }

    /// Construct the canonical repository-state kernel from explicit selected
    /// root capabilities. The ambient filesystem is not consulted after this
    /// boundary.
    pub(crate) fn for_repository_layout(
        layout: RepositoryLayout,
        worktree: Dir,
        data: Option<Dir>,
        data_parent: Dir,
        data_leaf: String,
        injector: Arc<dyn TransactionFailureInjector>,
    ) -> Result<Self> {
        validate_relative_component(&data_leaf)?;
        let worktree = sync_capable_directory(&worktree)?;
        let data = data.as_ref().map(sync_capable_directory).transpose()?;
        let data_parent = sync_capable_directory(&data_parent)?;
        Ok(Self {
            root: worktree.try_clone()?,
            injector,
            repository: Some(RepositoryKernelRoots {
                layout,
                worktree,
                data,
                data_parent,
                data_leaf,
            }),
        })
    }

    /// Prepare, publish, commit, and clean one action set under `guard`.
    ///
    /// Preflight and conflict errors occur before any target mutation. Once the
    /// prepared journal is durable, an ordinary failure is returned only after
    /// exact rollback. If rollback cannot prove identities or synchronize its
    /// result, [`RecoveryRequiredError`] retains the authoritative journal.
    pub fn execute(
        &self,
        _guard: &RepoWriteGuard,
        plan: FileTransactionPlan,
    ) -> Result<FileTransactionOutcome> {
        validate_transaction_id(&plan.transaction_id)?;
        let actions = normalize_actions(&self.root, plan.actions)?;
        let plan_hash = hash_plan(&actions);
        if actions.is_empty() {
            return Ok(FileTransactionOutcome {
                plan_hash,
                recovery_state: RecoveryState::Clean,
            });
        }
        let fresh_root = metadata_optional(&self.root, ".jit")?.is_none();
        let control = self.create_control(&plan.transaction_id, fresh_root)?;

        let prepared = self.prepare_journal(
            &control,
            &plan.transaction_id,
            &plan_hash,
            fresh_root,
            actions,
        );
        let mut journal = match prepared {
            Ok(journal) => journal,
            Err(error) => {
                if metadata_optional(&control.transaction, JOURNAL_FILE)?.is_some() {
                    return Err(RecoveryRequiredError {
                        transaction_id: plan.transaction_id,
                        state: durable_state(&control, RecoveryState::Prepared),
                        source: error,
                    }
                    .into());
                }
                let _ = self.cleanup_control(control, fresh_root, &plan.transaction_id);
                return Err(error);
            }
        };

        if let Err(error) = self.publish_actions(&control, &journal) {
            return match self.rollback(&control, &mut journal) {
                Ok(()) => {
                    let _ = self.cleanup_control(control, fresh_root, &plan.transaction_id);
                    Err(error)
                }
                Err(rollback_error) => Err(RecoveryRequiredError {
                    transaction_id: plan.transaction_id,
                    state: durable_state(&control, RecoveryState::Prepared),
                    source: rollback_error
                        .context(format!("rollback after publication error: {error:#}")),
                }
                .into()),
            };
        }

        journal.decision = TransactionDecision::Committed;
        if let Err(source) = self
            .write_journal(&control.transaction, &journal)
            .and_then(|()| {
                self.injector
                    .check(&FailurePoint::SyncJournal {
                        decision: RecoveryState::Committed,
                    })
                    .map_err(Into::into)
            })
        {
            return Err(RecoveryRequiredError {
                transaction_id: plan.transaction_id,
                state: durable_state(&control, RecoveryState::Prepared),
                source,
            }
            .into());
        }
        if let Err(error) = self.cleanup_control(control, fresh_root, &plan.transaction_id) {
            return Err(RecoveryRequiredError {
                transaction_id: plan.transaction_id,
                state: RecoveryState::Committed,
                source: error,
            }
            .into());
        }

        Ok(FileTransactionOutcome {
            plan_hash,
            recovery_state: RecoveryState::Clean,
        })
    }

    /// Inspect one durable journal without changing repository state.
    pub fn recovery_state(&self, transaction_id: &str) -> Result<Option<RecoveryState>> {
        validate_transaction_id(transaction_id)?;
        let Some(control) = self.open_existing_control(transaction_id)? else {
            return Ok(None);
        };
        Ok(Some(read_journal(&control.transaction)?.decision.into()))
    }

    /// Enumerate pending transaction ids in one control location.
    ///
    /// Results are sorted so recovery order is deterministic. Invalid protocol
    /// contents fail closed instead of being skipped.
    pub fn pending_transaction_ids(
        &self,
        location: TransactionControlLocation,
    ) -> Result<Vec<String>> {
        let transactions = match location {
            TransactionControlLocation::ExternalBootstrap => {
                let Some(metadata) = metadata_optional(&self.root, BOOTSTRAP_DIR)? else {
                    return Ok(Vec::new());
                };
                if !metadata.is_dir() {
                    return Err(FileTransactionError::UnexpectedBootstrapOccupant.into());
                }
                let bootstrap = open_existing_dir(&self.root, BOOTSTRAP_DIR)?;
                if directory_is_empty(&bootstrap)? {
                    return Ok(Vec::new());
                }
                ensure_existing_protocol_marker(&bootstrap)?;
                let Some(metadata) = metadata_optional(&bootstrap, "transactions")? else {
                    return Ok(Vec::new());
                };
                if !metadata.is_dir() {
                    return Err(FileTransactionError::UnexpectedBootstrapOccupant.into());
                }
                open_existing_dir(&bootstrap, "transactions")?
            }
            TransactionControlLocation::InternalRepository => {
                let Some(jit) = metadata_optional(&self.root, ".jit")? else {
                    return Ok(Vec::new());
                };
                if !jit.is_dir() {
                    return Err(FileTransactionError::UnsupportedTarget {
                        path: ".jit".to_string(),
                    }
                    .into());
                }
                let jit = open_existing_dir(&self.root, ".jit")?;
                let Some(tmp) = metadata_optional(&jit, "tmp")? else {
                    return Ok(Vec::new());
                };
                if !tmp.is_dir() {
                    return Err(FileTransactionError::UnsupportedTarget {
                        path: ".jit/tmp".to_string(),
                    }
                    .into());
                }
                let tmp = open_existing_dir(&jit, "tmp")?;
                let Some(transactions) = metadata_optional(&tmp, "transactions")? else {
                    return Ok(Vec::new());
                };
                if !transactions.is_dir() {
                    return Err(FileTransactionError::UnsupportedTarget {
                        path: ".jit/tmp/transactions".to_string(),
                    }
                    .into());
                }
                open_existing_dir(&tmp, "transactions")?
            }
        };

        let mut ids = transactions
            .entries()?
            .map(|entry| {
                let entry = entry?;
                if !entry.file_type()?.is_dir() {
                    return Err(FileTransactionError::UnexpectedOccupant {
                        path: entry.file_name().to_string_lossy().into_owned(),
                    }
                    .into());
                }
                let id = entry.file_name().to_string_lossy().into_owned();
                validate_transaction_id(&id)?;
                Ok(id)
            })
            .collect::<Result<Vec<_>>>()?;
        ids.sort();
        Ok(ids)
    }

    /// Remove an empty or marker-only external control directory.
    ///
    /// This is safe only while the bootstrap lock is held. A non-empty
    /// transaction directory is left intact for ordinary journal recovery.
    pub fn cleanup_empty_external_control(&self, _guard: &RepoWriteGuard) -> Result<()> {
        let Some(_) = metadata_optional(&self.root, BOOTSTRAP_DIR)? else {
            return Ok(());
        };
        let bootstrap = open_existing_dir(&self.root, BOOTSTRAP_DIR)?;
        if directory_is_empty(&bootstrap)? {
            drop(bootstrap);
            self.root.remove_dir(BOOTSTRAP_DIR)?;
            sync_directory(&self.root)?;
            return Ok(());
        }
        ensure_existing_protocol_marker(&bootstrap)?;
        if let Some(metadata) = metadata_optional(&bootstrap, "transactions")? {
            if !metadata.is_dir() {
                return Err(FileTransactionError::UnexpectedBootstrapOccupant.into());
            }
            let transactions = open_existing_dir(&bootstrap, "transactions")?;
            if transactions.entries()?.next().is_some() {
                return Ok(());
            }
            drop(transactions);
            bootstrap.remove_dir("transactions")?;
        }
        remove_optional_file(&bootstrap, PROTOCOL_MARKER)?;
        sync_directory(&bootstrap)?;
        drop(bootstrap);
        self.root.remove_dir(BOOTSTRAP_DIR)?;
        sync_directory(&self.root)?;
        Ok(())
    }

    /// Resume one prepared, committed, or rolled-back transaction.
    ///
    /// Prepared state converges to all-old. Committed state first proves every
    /// final identity, then removes recovery residue. Rolled-back state performs
    /// cleanup only. An identity mismatch leaves the journal untouched.
    pub fn recover(
        &self,
        _guard: &RepoWriteGuard,
        transaction_id: &str,
    ) -> Result<FileTransactionOutcome> {
        validate_transaction_id(transaction_id)?;
        let control = self.open_existing_control(transaction_id)?.ok_or_else(|| {
            FileTransactionError::UnexpectedOccupant {
                path: format!("missing transaction {transaction_id}"),
            }
        })?;
        let mut journal = read_journal(&control.transaction)?;
        if journal.version != JOURNAL_VERSION || journal.transaction_id != transaction_id {
            return Err(FileTransactionError::UnsupportedFilesystem {
                operation: "unsupported or mismatched transaction journal".to_string(),
            }
            .into());
        }
        let initial = RecoveryState::from(journal.decision);
        let result = match journal.decision {
            TransactionDecision::Prepared => self.rollback(&control, &mut journal),
            TransactionDecision::Committed => verify_final_actions(&self.root, &journal),
            TransactionDecision::RolledBack => Ok(()),
        };
        if let Err(source) = result {
            return Err(RecoveryRequiredError {
                transaction_id: transaction_id.to_string(),
                state: initial,
                source,
            }
            .into());
        }
        self.cleanup_control(control, journal.fresh_root, transaction_id)
            .map_err(|source| RecoveryRequiredError {
                transaction_id: transaction_id.to_string(),
                state: RecoveryState::from(journal.decision),
                source,
            })?;
        Ok(FileTransactionOutcome {
            plan_hash: journal.plan_hash,
            recovery_state: RecoveryState::Clean,
        })
    }

    fn open_existing_control(&self, id: &str) -> Result<Option<ControlDirs>> {
        if metadata_optional(&self.root, BOOTSTRAP_DIR)?.is_some() {
            let bootstrap = open_existing_dir(&self.root, BOOTSTRAP_DIR)?;
            if directory_is_empty(&bootstrap)? {
                return Ok(None);
            }
            ensure_existing_protocol_marker(&bootstrap)?;
            let transactions = open_existing_dir(&bootstrap, "transactions")?;
            if metadata_optional(&transactions, id)?.is_some() {
                return open_transaction_dirs(bootstrap, transactions, id).map(Some);
            }
        }
        if metadata_optional(&self.root, ".jit")?.is_some() {
            let jit = open_existing_dir(&self.root, ".jit")?;
            if metadata_optional(&jit, "tmp")?.is_none() {
                return Ok(None);
            }
            let tmp = open_existing_dir(&jit, "tmp")?;
            if metadata_optional(&tmp, "transactions")?.is_none() {
                return Ok(None);
            }
            let transactions = open_existing_dir(&tmp, "transactions")?;
            if metadata_optional(&transactions, id)?.is_some() {
                return open_transaction_dirs(tmp, transactions, id).map(Some);
            }
        }
        Ok(None)
    }

    fn create_control(&self, id: &str, fresh_root: bool) -> Result<ControlDirs> {
        if fresh_root {
            self.injector.check(&FailurePoint::CreateExternalControl)?;
            let bootstrap = open_or_create_protocol_dir(&self.root, BOOTSTRAP_DIR)?;
            ensure_protocol_marker(&bootstrap)?;
            let transactions = open_or_create_dir(&bootstrap, "transactions")?;
            create_transaction_dirs(bootstrap, transactions, id)
        } else {
            self.injector.check(&FailurePoint::CreateInternalControl)?;
            let jit = open_existing_dir(&self.root, ".jit")?;
            let tmp = open_or_create_dir(&jit, "tmp")?;
            let transactions = open_or_create_dir(&tmp, "transactions")?;
            create_transaction_dirs(tmp, transactions, id)
        }
    }

    fn prepare_journal(
        &self,
        control: &ControlDirs,
        transaction_id: &str,
        plan_hash: &str,
        fresh_root: bool,
        actions: Vec<TransactionAction>,
    ) -> Result<TransactionJournal> {
        let journal_actions = actions
            .into_iter()
            .enumerate()
            .map(|(index, action)| self.prepare_action(control, index, action))
            .collect::<Result<Vec<_>>>()?;
        let journal = TransactionJournal {
            version: JOURNAL_VERSION,
            transaction_id: transaction_id.to_string(),
            plan_hash: plan_hash.to_string(),
            fresh_root,
            decision: TransactionDecision::Prepared,
            actions: journal_actions,
        };
        self.injector.check(&FailurePoint::CreateJournal)?;
        create_journal(&control.transaction, &journal)?;
        self.injector.check(&FailurePoint::SyncJournal {
            decision: RecoveryState::Prepared,
        })?;
        Ok(journal)
    }

    fn prepare_action(
        &self,
        control: &ControlDirs,
        index: usize,
        action: TransactionAction,
    ) -> Result<JournalAction> {
        let original = inspect_target(&self.root, action.path())?;
        let action = match action {
            TransactionAction::CreateDirectory { path, unix_mode } => {
                if !matches!(original, TargetIdentity::Absent | TargetIdentity::Directory) {
                    return Err(FileTransactionError::UnsupportedTarget { path }.into());
                }
                JournalActionKind::CreateDirectory { path, unix_mode }
            }
            TransactionAction::WriteFile {
                path,
                contents,
                unix_mode,
            } => {
                if matches!(original, TargetIdentity::Directory) {
                    return Err(FileTransactionError::UnsupportedTarget { path }.into());
                }
                self.injector
                    .check(&FailurePoint::Stage { action: index })?;
                let stage_name = format!("stage-{index}");
                stage_bytes(&control.stages, &stage_name, &contents)?;
                set_mode(&control.stages, &stage_name, unix_mode)?;
                self.injector
                    .check(&FailurePoint::SyncStage { action: index })?;
                sync_directory(&control.stages)?;
                let final_identity = inspect_file(&control.stages, &stage_name)?;
                JournalActionKind::WriteFile {
                    path,
                    unix_mode,
                    final_identity,
                    stage_name,
                    backup_name: format!("backup-{index}"),
                }
            }
            TransactionAction::SetMode { path, unix_mode } => {
                let TargetIdentity::File { identity } = &original else {
                    return Err(FileTransactionError::UnsupportedTarget { path }.into());
                };
                let mut final_identity = identity.clone();
                #[cfg(unix)]
                {
                    final_identity.unix_mode = Some(unix_mode & 0o7777);
                }
                JournalActionKind::SetMode {
                    path,
                    unix_mode,
                    original_mode: identity.unix_mode,
                    final_identity,
                }
            }
        };
        preflight_volume(&control.stages, &self.root, action.path())?;
        Ok(JournalAction {
            action,
            original,
            rollback_state: RollbackActionState::Pending,
        })
    }

    fn publish_actions(&self, control: &ControlDirs, journal: &TransactionJournal) -> Result<()> {
        journal
            .actions
            .iter()
            .enumerate()
            .try_for_each(|(index, action)| {
                self.injector
                    .check(&FailurePoint::BeforeAction { action: index })?;
                let (parent, leaf) = open_parent(&self.root, action.action.path(), true)?;
                self.injector
                    .check(&FailurePoint::AfterParentOpen { action: index })?;
                ensure_identity(&parent, &leaf, &action.original, action.action.path())?;
                match &action.action {
                    JournalActionKind::CreateDirectory { path, unix_mode } => {
                        if matches!(action.original, TargetIdentity::Absent) {
                            parent
                                .create_dir(&leaf)
                                .with_context(|| format!("creating {path}"))?;
                            let created = open_existing_dir(&parent, &leaf)?;
                            set_directory_mode(&created, *unix_mode)?;
                        }
                    }
                    JournalActionKind::WriteFile {
                        path,
                        stage_name,
                        backup_name,
                        ..
                    } => {
                        if matches!(action.original, TargetIdentity::File { .. }) {
                            parent
                                .rename(&leaf, &control.backups, backup_name)
                                .with_context(|| format!("renaming original aside for {path}"))?;
                            sync_directory(&control.backups)?;
                            self.injector
                                .check(&FailurePoint::AfterRenameAside { action: index })?;
                        }
                        control
                            .stages
                            .hard_link(stage_name, &parent, &leaf)
                            .map_err(|error| publication_error(error, path))?;
                        self.injector
                            .check(&FailurePoint::AfterPublish { action: index })?;
                    }
                    JournalActionKind::SetMode {
                        path, unix_mode, ..
                    } => {
                        self.injector
                            .check(&FailurePoint::BeforeModeMutation { action: index })?;
                        let TargetIdentity::File { identity } = &action.original else {
                            return Err(FileTransactionError::UnsupportedTarget {
                                path: path.clone(),
                            }
                            .into());
                        };
                        set_mode_if_identity(&parent, &leaf, identity, Some(*unix_mode))
                            .with_context(|| format!("setting mode on {path}"))?;
                    }
                }
                self.injector
                    .check(&FailurePoint::SyncTargetParent { action: index })?;
                sync_directory(&parent)?;
                ensure_published_identity(&self.root, action)?;
                Ok::<(), anyhow::Error>(())
            })
            .with_context(|| "publishing durable transaction action")
    }

    fn rollback(&self, control: &ControlDirs, journal: &mut TransactionJournal) -> Result<()> {
        for index in (0..journal.actions.len()).rev() {
            if journal.actions[index].rollback_state == RollbackActionState::Restored {
                verify_restored_action(&self.root, control, &journal.actions[index])?;
                continue;
            }
            self.injector
                .check(&FailurePoint::ReverseAction { action: index })?;
            reverse_action(
                &self.root,
                control,
                &journal.actions[index],
                &*self.injector,
                index,
            )
            .with_context(|| format!("reversing durable transaction action {index}"))?;
            journal.actions[index].rollback_state = RollbackActionState::Restored;
            self.write_journal(&control.transaction, journal)?;
            self.injector
                .check(&FailurePoint::SyncReverseParent { action: index })?;
        }

        if journal.fresh_root {
            self.injector.check(&FailurePoint::BeforeFreshRootRemoval)?;
            remove_fresh_root_if_empty(&self.root)?;
            sync_directory(&self.root)?;
            self.injector.check(&FailurePoint::AfterFreshRootRemoval)?;
        }

        journal.decision = TransactionDecision::RolledBack;
        self.write_journal(&control.transaction, journal)?;
        self.injector.check(&FailurePoint::SyncJournal {
            decision: RecoveryState::RolledBack,
        })?;
        Ok(())
    }

    fn write_journal(&self, transaction: &Dir, journal: &TransactionJournal) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(journal)?;
        let next = "journal.next";
        remove_optional_file(transaction, next)?;
        stage_bytes(transaction, next, &bytes)?;
        transaction.rename(next, transaction, JOURNAL_FILE)?;
        sync_directory(transaction)?;
        Ok(())
    }

    fn cleanup_control(&self, control: ControlDirs, fresh: bool, id: &str) -> Result<()> {
        self.injector.check(&FailurePoint::CleanupTerminalResidue)?;
        let ControlDirs {
            base,
            transactions,
            transaction,
            stages,
            backups,
            companion: _,
        } = control;
        drop(stages);
        drop(backups);
        drop(transaction);
        transactions.remove_dir_all(id)?;
        sync_directory(&transactions)?;
        if fresh && transactions.entries()?.next().is_none() {
            drop(transactions);
            base.remove_dir("transactions")?;
            remove_optional_file(&base, PROTOCOL_MARKER)?;
            sync_directory(&base)?;
            drop(base);
            self.root.remove_dir(BOOTSTRAP_DIR)?;
            sync_directory(&self.root)?;
        }
        Ok(())
    }

    pub(crate) fn pending_repository_transactions(
        &self,
        location: TransactionControlLocation,
    ) -> Result<Vec<String>> {
        let roots = self.repository_roots()?;
        repository_pending_ids(roots, location)
    }

    /// Remove worktree-side companions whose owning internal transaction is gone.
    /// Called after internal-journal recovery under the data-root guards.
    pub(crate) fn sweep_orphan_companions(&self, _guard: &RepoWriteGuard) -> Result<Vec<String>> {
        repository_check(&*self.injector, FailurePoint::RepositorySweepCompanions)?;
        sweep_orphan_companions(self.repository_roots()?)
    }

    pub(crate) fn recover_repository_transaction(
        &self,
        guard: &RepoWriteGuard,
        location: TransactionControlLocation,
        id: &str,
    ) -> Result<RepositoryRecoveryDisposition> {
        let roots = self.repository_roots()?;
        let Some((base, transactions, transaction)) =
            open_repository_transaction_dir(roots, location, id)?
        else {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: format!("missing transaction {id}"),
            }
            .into());
        };
        // A worktree-side companion under the external location carries no journal
        // and belongs to an internal transaction (of this or another data root).
        // External recovery runs before the data-root guards are held and cannot
        // validate it, so any companion is skipped here; the internal-journal
        // recovery and the orphan sweep own it.
        if location == TransactionControlLocation::ExternalBootstrap
            && companion_marker_owner(&transaction)?.is_some()
        {
            return Ok(RepositoryRecoveryDisposition::SkippedCompanion);
        }
        if metadata_optional(&transaction, JOURNAL_FILE)?.is_none() {
            // No durable journal means nothing was ever published (publication
            // only follows a written journal), so the partial control — missing
            // `stages`/`backups`, a leftover `journal.next`, or bare `id` dir —
            // is removed wholesale rather than wedging recovery.
            return cleanup_incomplete_repository_control(
                roots,
                base,
                transactions,
                transaction,
                location,
                id,
            )
            .map(|_| RepositoryRecoveryDisposition::Recovered);
        }
        let bytes = transaction.read(JOURNAL_FILE)?;
        let version = serde_json::from_slice::<serde_json::Value>(&bytes)?
            .get("version")
            .and_then(serde_json::Value::as_u64);
        if version == Some(u64::from(JOURNAL_VERSION)) {
            drop((transaction, transactions, base));
            return self
                .recover(guard, id)
                .map(|_| RepositoryRecoveryDisposition::Recovered);
        }
        let journal: RepositoryTransactionJournal = serde_json::from_slice(&bytes)?;
        // A foreign-owner external journal is another data root's absent-root
        // publication residue that happens to live under this shared worktree
        // bootstrap namespace. It is neither ours to recover (recovering it against
        // our layout would be wrong) nor ours to remove; skip it cleanly so this
        // open succeeds and leave it for its owning data root's session. Internal
        // journals live under our own data root and are always ours.
        if location == TransactionControlLocation::ExternalBootstrap
            && journal.owner_digest != repository_owner_digest(&roots.layout)
        {
            return Ok(RepositoryRecoveryDisposition::SkippedForeignOwner);
        }
        let stages = open_existing_dir(&transaction, "stages")?;
        let backups = open_existing_dir(&transaction, "backups")?;
        // An internal journal with Worktree actions has a worktree-side companion
        // holding those actions' backups; open it so rollback restores from the
        // same-filesystem authority and cleanup reclaims it.
        let companion = if journal_has_worktree_action(&journal) {
            open_companion_control(roots, id)?
        } else {
            None
        };
        let control = ControlDirs {
            base,
            transactions,
            transaction,
            stages,
            backups,
            companion,
        };
        recover_repository_journal(roots, control, journal, id, &*self.injector)
            .map(|_| RepositoryRecoveryDisposition::Recovered)
    }

    pub(crate) fn execute_repository_delta(
        &self,
        _guard: &RepoWriteGuard,
        transaction_id: &str,
        delta: &RepositoryDelta,
        plan_hash: &str,
    ) -> Result<FileTransactionOutcome> {
        validate_transaction_id(transaction_id)?;
        execute_repository_delta(
            self.repository_roots()?,
            transaction_id,
            delta,
            plan_hash,
            &*self.injector,
        )
    }

    fn repository_roots(&self) -> Result<&RepositoryKernelRoots> {
        self.repository.as_ref().ok_or_else(|| {
            FileTransactionError::UnsupportedFilesystem {
                operation: "repository-layout kernel was not constructed".into(),
            }
            .into()
        })
    }
}

/// Result of inspecting one repository transaction control during recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RepositoryRecoveryDisposition {
    /// This session recovered or cleaned the named control.
    Recovered,
    /// The external control is a companion owned by internal recovery/sweeping.
    SkippedCompanion,
    /// The external journal belongs to a different selected data root.
    SkippedForeignOwner,
}

struct ControlDirs {
    base: Dir,
    transactions: Dir,
    transaction: Dir,
    stages: Dir,
    backups: Dir,
    /// Worktree-side staging/backup authority for the Worktree actions of an
    /// internal transaction, colocated with the worktree so a disjoint data root
    /// on another filesystem never forces a cross-filesystem hard link or rename.
    /// `None` for external transactions (already worktree-colocated) and for
    /// internal transactions with no Worktree actions.
    companion: Option<CompanionDirs>,
}

/// A companion control directory rooted at worktree `.jit-bootstrap/transactions/{id}`.
/// Only the staging/backup handles are retained; the directory is removed by id
/// through [`remove_companion_control`] at cleanup so no parent handle is needed.
struct CompanionDirs {
    stages: Dir,
    backups: Dir,
}

/// Staging authority for `root`'s actions: the companion (worktree filesystem)
/// for Worktree actions when one exists, otherwise the primary control.
fn stage_authority(control: &ControlDirs, root: RepositoryRootClass) -> &Dir {
    match (root, &control.companion) {
        (RepositoryRootClass::Worktree, Some(companion)) => &companion.stages,
        _ => &control.stages,
    }
}

/// Backup authority for `root`'s actions, mirroring [`stage_authority`].
fn backup_authority(control: &ControlDirs, root: RepositoryRootClass) -> &Dir {
    match (root, &control.companion) {
        (RepositoryRootClass::Worktree, Some(companion)) => &companion.backups,
        _ => &control.backups,
    }
}

fn repository_layout_digest(layout: &RepositoryLayout) -> Result<String> {
    Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(layout)?)))
}

/// Stable owner identity of a transaction: a digest of the worktree and data-root
/// paths, symlink-canonicalized. Unlike the layout digest it is invariant to a
/// data root becoming present, so a session recognizes its own transactions in
/// the shared worktree bootstrap namespace while never claiming a different data
/// root's.
///
/// Canonicalization resolves symlinks so the SAME repository reached through a
/// different path spelling (e.g. a symlinked home) produces one digest, and its
/// crash residue is still recovered and reaped on the next open. The worktree
/// root always exists and is canonicalized directly; the data root may be absent,
/// so its parent is canonicalized and the leaf re-appended. A genuinely changed
/// canonical path (a remount, a moved repository) deliberately produces a new
/// digest and orphans the old residue rather than risk the opposite, unsafe
/// direction — reaping another owner's live transaction. If canonicalization
/// fails (a vanished parent) the lexical path is used as a last resort.
fn repository_owner_digest(layout: &RepositoryLayout) -> String {
    let worktree = std::fs::canonicalize(layout.worktree_root())
        .unwrap_or_else(|_| layout.worktree_root().to_path_buf());
    let data = canonicalize_possibly_absent(layout.data_root());
    let mut hasher = Sha256::new();
    hasher.update(worktree.to_string_lossy().as_bytes());
    hasher.update([0u8]);
    hasher.update(data.to_string_lossy().as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Canonicalize a path that may not exist: resolve it directly when present, else
/// canonicalize its parent and re-append the leaf so an absent data root still
/// yields a stable, symlink-resolved key.
fn canonicalize_possibly_absent(path: &Path) -> PathBuf {
    if let Ok(canonical) = std::fs::canonicalize(path) {
        return canonical;
    }
    match (path.parent(), path.file_name()) {
        (Some(parent), Some(leaf)) => std::fs::canonicalize(parent)
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(leaf),
        _ => path.to_path_buf(),
    }
}

#[derive(Debug, thiserror::Error)]
#[error("injected repository interruption at {point:?}: {source}")]
struct RepositoryInterruption {
    point: FailurePoint,
    #[source]
    source: std::io::Error,
}

fn repository_check(injector: &dyn TransactionFailureInjector, point: FailurePoint) -> Result<()> {
    injector
        .check(&point)
        .map_err(|source| RepositoryInterruption { point, source }.into())
}

fn repository_pending_ids(
    roots: &RepositoryKernelRoots,
    location: TransactionControlLocation,
) -> Result<Vec<String>> {
    let Some(transactions) = repository_transactions_dir(roots, location)? else {
        return Ok(Vec::new());
    };
    let mut ids = transactions
        .entries()?
        .map(|entry| {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                return Err(FileTransactionError::UnexpectedOccupant {
                    path: entry.file_name().to_string_lossy().into_owned(),
                }
                .into());
            }
            let id = entry.file_name().to_string_lossy().into_owned();
            validate_transaction_id(&id)?;
            Ok(id)
        })
        .collect::<Result<Vec<_>>>()?;
    ids.sort();
    Ok(ids)
}

fn repository_transactions_dir(
    roots: &RepositoryKernelRoots,
    location: TransactionControlLocation,
) -> Result<Option<Dir>> {
    match location {
        TransactionControlLocation::ExternalBootstrap => {
            if metadata_optional(&roots.worktree, BOOTSTRAP_DIR)?.is_none() {
                return Ok(None);
            }
            let bootstrap = open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?;
            ensure_existing_protocol_marker(&bootstrap)?;
            if metadata_optional(&bootstrap, "transactions")?.is_none() {
                return Ok(None);
            }
            open_existing_dir(&bootstrap, "transactions").map(Some)
        }
        TransactionControlLocation::InternalRepository => {
            let Some(data) = &roots.data else {
                return Ok(None);
            };
            if metadata_optional(data, "tmp")?.is_none() {
                return Ok(None);
            }
            let tmp = open_existing_dir(data, "tmp")?;
            if metadata_optional(&tmp, "transactions")?.is_none() {
                return Ok(None);
            }
            open_existing_dir(&tmp, "transactions").map(Some)
        }
    }
}

fn create_repository_control(
    roots: &RepositoryKernelRoots,
    id: &str,
    location: TransactionControlLocation,
    needs_worktree_companion: bool,
    injector: &dyn TransactionFailureInjector,
) -> Result<ControlDirs> {
    let (base, transactions) =
        match location {
            TransactionControlLocation::ExternalBootstrap => {
                let bootstrap = open_or_create_protocol_dir(&roots.worktree, BOOTSTRAP_DIR)?;
                ensure_protocol_marker(&bootstrap)?;
                let transactions = open_or_create_dir(&bootstrap, "transactions")?;
                (bootstrap, transactions)
            }
            TransactionControlLocation::InternalRepository => {
                let data = roots.data.as_ref().ok_or_else(|| {
                    FileTransactionError::UnsupportedFilesystem {
                        operation: "internal transaction without a data-root capability".into(),
                    }
                })?;
                let tmp = open_or_create_dir(data, "tmp")?;
                let transactions = open_or_create_dir(&tmp, "transactions")?;
                (tmp, transactions)
            }
        };
    let mut control = create_transaction_dirs(base, transactions, id)?;
    // An internal transaction publishing Worktree actions gets a worktree-side
    // companion so those actions stage and back up on the worktree filesystem.
    if needs_worktree_companion && location == TransactionControlLocation::InternalRepository {
        repository_check(injector, FailurePoint::RepositoryCreateCompanion)?;
        let owner = repository_owner_digest(&roots.layout);
        control.companion = Some(create_companion_control(&roots.worktree, id, &owner)?);
    }
    Ok(control)
}

/// Create the worktree-side companion `.jit-bootstrap/transactions/{id}` with its
/// marker, stages, and backups. The marker records the owning data root's stable
/// owner digest, so the external-recovery scan skips it and the orphan sweep only
/// reclaims companions belonging to the running session's own data root.
fn create_companion_control(worktree: &Dir, id: &str, owner: &str) -> Result<CompanionDirs> {
    let bootstrap = open_or_create_protocol_dir(worktree, BOOTSTRAP_DIR)?;
    ensure_protocol_marker(&bootstrap)?;
    let transactions = open_or_create_dir(&bootstrap, "transactions")?;
    transactions.create_dir(id).map_err(|error| {
        if error.kind() == ErrorKind::AlreadyExists {
            anyhow::Error::new(FileTransactionError::UnexpectedOccupant {
                path: format!("companion control {id}"),
            })
        } else {
            anyhow::Error::new(error)
        }
    })?;
    sync_directory(&transactions)?;
    let transaction = open_existing_dir(&transactions, id)?;
    stage_bytes(&transaction, COMPANION_MARKER, owner.as_bytes())?;
    let stages = open_or_create_dir(&transaction, "stages")?;
    let backups = open_or_create_dir(&transaction, "backups")?;
    sync_directory(&transaction)?;
    Ok(CompanionDirs { stages, backups })
}

/// Owner digest recorded in a worktree `.jit-bootstrap/transactions/{id}` companion
/// marker, or `None` when the directory holds no companion marker.
fn companion_marker_owner(transaction: &Dir) -> Result<Option<String>> {
    match transaction.read_to_string(COMPANION_MARKER) {
        Ok(contents) => Ok(Some(contents)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn journal_has_worktree_action(journal: &RepositoryTransactionJournal) -> bool {
    journal
        .actions
        .iter()
        .any(|action| action.path.root == RepositoryRootClass::Worktree)
}

/// Open the worktree-side companion for `id` if it exists and its marker matches.
fn open_companion_control(
    roots: &RepositoryKernelRoots,
    id: &str,
) -> Result<Option<CompanionDirs>> {
    if metadata_optional(&roots.worktree, BOOTSTRAP_DIR)?.is_none() {
        return Ok(None);
    }
    let bootstrap = open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?;
    if metadata_optional(&bootstrap, "transactions")?.is_none() {
        return Ok(None);
    }
    let transactions = open_existing_dir(&bootstrap, "transactions")?;
    if metadata_optional(&transactions, id)?.is_none() {
        return Ok(None);
    }
    let transaction = open_existing_dir(&transactions, id)?;
    let owner = repository_owner_digest(&roots.layout);
    match companion_marker_owner(&transaction)? {
        Some(marker) if marker == owner => Ok(Some(CompanionDirs {
            stages: open_existing_dir(&transaction, "stages")?,
            backups: open_existing_dir(&transaction, "backups")?,
        })),
        Some(_) => Err(FileTransactionError::LayoutMismatch.into()),
        None => Ok(None),
    }
}

/// Remove the worktree-side companion for `id`, reclaiming the bootstrap protocol
/// directory when it was the last resident.
fn remove_companion_control(roots: &RepositoryKernelRoots, id: &str) -> Result<()> {
    if metadata_optional(&roots.worktree, BOOTSTRAP_DIR)?.is_none() {
        return Ok(());
    }
    let bootstrap = open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?;
    if metadata_optional(&bootstrap, "transactions")?.is_none() {
        return Ok(());
    }
    let transactions = open_existing_dir(&bootstrap, "transactions")?;
    if metadata_optional(&transactions, id)?.is_some() {
        transactions.remove_dir_all(id)?;
        sync_directory(&transactions)?;
    }
    if transactions.entries()?.next().is_none() {
        drop(transactions);
        bootstrap.remove_dir("transactions")?;
        remove_optional_file(&bootstrap, PROTOCOL_MARKER)?;
        sync_directory(&bootstrap)?;
        drop(bootstrap);
        roots.worktree.remove_dir(BOOTSTRAP_DIR)?;
        sync_directory(&roots.worktree)?;
    }
    Ok(())
}

fn internal_transaction_exists(roots: &RepositoryKernelRoots, id: &str) -> Result<bool> {
    let Some(transactions) =
        repository_transactions_dir(roots, TransactionControlLocation::InternalRepository)?
    else {
        return Ok(false);
    };
    Ok(metadata_optional(&transactions, id)?.is_some())
}

/// Remove every worktree-side companion OWNED BY THIS data root whose internal
/// transaction no longer exists. Runs after internal-journal recovery, under the
/// data-root guards. A companion whose marker names a different owner belongs to
/// another data root sharing this worktree and is left untouched — only that
/// owner can decide whether its transaction is live or orphaned.
fn sweep_orphan_companions(roots: &RepositoryKernelRoots) -> Result<Vec<String>> {
    if metadata_optional(&roots.worktree, BOOTSTRAP_DIR)?.is_none() {
        return Ok(Vec::new());
    }
    let bootstrap = open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?;
    let Some(transactions_meta) = metadata_optional(&bootstrap, "transactions")? else {
        return Ok(Vec::new());
    };
    if !transactions_meta.is_dir() {
        return Ok(Vec::new());
    }
    let owner = repository_owner_digest(&roots.layout);
    let transactions = open_existing_dir(&bootstrap, "transactions")?;
    let mut ids = transactions
        .entries()?
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    ids.sort();
    let mut orphans = Vec::new();
    for id in ids {
        let transaction = open_existing_dir(&transactions, &id)?;
        if let Some(marker) = companion_marker_owner(&transaction)? {
            if marker == owner && !internal_transaction_exists(roots, &id)? {
                orphans.push(id);
            }
        }
    }
    drop(transactions);
    drop(bootstrap);
    orphans.sort();
    for id in &orphans {
        remove_companion_control(roots, id)?;
    }
    Ok(orphans)
}

/// Open a transaction's base, transactions, and `id` directories without
/// requiring `stages`/`backups`, so recovery can inspect a control created only
/// partway before a crash. Callers open `stages`/`backups` themselves only once
/// a durable journal proves preparation reached them.
fn open_repository_transaction_dir(
    roots: &RepositoryKernelRoots,
    location: TransactionControlLocation,
    id: &str,
) -> Result<Option<(Dir, Dir, Dir)>> {
    let Some(transactions) = repository_transactions_dir(roots, location)? else {
        return Ok(None);
    };
    if metadata_optional(&transactions, id)?.is_none() {
        return Ok(None);
    }
    let base = match location {
        TransactionControlLocation::ExternalBootstrap => {
            open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?
        }
        TransactionControlLocation::InternalRepository => {
            open_existing_dir(roots.data.as_ref().expect("checked above"), "tmp")?
        }
    };
    let transaction = open_existing_dir(&transactions, id)?;
    Ok(Some((base, transactions, transaction)))
}

fn delta_has_data_action(delta: &RepositoryDelta) -> bool {
    delta
        .actions()
        .iter()
        .any(|action| action.path().root_class() == RepositoryRootClass::Data)
}

fn initial_repository_journal(
    roots: &RepositoryKernelRoots,
    id: &str,
    delta: &RepositoryDelta,
    plan_hash: &str,
) -> Result<RepositoryTransactionJournal> {
    let actions = delta
        .actions()
        .iter()
        .enumerate()
        .map(|(index, action)| {
            let path = RepositoryJournalPath {
                root: action.path().root_class(),
                relative: action.path().relative().clone(),
            };
            let (kind, final_identity) = match action {
                RepositoryAction::CreateDirectory { .. } => (
                    RepositoryJournalActionKind::CreateDirectory {
                        mode: FileMode::Executable,
                        stage: ControlName::new(format!("dir-{index}"))
                            .map_err(anyhow::Error::msg)?,
                    },
                    RepositoryFinalIdentity::Directory {
                        identity: EntryIdentity::for_bytes(
                            format!("planned-dir-{index}"),
                            b"directory",
                        )?,
                        mode: FileMode::Executable,
                    },
                ),
                RepositoryAction::WriteFile { bytes, mode, .. } => (
                    RepositoryJournalActionKind::WriteFile {
                        mode: *mode,
                        stage: ControlName::new(format!("file-{index}"))
                            .map_err(anyhow::Error::msg)?,
                        backup: ControlName::new(format!("backup-{index}"))
                            .map_err(anyhow::Error::msg)?,
                    },
                    RepositoryFinalIdentity::File {
                        identity: EntryIdentity::for_bytes(format!("planned-file-{index}"), bytes)?,
                        mode: *mode,
                    },
                ),
                RepositoryAction::SetMode { expected, mode, .. } => {
                    let ExpectedPreimage::File { identity, .. } = expected else {
                        unreachable!("RepositoryDelta validates SetMode preimages")
                    };
                    (
                        RepositoryJournalActionKind::SetMode { mode: *mode },
                        RepositoryFinalIdentity::File {
                            identity: identity.clone(),
                            mode: *mode,
                        },
                    )
                }
                RepositoryAction::DeleteFile { .. } => (
                    RepositoryJournalActionKind::DeleteFile {
                        backup: ControlName::new(format!("backup-{index}"))
                            .map_err(anyhow::Error::msg)?,
                    },
                    RepositoryFinalIdentity::Absent,
                ),
            };
            Ok(RepositoryJournalAction {
                path,
                owner: action.owner().to_string(),
                expected: action.expected().clone(),
                final_identity,
                action: kind,
                progress: RepositoryActionProgress::Planned,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(RepositoryTransactionJournal {
        version: REPOSITORY_JOURNAL_VERSION,
        transaction_id: id.to_string(),
        layout_digest: repository_layout_digest(&roots.layout)?,
        owner_digest: repository_owner_digest(&roots.layout),
        plan_hash: plan_hash.to_string(),
        data_root_was_absent: roots.data.is_none(),
        // Materialize an absent data root ONLY when the delta carries a Data
        // action. A worktree-only delta over an absent root creates no staged
        // root and no rename: publishing an empty `.jit` as a side effect would
        // leave a present-but-contentless repository that later opens accept as a
        // real root, and it would diverge from InMemoryStorage, which marks the
        // data root present only when a Data action lands. Its worktree actions
        // still commit through the ordinary external journal.
        data_stage: (roots.data.is_none() && delta_has_data_action(delta))
            .then(|| ControlName::new(format!("jit-stage-{id}")))
            .transpose()
            .map_err(anyhow::Error::msg)?,
        data_stage_identity: None,
        decision: TransactionDecision::Prepared,
        actions,
    })
}

fn write_repository_journal(
    transaction: &Dir,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    let next = "journal.next";
    remove_optional_file(transaction, next)?;
    stage_bytes(transaction, next, &serde_json::to_vec_pretty(journal)?)?;
    transaction.rename(next, transaction, JOURNAL_FILE)?;
    sync_directory(transaction)?;
    Ok(())
}

fn execute_repository_delta(
    roots: &RepositoryKernelRoots,
    id: &str,
    delta: &RepositoryDelta,
    plan_hash: &str,
    injector: &dyn TransactionFailureInjector,
) -> Result<FileTransactionOutcome> {
    for action in delta.actions() {
        roots.layout.ensure_canonical(action.path())?;
        let actual = inspect_repository_target(roots, action.path(), None)?;
        ensure_repository_expected(action.path(), action.expected(), &actual)?;
    }
    if delta.actions().is_empty() {
        return Ok(FileTransactionOutcome {
            plan_hash: plan_hash.to_string(),
            recovery_state: RecoveryState::Clean,
        });
    }
    let location = if roots.data.is_some() {
        TransactionControlLocation::InternalRepository
    } else {
        TransactionControlLocation::ExternalBootstrap
    };
    let needs_worktree_companion = delta
        .actions()
        .iter()
        .any(|action| action.path().root_class() == RepositoryRootClass::Worktree);
    let control =
        create_repository_control(roots, id, location, needs_worktree_companion, injector)?;
    let mut journal = initial_repository_journal(roots, id, delta, plan_hash)?;
    write_repository_journal(&control.transaction, &journal)?;
    repository_check(injector, FailurePoint::RepositoryPrepareIntent)?;

    let prepared = prepare_repository_actions(roots, &control, &mut journal, delta, injector);
    if let Err(error) = prepared {
        if error.downcast_ref::<RepositoryInterruption>().is_some() {
            return Err(RecoveryRequiredError {
                transaction_id: id.to_string(),
                state: RecoveryState::Prepared,
                source: error,
            }
            .into());
        }
        cleanup_repository_control(roots, control, location, id, &journal)?;
        return Err(error);
    }

    let published = publish_repository_actions(roots, &control, &mut journal, injector);
    if let Err(error) = published {
        if error.downcast_ref::<RepositoryInterruption>().is_some()
            || data_stage_was_published(roots, &journal)?
        {
            return Err(RecoveryRequiredError {
                transaction_id: id.to_string(),
                state: RecoveryState::Prepared,
                source: error,
            }
            .into());
        }
        rollback_repository_actions(roots, &control, &mut journal)?;
        cleanup_repository_control(roots, control, location, id, &journal)?;
        return Err(error);
    }

    journal.decision = TransactionDecision::Committed;
    write_repository_journal(&control.transaction, &journal)?;
    if let Err(source) = repository_check(injector, FailurePoint::RepositoryAfterCommit) {
        return Err(RecoveryRequiredError {
            transaction_id: id.to_string(),
            state: RecoveryState::Committed,
            source,
        }
        .into());
    }
    repository_check(injector, FailurePoint::RepositoryCleanup)?;
    cleanup_repository_control(roots, control, location, id, &journal)?;
    Ok(FileTransactionOutcome {
        plan_hash: plan_hash.to_string(),
        recovery_state: RecoveryState::Clean,
    })
}

fn prepare_repository_actions(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &mut RepositoryTransactionJournal,
    delta: &RepositoryDelta,
    injector: &dyn TransactionFailureInjector,
) -> Result<()> {
    let data_stage = if let Some(name) = &journal.data_stage {
        roots
            .data_parent
            .create_dir(name.as_str())
            .map_err(|error| {
                if error.kind() == ErrorKind::AlreadyExists {
                    FileTransactionError::UnexpectedOccupant {
                        path: name.as_str().to_string(),
                    }
                    .into()
                } else {
                    anyhow::Error::new(error)
                }
            })?;
        sync_directory(&roots.data_parent)?;
        let stage = open_existing_dir(&roots.data_parent, name.as_str())?;
        journal.data_stage_identity = inspect_repository_root(&stage)?.identity().cloned();
        write_repository_journal(&control.transaction, journal)?;
        Some(stage)
    } else {
        None
    };

    for (index, action) in delta.actions().iter().enumerate() {
        repository_check(
            injector,
            FailurePoint::RepositoryPrepareAction { action: index },
        )?;
        if action.path().root_class() == RepositoryRootClass::Data && roots.data.is_none() {
            prepare_staged_data_action(
                data_stage.as_ref().expect("created above"),
                action,
                &mut journal.actions[index],
            )?;
            journal.actions[index].progress = RepositoryActionProgress::Prepared;
        } else {
            let path = journal_virtual_path(roots, &journal.actions[index].path)?;
            let root = repository_live_root(roots, path.root_class())?;
            let relative = path.relative().as_path().to_string_lossy().into_owned();
            // Route staging and backup to the authority colocated with the
            // action's target filesystem, so a Worktree action never stages or
            // hard-links across a data-root filesystem boundary.
            let stages = stage_authority(control, path.root_class());
            let backups = backup_authority(control, path.root_class());
            match action {
                RepositoryAction::CreateDirectory { .. } => {
                    let stage = create_directory_stage_name(&journal.actions[index].action);
                    stages.create_dir(stage.as_str())?;
                    sync_directory(stages)?;
                    let staged = open_existing_dir(stages, stage.as_str())?;
                    journal.actions[index].final_identity =
                        repository_final_identity(&inspect_repository_root(&staged)?)?;
                    journal.actions[index].progress = RepositoryActionProgress::Prepared;
                }
                RepositoryAction::WriteFile { bytes, mode, .. } => {
                    let (stage, backup) = write_file_control_names(&journal.actions[index].action);
                    stage_bytes(stages, stage.as_str(), bytes)?;
                    set_mode(stages, stage.as_str(), repository_unix_mode(*mode))?;
                    sync_directory(stages)?;
                    journal.actions[index].final_identity = repository_final_identity(
                        &inspect_repository_leaf(stages, stage.as_str())?,
                    )?;
                    // A replace prepares its rollback backup now, before anything
                    // is published, so publication is a verified atomic swap and
                    // never a check-then-rename-by-name occupant race.
                    journal.actions[index].progress = if matches!(
                        journal.actions[index].expected,
                        ExpectedPreimage::File { .. }
                    ) {
                        let (parent, leaf) = open_parent(root, &relative, false)?;
                        prepare_repository_backup(
                            &parent,
                            &leaf,
                            backups,
                            &backup,
                            &journal.actions[index].expected,
                            &path,
                        )?;
                        RepositoryActionProgress::BackupReady
                    } else {
                        RepositoryActionProgress::Prepared
                    };
                }
                RepositoryAction::SetMode { .. } => {
                    journal.actions[index].progress = RepositoryActionProgress::Prepared;
                }
                RepositoryAction::DeleteFile { .. } => {
                    let backup = delete_file_backup_name(&journal.actions[index].action);
                    let (parent, leaf) = open_parent(root, &relative, false)?;
                    prepare_repository_backup(
                        &parent,
                        &leaf,
                        backups,
                        &backup,
                        &journal.actions[index].expected,
                        &path,
                    )?;
                    journal.actions[index].progress = RepositoryActionProgress::BackupReady;
                }
            }
        }
        write_repository_journal(&control.transaction, journal)?;
        repository_check(
            injector,
            FailurePoint::RepositorySyncPreparedAction { action: index },
        )?;
    }
    if let Some(stage) = &data_stage {
        sync_directory(stage)?;
    }
    Ok(())
}

fn create_directory_stage_name(kind: &RepositoryJournalActionKind) -> ControlName {
    match kind {
        RepositoryJournalActionKind::CreateDirectory { stage, .. } => stage.clone(),
        _ => unreachable!("journal and normalized delta stay aligned"),
    }
}

fn write_file_control_names(kind: &RepositoryJournalActionKind) -> (ControlName, ControlName) {
    match kind {
        RepositoryJournalActionKind::WriteFile { stage, backup, .. } => {
            (stage.clone(), backup.clone())
        }
        _ => unreachable!("journal and normalized delta stay aligned"),
    }
}

fn delete_file_backup_name(kind: &RepositoryJournalActionKind) -> ControlName {
    match kind {
        RepositoryJournalActionKind::DeleteFile { backup } => backup.clone(),
        _ => unreachable!("journal and normalized delta stay aligned"),
    }
}

/// Create and synchronize the rollback backup of a replace/delete target during
/// preparation. The live target is verified against the recorded preimage, then
/// hard-linked into the backup area and reverified there, so a subsequent
/// publication converges to the exact original file even if a non-cooperating
/// writer swaps the target afterward.
fn prepare_repository_backup(
    parent: &Dir,
    leaf: &str,
    backups: &Dir,
    backup: &ControlName,
    expected: &ExpectedPreimage,
    path: &VirtualPath,
) -> Result<()> {
    let current = inspect_repository_leaf(parent, leaf)?;
    ensure_repository_expected(path, expected, &current)?;
    parent
        .hard_link(leaf, backups, backup.as_str())
        .map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                FileTransactionError::UnexpectedOccupant {
                    path: format!("backup {}", backup.as_str()),
                }
                .into()
            } else {
                anyhow::Error::new(error)
            }
        })?;
    let saved = inspect_repository_leaf(backups, backup.as_str())?;
    if !repository_matches_expected(expected, &saved) {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{path:?}"),
        }
        .into());
    }
    sync_directory(backups)?;
    Ok(())
}

fn prepare_staged_data_action(
    stage: &Dir,
    action: &RepositoryAction,
    journal: &mut RepositoryJournalAction,
) -> Result<()> {
    if action.path().relative().is_root() {
        if !matches!(action, RepositoryAction::CreateDirectory { .. }) {
            return Err(FileTransactionError::UnsupportedTarget {
                path: "data root".into(),
            }
            .into());
        }
        journal.final_identity = repository_final_identity(&inspect_repository_root(stage)?)?;
        return Ok(());
    }
    let relative = action.path().relative().as_path().to_string_lossy();
    let (parent, leaf) = open_parent(stage, &relative, false)?;
    match action {
        RepositoryAction::CreateDirectory { .. } => {
            parent.create_dir(&leaf)?;
            let directory = open_existing_dir(&parent, &leaf)?;
            journal.final_identity =
                repository_final_identity(&inspect_repository_root(&directory)?)?;
            sync_directory(&parent)?;
        }
        RepositoryAction::WriteFile { bytes, mode, .. } => {
            stage_bytes(&parent, &leaf, bytes)?;
            set_mode(&parent, &leaf, repository_unix_mode(*mode))?;
            journal.final_identity =
                repository_final_identity(&inspect_repository_leaf(&parent, &leaf)?)?;
            sync_directory(&parent)?;
        }
        RepositoryAction::SetMode { .. } | RepositoryAction::DeleteFile { .. } => {
            return Err(FileTransactionError::UnsupportedTarget {
                path: relative.into_owned(),
            }
            .into())
        }
    }
    Ok(())
}

fn publish_repository_actions(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &mut RepositoryTransactionJournal,
    injector: &dyn TransactionFailureInjector,
) -> Result<()> {
    for index in 0..journal.actions.len() {
        if journal.data_root_was_absent
            && journal.actions[index].path.root == RepositoryRootClass::Data
        {
            continue;
        }
        repository_check(
            injector,
            FailurePoint::RepositoryBeforeAction { action: index },
        )?;
        publish_repository_action(roots, control, journal, index)?;
        journal.actions[index].progress = RepositoryActionProgress::Published;
        write_repository_journal(&control.transaction, journal)?;
        repository_check(
            injector,
            FailurePoint::RepositoryAfterAction { action: index },
        )?;
    }

    // The staged data root is published only when the absent-root delta carried a
    // Data action (a `data_stage` was allocated). A worktree-only delta over an
    // absent root has already published its worktree actions above and commits
    // through the external journal without materializing an empty `.jit`.
    if let Some(stage_name) = journal.data_stage.clone() {
        let stage = open_existing_dir(&roots.data_parent, stage_name.as_str())?;
        // Reverify every staged data action against its recorded final identity
        // while the stage is still mutable and nothing is committed. A staged
        // object that no longer matches its plan aborts before the irreversible
        // rename instead of publishing an unverified root.
        for action in &journal.actions {
            if action.path.root != RepositoryRootClass::Data {
                continue;
            }
            let path = journal_virtual_path(roots, &action.path)?;
            let staged = inspect_repository_target(roots, &path, Some(&stage))?;
            ensure_repository_final(&path, &action.final_identity, &staged)?;
        }
        drop(stage);

        repository_check(injector, FailurePoint::RepositoryBeforeDataRootPublication)?;
        rename_noreplace_cap(
            &roots.data_parent,
            stage_name.as_str(),
            &roots.data_parent,
            &roots.data_leaf,
        )
        .map_err(|error| {
            if error.kind() == ErrorKind::AlreadyExists {
                FileTransactionError::OccupiedDataRoot {
                    path: roots.data_leaf.clone(),
                }
                .into()
            } else if error.kind() == ErrorKind::Unsupported {
                FileTransactionError::UnsupportedFilesystem {
                    operation: "atomic no-replace root publication".into(),
                }
                .into()
            } else {
                anyhow::Error::new(error)
            }
        })?;
        sync_directory(&roots.data_parent)?;
        let published = open_existing_dir(&roots.data_parent, &roots.data_leaf)?;
        let actual = inspect_repository_root(&published)?;
        let expected = journal.data_stage_identity.as_ref().ok_or_else(|| {
            FileTransactionError::UnexpectedOccupant {
                path: roots.data_leaf.clone(),
            }
        })?;
        if actual.identity() != Some(expected) {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: roots.data_leaf.clone(),
            }
            .into());
        }
        // Verify every final action after publication: worktree actions against
        // the worktree root, data actions within the freshly opened published
        // root (rename preserves each staged inode, so identities must match).
        for action in &journal.actions {
            let path = journal_virtual_path(roots, &action.path)?;
            let actual = match action.path.root {
                RepositoryRootClass::Data => {
                    inspect_repository_target(roots, &path, Some(&published))?
                }
                RepositoryRootClass::Worktree => inspect_repository_target(roots, &path, None)?,
            };
            ensure_repository_final(&path, &action.final_identity, &actual)?;
        }
        repository_check(injector, FailurePoint::RepositoryAfterDataRootPublication)?;
    }
    Ok(())
}

fn publish_repository_action(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &RepositoryTransactionJournal,
    index: usize,
) -> Result<()> {
    let path = journal_virtual_path(roots, &journal.actions[index].path)?;
    let root = repository_live_root(roots, path.root_class())?;
    let relative = path.relative().as_path().to_string_lossy();
    let (parent, leaf) = open_parent(root, &relative, false)?;
    let current = inspect_repository_leaf(&parent, &leaf)?;
    let expected = &journal.actions[index].expected;
    ensure_repository_expected(&path, expected, &current)?;
    let stages = stage_authority(control, path.root_class());
    match &journal.actions[index].action {
        RepositoryJournalActionKind::CreateDirectory { stage, .. } => {
            rename_noreplace_cap(stages, stage.as_str(), &parent, &leaf)
                .map_err(map_noreplace_error)?;
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::WriteFile { stage, .. } => {
            if matches!(expected, ExpectedPreimage::File { .. }) {
                // The verified backup already exists; publish is a single
                // replacing rename, so there is no check-then-rename occupant
                // window and no post-crash occupant is ever removed by name.
                stages.rename(stage.as_str(), &parent, &leaf)?;
            } else {
                stages
                    .hard_link(stage.as_str(), &parent, &leaf)
                    .map_err(map_noreplace_error)?;
            }
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::SetMode { mode } => {
            set_repository_mode_if_identity(&parent, &leaf, expected, *mode, &path)?;
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::DeleteFile { .. } => {
            // The verified backup already exists; the preimage recheck above
            // rejected a symlink or wrong-identity occupant, so removal targets
            // exactly the file we verified and never a post-crash occupant.
            remove_repository_file_if_identity(&parent, &leaf, expected, &path)?;
            sync_directory(&parent)?;
        }
    }
    let actual = inspect_repository_target(roots, &path, None)?;
    ensure_repository_final(&path, &journal.actions[index].final_identity, &actual)
}

/// Set `mode` on `leaf` only if its live identity still matches `expected`,
/// verified on the same handle the permission change is applied to.
fn set_repository_mode_if_identity(
    parent: &Dir,
    leaf: &str,
    expected: &ExpectedPreimage,
    mode: FileMode,
    path: &VirtualPath,
) -> Result<()> {
    let mut file = open_regular_file_nofollow(parent, leaf)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let metadata = file.metadata()?;
    let observed = RepositoryEntry::File {
        identity: repository_entry_identity(&metadata, &bytes)?,
        bytes,
        mode: repository_file_mode(&metadata),
    };
    if !repository_matches_expected(expected, &observed) {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{path:?}"),
        }
        .into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        if let Some(mode) = repository_unix_mode(mode) {
            file.set_permissions(cap_std::fs::Permissions::from_mode(mode & 0o7777))?;
            file.sync_all()?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (mode, file);
    }
    Ok(())
}

/// Remove `leaf` only if its live identity still matches the regular-file
/// preimage, verified on an open handle immediately before removal.
fn remove_repository_file_if_identity(
    parent: &Dir,
    leaf: &str,
    expected: &ExpectedPreimage,
    path: &VirtualPath,
) -> Result<()> {
    let current = inspect_repository_leaf(parent, leaf)?;
    if !repository_matches_expected(expected, &current) {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{path:?}"),
        }
        .into());
    }
    if !matches!(current, RepositoryEntry::File { .. }) {
        return Err(FileTransactionError::UnsupportedTarget {
            path: format!("{path:?}"),
        }
        .into());
    }
    parent.remove_file(leaf)?;
    Ok(())
}

fn rollback_repository_actions(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &mut RepositoryTransactionJournal,
) -> Result<()> {
    for index in (0..journal.actions.len()).rev() {
        if journal.data_root_was_absent
            && journal.actions[index].path.root == RepositoryRootClass::Data
        {
            continue;
        }
        rollback_repository_action(roots, control, &journal.actions[index])?;
        journal.actions[index].progress = RepositoryActionProgress::Restored;
        write_repository_journal(&control.transaction, journal)?;
    }
    remove_data_stage_if_owned(roots, journal)?;
    journal.decision = TransactionDecision::RolledBack;
    write_repository_journal(&control.transaction, journal)?;
    Ok(())
}

fn rollback_repository_action(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    action: &RepositoryJournalAction,
) -> Result<()> {
    let path = journal_virtual_path(roots, &action.path)?;
    let root = repository_live_root(roots, path.root_class())?;
    let relative = path.relative().as_path().to_string_lossy();
    let (parent, leaf) = match open_parent(root, &relative, false) {
        Ok(value) => value,
        Err(error) if error.downcast_ref::<MissingParent>().is_some() => return Ok(()),
        Err(error) => return Err(error),
    };
    let current = inspect_repository_leaf(&parent, &leaf)?;
    let backups = backup_authority(control, path.root_class());
    if repository_matches_expected(&action.expected, &current) {
        remove_redundant_backup(control, action)?;
        return Ok(());
    }
    match &action.action {
        RepositoryJournalActionKind::CreateDirectory { .. } => {
            ensure_repository_final(&path, &action.final_identity, &current)?;
            parent.remove_dir(&leaf)?;
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::WriteFile { backup, .. } => {
            if !matches!(current, RepositoryEntry::Absent) {
                ensure_repository_final(&path, &action.final_identity, &current)?;
                parent.remove_file(&leaf)?;
                sync_directory(&parent)?;
            }
            if matches!(action.expected, ExpectedPreimage::File { .. }) {
                restore_verified_backup(&parent, &leaf, backups, backup, &action.expected)?;
            }
        }
        RepositoryJournalActionKind::SetMode { .. } => {
            let RepositoryFinalIdentity::File {
                identity: final_identity,
                mode: final_mode,
            } = &action.final_identity
            else {
                unreachable!("SetMode has a file final identity")
            };
            let ExpectedPreimage::File {
                mode: original_mode,
                ..
            } = action.expected
            else {
                unreachable!("SetMode has a file preimage")
            };
            // Verify the live target is the mode-changed file and restore the
            // original mode on the SAME handle, so a bystander swapped in after the
            // inspect above is never chmodded (the publication side uses the same
            // helper, covered by test_set_mode_leaf_symlink_swap_cannot_mutate_
            // external_target and test_set_mode_revalidates_identity_on_the_mutated_handle).
            let expected_final = ExpectedPreimage::File {
                identity: final_identity.clone(),
                mode: *final_mode,
            };
            set_repository_mode_if_identity(&parent, &leaf, &expected_final, original_mode, &path)?;
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::DeleteFile { backup } => {
            if !matches!(current, RepositoryEntry::Absent) {
                return Err(FileTransactionError::UnexpectedOccupant {
                    path: relative.into_owned(),
                }
                .into());
            }
            restore_verified_backup(&parent, &leaf, backups, backup, &action.expected)?;
        }
    }
    let restored = inspect_repository_leaf(&parent, &leaf)?;
    ensure_repository_expected(&path, &action.expected, &restored)
}

fn restore_verified_backup(
    parent: &Dir,
    leaf: &str,
    backups: &Dir,
    backup: &ControlName,
    expected: &ExpectedPreimage,
) -> Result<()> {
    let saved = inspect_repository_leaf(backups, backup.as_str())?;
    if !repository_matches_expected(expected, &saved) {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: format!("backup {}", backup.as_str()),
        }
        .into());
    }
    backups.hard_link(backup.as_str(), parent, leaf)?;
    sync_directory(parent)?;
    backups.remove_file(backup.as_str())?;
    sync_directory(backups)?;
    Ok(())
}

fn remove_redundant_backup(control: &ControlDirs, action: &RepositoryJournalAction) -> Result<()> {
    let backup = match &action.action {
        RepositoryJournalActionKind::WriteFile { backup, .. }
        | RepositoryJournalActionKind::DeleteFile { backup } => Some(backup),
        _ => None,
    };
    if let Some(backup) = backup {
        let backups = backup_authority(control, action.path.root);
        let saved = inspect_repository_leaf(backups, backup.as_str())?;
        if !matches!(saved, RepositoryEntry::Absent) {
            if !repository_matches_expected(&action.expected, &saved) {
                return Err(FileTransactionError::UnexpectedOccupant {
                    path: format!("backup {}", backup.as_str()),
                }
                .into());
            }
            backups.remove_file(backup.as_str())?;
            sync_directory(backups)?;
        }
    }
    Ok(())
}

fn recover_repository_journal(
    roots: &RepositoryKernelRoots,
    control: ControlDirs,
    mut journal: RepositoryTransactionJournal,
    id: &str,
    injector: &dyn TransactionFailureInjector,
) -> Result<FileTransactionOutcome> {
    validate_repository_journal(&journal, id)?;
    if journal.version != REPOSITORY_JOURNAL_VERSION {
        return Err(FileTransactionError::LayoutMismatch.into());
    }
    let published_absent_root =
        journal.data_root_was_absent && data_stage_was_published(roots, &journal)?;
    if journal.layout_digest != repository_layout_digest(&roots.layout)? && !published_absent_root {
        return Err(FileTransactionError::LayoutMismatch.into());
    }
    let location = if journal.data_root_was_absent {
        TransactionControlLocation::ExternalBootstrap
    } else {
        TransactionControlLocation::InternalRepository
    };
    match journal.decision {
        TransactionDecision::Prepared if published_absent_root => {
            verify_repository_final_actions(roots, &journal)?;
            journal.decision = TransactionDecision::Committed;
            write_repository_journal(&control.transaction, &journal)?;
        }
        TransactionDecision::Prepared => {
            rollback_repository_actions(roots, &control, &mut journal)?;
        }
        TransactionDecision::Committed => verify_repository_final_actions(roots, &journal)?,
        TransactionDecision::RolledBack => {
            verify_repository_restored_actions(roots, &control, &journal)?
        }
    }
    repository_check(injector, FailurePoint::RepositoryCleanup)?;
    cleanup_repository_control(roots, control, location, id, &journal)?;
    Ok(FileTransactionOutcome {
        plan_hash: journal.plan_hash,
        recovery_state: RecoveryState::Clean,
    })
}

/// Validate a decoded journal against the directory that contained it before any
/// recovery mutation. `ControlName` typing already rejects unsafe stage/backup
/// names on decode; this adds the bindings recovery relies on: the journal must
/// name its own containing directory, and no two actions may target one path.
fn validate_repository_journal(journal: &RepositoryTransactionJournal, id: &str) -> Result<()> {
    if journal.transaction_id != id {
        return Err(FileTransactionError::LayoutMismatch.into());
    }
    let mut seen = HashSet::new();
    let mut identities: BTreeMap<&EntryIdentity, ()> = BTreeMap::new();
    for action in &journal.actions {
        if !seen.insert((action.path.root, action.path.relative.clone())) {
            return Err(FileTransactionError::DuplicateTarget {
                path: format!("{:?}", action.path.relative),
            }
            .into());
        }
        // Recheck physical uniqueness before recovery mutates anything: two actions
        // whose recorded preimages carry one physical identity at distinct canonical
        // paths are a hard-link alias (the same pairwise rule delta normalization
        // enforces). An `Absent` preimage carries no identity and cannot collide.
        if let Some(identity) = action.expected.identity() {
            if identities.insert(identity, ()).is_some() {
                return Err(FileTransactionError::AliasedTarget {
                    path: format!("{:?}", action.path.relative),
                }
                .into());
            }
        }
    }
    Ok(())
}

fn cleanup_incomplete_repository_control(
    roots: &RepositoryKernelRoots,
    base: Dir,
    transactions: Dir,
    transaction: Dir,
    location: TransactionControlLocation,
    id: &str,
) -> Result<FileTransactionOutcome> {
    drop(transaction);
    transactions.remove_dir_all(id)?;
    sync_directory(&transactions)?;
    if location == TransactionControlLocation::ExternalBootstrap
        && transactions.entries()?.next().is_none()
    {
        drop(transactions);
        base.remove_dir("transactions")?;
        remove_optional_file(&base, PROTOCOL_MARKER)?;
        sync_directory(&base)?;
        drop(base);
        roots.worktree.remove_dir(BOOTSTRAP_DIR)?;
        sync_directory(&roots.worktree)?;
    }
    Ok(FileTransactionOutcome {
        plan_hash: String::new(),
        recovery_state: RecoveryState::Clean,
    })
}

/// Verify final identities during recovery of a committed (or published-absent-root)
/// transaction, tolerating post-commit divergence of Worktree targets.
///
/// Past the commit point the transaction is done and its residue only needs
/// cleanup; recovery must converge forward idempotently. A user may legitimately
/// edit a published Worktree target (for example `.gitattributes`) before the
/// crashed cleanup runs, so re-asserting its final identity would wedge every
/// later `open_mutation_session` even though the transaction committed. Data-root
/// targets are jit-owned and are still verified — that cannot wedge legitimate
/// use — which keeps detection of a raced post-crash occupant of a `.jit` file.
fn verify_repository_final_actions(
    roots: &RepositoryKernelRoots,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    for action in &journal.actions {
        if action.path.root == RepositoryRootClass::Worktree {
            continue;
        }
        let path = journal_virtual_path(roots, &action.path)?;
        let actual = inspect_repository_target(roots, &path, None)?;
        ensure_repository_final(&path, &action.final_identity, &actual)?;
    }
    Ok(())
}

fn verify_repository_restored_actions(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    for action in &journal.actions {
        // A RolledBack decision is written only after every action's restore was
        // already verified against its preimage (see rollback_repository_action).
        // Recovery of that terminal residue must therefore only finish cleanup, not
        // re-assert Worktree preimages — a user may have legitimately edited a
        // restored Worktree target before the interrupted cleanup ran, exactly as
        // the committed arm tolerates. Absent-root Data actions have no live target.
        if action.path.root == RepositoryRootClass::Worktree
            || (journal.data_root_was_absent && action.path.root == RepositoryRootClass::Data)
        {
            continue;
        }
        let path = journal_virtual_path(roots, &action.path)?;
        let actual = inspect_repository_target(roots, &path, None)?;
        ensure_repository_expected(&path, &action.expected, &actual)?;
        remove_redundant_backup(control, action)?;
    }
    Ok(())
}

fn data_stage_was_published(
    roots: &RepositoryKernelRoots,
    journal: &RepositoryTransactionJournal,
) -> Result<bool> {
    if !journal.data_root_was_absent {
        return Ok(false);
    }
    let Some(expected) = journal.data_stage_identity.as_ref() else {
        return Ok(false);
    };
    let stage = journal.data_stage.as_ref().expect("absent root has stage");
    if metadata_optional(&roots.data_parent, stage.as_str())?.is_some() {
        return Ok(false);
    }
    let destination = match roots.data_parent.symlink_metadata(&roots.data_leaf) {
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {
            let directory = open_existing_dir(&roots.data_parent, &roots.data_leaf)?;
            inspect_repository_root(&directory)?
        }
        Ok(_) => {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: roots.data_leaf.clone(),
            }
            .into())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if destination.identity() == Some(expected) {
        Ok(true)
    } else {
        Err(FileTransactionError::OccupiedDataRoot {
            path: roots.data_leaf.clone(),
        }
        .into())
    }
}

fn remove_data_stage_if_owned(
    roots: &RepositoryKernelRoots,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    let Some(stage) = &journal.data_stage else {
        return Ok(());
    };
    match roots.data_parent.symlink_metadata(stage.as_str()) {
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => {}
        Ok(_) => {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: stage.as_str().to_string(),
            }
            .into())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    // A recorded identity is verified before removal. When the crash landed
    // between stage creation and the identity write, the deterministically named
    // stage is still ours by the very journal being recovered, so it is removed
    // rather than left as residue that would wedge the next attempt.
    if let Some(expected) = &journal.data_stage_identity {
        let directory = open_existing_dir(&roots.data_parent, stage.as_str())?;
        let actual = inspect_repository_root(&directory)?;
        if actual.identity() != Some(expected) {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: stage.as_str().to_string(),
            }
            .into());
        }
    }
    roots.data_parent.remove_dir_all(stage.as_str())?;
    sync_directory(&roots.data_parent)?;
    Ok(())
}

fn cleanup_repository_control(
    roots: &RepositoryKernelRoots,
    control: ControlDirs,
    location: TransactionControlLocation,
    id: &str,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    if !data_stage_was_published(roots, journal)? {
        remove_data_stage_if_owned(roots, journal)?;
    }
    let ControlDirs {
        base,
        transactions,
        transaction,
        stages,
        backups,
        companion,
    } = control;
    drop(stages);
    drop(backups);
    drop(transaction);
    // Reclaim the worktree-side companion (if any) before the primary control, so
    // a crash between the two is caught by the orphan sweep on the next open.
    if let Some(companion) = companion {
        drop(companion);
        remove_companion_control(roots, id)?;
    }
    transactions.remove_dir_all(id)?;
    sync_directory(&transactions)?;
    if location == TransactionControlLocation::ExternalBootstrap
        && transactions.entries()?.next().is_none()
    {
        drop(transactions);
        base.remove_dir("transactions")?;
        remove_optional_file(&base, PROTOCOL_MARKER)?;
        sync_directory(&base)?;
        drop(base);
        roots.worktree.remove_dir(BOOTSTRAP_DIR)?;
        sync_directory(&roots.worktree)?;
    }
    Ok(())
}

fn journal_virtual_path(
    roots: &RepositoryKernelRoots,
    path: &RepositoryJournalPath,
) -> Result<VirtualPath> {
    let path = VirtualPath::from_root(path.root, path.relative.clone())?;
    roots.layout.ensure_canonical(&path)?;
    Ok(path)
}

fn repository_live_root(roots: &RepositoryKernelRoots, class: RepositoryRootClass) -> Result<&Dir> {
    match class {
        RepositoryRootClass::Worktree => Ok(&roots.worktree),
        RepositoryRootClass::Data => roots.data.as_ref().ok_or_else(|| {
            FileTransactionError::UnsupportedFilesystem {
                operation: "data root is not published".into(),
            }
            .into()
        }),
    }
}

fn inspect_repository_target(
    roots: &RepositoryKernelRoots,
    path: &VirtualPath,
    data_stage: Option<&Dir>,
) -> Result<RepositoryEntry> {
    roots.layout.ensure_canonical(path)?;
    let root = match path.root_class() {
        RepositoryRootClass::Worktree => &roots.worktree,
        RepositoryRootClass::Data => match (data_stage, roots.data.as_ref()) {
            (Some(stage), _) => stage,
            (None, Some(data)) => data,
            (None, None) => return Ok(RepositoryEntry::Absent),
        },
    };
    if path.relative().is_root() {
        return inspect_repository_root(root);
    }
    let relative = path.relative().as_path().to_string_lossy();
    match open_parent(root, &relative, false) {
        Ok((parent, leaf)) => inspect_repository_leaf(&parent, &leaf),
        Err(error) if error.downcast_ref::<MissingParent>().is_some() => {
            Ok(RepositoryEntry::Absent)
        }
        Err(error) => Err(error),
    }
}

fn inspect_repository_root(root: &Dir) -> Result<RepositoryEntry> {
    let metadata = root.dir_metadata()?;
    Ok(RepositoryEntry::Directory {
        identity: repository_entry_identity(&metadata, b"directory")?,
        mode: repository_file_mode(&metadata),
    })
}

fn inspect_repository_leaf(parent: &Dir, leaf: &str) -> Result<RepositoryEntry> {
    let metadata = match parent.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(RepositoryEntry::Absent),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() {
        let target = parent.read_link(leaf)?;
        let bytes = target.as_os_str().as_encoded_bytes().to_vec();
        return Ok(RepositoryEntry::Symlink {
            identity: repository_entry_identity(&metadata, &bytes)?,
            target: bytes,
            mode: repository_file_mode(&metadata),
        });
    }
    if metadata.is_dir() {
        return Ok(RepositoryEntry::Directory {
            identity: repository_entry_identity(&metadata, b"directory")?,
            mode: repository_file_mode(&metadata),
        });
    }
    if metadata.is_file() {
        let mut file = open_regular_file_nofollow(parent, leaf)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let metadata = file.metadata()?;
        return Ok(RepositoryEntry::File {
            identity: repository_entry_identity(&metadata, &bytes)?,
            bytes,
            mode: repository_file_mode(&metadata),
        });
    }
    Ok(RepositoryEntry::Unsupported {
        identity: repository_entry_identity(&metadata, b"unsupported")?,
        reason: "unsupported filesystem object".into(),
        mode: repository_file_mode(&metadata),
    })
}

fn repository_entry_identity(
    metadata: &cap_std::fs::Metadata,
    bytes: &[u8],
) -> Result<EntryIdentity> {
    #[cfg(unix)]
    let object = format!("{}:{}", metadata.dev(), metadata.ino());
    #[cfg(not(unix))]
    let object = format!("{}:{}", metadata.len(), metadata.permissions().readonly());
    EntryIdentity::for_bytes(object, bytes).map_err(Into::into)
}

fn repository_file_mode(metadata: &cap_std::fs::Metadata) -> FileMode {
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

fn repository_unix_mode(mode: FileMode) -> Option<u32> {
    #[cfg(unix)]
    {
        Some(match mode {
            FileMode::Regular => 0o644,
            FileMode::Executable => 0o755,
        })
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        None
    }
}

fn repository_matches_expected(expected: &ExpectedPreimage, actual: &RepositoryEntry) -> bool {
    ExpectedPreimage::of(actual) == *expected
}

fn ensure_repository_expected(
    path: &VirtualPath,
    expected: &ExpectedPreimage,
    actual: &RepositoryEntry,
) -> Result<()> {
    if repository_matches_expected(expected, actual) {
        Ok(())
    } else {
        Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{:?}", path),
        }
        .into())
    }
}

fn repository_final_identity(entry: &RepositoryEntry) -> Result<RepositoryFinalIdentity> {
    match entry {
        RepositoryEntry::Absent => Ok(RepositoryFinalIdentity::Absent),
        RepositoryEntry::Directory { identity, mode } => Ok(RepositoryFinalIdentity::Directory {
            identity: identity.clone(),
            mode: *mode,
        }),
        RepositoryEntry::File { identity, mode, .. } => Ok(RepositoryFinalIdentity::File {
            identity: identity.clone(),
            mode: *mode,
        }),
        RepositoryEntry::Symlink { .. } | RepositoryEntry::Unsupported { .. } => {
            Err(FileTransactionError::UnsupportedObjectKind {
                path: "transaction final target".into(),
            }
            .into())
        }
    }
}

fn ensure_repository_final(
    path: &VirtualPath,
    expected: &RepositoryFinalIdentity,
    actual: &RepositoryEntry,
) -> Result<()> {
    if matches!(repository_final_identity(actual), Ok(actual) if &actual == expected) {
        Ok(())
    } else {
        Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{:?}", path),
        }
        .into())
    }
}

fn validate_relative_component(value: &str) -> Result<()> {
    if value.is_empty()
        || value.contains(['/', '\\'])
        || value == "."
        || value == ".."
        || value.chars().any(char::is_control)
    {
        return Err(FileTransactionError::InvalidPath {
            path: value.to_string(),
        }
        .into());
    }
    Ok(())
}

/// Map a no-replace rename failure to a typed error: an occupied destination is
/// an unexpected occupant, and a platform without atomic no-replace rename is an
/// unsupported filesystem rather than an opaque I/O error.
fn map_noreplace_error(error: std::io::Error) -> anyhow::Error {
    match error.kind() {
        ErrorKind::AlreadyExists => FileTransactionError::UnexpectedOccupant {
            path: "no-replace rename destination".into(),
        }
        .into(),
        ErrorKind::Unsupported => FileTransactionError::UnsupportedFilesystem {
            operation: "atomic no-replace rename".into(),
        }
        .into(),
        _ => anyhow::Error::new(error),
    }
}

#[cfg(target_os = "linux")]
fn rename_noreplace_cap(
    source_dir: &Dir,
    source: impl AsRef<OsStr>,
    target_dir: &Dir,
    target: impl AsRef<OsStr>,
) -> std::io::Result<()> {
    use nix::fcntl::{renameat2, RenameFlags};
    renameat2(
        source_dir,
        source.as_ref(),
        target_dir,
        target.as_ref(),
        RenameFlags::RENAME_NOREPLACE,
    )
    .map_err(std::io::Error::from)
}

#[cfg(not(target_os = "linux"))]
fn rename_noreplace_cap(
    _source_dir: &Dir,
    _source: impl AsRef<OsStr>,
    _target_dir: &Dir,
    _target: impl AsRef<OsStr>,
) -> std::io::Result<()> {
    Err(std::io::Error::new(
        ErrorKind::Unsupported,
        "atomic no-replace rename is unsupported on this target",
    ))
}

fn create_transaction_dirs(base: Dir, transactions: Dir, id: &str) -> Result<ControlDirs> {
    transactions.create_dir(id).map_err(|error| {
        if error.kind() == ErrorKind::AlreadyExists {
            anyhow::Error::new(FileTransactionError::UnexpectedOccupant {
                path: format!("transaction control {id}"),
            })
        } else {
            anyhow::Error::new(error)
        }
    })?;
    sync_directory(&transactions)?;
    let transaction = open_existing_dir(&transactions, id)?;
    let stages = open_or_create_dir(&transaction, "stages")?;
    let backups = open_or_create_dir(&transaction, "backups")?;
    Ok(ControlDirs {
        base,
        transactions,
        transaction,
        stages,
        backups,
        companion: None,
    })
}

fn open_transaction_dirs(base: Dir, transactions: Dir, id: &str) -> Result<ControlDirs> {
    let transaction = open_existing_dir(&transactions, id)?;
    let stages = open_existing_dir(&transaction, "stages")?;
    let backups = open_existing_dir(&transaction, "backups")?;
    Ok(ControlDirs {
        base,
        transactions,
        transaction,
        stages,
        backups,
        companion: None,
    })
}

fn create_journal(directory: &Dir, journal: &TransactionJournal) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(journal)?;
    stage_bytes(directory, JOURNAL_FILE, &bytes)?;
    sync_directory(directory)?;
    Ok(())
}

fn read_journal(directory: &Dir) -> Result<TransactionJournal> {
    let bytes = directory.read(JOURNAL_FILE)?;
    serde_json::from_slice(&bytes).context("reading durable transaction journal")
}

fn durable_state(control: &ControlDirs, fallback: RecoveryState) -> RecoveryState {
    read_journal(&control.transaction)
        .map(|journal| RecoveryState::from(journal.decision))
        .unwrap_or(fallback)
}

fn verify_final_actions(root: &Dir, journal: &TransactionJournal) -> Result<()> {
    for action in &journal.actions {
        ensure_published_identity(root, action)?;
    }
    Ok(())
}

fn ensure_published_identity(root: &Dir, action: &JournalAction) -> Result<()> {
    let expected = match &action.action {
        JournalActionKind::CreateDirectory { .. } => TargetIdentity::Directory,
        JournalActionKind::WriteFile { final_identity, .. }
        | JournalActionKind::SetMode { final_identity, .. } => TargetIdentity::File {
            identity: final_identity.clone(),
        },
    };
    let actual = inspect_target(root, action.action.path())?;
    if actual == expected {
        Ok(())
    } else {
        Err(FileTransactionError::UnexpectedOccupant {
            path: action.action.path().to_string(),
        }
        .into())
    }
}

fn reverse_action(
    root: &Dir,
    control: &ControlDirs,
    entry: &JournalAction,
    injector: &dyn TransactionFailureInjector,
    action_index: usize,
) -> Result<()> {
    let (parent, leaf) = match open_parent(root, entry.action.path(), false) {
        Ok(value) => value,
        Err(error)
            if matches!(entry.original, TargetIdentity::Absent)
                && error.downcast_ref::<MissingParent>().is_some() =>
        {
            return Ok(())
        }
        Err(error) => return Err(error),
    };
    match &entry.action {
        JournalActionKind::CreateDirectory { path, .. } => {
            if matches!(entry.original, TargetIdentity::Absent) {
                match parent.remove_dir(&leaf) {
                    Ok(()) => sync_directory(&parent)?,
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(error).with_context(|| format!("removing created {path}"))
                    }
                }
            }
        }
        JournalActionKind::WriteFile {
            path,
            final_identity,
            backup_name,
            ..
        } => {
            let current = inspect_leaf(&parent, &leaf)?;
            if current == entry.original {
                let backup = inspect_leaf(&control.backups, backup_name)?;
                match backup {
                    TargetIdentity::Absent => return Ok(()),
                    value if value == entry.original => {
                        control.backups.remove_file(backup_name)?;
                        sync_directory(&control.backups)?;
                        return Ok(());
                    }
                    _ => {
                        return Err(FileTransactionError::UnexpectedOccupant {
                            path: format!("backup for {path}"),
                        }
                        .into())
                    }
                }
            }
            match current {
                TargetIdentity::Absent => {}
                TargetIdentity::File { identity } if identity == *final_identity => {
                    parent.remove_file(&leaf)?;
                    sync_directory(&parent)?;
                }
                _ => {
                    return Err(
                        FileTransactionError::UnexpectedOccupant { path: path.clone() }.into(),
                    )
                }
            }
            if matches!(entry.original, TargetIdentity::File { .. }) {
                let backup = inspect_leaf(&control.backups, backup_name)?;
                if backup != entry.original {
                    return Err(FileTransactionError::UnexpectedOccupant {
                        path: format!("backup for {path}"),
                    }
                    .into());
                }
                control.backups.hard_link(backup_name, &parent, &leaf)?;
                sync_directory(&parent)?;
                control.backups.remove_file(backup_name)?;
                sync_directory(&control.backups)?;
            }
        }
        JournalActionKind::SetMode {
            path,
            original_mode,
            final_identity,
            ..
        } => {
            let current = inspect_leaf(&parent, &leaf)?;
            if current == entry.original {
                return Ok(());
            }
            if current
                != (TargetIdentity::File {
                    identity: final_identity.clone(),
                })
            {
                return Err(FileTransactionError::UnexpectedOccupant { path: path.clone() }.into());
            }
            injector.check(&FailurePoint::BeforeReverseModeMutation {
                action: action_index,
            })?;
            set_mode_if_identity(&parent, &leaf, final_identity, *original_mode)?;
            sync_directory(&parent)?;
        }
    }
    Ok(())
}

fn verify_restored_action(root: &Dir, control: &ControlDirs, entry: &JournalAction) -> Result<()> {
    if inspect_target(root, entry.action.path())? != entry.original {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: entry.action.path().to_string(),
        }
        .into());
    }
    if let JournalActionKind::WriteFile {
        path, backup_name, ..
    } = &entry.action
    {
        if inspect_leaf(&control.backups, backup_name)? != TargetIdentity::Absent {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: format!("backup for {path}"),
            }
            .into());
        }
    }
    Ok(())
}

fn remove_fresh_root_if_empty(root: &Dir) -> Result<()> {
    match root.remove_dir(".jit") {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("removing fresh .jit root after rollback"),
    }
}

fn normalize_actions(
    root: &Dir,
    actions: Vec<TransactionAction>,
) -> Result<Vec<TransactionAction>> {
    let mut targets = HashSet::new();
    for action in &actions {
        validate_relative(action.path())?;
        if !targets.insert(action.path().to_string()) {
            return Err(FileTransactionError::DuplicateTarget {
                path: action.path().to_string(),
            }
            .into());
        }
        if action.path() == BOOTSTRAP_DIR
            || action.path().starts_with(&format!("{BOOTSTRAP_DIR}/"))
            || action.path().starts_with(".jit/tmp/transactions")
        {
            return Err(FileTransactionError::InvalidPath {
                path: action.path().to_string(),
            }
            .into());
        }
    }

    let mut required = HashSet::new();
    for action in &actions {
        let path = Path::new(action.path());
        let directory = matches!(action, TransactionAction::CreateDirectory { .. });
        let end = if directory { Some(path) } else { path.parent() };
        let Some(end) = end else { continue };
        let mut current = String::new();
        for component in end.components() {
            let text = component.as_os_str().to_string_lossy();
            if !current.is_empty() {
                current.push('/');
            }
            current.push_str(&text);
            if inspect_target(root, &current)? == TargetIdentity::Absent {
                required.insert(current.clone());
            }
        }
    }
    let explicit_dirs = actions
        .iter()
        .filter_map(|action| match action {
            TransactionAction::CreateDirectory { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect::<HashSet<_>>();
    let mut implicit = required
        .into_iter()
        .filter(|path| !explicit_dirs.contains(path.as_str()))
        .map(|path| TransactionAction::CreateDirectory {
            path,
            unix_mode: None,
        })
        .collect::<Vec<_>>();
    implicit.sort_by_key(|action| {
        (
            action.path().matches('/').count(),
            action.path().to_string(),
        )
    });
    implicit.extend(actions);
    Ok(implicit)
}

fn validate_transaction_id(id: &str) -> Result<()> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(FileTransactionError::InvalidPath {
            path: format!("transaction id {id}"),
        }
        .into());
    }
    Ok(())
}

fn validate_relative(path: &str) -> Result<()> {
    let windows_absolute = path.starts_with(['/', '\\'])
        || path.as_bytes().get(1) == Some(&b':')
        || path.contains('\\');
    let valid = !path.is_empty()
        && !windows_absolute
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)));
    if !valid {
        return Err(FileTransactionError::InvalidPath {
            path: path.to_string(),
        }
        .into());
    }
    Ok(())
}

fn hash_plan(actions: &[TransactionAction]) -> String {
    let mut hasher = Sha256::new();
    for action in actions {
        match action {
            TransactionAction::CreateDirectory { path, unix_mode } => {
                hasher.update(b"dir\0");
                hasher.update(path.as_bytes());
                hasher.update(unix_mode.unwrap_or_default().to_le_bytes());
            }
            TransactionAction::WriteFile {
                path,
                contents,
                unix_mode,
            } => {
                hasher.update(b"file\0");
                hasher.update(path.as_bytes());
                hasher.update(unix_mode.unwrap_or_default().to_le_bytes());
                hasher.update((contents.len() as u64).to_le_bytes());
                hasher.update(contents);
            }
            TransactionAction::SetMode { path, unix_mode } => {
                hasher.update(b"mode\0");
                hasher.update(path.as_bytes());
                hasher.update(unix_mode.to_le_bytes());
            }
        }
    }
    format!("{:x}", hasher.finalize())
}

fn inspect_target(root: &Dir, path: &str) -> Result<TargetIdentity> {
    match open_parent(root, path, false) {
        Ok((parent, leaf)) => inspect_leaf(&parent, &leaf),
        Err(error) if error.downcast_ref::<MissingParent>().is_some() => Ok(TargetIdentity::Absent),
        Err(error) => Err(error),
    }
}

fn inspect_leaf(parent: &Dir, leaf: &str) -> Result<TargetIdentity> {
    let metadata = match parent.symlink_metadata(leaf) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(TargetIdentity::Absent),
        Err(error) => return Err(error.into()),
    };
    if metadata.is_symlink() {
        return Err(FileTransactionError::SymlinkComponent {
            path: leaf.to_string(),
        }
        .into());
    }
    if metadata.is_dir() {
        return Ok(TargetIdentity::Directory);
    }
    if !metadata.is_file() {
        return Err(FileTransactionError::UnsupportedTarget {
            path: leaf.to_string(),
        }
        .into());
    }
    Ok(TargetIdentity::File {
        identity: inspect_file(parent, leaf)?,
    })
}

fn inspect_file(parent: &Dir, leaf: &str) -> Result<FileIdentity> {
    let mut file = open_regular_file_nofollow(parent, leaf)?;
    inspect_open_file(&mut file, leaf)
}

fn inspect_open_file(file: &mut cap_std::fs::File, path: &str) -> Result<FileIdentity> {
    #[cfg(unix)]
    use cap_std::fs::PermissionsExt as _;

    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(FileTransactionError::UnsupportedTarget {
            path: path.to_string(),
        }
        .into());
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    #[cfg(unix)]
    let unix_mode = Some(metadata.permissions().mode() & 0o7777);
    #[cfg(not(unix))]
    let unix_mode = None;
    Ok(FileIdentity {
        sha256,
        byte_size: bytes.len() as u64,
        unix_mode,
    })
}

pub(crate) fn open_regular_file_nofollow(parent: &Dir, leaf: &str) -> Result<cap_std::fs::File> {
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let file = parent.open_with(leaf, &options)?;
    if !file.metadata()?.is_file() {
        return Err(FileTransactionError::UnsupportedTarget {
            path: leaf.to_string(),
        }
        .into());
    }
    Ok(file)
}

fn ensure_identity(parent: &Dir, leaf: &str, expected: &TargetIdentity, path: &str) -> Result<()> {
    if inspect_leaf(parent, leaf)? == *expected {
        Ok(())
    } else {
        Err(FileTransactionError::UnexpectedOccupant {
            path: path.to_string(),
        }
        .into())
    }
}

#[derive(Debug, thiserror::Error)]
#[error("missing target parent: {0}")]
struct MissingParent(String);

fn open_parent(root: &Dir, path: &str, create: bool) -> Result<(Dir, String)> {
    validate_relative(path)?;
    let components = Path::new(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    let (leaf, parents) =
        components
            .split_last()
            .ok_or_else(|| FileTransactionError::InvalidPath {
                path: path.to_string(),
            })?;
    let mut current = root.try_clone()?;
    let mut traversed = String::new();
    for component in parents {
        if !traversed.is_empty() {
            traversed.push('/');
        }
        traversed.push_str(component);
        match current.symlink_metadata(component) {
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                return Err(FileTransactionError::SymlinkComponent { path: traversed }.into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound && create => {
                current.create_dir(component)?;
                sync_directory(&current)?;
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                return Err(MissingParent(traversed).into())
            }
            Err(error) => return Err(error.into()),
        }
        current = open_existing_dir(&current, component)?;
    }
    Ok((current, leaf.clone()))
}

fn open_existing_dir(parent: &Dir, name: &str) -> Result<Dir> {
    let metadata = parent.symlink_metadata(name)?;
    if metadata.is_symlink() || !metadata.is_dir() {
        return Err(FileTransactionError::SymlinkComponent {
            path: name.to_string(),
        }
        .into());
    }
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let file = parent.open_with(name, &options)?;
    if !file.metadata()?.is_dir() {
        return Err(FileTransactionError::SymlinkComponent {
            path: name.to_string(),
        }
        .into());
    }
    Ok(Dir::from_std_file(file.into_std()))
}

fn sync_capable_directory(directory: &Dir) -> std::io::Result<Dir> {
    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let file = directory.open_with(".", &options)?;
    if !file.metadata()?.is_dir() {
        return Err(std::io::Error::other("capability is not a directory"));
    }
    Ok(Dir::from_std_file(file.into_std()))
}

fn open_or_create_dir(parent: &Dir, name: &str) -> Result<Dir> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
            return Err(FileTransactionError::SymlinkComponent {
                path: name.to_string(),
            }
            .into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            parent.create_dir(name)?;
            sync_directory(parent)?;
        }
        Err(error) => return Err(error.into()),
    }
    open_existing_dir(parent, name)
}

fn open_or_create_protocol_dir(parent: &Dir, name: &str) -> Result<Dir> {
    match parent.symlink_metadata(name) {
        Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
            return Err(FileTransactionError::UnexpectedBootstrapOccupant.into())
        }
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {
            parent.create_dir(name)?;
            sync_directory(parent)?;
        }
        Err(error) => return Err(error.into()),
    }
    open_existing_dir(parent, name)
}

fn ensure_protocol_marker(directory: &Dir) -> Result<()> {
    match directory.read_to_string(PROTOCOL_MARKER) {
        Ok(contents) if contents == "1\n" => validate_protocol_contents(directory),
        Ok(_) => Err(FileTransactionError::UnexpectedBootstrapOccupant.into()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            if directory.entries()?.next().is_some() {
                return Err(FileTransactionError::UnexpectedBootstrapOccupant.into());
            }
            stage_bytes(directory, PROTOCOL_MARKER, b"1\n")?;
            sync_directory(directory)?;
            Ok(())
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_existing_protocol_marker(directory: &Dir) -> Result<()> {
    match directory.read_to_string(PROTOCOL_MARKER) {
        Ok(contents) if contents == "1\n" => validate_protocol_contents(directory),
        _ => Err(FileTransactionError::UnexpectedBootstrapOccupant.into()),
    }
}

fn validate_protocol_contents(directory: &Dir) -> Result<()> {
    let valid = directory.entries()?.all(|entry| {
        entry
            .ok()
            .and_then(|entry| entry.file_name().into_string().ok())
            .is_some_and(|name| name == PROTOCOL_MARKER || name == "transactions")
    });
    if valid {
        Ok(())
    } else {
        Err(FileTransactionError::UnexpectedBootstrapOccupant.into())
    }
}

fn directory_is_empty(directory: &Dir) -> Result<bool> {
    Ok(directory.entries()?.next().is_none())
}

fn metadata_optional(directory: &Dir, path: &str) -> Result<Option<cap_std::fs::Metadata>> {
    match directory.symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.is_symlink() {
                return Err(FileTransactionError::SymlinkComponent {
                    path: path.to_string(),
                }
                .into());
            }
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn set_mode(parent: &Dir, leaf: &str, mode: Option<u32>) -> Result<()> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use cap_std::fs::PermissionsExt;
        let file = open_regular_file_nofollow(parent, leaf)?;
        file.set_permissions(cap_std::fs::Permissions::from_mode(mode & 0o7777))?;
        file.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = (parent, leaf, mode);
    Ok(())
}

fn set_mode_if_identity(
    parent: &Dir,
    leaf: &str,
    expected: &FileIdentity,
    mode: Option<u32>,
) -> Result<()> {
    let mut file = open_regular_file_nofollow(parent, leaf)?;
    let actual = inspect_open_file(&mut file, leaf)?;
    if actual != *expected {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: leaf.to_string(),
        }
        .into());
    }
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt;
        if let Some(mode) = mode {
            file.set_permissions(cap_std::fs::Permissions::from_mode(mode & 0o7777))?;
            file.sync_all()?;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }
    Ok(())
}

fn set_directory_mode(directory: &Dir, mode: Option<u32>) -> Result<()> {
    #[cfg(unix)]
    if let Some(mode) = mode {
        use cap_std::fs::PermissionsExt;
        directory.set_permissions(".", cap_std::fs::Permissions::from_mode(mode & 0o7777))?;
    }
    #[cfg(not(unix))]
    let _ = (directory, mode);
    sync_directory(directory)?;
    Ok(())
}

fn preflight_volume(stage: &Dir, root: &Dir, path: &str) -> Result<()> {
    let target_parent = nearest_existing_parent(root, path)?;
    #[cfg(unix)]
    {
        if stage.dir_metadata()?.dev() != target_parent.dir_metadata()?.dev() {
            return Err(FileTransactionError::CrossVolume {
                path: path.to_string(),
            }
            .into());
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        let stage_meta = stage.try_clone()?.into_std_file().metadata()?;
        let target_meta = target_parent.try_clone()?.into_std_file().metadata()?;
        match (
            stage_meta.volume_serial_number(),
            target_meta.volume_serial_number(),
        ) {
            (Some(left), Some(right)) if left == right => {}
            (Some(_), Some(_)) => {
                return Err(FileTransactionError::CrossVolume {
                    path: path.to_string(),
                }
                .into())
            }
            _ => {
                return Err(FileTransactionError::UnsupportedFilesystem {
                    operation: "volume identity".to_string(),
                }
                .into())
            }
        }
    }
    Ok(())
}

fn nearest_existing_parent(root: &Dir, path: &str) -> Result<Dir> {
    let components = Path::new(path)
        .parent()
        .into_iter()
        .flat_map(Path::components)
        .map(|component| component.as_os_str().to_string_lossy().into_owned());
    let mut current = root.try_clone()?;
    for component in components {
        match current.symlink_metadata(&component) {
            Ok(metadata) if metadata.is_symlink() || !metadata.is_dir() => {
                return Err(FileTransactionError::SymlinkComponent { path: component }.into())
            }
            Ok(_) => current = open_existing_dir(&current, &component)?,
            Err(error) if error.kind() == ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(current)
}

fn publication_error(error: std::io::Error, path: &str) -> anyhow::Error {
    if error.kind() == ErrorKind::AlreadyExists {
        FileTransactionError::UnexpectedOccupant {
            path: path.to_string(),
        }
        .into()
    } else if is_cross_volume(&error) {
        FileTransactionError::CrossVolume {
            path: path.to_string(),
        }
        .into()
    } else {
        error.into()
    }
}

#[cfg(unix)]
fn is_cross_volume(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(libc_exdev())
}

#[cfg(unix)]
const fn libc_exdev() -> i32 {
    18
}

#[cfg(windows)]
fn is_cross_volume(error: &std::io::Error) -> bool {
    error.raw_os_error() == Some(17)
}

#[cfg(not(any(unix, windows)))]
fn is_cross_volume(_error: &std::io::Error) -> bool {
    false
}

fn remove_optional_file(directory: &Dir, name: &str) -> Result<()> {
    match directory.remove_file(name) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cap_std::ambient_authority;
    use std::collections::HashSet;
    use std::sync::Mutex;
    use std::time::Duration;
    use tempfile::TempDir;

    struct SelectedFailures {
        failures: Mutex<HashSet<FailurePoint>>,
        observed: Mutex<Vec<FailurePoint>>,
    }

    impl SelectedFailures {
        fn new(failures: impl IntoIterator<Item = FailurePoint>) -> Self {
            Self {
                failures: Mutex::new(failures.into_iter().collect()),
                observed: Mutex::new(Vec::new()),
            }
        }

        fn observed(&self) -> Vec<FailurePoint> {
            self.observed.lock().unwrap().clone()
        }
    }

    impl TransactionFailureInjector for SelectedFailures {
        fn check(&self, point: &FailurePoint) -> std::io::Result<()> {
            self.observed.lock().unwrap().push(point.clone());
            if self.failures.lock().unwrap().remove(point) {
                Err(std::io::Error::other(format!("injected at {point:?}")))
            } else {
                Ok(())
            }
        }
    }

    struct HookInjector {
        point: FailurePoint,
        hook: Mutex<Option<Box<dyn FnOnce() + Send>>>,
    }

    impl TransactionFailureInjector for HookInjector {
        fn check(&self, point: &FailurePoint) -> std::io::Result<()> {
            if point == &self.point {
                if let Some(hook) = self.hook.lock().unwrap().take() {
                    hook();
                }
            }
            Ok(())
        }
    }

    fn root_capability(temp: &TempDir) -> Dir {
        Dir::open_ambient_dir(temp.path(), ambient_authority()).unwrap()
    }

    fn acquired_guard(
        temp: &TempDir,
    ) -> (Arc<super::super::repo_lock::RepoWriteLock>, RepoWriteGuard) {
        let lock = super::super::repo_lock::RepoWriteLock::for_storage_root(
            temp.path().join("guard"),
            Duration::from_secs(2),
        );
        let guard = lock.acquire().unwrap();
        (lock, guard)
    }

    fn write(path: &str, contents: &[u8]) -> TransactionAction {
        TransactionAction::WriteFile {
            path: path.to_string(),
            contents: contents.to_vec(),
            unix_mode: None,
        }
    }

    #[test]
    fn test_transaction_kernel_publishes_deterministic_existing_repository_set() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::write(temp.path().join(".jit/config.toml"), b"old").unwrap();
        let kernel = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let first = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "existing-commit".to_string(),
                    actions: vec![
                        write(".jit/config.toml", b"new"),
                        write("generated/nested/file.txt", b"complete"),
                    ],
                },
            )
            .unwrap();

        assert_eq!(first.recovery_state, RecoveryState::Clean);
        assert_eq!(
            std::fs::read(temp.path().join(".jit/config.toml")).unwrap(),
            b"new"
        );
        assert_eq!(
            std::fs::read(temp.path().join("generated/nested/file.txt")).unwrap(),
            b"complete"
        );
        assert_eq!(kernel.recovery_state("existing-commit").unwrap(), None);
    }

    #[test]
    fn test_empty_transaction_is_a_journal_free_no_op() {
        let temp = TempDir::new().unwrap();
        let kernel = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let outcome = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "no-op".to_string(),
                    actions: Vec::new(),
                },
            )
            .unwrap();
        assert_eq!(outcome.recovery_state, RecoveryState::Clean);
        assert!(!temp.path().join(BOOTSTRAP_DIR).exists());
        assert!(!temp.path().join(".jit").exists());
    }

    #[test]
    fn test_transaction_kernel_rolls_back_existing_repository_exactly() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::write(temp.path().join(".jit/config.toml"), b"old").unwrap();
        let failures = Arc::new(SelectedFailures::new([FailurePoint::AfterPublish {
            action: 1,
        }]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures.clone()).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel.execute(
            &guard,
            FileTransactionPlan {
                transaction_id: "existing-rollback".to_string(),
                actions: vec![
                    write(".jit/config.toml", b"new"),
                    write("second.txt", b"partial"),
                ],
            },
        );

        assert!(error.is_err());
        assert_eq!(
            std::fs::read(temp.path().join(".jit/config.toml")).unwrap(),
            b"old"
        );
        assert!(!temp.path().join("second.txt").exists());
        let observed = failures.observed();
        assert!(observed.contains(&FailurePoint::ReverseAction { action: 1 }));
        assert!(observed.contains(&FailurePoint::ReverseAction { action: 0 }));
        assert!(observed.contains(&FailurePoint::SyncReverseParent { action: 0 }));
    }

    #[test]
    fn test_sync_reverse_parent_failure_resumes_from_durable_action_progress() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::write(temp.path().join("target.txt"), b"old").unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::AfterPublish { action: 0 },
            FailurePoint::SyncReverseParent { action: 0 },
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "reverse-parent-progress".to_string(),
                    actions: vec![write("target.txt", b"new")],
                },
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
            RecoveryState::Prepared
        );
        assert_eq!(
            std::fs::read(temp.path().join("target.txt")).unwrap(),
            b"old"
        );

        kernel.recover(&guard, "reverse-parent-progress").unwrap();
        assert_eq!(
            kernel.recovery_state("reverse-parent-progress").unwrap(),
            None
        );
        assert_eq!(
            std::fs::read(temp.path().join("target.txt")).unwrap(),
            b"old"
        );
    }

    #[test]
    fn test_recovery_skips_durably_restored_replacement_after_later_reverse_failure() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::write(temp.path().join("first.txt"), b"first-old").unwrap();
        std::fs::write(temp.path().join("second.txt"), b"second-old").unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::AfterPublish { action: 1 },
            FailurePoint::ReverseAction { action: 0 },
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "replacement-progress".to_string(),
                    actions: vec![
                        write("first.txt", b"first-new"),
                        write("second.txt", b"second-new"),
                    ],
                },
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
            RecoveryState::Prepared
        );
        let journal: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                temp.path()
                    .join(".jit/tmp/transactions/replacement-progress/journal.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(journal["actions"][1]["rollback_state"], "restored");
        assert_eq!(
            std::fs::read(temp.path().join("second.txt")).unwrap(),
            b"second-old"
        );

        FileTransactionKernel::new(root_capability(&temp))
            .unwrap()
            .recover(&guard, "replacement-progress")
            .unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("first.txt")).unwrap(),
            b"first-old"
        );
        assert_eq!(
            std::fs::read(temp.path().join("second.txt")).unwrap(),
            b"second-old"
        );
    }

    #[test]
    fn test_after_rename_aside_failure_restores_original_without_overwrite() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::write(temp.path().join("replace.txt"), b"old").unwrap();
        let failures = Arc::new(SelectedFailures::new([FailurePoint::AfterRenameAside {
            action: 0,
        }]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "rename-aside".to_string(),
                    actions: vec![write("replace.txt", b"new")],
                },
            )
            .is_err());
        assert_eq!(
            std::fs::read(temp.path().join("replace.txt")).unwrap(),
            b"old"
        );
        assert_eq!(kernel.recovery_state("rename-aside").unwrap(), None);
    }

    #[test]
    fn test_fresh_root_rollback_removes_jit_before_terminal_journal_cleanup() {
        let temp = TempDir::new().unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::AfterPublish { action: 1 },
            FailurePoint::CleanupTerminalResidue,
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures.clone()).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "fresh-rollback".to_string(),
                    actions: vec![write(".jit/index.json", b"{}")],
                },
            )
            .is_err());

        assert!(!temp.path().join(".jit").exists());
        assert_eq!(
            kernel.recovery_state("fresh-rollback").unwrap(),
            Some(RecoveryState::RolledBack)
        );
        let observed = failures.observed();
        let removed = observed
            .iter()
            .position(|point| point == &FailurePoint::AfterFreshRootRemoval)
            .unwrap();
        let terminal = observed
            .iter()
            .position(|point| {
                point
                    == &FailurePoint::SyncJournal {
                        decision: RecoveryState::RolledBack,
                    }
            })
            .unwrap();
        assert!(
            removed < terminal,
            "absence is synchronized before terminal rollback"
        );
    }

    #[test]
    fn test_fresh_root_removal_boundaries_retain_prepared_journal_and_resume() {
        for (id, boundary) in [
            ("before-root-remove", FailurePoint::BeforeFreshRootRemoval),
            ("after-root-remove", FailurePoint::AfterFreshRootRemoval),
        ] {
            let temp = TempDir::new().unwrap();
            let failures = Arc::new(SelectedFailures::new([
                FailurePoint::AfterPublish { action: 1 },
                boundary,
            ]));
            let kernel =
                FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
            let (_lock, guard) = acquired_guard(&temp);
            let error = kernel
                .execute(
                    &guard,
                    FileTransactionPlan {
                        transaction_id: id.to_string(),
                        actions: vec![write(".jit/index.json", b"{}")],
                    },
                )
                .unwrap_err();
            assert_eq!(
                error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
                RecoveryState::Prepared
            );
            assert_eq!(
                kernel.recovery_state(id).unwrap(),
                Some(RecoveryState::Prepared)
            );
            FileTransactionKernel::new(root_capability(&temp))
                .unwrap()
                .recover(&guard, id)
                .unwrap();
            assert!(!temp.path().join(".jit").exists());
        }
    }

    #[test]
    fn test_prepared_transaction_is_resumable_after_reverse_failure() {
        let temp = TempDir::new().unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::AfterPublish { action: 1 },
            FailurePoint::ReverseAction { action: 1 },
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "prepared-resume".to_string(),
                    actions: vec![write(".jit/index.json", b"{}")],
                },
            )
            .unwrap_err();
        assert!(error.downcast_ref::<RecoveryRequiredError>().is_some());
        assert_eq!(
            kernel.recovery_state("prepared-resume").unwrap(),
            Some(RecoveryState::Prepared)
        );

        let recovery = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        recovery.recover(&guard, "prepared-resume").unwrap();
        assert!(!temp.path().join(".jit").exists());
        assert_eq!(recovery.recovery_state("prepared-resume").unwrap(), None);
    }

    #[test]
    fn test_committed_and_rolled_back_terminal_residue_are_resumable() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::CleanupTerminalResidue,
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "committed-resume".to_string(),
                    actions: vec![write("committed.txt", b"final")],
                },
            )
            .unwrap_err();
        let recovery_required = error.downcast_ref::<RecoveryRequiredError>().unwrap();
        assert_eq!(recovery_required.state, RecoveryState::Committed);

        let recovery = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        recovery.recover(&guard, "committed-resume").unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("committed.txt")).unwrap(),
            b"final"
        );

        let fresh = TempDir::new().unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::AfterPublish { action: 1 },
            FailurePoint::CleanupTerminalResidue,
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&fresh), failures).unwrap();
        let (_lock, guard) = acquired_guard(&fresh);
        let _ = kernel.execute(
            &guard,
            FileTransactionPlan {
                transaction_id: "rolled-resume".to_string(),
                actions: vec![write(".jit/index.json", b"{}")],
            },
        );
        let recovery = FileTransactionKernel::new(root_capability(&fresh)).unwrap();
        recovery.recover(&guard, "rolled-resume").unwrap();
        assert_eq!(recovery.recovery_state("rolled-resume").unwrap(), None);
    }

    #[test]
    fn test_each_durable_decision_failure_retains_typed_recovery_state() {
        for (id, failure, expected) in [
            (
                "prepared-decision",
                FailurePoint::SyncJournal {
                    decision: RecoveryState::Prepared,
                },
                RecoveryState::Prepared,
            ),
            (
                "committed-decision",
                FailurePoint::SyncJournal {
                    decision: RecoveryState::Committed,
                },
                RecoveryState::Committed,
            ),
        ] {
            let temp = TempDir::new().unwrap();
            std::fs::create_dir(temp.path().join(".jit")).unwrap();
            let failures = Arc::new(SelectedFailures::new([failure]));
            let kernel =
                FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
            let (_lock, guard) = acquired_guard(&temp);
            let error = kernel
                .execute(
                    &guard,
                    FileTransactionPlan {
                        transaction_id: id.to_string(),
                        actions: vec![write("decision.txt", b"state")],
                    },
                )
                .unwrap_err();
            let recovery = error.downcast_ref::<RecoveryRequiredError>().unwrap();
            assert_eq!(recovery.state, expected);
            assert_eq!(kernel.recovery_state(id).unwrap(), Some(expected));
        }

        let temp = TempDir::new().unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::AfterPublish { action: 1 },
            FailurePoint::SyncJournal {
                decision: RecoveryState::RolledBack,
            },
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "rolled-decision".to_string(),
                    actions: vec![write(".jit/index.json", b"{}")],
                },
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
            RecoveryState::RolledBack
        );
        assert!(!temp.path().join(".jit").exists());
        assert_eq!(
            kernel.recovery_state("rolled-decision").unwrap(),
            Some(RecoveryState::RolledBack)
        );
    }

    #[test]
    fn test_target_occupant_race_fails_without_overwrite() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let target = temp.path().join("raced.txt");
        let hook_target = target.clone();
        let injector = Arc::new(HookInjector {
            point: FailurePoint::AfterParentOpen { action: 0 },
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::write(&hook_target, b"other-writer").unwrap();
            }))),
        });
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "target-race".to_string(),
                    actions: vec![write("raced.txt", b"ours")],
                },
            )
            .is_err());
        assert_eq!(std::fs::read(target).unwrap(), b"other-writer");
    }

    #[cfg(unix)]
    #[test]
    fn test_parent_symlink_swap_never_publishes_outside_held_capability() {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::create_dir(temp.path().join("safe")).unwrap();
        let outside = TempDir::new().unwrap();
        let safe = temp.path().join("safe");
        let moved = temp.path().join("safe-moved");
        let outside_path = outside.path().to_path_buf();
        let injector = Arc::new(HookInjector {
            point: FailurePoint::AfterParentOpen { action: 0 },
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&safe, &moved).unwrap();
                symlink(&outside_path, &safe).unwrap();
            }))),
        });
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "parent-swap".to_string(),
                    actions: vec![write("safe/out.txt", b"ours")],
                },
            )
            .unwrap_err();
        assert!(error.downcast_ref::<RecoveryRequiredError>().is_some());
        assert!(!outside.path().join("out.txt").exists());
    }

    #[cfg(windows)]
    #[test]
    fn test_parent_symlink_or_junction_swap_never_publishes_outside_held_capability() {
        use std::os::windows::fs::symlink_dir;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        std::fs::create_dir(temp.path().join("safe")).unwrap();
        let outside = TempDir::new().unwrap();
        let safe = temp.path().join("safe");
        let moved = temp.path().join("safe-moved");
        let outside_path = outside.path().to_path_buf();
        let injector = Arc::new(HookInjector {
            point: FailurePoint::AfterParentOpen { action: 0 },
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&safe, &moved).unwrap();
                symlink_dir(&outside_path, &safe).unwrap();
            }))),
        });
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "parent-swap".to_string(),
                    actions: vec![write("safe/out.txt", b"ours")],
                },
            )
            .unwrap_err();
        assert!(error.downcast_ref::<RecoveryRequiredError>().is_some());
        assert!(!outside.path().join("out.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn test_write_file_applies_mode_before_publication() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let kernel = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode".to_string(),
                    actions: vec![TransactionAction::WriteFile {
                        path: "script.sh".to_string(),
                        contents: b"#!/bin/sh\n".to_vec(),
                        unix_mode: Some(0o755),
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            std::fs::metadata(temp.path().join("script.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_before_mode_mutation_failure_leaves_original_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let target = temp.path().join("mode.txt");
        std::fs::write(&target, b"mode").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let failures = Arc::new(SelectedFailures::new([FailurePoint::BeforeModeMutation {
            action: 0,
        }]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode-forward".to_string(),
                    actions: vec![TransactionAction::SetMode {
                        path: "mode.txt".to_string(),
                        unix_mode: 0o755,
                    }],
                },
            )
            .is_err());
        assert_eq!(kernel.recovery_state("mode-forward").unwrap(), None);
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_set_mode_forward_and_reverse_failures_are_injectable_and_resumable() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let target = temp.path().join("mode.txt");
        std::fs::write(&target, b"mode").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::SyncTargetParent { action: 0 },
            FailurePoint::BeforeReverseModeMutation { action: 0 },
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures.clone()).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode-reverse".to_string(),
                    actions: vec![TransactionAction::SetMode {
                        path: "mode.txt".to_string(),
                        unix_mode: 0o755,
                    }],
                },
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
            RecoveryState::Prepared
        );
        assert!(failures
            .observed()
            .contains(&FailurePoint::BeforeModeMutation { action: 0 }));

        FileTransactionKernel::new(root_capability(&temp))
            .unwrap()
            .recover(&guard, "mode-reverse")
            .unwrap();
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_recovery_skips_durably_restored_mode_after_later_reverse_failure() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        for name in ["first-mode.txt", "second-mode.txt"] {
            let path = temp.path().join(name);
            std::fs::write(&path, b"mode").unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let failures = Arc::new(SelectedFailures::new([
            FailurePoint::SyncTargetParent { action: 1 },
            FailurePoint::ReverseAction { action: 0 },
        ]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode-progress".to_string(),
                    actions: vec![
                        TransactionAction::SetMode {
                            path: "first-mode.txt".to_string(),
                            unix_mode: 0o755,
                        },
                        TransactionAction::SetMode {
                            path: "second-mode.txt".to_string(),
                            unix_mode: 0o755,
                        },
                    ],
                },
            )
            .unwrap_err();
        assert_eq!(
            error.downcast_ref::<RecoveryRequiredError>().unwrap().state,
            RecoveryState::Prepared
        );
        let journal: serde_json::Value = serde_json::from_slice(
            &std::fs::read(
                temp.path()
                    .join(".jit/tmp/transactions/mode-progress/journal.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(journal["actions"][1]["rollback_state"], "restored");

        FileTransactionKernel::new(root_capability(&temp))
            .unwrap()
            .recover(&guard, "mode-progress")
            .unwrap();
        for name in ["first-mode.txt", "second-mode.txt"] {
            assert_eq!(
                std::fs::metadata(temp.path().join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn test_set_mode_leaf_symlink_swap_cannot_mutate_external_target() {
        use std::os::unix::fs::{symlink, PermissionsExt as _};

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let target = temp.path().join("mode.txt");
        let moved = temp.path().join("mode-original.txt");
        std::fs::write(&target, b"inside").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let outside = TempDir::new().unwrap();
        let outside_target = outside.path().join("outside.txt");
        std::fs::write(&outside_target, b"outside").unwrap();
        std::fs::set_permissions(&outside_target, std::fs::Permissions::from_mode(0o640)).unwrap();
        let hook_target = target.clone();
        let hook_moved = moved.clone();
        let hook_outside = outside_target.clone();
        let injector = Arc::new(HookInjector {
            point: FailurePoint::BeforeModeMutation { action: 0 },
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&hook_target, &hook_moved).unwrap();
                symlink(&hook_outside, &hook_target).unwrap();
            }))),
        });
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode-symlink-race".to_string(),
                    actions: vec![TransactionAction::SetMode {
                        path: "mode.txt".to_string(),
                        unix_mode: 0o755,
                    }],
                },
            )
            .unwrap_err();
        assert!(error.downcast_ref::<RecoveryRequiredError>().is_some());
        assert_eq!(
            std::fs::metadata(&outside_target)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_set_mode_leaf_symlink_swap_cannot_mutate_external_target_on_windows() {
        use std::os::windows::fs::symlink_file;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let target = temp.path().join("target.txt");
        let moved = temp.path().join("target.moved");
        let outside = temp.path().join("outside.txt");
        let probe = temp.path().join("symlink-probe");
        std::fs::write(&target, b"target").unwrap();
        std::fs::write(&outside, b"outside").unwrap();
        if symlink_file(&outside, &probe).is_err() {
            return;
        }
        std::fs::remove_file(&probe).unwrap();

        let hook_target = target.clone();
        let hook_moved = moved.clone();
        let hook_outside = outside.clone();
        let injector = Arc::new(HookInjector {
            point: FailurePoint::BeforeModeMutation { action: 0 },
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&hook_target, &hook_moved).unwrap();
                symlink_file(&hook_outside, &hook_target).unwrap();
            }))),
        });
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode-symlink-race-windows".to_string(),
                    actions: vec![TransactionAction::SetMode {
                        path: "target.txt".to_string(),
                        unix_mode: 0o755,
                    }],
                },
            )
            .unwrap_err();
        assert!(error.downcast_ref::<RecoveryRequiredError>().is_some());
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");

        std::fs::remove_file(&target).unwrap();
        std::fs::rename(&moved, &target).unwrap();
        kernel.recover(&guard, "mode-symlink-race-windows").unwrap();
        assert_eq!(std::fs::read(&outside).unwrap(), b"outside");
    }

    #[cfg(unix)]
    #[test]
    fn test_set_mode_revalidates_identity_on_the_mutated_handle() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let target = temp.path().join("mode.txt");
        let moved = temp.path().join("mode-original.txt");
        std::fs::write(&target, b"inside").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        let hook_target = target.clone();
        let hook_moved = moved.clone();
        let injector = Arc::new(HookInjector {
            point: FailurePoint::BeforeModeMutation { action: 0 },
            hook: Mutex::new(Some(Box::new(move || {
                std::fs::rename(&hook_target, &hook_moved).unwrap();
                std::fs::write(&hook_target, b"unexpected").unwrap();
                std::fs::set_permissions(&hook_target, std::fs::Permissions::from_mode(0o640))
                    .unwrap();
            }))),
        });
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector).unwrap();
        let (_lock, guard) = acquired_guard(&temp);

        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode-regular-race".to_string(),
                    actions: vec![TransactionAction::SetMode {
                        path: "mode.txt".to_string(),
                        unix_mode: 0o755,
                    }],
                },
            )
            .unwrap_err();
        assert!(error.downcast_ref::<RecoveryRequiredError>().is_some());
        assert_eq!(
            std::fs::metadata(&target).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            std::fs::metadata(&moved).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(windows)]
    #[test]
    fn test_write_file_reports_unix_mode_as_not_applicable_on_windows() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let kernel = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "mode".to_string(),
                    actions: vec![TransactionAction::WriteFile {
                        path: "script.cmd".to_string(),
                        contents: b"echo ok\r\n".to_vec(),
                        unix_mode: Some(0o755),
                    }],
                },
            )
            .unwrap();
        assert_eq!(
            std::fs::read(temp.path().join("script.cmd")).unwrap(),
            b"echo ok\r\n"
        );
    }

    #[test]
    fn test_all_forward_durability_boundaries_are_injectable() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(".jit")).unwrap();
        let injector = Arc::new(SelectedFailures::new([]));
        let kernel =
            FileTransactionKernel::with_injector(root_capability(&temp), injector.clone()).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "observed".to_string(),
                    actions: vec![write("observed.txt", b"bytes")],
                },
            )
            .unwrap();
        let observed = injector.observed();
        for expected in [
            FailurePoint::CreateInternalControl,
            FailurePoint::Stage { action: 0 },
            FailurePoint::SyncStage { action: 0 },
            FailurePoint::CreateJournal,
            FailurePoint::SyncJournal {
                decision: RecoveryState::Prepared,
            },
            FailurePoint::BeforeAction { action: 0 },
            FailurePoint::AfterParentOpen { action: 0 },
            FailurePoint::AfterPublish { action: 0 },
            FailurePoint::SyncTargetParent { action: 0 },
            FailurePoint::SyncJournal {
                decision: RecoveryState::Committed,
            },
            FailurePoint::CleanupTerminalResidue,
        ] {
            assert!(
                observed.contains(&expected),
                "missing injection point {expected:?}"
            );
        }
    }

    #[test]
    fn test_each_forward_failure_point_preserves_atomic_outcome() {
        for (id, failure, expected_final) in [
            ("fail-stage", FailurePoint::Stage { action: 0 }, false),
            (
                "fail-stage-sync",
                FailurePoint::SyncStage { action: 0 },
                false,
            ),
            (
                "fail-prepared-sync",
                FailurePoint::SyncJournal {
                    decision: RecoveryState::Prepared,
                },
                false,
            ),
            (
                "fail-before-action",
                FailurePoint::BeforeAction { action: 0 },
                false,
            ),
            (
                "fail-after-parent-open",
                FailurePoint::AfterParentOpen { action: 0 },
                false,
            ),
            (
                "fail-after-publish",
                FailurePoint::AfterPublish { action: 0 },
                false,
            ),
            (
                "fail-target-parent-sync",
                FailurePoint::SyncTargetParent { action: 0 },
                false,
            ),
            (
                "fail-committed-sync",
                FailurePoint::SyncJournal {
                    decision: RecoveryState::Committed,
                },
                true,
            ),
            ("fail-cleanup", FailurePoint::CleanupTerminalResidue, true),
        ] {
            let temp = TempDir::new().unwrap();
            std::fs::create_dir(temp.path().join(".jit")).unwrap();
            let failures = Arc::new(SelectedFailures::new([failure]));
            let kernel =
                FileTransactionKernel::with_injector(root_capability(&temp), failures).unwrap();
            let (_lock, guard) = acquired_guard(&temp);
            assert!(kernel
                .execute(
                    &guard,
                    FileTransactionPlan {
                        transaction_id: id.to_string(),
                        actions: vec![write("target.txt", b"final")],
                    },
                )
                .is_err());

            assert_eq!(
                temp.path().join("target.txt").exists(),
                expected_final,
                "unexpected target outcome after {id}"
            );
            if let Some(state) = kernel.recovery_state(id).unwrap() {
                assert!(matches!(
                    state,
                    RecoveryState::Prepared | RecoveryState::Committed
                ));
                kernel.recover(&guard, id).unwrap();
            }
            assert_eq!(kernel.recovery_state(id).unwrap(), None);
            if expected_final {
                assert_eq!(
                    std::fs::read(temp.path().join("target.txt")).unwrap(),
                    b"final"
                );
            } else {
                assert!(!temp.path().join("target.txt").exists());
            }
        }
    }

    #[test]
    fn test_control_and_journal_creation_failures_do_not_mutate_targets() {
        let fresh = TempDir::new().unwrap();
        let kernel = FileTransactionKernel::with_injector(
            root_capability(&fresh),
            Arc::new(SelectedFailures::new([FailurePoint::CreateExternalControl])),
        )
        .unwrap();
        let (_lock, guard) = acquired_guard(&fresh);
        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "external-create".to_string(),
                    actions: vec![write(".jit/index.json", b"{}")],
                },
            )
            .is_err());
        assert!(!fresh.path().join(".jit").exists());
        assert!(!fresh.path().join(BOOTSTRAP_DIR).exists());

        let existing = TempDir::new().unwrap();
        std::fs::create_dir(existing.path().join(".jit")).unwrap();
        let kernel = FileTransactionKernel::with_injector(
            root_capability(&existing),
            Arc::new(SelectedFailures::new([FailurePoint::CreateInternalControl])),
        )
        .unwrap();
        let (_lock, guard) = acquired_guard(&existing);
        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "internal-create".to_string(),
                    actions: vec![write("target.txt", b"bytes")],
                },
            )
            .is_err());
        assert!(!existing.path().join("target.txt").exists());
        assert!(!existing.path().join(".jit/tmp").exists());

        let kernel = FileTransactionKernel::with_injector(
            root_capability(&existing),
            Arc::new(SelectedFailures::new([FailurePoint::CreateJournal])),
        )
        .unwrap();
        assert!(kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "journal-create".to_string(),
                    actions: vec![write("target.txt", b"bytes")],
                },
            )
            .is_err());
        assert!(!existing.path().join("target.txt").exists());
        assert_eq!(kernel.recovery_state("journal-create").unwrap(), None);
    }

    #[test]
    fn test_bootstrap_occupant_and_cross_volume_are_typed() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join(BOOTSTRAP_DIR), b"unexpected").unwrap();
        let kernel = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "bootstrap-occupant".to_string(),
                    actions: vec![write(".jit/index.json", b"{}")],
                },
            )
            .unwrap_err();
        assert!(error
            .downcast_ref::<FileTransactionError>()
            .is_some_and(|error| matches!(
                error,
                FileTransactionError::UnexpectedBootstrapOccupant
            )));

        let temp = TempDir::new().unwrap();
        std::fs::create_dir(temp.path().join(BOOTSTRAP_DIR)).unwrap();
        std::fs::write(
            temp.path().join(BOOTSTRAP_DIR).join(PROTOCOL_MARKER),
            b"1\n",
        )
        .unwrap();
        std::fs::write(temp.path().join(BOOTSTRAP_DIR).join("foreign"), b"occupant").unwrap();
        let kernel = FileTransactionKernel::new(root_capability(&temp)).unwrap();
        let (_lock, guard) = acquired_guard(&temp);
        let error = kernel
            .execute(
                &guard,
                FileTransactionPlan {
                    transaction_id: "bootstrap-junk".to_string(),
                    actions: vec![write(".jit/index.json", b"{}")],
                },
            )
            .unwrap_err();
        assert!(error
            .downcast_ref::<FileTransactionError>()
            .is_some_and(|error| matches!(
                error,
                FileTransactionError::UnexpectedBootstrapOccupant
            )));

        #[cfg(unix)]
        let cross_volume_code = libc_exdev();
        #[cfg(windows)]
        let cross_volume_code = 17;
        #[cfg(not(any(unix, windows)))]
        let cross_volume_code = 1;
        let error = publication_error(std::io::Error::from_raw_os_error(cross_volume_code), "x");
        assert!(error
            .downcast_ref::<FileTransactionError>()
            .is_some_and(|error| matches!(error, FileTransactionError::CrossVolume { .. })));
    }

    #[cfg(unix)]
    #[test]
    fn test_owner_digest_is_stable_across_symlinked_spellings() {
        use crate::repository_state::RepositoryRootEvidence;
        use std::os::unix::fs::symlink;

        let real = TempDir::new().unwrap();
        let worktree = real.path().join("proj");
        std::fs::create_dir(&worktree).unwrap();
        let data = worktree.join(".jit"); // absent data root

        // A symlinked spelling of the same repository: link -> real.path(), so
        // link/proj and proj are the same directory reached two ways.
        let link = real.path().join("link");
        symlink(real.path(), &link).unwrap();
        let symlink_worktree = link.join("proj");
        let symlink_data = symlink_worktree.join(".jit");

        let layout = |worktree: &Path, data: &Path| {
            RepositoryLayout::new(
                RepositoryRootEvidence::new(worktree, "w", true),
                RepositoryRootEvidence::new(data, "d", true),
            )
            .unwrap()
        };

        // Same repository, two spellings -> one owner digest, so crash residue
        // written under one spelling is recognized as own under the other.
        assert_eq!(
            repository_owner_digest(&layout(&worktree, &data)),
            repository_owner_digest(&layout(&symlink_worktree, &symlink_data)),
        );

        // A genuinely distinct repository -> a distinct digest: never mistaken for
        // own, preserving the fail-safe direction.
        let other = real.path().join("other");
        std::fs::create_dir(&other).unwrap();
        assert_ne!(
            repository_owner_digest(&layout(&worktree, &data)),
            repository_owner_digest(&layout(&other, &other.join(".jit"))),
        );
    }

    #[test]
    fn test_validate_repository_journal_rejects_aliased_actions() {
        use crate::repository_state::RootRelativePath;

        let identity = EntryIdentity::for_bytes("7:99", b"linked").unwrap();
        let aliased = |root, relative: &str| RepositoryJournalAction {
            path: RepositoryJournalPath {
                root,
                relative: RootRelativePath::parse(relative).unwrap(),
            },
            owner: "owner".into(),
            expected: ExpectedPreimage::File {
                identity: identity.clone(),
                mode: FileMode::Regular,
            },
            final_identity: RepositoryFinalIdentity::File {
                identity: identity.clone(),
                mode: FileMode::Executable,
            },
            action: RepositoryJournalActionKind::SetMode {
                mode: FileMode::Executable,
            },
            progress: RepositoryActionProgress::Planned,
        };
        let journal = RepositoryTransactionJournal {
            version: REPOSITORY_JOURNAL_VERSION,
            transaction_id: "txn".into(),
            layout_digest: "layout".into(),
            owner_digest: "owner".into(),
            plan_hash: "plan".into(),
            data_root_was_absent: false,
            data_stage: None,
            data_stage_identity: None,
            decision: TransactionDecision::Prepared,
            actions: vec![
                aliased(RepositoryRootClass::Worktree, "shared"),
                aliased(RepositoryRootClass::Data, "shared"),
            ],
        };

        // Recovery validation rejects the aliased pair before any action runs.
        let error = validate_repository_journal(&journal, "txn").unwrap_err();
        assert!(error
            .downcast_ref::<FileTransactionError>()
            .is_some_and(|error| matches!(error, FileTransactionError::AliasedTarget { .. })));

        // Distinct recorded identities (distinct inodes) validate cleanly.
        let other = EntryIdentity::for_bytes("7:100", b"distinct").unwrap();
        let mut valid = journal.clone();
        valid.actions[1].expected = ExpectedPreimage::File {
            identity: other.clone(),
            mode: FileMode::Regular,
        };
        valid.actions[1].final_identity = RepositoryFinalIdentity::File {
            identity: other,
            mode: FileMode::Executable,
        };
        assert!(validate_repository_journal(&valid, "txn").is_ok());
    }
}
