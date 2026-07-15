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
    JournalAction, TransactionDecision, TransactionJournal, JOURNAL_FILE, JOURNAL_VERSION,
};
use super::transaction_recovery::{
    FailurePoint, FileTransactionError, NoTransactionFailures, RecoveryRequiredError,
    RecoveryState, TransactionFailureInjector,
};
use super::transaction_staging::{stage_bytes, sync_directory};
use anyhow::{Context, Result};
use cap_primitives::fs::FollowSymlinks;
#[cfg(unix)]
use cap_std::fs::MetadataExt as _;
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{ErrorKind, Read};
use std::path::{Component, Path};
use std::sync::Arc;

const BOOTSTRAP_DIR: &str = ".jit-bootstrap";
const PROTOCOL_MARKER: &str = "transaction-protocol-v1";

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

/// Capability-based transaction service rooted at an already-open repository
/// parent directory. No publication operation accepts an ambient path.
pub struct FileTransactionKernel {
    root: Dir,
    injector: Arc<dyn TransactionFailureInjector>,
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
        Ok(Self { root, injector })
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
        Ok(JournalAction { action, original })
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
                    } => set_mode(&parent, &leaf, Some(*unix_mode))
                        .with_context(|| format!("setting mode on {path}"))?,
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
        for (index, action) in journal.actions.iter().enumerate().rev() {
            self.injector
                .check(&FailurePoint::ReverseAction { action: index })?;
            reverse_action(&self.root, control, action)
                .with_context(|| format!("reversing durable transaction action {index}"))?;
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
}

struct ControlDirs {
    base: Dir,
    transactions: Dir,
    transaction: Dir,
    stages: Dir,
    backups: Dir,
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

fn reverse_action(root: &Dir, control: &ControlDirs, entry: &JournalAction) -> Result<()> {
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
            match inspect_leaf(&parent, &leaf)? {
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
            ensure_identity(
                &parent,
                &leaf,
                &TargetIdentity::File {
                    identity: final_identity.clone(),
                },
                path,
            )?;
            set_mode(&parent, &leaf, *original_mode)?;
            sync_directory(&parent)?;
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
    #[cfg(unix)]
    use cap_std::fs::PermissionsExt as _;

    let mut options = OpenOptions::new();
    options.read(true);
    options._cap_fs_ext_follow(FollowSymlinks::No);
    let mut file = parent.open_with(leaf, &options)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(FileTransactionError::UnsupportedTarget {
            path: leaf.to_string(),
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
        parent.set_permissions(leaf, cap_std::fs::Permissions::from_mode(mode & 0o7777))?;
        parent.open(leaf)?.sync_all()?;
    }
    #[cfg(not(unix))]
    let _ = (parent, leaf, mode);
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
}
