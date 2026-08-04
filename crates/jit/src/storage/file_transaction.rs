//! Durable, capability-confined publication of a set of files and directories.
//!
//! The caller owns serialization and passes a held repository write guard. This
//! module owns only storage mechanics: synchronized stages, a versioned journal,
//! identity-checked forward/reverse actions, durable decisions, and cleanup.

use super::atomic_write::rename_noreplace_cap;
use super::repo_lock::RepoWriteGuard;
use super::transaction_journal::{
    ActionTag, ControlName, RepositoryActionProgress, RepositoryFinalIdentity,
    RepositoryJournalAction, RepositoryJournalActionKind, RepositoryJournalPath,
    RepositoryTransactionJournal, TransactionDecision, JOURNAL_FILE, REPOSITORY_JOURNAL_VERSION,
};
use super::transaction_recovery::{
    FailurePoint, FileTransactionError, RecoveryRequiredError, RecoveryState,
    TransactionFailureInjector,
};
use super::transaction_staging::{stage_bytes, sync_directory};
use crate::repository_state::{
    EntryIdentity, ExpectedPreimage, FileMode, RepositoryAction, RepositoryDelta, RepositoryEntry,
    RepositoryLayout, RepositoryRootClass, VirtualPath,
};
use anyhow::Result;
use cap_primitives::fs::FollowSymlinks;
#[cfg(unix)]
use cap_std::fs::MetadataExt as _;
use cap_std::fs::{Dir, OpenOptions};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};
use std::io::{ErrorKind, Read};
use std::path::{Component, Path};
use std::sync::Arc;

const BOOTSTRAP_DIR: &str = ".jit-bootstrap";
const PROTOCOL_MARKER: &str = "transaction-protocol-v1";
/// Marker file distinguishing a worktree-side companion control directory (a
/// same-filesystem staging/backup area for the Worktree actions of an internal
/// transaction) from a genuine external transaction control. It carries the
/// owning internal transaction id. The companion lives under the already
/// permitted `.jit-bootstrap/transactions/{id}` literal and holds no journal.
const COMPANION_MARKER: &str = "companion";

/// Successful publication and cleanup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileTransactionOutcome {
    /// Hash of the normalized action set recorded by the journal.
    pub(crate) plan_hash: String,
}

/// Machine-local control location containing pending transaction journals.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::storage) enum TransactionControlLocation {
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
///
/// Storage-only by construction: repository publication is reachable to the rest
/// of the crate solely through `RepositoryStateStore` mutation sessions, so
/// `crate::commands` cannot name this type.
pub(in crate::storage) struct FileTransactionKernel {
    injector: Arc<dyn TransactionFailureInjector>,
    repository: RepositoryKernelRoots,
}

struct RepositoryKernelRoots {
    layout: RepositoryLayout,
    worktree: Dir,
    data: Option<Dir>,
    data_parent: Dir,
    data_leaf: String,
}

impl FileTransactionKernel {
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
            injector,
            repository: RepositoryKernelRoots {
                layout,
                worktree,
                data,
                data_parent,
                data_leaf,
            },
        })
    }

    pub(crate) fn pending_repository_transactions(
        &self,
        location: TransactionControlLocation,
    ) -> Result<Vec<String>> {
        let roots = self.repository_roots()?;
        repository_pending_ids(roots, location)
    }

    /// Reclaim marker-only or empty transaction-control parents left by a crash
    /// before a transaction id or durable journal was published.
    pub(crate) fn cleanup_empty_repository_control(
        &self,
        location: TransactionControlLocation,
    ) -> Result<()> {
        cleanup_empty_repository_control(self.repository_roots()?, location)
    }

    /// Remove worktree-side companions whose owning internal transaction is gone.
    /// Called after internal-journal recovery under the data-root guards.
    pub(crate) fn sweep_orphan_companions(&self, _guard: &RepoWriteGuard) -> Result<Vec<String>> {
        repository_check(&*self.injector, FailurePoint::RepositorySweepCompanions)?;
        sweep_orphan_companions(self.repository_roots()?)
    }

    pub(crate) fn recover_repository_transaction(
        &self,
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
            repository_check(
                &*self.injector,
                FailurePoint::RepositoryBeforeControlCleanup,
            )?;
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
        let transaction_identity = inspect_repository_root(&transaction)?
            .identity()
            .cloned()
            .ok_or_else(|| FileTransactionError::UnexpectedOccupant {
                path: format!("transaction control {id}"),
            })?;
        let control = ControlDirs {
            base,
            transactions,
            transaction,
            transaction_identity,
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
        Ok(&self.repository)
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
    transaction_identity: EntryIdentity,
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
    transaction_identity: EntryIdentity,
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

/// Stable owner identity of a transaction, derived only from the already
/// validated explicit layout. Layout discovery rejects aliased roots, so the
/// kernel must not reopen ambient paths to manufacture a second authority.
fn repository_owner_digest(layout: &RepositoryLayout) -> String {
    let mut hasher = Sha256::new();
    hasher.update(layout.worktree_root().as_os_str().as_encoded_bytes());
    hasher.update([0u8]);
    hasher.update(layout.data_root().as_os_str().as_encoded_bytes());
    format!("{:x}", hasher.finalize())
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
    normalize_cleanup_control_names(&transactions)?;
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

fn normalize_cleanup_control_names(transactions: &Dir) -> Result<()> {
    let mut quarantines = transactions
        .entries()?
        .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
        .collect::<std::io::Result<Vec<_>>>()?;
    quarantines.sort();
    for quarantine in quarantines {
        let Some(id) = quarantine.strip_prefix("cleanup-") else {
            continue;
        };
        validate_transaction_id(id)?;
        rename_noreplace_cap(transactions, &quarantine, transactions, id)
            .map_err(map_noreplace_error)?;
        sync_directory(transactions)?;
    }
    Ok(())
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
            // Creating the bootstrap directory and publishing its protocol marker
            // are separate durable operations. A crash between them leaves an
            // unambiguously empty directory, which recovery may safely reclaim.
            if bootstrap.entries()?.next().is_none() {
                return Ok(None);
            }
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

fn cleanup_empty_repository_control(
    roots: &RepositoryKernelRoots,
    location: TransactionControlLocation,
) -> Result<()> {
    match location {
        TransactionControlLocation::ExternalBootstrap => {
            if metadata_optional(&roots.worktree, BOOTSTRAP_DIR)?.is_none() {
                return Ok(());
            }
            let bootstrap = open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?;
            if bootstrap.entries()?.next().is_none() {
                drop(bootstrap);
                roots.worktree.remove_dir(BOOTSTRAP_DIR)?;
                sync_directory(&roots.worktree)?;
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
            roots.worktree.remove_dir(BOOTSTRAP_DIR)?;
            sync_directory(&roots.worktree)?;
            Ok(())
        }
        TransactionControlLocation::InternalRepository => {
            let Some(data) = &roots.data else {
                return Ok(());
            };
            if metadata_optional(data, "tmp")?.is_none() {
                return Ok(());
            }
            let tmp = open_existing_dir(data, "tmp")?;
            if let Some(metadata) = metadata_optional(&tmp, "transactions")? {
                if !metadata.is_dir() {
                    return Err(FileTransactionError::UnsupportedTarget {
                        path: "tmp/transactions".into(),
                    }
                    .into());
                }
                let transactions = open_existing_dir(&tmp, "transactions")?;
                if transactions.entries()?.next().is_some() {
                    return Ok(());
                }
                drop(transactions);
                tmp.remove_dir("transactions")?;
            }
            if tmp.entries()?.next().is_none() {
                drop(tmp);
                data.remove_dir("tmp")?;
                sync_directory(data)?;
            }
            Ok(())
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
    let transaction_identity = inspect_repository_root(&transaction)?
        .identity()
        .cloned()
        .ok_or_else(|| FileTransactionError::UnexpectedOccupant {
            path: format!("companion control {id}"),
        })?;
    Ok(CompanionDirs {
        transaction_identity,
        stages,
        backups,
    })
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
        Some(marker) if marker == owner => {
            let transaction_identity = inspect_repository_root(&transaction)?
                .identity()
                .cloned()
                .ok_or_else(|| FileTransactionError::UnexpectedOccupant {
                path: format!("companion control {id}"),
            })?;
            Ok(Some(CompanionDirs {
                transaction_identity,
                stages: open_existing_dir(&transaction, "stages")?,
                backups: open_existing_dir(&transaction, "backups")?,
            }))
        }
        Some(_) => Err(FileTransactionError::LayoutMismatch.into()),
        None => Ok(None),
    }
}

fn remove_companion_control_if_identity(
    roots: &RepositoryKernelRoots,
    id: &str,
    expected: &EntryIdentity,
) -> Result<()> {
    if metadata_optional(&roots.worktree, BOOTSTRAP_DIR)?.is_none() {
        return Ok(());
    }
    let bootstrap = open_existing_dir(&roots.worktree, BOOTSTRAP_DIR)?;
    if metadata_optional(&bootstrap, "transactions")?.is_none() {
        return Ok(());
    }
    let transactions = open_existing_dir(&bootstrap, "transactions")?;
    if metadata_optional(&transactions, id)?.is_some() {
        remove_owned_directory(&transactions, id, expected)?;
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
    normalize_cleanup_control_names(&transactions)?;
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
                let identity = inspect_repository_root(&transaction)?
                    .identity()
                    .cloned()
                    .ok_or_else(|| FileTransactionError::UnexpectedOccupant {
                        path: format!("companion control {id}"),
                    })?;
                orphans.push((id, identity));
            }
        }
    }
    drop(transactions);
    drop(bootstrap);
    orphans.sort_by(|left, right| left.0.cmp(&right.0));
    for (id, identity) in &orphans {
        remove_companion_control_if_identity(roots, id, identity)?;
    }
    Ok(orphans.into_iter().map(|(id, _)| id).collect())
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
                    let identity = set_mode_final_file_identity(expected, index)?;
                    (
                        RepositoryJournalActionKind::SetMode {
                            mode: *mode,
                            stage: ControlName::new(format!("mode-{index}"))
                                .map_err(anyhow::Error::msg)?,
                            backup: ControlName::new(format!("backup-{index}"))
                                .map_err(anyhow::Error::msg)?,
                        },
                        RepositoryFinalIdentity::File {
                            identity,
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
    repository_check(injector, FailurePoint::RepositoryBeforeControlCreation)?;
    let control =
        create_repository_control(roots, id, location, needs_worktree_companion, injector)?;
    repository_check(injector, FailurePoint::RepositoryCreateControl)?;
    // Journal construction runs after control exists but before a complete
    // journal exists. A `JournalActionMismatch` (or any construction error) here
    // must tear the created control down immediately through the control-only
    // teardown, which skips the journal-dependent data-stage steps (no stage has
    // been created yet, so they would be no-ops), rather than leaking control for
    // later recovery.
    let mut journal = match initial_repository_journal(roots, id, delta, plan_hash) {
        Ok(journal) => journal,
        Err(error) => {
            cleanup_repository_control_only(roots, control, location, id, injector)?;
            return Err(error);
        }
    };
    repository_check(injector, FailurePoint::RepositoryBeforeInitialJournal)?;
    write_repository_journal(&control.transaction, &journal)?;
    repository_check(injector, FailurePoint::RepositorySyncInitialJournal)?;
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
        cleanup_repository_control(roots, control, location, id, &journal, injector)?;
        return Err(error);
    }

    let published =
        publish_repository_actions(roots, &control, &mut journal, injector).and_then(|_| {
            repository_check(injector, FailurePoint::RepositoryBeforeCommitDecision)?;
            ensure_repository_commit_roots_are_live(roots, &journal)
        });
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
        rollback_repository_actions(roots, &control, &mut journal, injector)?;
        cleanup_repository_control(roots, control, location, id, &journal, injector)?;
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
    cleanup_repository_control(roots, control, location, id, &journal, injector)?;
    Ok(FileTransactionOutcome {
        plan_hash: plan_hash.to_string(),
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
        repository_check(injector, FailurePoint::RepositoryBeforeDataStageJournal)?;
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
        repository_check(
            injector,
            FailurePoint::RepositoryStageAction { action: index },
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
                    let stage = create_directory_stage_name(&journal.actions[index].action, index)?;
                    stages.create_dir(stage.as_str())?;
                    sync_directory(stages)?;
                    let staged = open_existing_dir(stages, stage.as_str())?;
                    journal.actions[index].final_identity =
                        repository_final_identity(&inspect_repository_root(&staged)?)?;
                    journal.actions[index].progress = RepositoryActionProgress::Prepared;
                }
                RepositoryAction::WriteFile { bytes, mode, .. } => {
                    let (stage, backup) =
                        write_file_control_names(&journal.actions[index].action, index)?;
                    stage_bytes(stages, stage.as_str(), bytes)?;
                    set_mode(stages, stage.as_str(), repository_unix_mode(*mode))?;
                    sync_directory(stages)?;
                    journal.actions[index].final_identity = repository_final_identity(
                        &inspect_repository_leaf(stages, stage.as_str())?,
                    )?;
                    // A replace prepares its rollback backup before publication,
                    // so an interrupted replacement can restore the exact original.
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
                            (injector, index),
                        )?;
                        RepositoryActionProgress::BackupReady
                    } else {
                        RepositoryActionProgress::Prepared
                    };
                }
                RepositoryAction::SetMode { mode, .. } => {
                    let (stage, backup) =
                        set_mode_control_names(&journal.actions[index].action, index)?;
                    let (parent, leaf) = open_parent(root, &relative, false)?;
                    let current = inspect_repository_leaf(&parent, &leaf)?;
                    ensure_repository_expected(&path, &journal.actions[index].expected, &current)?;
                    let RepositoryEntry::File { bytes, .. } = current else {
                        return Err(FileTransactionError::UnsupportedTarget {
                            path: format!("{path:?}"),
                        }
                        .into());
                    };
                    stage_bytes(stages, stage.as_str(), &bytes)?;
                    set_mode(stages, stage.as_str(), repository_unix_mode(*mode))?;
                    sync_directory(stages)?;
                    journal.actions[index].final_identity = repository_final_identity(
                        &inspect_repository_leaf(stages, stage.as_str())?,
                    )?;
                    prepare_repository_backup(
                        &parent,
                        &leaf,
                        backups,
                        &backup,
                        &journal.actions[index].expected,
                        &path,
                        (injector, index),
                    )?;
                    journal.actions[index].progress = RepositoryActionProgress::BackupReady;
                }
                RepositoryAction::DeleteFile { .. } => {
                    let backup = delete_file_backup_name(&journal.actions[index].action, index)?;
                    let (parent, leaf) = open_parent(root, &relative, false)?;
                    prepare_repository_backup(
                        &parent,
                        &leaf,
                        backups,
                        &backup,
                        &journal.actions[index].expected,
                        &path,
                        (injector, index),
                    )?;
                    journal.actions[index].progress = RepositoryActionProgress::BackupReady;
                }
            }
        }
        repository_check(
            injector,
            FailurePoint::RepositorySyncStage { action: index },
        )?;
        repository_check(
            injector,
            FailurePoint::RepositoryBeforePreparedJournal { action: index },
        )?;
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

/// Extract the file identity a `SetMode` action pins as its final identity. A
/// well-formed `SetMode` action carries a `File` preimage (enforced by
/// `validate_action` at `RepositoryDelta::new`); a non-file preimage means the
/// semantic delta and the journal action being constructed drifted, so
/// extraction fails with a typed [`FileTransactionError::JournalActionMismatch`]
/// instead of panicking mid-construction. The mismatch propagates into the
/// created-control teardown, so no partial write is left behind.
fn set_mode_final_file_identity(
    expected: &ExpectedPreimage,
    index: usize,
) -> Result<EntryIdentity, FileTransactionError> {
    match expected {
        ExpectedPreimage::File { identity, .. } => Ok(identity.clone()),
        _ => Err(FileTransactionError::JournalActionMismatch {
            index,
            expected: ActionTag::SetMode,
            found: action_tag_of_preimage(expected),
        }),
    }
}

/// Classify a preimage's occupant to the action tag that natively targets that
/// occupant category, for diagnostics when a required preimage invariant is
/// violated: a directory occupant is a `CreateDirectory` target, an absent
/// occupant is a `WriteFile` (create) target, and a foreign occupant would have
/// to be removed. Defensive only — `validate_action` guarantees a `SetMode`
/// preimage is `File`, so this classifies only drifted journal/delta input.
fn action_tag_of_preimage(preimage: &ExpectedPreimage) -> ActionTag {
    match preimage {
        ExpectedPreimage::File { .. } => ActionTag::SetMode,
        ExpectedPreimage::Directory { .. } => ActionTag::CreateDirectory,
        ExpectedPreimage::Absent => ActionTag::WriteFile,
        ExpectedPreimage::Symlink { .. } | ExpectedPreimage::Unsupported { .. } => {
            ActionTag::DeleteFile
        }
    }
}

/// Extract the staging control-name a `CreateDirectory` journal action pins.
/// Total: a journal action whose kind does not match the semantic action at
/// `index` aborts with [`FileTransactionError::JournalActionMismatch`] instead
/// of panicking.
fn create_directory_stage_name(
    kind: &RepositoryJournalActionKind,
    index: usize,
) -> Result<ControlName, FileTransactionError> {
    match kind {
        RepositoryJournalActionKind::CreateDirectory { stage, .. } => Ok(stage.clone()),
        _ => Err(FileTransactionError::JournalActionMismatch {
            index,
            expected: ActionTag::CreateDirectory,
            found: kind.tag(),
        }),
    }
}

/// Extract the stage and backup control-names a `WriteFile` journal action pins.
/// Total: see [`create_directory_stage_name`].
fn write_file_control_names(
    kind: &RepositoryJournalActionKind,
    index: usize,
) -> Result<(ControlName, ControlName), FileTransactionError> {
    match kind {
        RepositoryJournalActionKind::WriteFile { stage, backup, .. } => {
            Ok((stage.clone(), backup.clone()))
        }
        _ => Err(FileTransactionError::JournalActionMismatch {
            index,
            expected: ActionTag::WriteFile,
            found: kind.tag(),
        }),
    }
}

/// Extract the stage and backup control-names a `SetMode` journal action pins.
/// Total: see [`create_directory_stage_name`].
fn set_mode_control_names(
    kind: &RepositoryJournalActionKind,
    index: usize,
) -> Result<(ControlName, ControlName), FileTransactionError> {
    match kind {
        RepositoryJournalActionKind::SetMode { stage, backup, .. } => {
            Ok((stage.clone(), backup.clone()))
        }
        _ => Err(FileTransactionError::JournalActionMismatch {
            index,
            expected: ActionTag::SetMode,
            found: kind.tag(),
        }),
    }
}

/// Extract the backup control-name a `DeleteFile` journal action pins. Total:
/// see [`create_directory_stage_name`].
fn delete_file_backup_name(
    kind: &RepositoryJournalActionKind,
    index: usize,
) -> Result<ControlName, FileTransactionError> {
    match kind {
        RepositoryJournalActionKind::DeleteFile { backup } => Ok(backup.clone()),
        _ => Err(FileTransactionError::JournalActionMismatch {
            index,
            expected: ActionTag::DeleteFile,
            found: kind.tag(),
        }),
    }
}

/// Create and synchronize the rollback backup of a replace/delete target during
/// preparation. The live target is verified against the recorded preimage, then
/// copied into the backup area and reverified there, so a subsequent
/// publication converges to the exact original file even if a non-cooperating
/// writer swaps the target afterward.
fn prepare_repository_backup(
    parent: &Dir,
    leaf: &str,
    backups: &Dir,
    backup: &ControlName,
    expected: &ExpectedPreimage,
    path: &VirtualPath,
    failure: (&dyn TransactionFailureInjector, usize),
) -> Result<()> {
    let (injector, index) = failure;
    let current = inspect_repository_leaf(parent, leaf)?;
    ensure_repository_expected(path, expected, &current)?;
    let RepositoryEntry::File { bytes, mode, .. } = current else {
        return Err(FileTransactionError::UnsupportedTarget {
            path: format!("{path:?}"),
        }
        .into());
    };
    stage_bytes(backups, backup.as_str(), &bytes).map_err(|error| {
        if error.kind() == ErrorKind::AlreadyExists {
            FileTransactionError::UnexpectedOccupant {
                path: format!("backup {}", backup.as_str()),
            }
            .into()
        } else {
            anyhow::Error::new(error)
        }
    })?;
    set_mode(backups, backup.as_str(), repository_unix_mode(mode))?;
    let saved = inspect_repository_leaf(backups, backup.as_str())?;
    if !repository_file_content_matches_expected(expected, &saved) {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{path:?}"),
        }
        .into());
    }
    sync_directory(backups)?;
    repository_check(
        injector,
        FailurePoint::RepositorySyncBackup { action: index },
    )?;
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
        publish_repository_action(roots, control, journal, index, injector)?;
        journal.actions[index].progress = RepositoryActionProgress::Published;
        repository_check(
            injector,
            FailurePoint::RepositoryBeforePublishedJournal { action: index },
        )?;
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
        // The injector boundary is also the last point at which an external
        // writer can deterministically replace the absent root's parent after
        // session-level revalidation. Tie the held parent capability back to
        // its live ambient name immediately before the irreversible rename;
        // the post-publication check detects a later replacement.
        ensure_repository_data_parent_is_live(roots)?;
        repository_check(
            injector,
            FailurePoint::RepositoryAfterDataParentBindingCheck,
        )?;
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
        ensure_repository_published_data_root_is_live(roots, journal)?;
    }
    Ok(())
}

fn publish_repository_action(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &RepositoryTransactionJournal,
    index: usize,
    injector: &dyn TransactionFailureInjector,
) -> Result<()> {
    let path = journal_virtual_path(roots, &journal.actions[index].path)?;
    let root = repository_live_root(roots, path.root_class())?;
    let relative = path.relative().as_path().to_string_lossy();
    let (parent, leaf) = open_parent(root, &relative, false)?;
    let current = inspect_repository_leaf(&parent, &leaf)?;
    let expected = &journal.actions[index].expected;
    ensure_repository_expected(&path, expected, &current)?;
    repository_check(
        injector,
        FailurePoint::RepositoryBeforeTargetMutation { action: index },
    )?;
    // Session apply revalidates roots before entering the kernel, but the
    // failure-injection boundary above can model a whole-root replacement in
    // the remaining window. Rebind the held capability to the live root name
    // before mutation; the final check detects a later replacement.
    ensure_repository_mutation_root_is_live(roots, path.root_class())?;
    repository_check(
        injector,
        FailurePoint::RepositoryAfterRootBindingCheck { action: index },
    )?;
    let stages = stage_authority(control, path.root_class());
    match &journal.actions[index].action {
        RepositoryJournalActionKind::CreateDirectory { stage, .. } => {
            rename_noreplace_cap(stages, stage.as_str(), &parent, &leaf)
                .map_err(map_noreplace_error)?;
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::WriteFile { stage, .. } => {
            if matches!(expected, ExpectedPreimage::File { .. }) {
                // Move the live name aside with no replacement, verify the name
                // that was actually moved, then publish the stage with another
                // no-replace operation. A raced-in bystander is restored rather
                // than overwritten.
                move_repository_file_aside_if_identity(
                    &parent, &leaf, expected, &path, control, index,
                )?;
                rename_noreplace_cap(stages, stage.as_str(), &parent, &leaf)
                    .map_err(map_noreplace_error)?;
            } else {
                stages
                    .hard_link(stage.as_str(), &parent, &leaf)
                    .map_err(map_noreplace_error)?;
            }
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::SetMode { stage, .. } => {
            move_repository_file_aside_if_identity(
                &parent, &leaf, expected, &path, control, index,
            )?;
            rename_noreplace_cap(stages, stage.as_str(), &parent, &leaf)
                .map_err(map_noreplace_error)?;
            sync_directory(&parent)?;
        }
        RepositoryJournalActionKind::DeleteFile { .. } => {
            // Move the live name into transaction control, then verify what the
            // atomic rename actually removed. A raced-in occupant is restored
            // instead of being unlinked.
            remove_repository_file_if_identity(
                &parent, &leaf, expected, &path, control, index, injector,
            )?;
            sync_directory(&parent)?;
        }
    }
    repository_check(
        injector,
        FailurePoint::RepositorySyncTargetParent { action: index },
    )?;
    let actual = inspect_repository_target(roots, &path, None)?;
    repository_check(
        injector,
        FailurePoint::RepositoryVerifyFinalIdentity { action: index },
    )?;
    ensure_repository_final(&path, &journal.actions[index].final_identity, &actual)?;
    ensure_repository_mutation_root_is_live(roots, path.root_class())
}

fn move_repository_file_aside_if_identity(
    parent: &Dir,
    leaf: &str,
    expected: &ExpectedPreimage,
    path: &VirtualPath,
    control: &ControlDirs,
    index: usize,
) -> Result<()> {
    let backups = backup_authority(control, path.root_class());
    let aside = ControlName::new(format!("replaced-{index}")).map_err(anyhow::Error::msg)?;
    rename_noreplace_cap(parent, leaf, backups, aside.as_str()).map_err(map_noreplace_error)?;
    let moved = inspect_repository_leaf(backups, aside.as_str())?;
    if repository_matches_expected(expected, &moved) {
        sync_directory(backups)?;
        return Ok(());
    }
    rename_noreplace_cap(backups, aside.as_str(), parent, leaf).map_err(map_noreplace_error)?;
    sync_directory(parent)?;
    sync_directory(backups)?;
    Err(FileTransactionError::UnexpectedOccupant {
        path: format!("{path:?}"),
    }
    .into())
}

/// Atomically rename `leaf` into transaction control and verify that the removed
/// name still has the expected identity. A raced-in occupant is renamed back.
fn remove_repository_file_if_identity(
    parent: &Dir,
    leaf: &str,
    expected: &ExpectedPreimage,
    path: &VirtualPath,
    control: &ControlDirs,
    index: usize,
    injector: &dyn TransactionFailureInjector,
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
    repository_check(
        injector,
        FailurePoint::RepositoryBeforeDeleteRename { action: index },
    )?;
    let backups = backup_authority(control, path.root_class());
    let aside = ControlName::new(format!("deleted-{index}")).map_err(anyhow::Error::msg)?;
    rename_noreplace_cap(parent, leaf, backups, aside.as_str()).map_err(map_noreplace_error)?;
    let removed = inspect_repository_leaf(backups, aside.as_str())?;
    if !repository_matches_expected(expected, &removed) {
        // The name changed after the identity check. Put that exact occupant back
        // without replacing anything a concurrent writer may have created since.
        rename_noreplace_cap(backups, aside.as_str(), parent, leaf).map_err(map_noreplace_error)?;
        sync_directory(parent)?;
        sync_directory(backups)?;
        return Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{path:?}"),
        }
        .into());
    }
    sync_directory(backups)?;
    Ok(())
}

fn rollback_repository_actions(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    journal: &mut RepositoryTransactionJournal,
    injector: &dyn TransactionFailureInjector,
) -> Result<()> {
    for index in (0..journal.actions.len()).rev() {
        if journal.data_root_was_absent
            && journal.actions[index].path.root == RepositoryRootClass::Data
        {
            continue;
        }
        repository_check(
            injector,
            FailurePoint::RepositoryBeforeReverseAction { action: index },
        )?;
        rollback_repository_action(roots, control, &journal.actions[index], index)?;
        journal.actions[index].progress = RepositoryActionProgress::Restored;
        write_repository_journal(&control.transaction, journal)?;
        repository_check(
            injector,
            FailurePoint::RepositorySyncRollbackJournal { action: index },
        )?;
    }
    repository_check(injector, FailurePoint::RepositoryBeforeStageCleanup)?;
    remove_data_stage_if_owned(roots, journal)?;
    repository_check(injector, FailurePoint::RepositoryBeforeRollbackDecision)?;
    journal.decision = TransactionDecision::RolledBack;
    write_repository_journal(&control.transaction, journal)?;
    Ok(())
}

fn rollback_repository_action(
    roots: &RepositoryKernelRoots,
    control: &ControlDirs,
    action: &RepositoryJournalAction,
    index: usize,
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
    if matches!(
        action.action,
        RepositoryJournalActionKind::DeleteFile { .. }
    ) {
        let aside = ControlName::new(format!("deleted-{index}")).map_err(anyhow::Error::msg)?;
        let removed = inspect_repository_leaf(backups, aside.as_str())?;
        if !matches!(removed, RepositoryEntry::Absent)
            && !repository_matches_expected(&action.expected, &removed)
        {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: format!("delete aside {}", aside.as_str()),
            }
            .into());
        }
    }
    if repository_file_content_matches_expected(&action.expected, &current) {
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
        RepositoryJournalActionKind::SetMode { backup, .. } => {
            if !matches!(current, RepositoryEntry::Absent) {
                ensure_repository_final(&path, &action.final_identity, &current)?;
                parent.remove_file(&leaf)?;
                sync_directory(&parent)?;
            }
            restore_verified_backup(&parent, &leaf, backups, backup, &action.expected)?;
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
    if repository_file_content_matches_expected(&action.expected, &restored) {
        Ok(())
    } else {
        Err(FileTransactionError::UnexpectedOccupant {
            path: format!("{path:?}"),
        }
        .into())
    }
}

fn restore_verified_backup(
    parent: &Dir,
    leaf: &str,
    backups: &Dir,
    backup: &ControlName,
    expected: &ExpectedPreimage,
) -> Result<()> {
    let saved = inspect_repository_leaf(backups, backup.as_str())?;
    if !repository_file_content_matches_expected(expected, &saved) {
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
        | RepositoryJournalActionKind::SetMode { backup, .. }
        | RepositoryJournalActionKind::DeleteFile { backup } => Some(backup),
        _ => None,
    };
    if let Some(backup) = backup {
        let backups = backup_authority(control, action.path.root);
        let saved = inspect_repository_leaf(backups, backup.as_str())?;
        if !matches!(saved, RepositoryEntry::Absent) {
            if !repository_file_content_matches_expected(&action.expected, &saved) {
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
            repository_check(injector, FailurePoint::RepositoryBeforeCommitDecision)?;
            ensure_repository_commit_roots_are_live(roots, &journal)?;
            journal.decision = TransactionDecision::Committed;
            write_repository_journal(&control.transaction, &journal)?;
        }
        TransactionDecision::Prepared => {
            rollback_repository_actions(roots, &control, &mut journal, injector)?;
        }
        TransactionDecision::Committed => verify_repository_final_actions(roots, &journal)?,
        TransactionDecision::RolledBack => {
            verify_repository_restored_actions(roots, &control, &journal)?
        }
    }
    repository_check(injector, FailurePoint::RepositoryCleanup)?;
    cleanup_repository_control(roots, control, location, id, &journal, injector)?;
    Ok(FileTransactionOutcome {
        plan_hash: journal.plan_hash,
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
    if journal.data_root_was_absent {
        if journal.data_stage.is_none() && journal.data_stage_identity.is_some() {
            return Err(FileTransactionError::LayoutMismatch.into());
        }
    } else if journal.data_stage.is_some() || journal.data_stage_identity.is_some() {
        return Err(FileTransactionError::LayoutMismatch.into());
    }
    let mut seen = HashSet::new();
    let mut identities: BTreeMap<&EntryIdentity, ()> = BTreeMap::new();
    let mut control_names = HashSet::new();
    for action in &journal.actions {
        if action.owner.is_empty() || action.owner.chars().any(char::is_control) {
            return Err(FileTransactionError::LayoutMismatch.into());
        }
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
            identity
                .validate()
                .map_err(|_| FileTransactionError::LayoutMismatch)?;
            if identities.insert(identity, ()).is_some() {
                return Err(FileTransactionError::AliasedTarget {
                    path: format!("{:?}", action.path.relative),
                }
                .into());
            }
        }
        match &action.final_identity {
            RepositoryFinalIdentity::Absent => {}
            RepositoryFinalIdentity::Directory { identity, .. }
            | RepositoryFinalIdentity::File { identity, .. } => identity
                .validate()
                .map_err(|_| FileTransactionError::LayoutMismatch)?,
        }
        let valid_shape = match (&action.action, &action.expected, &action.final_identity) {
            (
                RepositoryJournalActionKind::CreateDirectory { mode, stage },
                ExpectedPreimage::Absent,
                RepositoryFinalIdentity::Directory {
                    mode: final_mode, ..
                },
            ) => *mode == *final_mode && control_names.insert((action.path.root, stage.as_str())),
            (
                RepositoryJournalActionKind::WriteFile {
                    mode,
                    stage,
                    backup,
                },
                ExpectedPreimage::Absent | ExpectedPreimage::File { .. },
                RepositoryFinalIdentity::File {
                    mode: final_mode, ..
                },
            ) => {
                *mode == *final_mode
                    && stage != backup
                    && control_names.insert((action.path.root, stage.as_str()))
                    && control_names.insert((action.path.root, backup.as_str()))
            }
            (
                RepositoryJournalActionKind::SetMode {
                    mode,
                    stage,
                    backup,
                },
                ExpectedPreimage::File { .. },
                RepositoryFinalIdentity::File {
                    mode: final_mode, ..
                },
            ) => {
                *mode == *final_mode
                    && stage != backup
                    && control_names.insert((action.path.root, stage.as_str()))
                    && control_names.insert((action.path.root, backup.as_str()))
            }
            (
                RepositoryJournalActionKind::DeleteFile { backup },
                ExpectedPreimage::File { .. },
                RepositoryFinalIdentity::Absent,
            ) => control_names.insert((action.path.root, backup.as_str())),
            _ => false,
        };
        if !valid_shape
            || (journal.data_root_was_absent
                && action.path.root == RepositoryRootClass::Data
                && !matches!(
                    (&action.action, &action.expected),
                    (
                        RepositoryJournalActionKind::CreateDirectory { .. },
                        ExpectedPreimage::Absent
                    ) | (
                        RepositoryJournalActionKind::WriteFile { .. },
                        ExpectedPreimage::Absent
                    )
                ))
        {
            return Err(FileTransactionError::LayoutMismatch.into());
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
    let expected = inspect_repository_root(&transaction)?
        .identity()
        .cloned()
        .ok_or_else(|| FileTransactionError::UnexpectedOccupant {
            path: format!("transaction control {id}"),
        })?;
    drop(transaction);
    remove_owned_directory(&transactions, id, &expected)?;
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
        if !repository_file_content_matches_expected(&action.expected, &actual) {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: format!("{path:?}"),
            }
            .into());
        }
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
    let stage = journal
        .data_stage
        .as_ref()
        .ok_or(FileTransactionError::LayoutMismatch)?;
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
    let quarantine = format!("cleanup-{}", stage.as_str());
    let stage_present = match roots.data_parent.symlink_metadata(stage.as_str()) {
        Ok(metadata) if metadata.is_dir() && !metadata.is_symlink() => true,
        Ok(_) => {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: stage.as_str().to_string(),
            }
            .into())
        }
        Err(error) if error.kind() == ErrorKind::NotFound => false,
        Err(error) => return Err(error.into()),
    };
    let quarantine_present = metadata_optional(&roots.data_parent, &quarantine)?.is_some();
    if !stage_present && !quarantine_present {
        return Ok(());
    }
    let expected = journal.data_stage_identity.as_ref().ok_or_else(|| {
        FileTransactionError::UnexpectedOccupant {
            path: stage.as_str().to_string(),
        }
    })?;
    remove_owned_directory(&roots.data_parent, stage.as_str(), expected)
}

/// Move a directory name into a deterministic quarantine with no replacement,
/// verify the moved inode, and only then recurse below the quarantined name.
/// This prevents a raced-in directory from being traversed and deleted.
fn remove_owned_directory(parent: &Dir, name: &str, expected: &EntryIdentity) -> Result<()> {
    let quarantine = ControlName::new(format!("cleanup-{name}")).map_err(anyhow::Error::msg)?;
    if metadata_optional(parent, quarantine.as_str())?.is_some() {
        if metadata_optional(parent, name)?.is_some() {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: quarantine.as_str().to_string(),
            }
            .into());
        }
        let moved = open_existing_dir(parent, quarantine.as_str())?;
        let actual = inspect_repository_root(&moved)?;
        if actual.identity() != Some(expected) {
            return Err(FileTransactionError::UnexpectedOccupant {
                path: quarantine.as_str().to_string(),
            }
            .into());
        }
        remove_verified_directory(parent, moved)?;
        return Ok(());
    }
    rename_noreplace_cap(parent, name, parent, quarantine.as_str()).map_err(map_noreplace_error)?;
    let moved = open_existing_dir(parent, quarantine.as_str())?;
    let actual = inspect_repository_root(&moved)?;
    if actual.identity() != Some(expected) {
        drop(moved);
        rename_noreplace_cap(parent, quarantine.as_str(), parent, name)
            .map_err(map_noreplace_error)?;
        sync_directory(parent)?;
        return Err(FileTransactionError::UnexpectedOccupant {
            path: name.to_string(),
        }
        .into());
    }
    remove_verified_directory(parent, moved)?;
    Ok(())
}

fn remove_verified_directory(parent: &Dir, directory: Dir) -> Result<()> {
    directory.remove_open_dir_all()?;
    sync_directory(parent)?;
    Ok(())
}

fn cleanup_repository_control(
    roots: &RepositoryKernelRoots,
    control: ControlDirs,
    location: TransactionControlLocation,
    id: &str,
    journal: &RepositoryTransactionJournal,
    injector: &dyn TransactionFailureInjector,
) -> Result<()> {
    if !data_stage_was_published(roots, journal)? {
        repository_check(injector, FailurePoint::RepositoryBeforeStageCleanup)?;
        remove_data_stage_if_owned(roots, journal)?;
    }
    cleanup_repository_control_only(roots, control, location, id, injector)
}

/// Tear down a created transaction control (companion, then primary, then empty
/// protocol roots), skipping the journal-dependent data-stage steps. Called
/// directly when a mismatch or other error aborts before any data stage exists
/// (the data-stage steps would be provable no-ops); `cleanup_repository_control`
/// wraps it with those steps for the post-prepare paths.
fn cleanup_repository_control_only(
    roots: &RepositoryKernelRoots,
    control: ControlDirs,
    location: TransactionControlLocation,
    id: &str,
    injector: &dyn TransactionFailureInjector,
) -> Result<()> {
    let ControlDirs {
        base,
        transactions,
        transaction,
        transaction_identity,
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
        repository_check(injector, FailurePoint::RepositoryBeforeCompanionCleanup)?;
        let companion_identity = companion.transaction_identity.clone();
        drop(companion);
        remove_companion_control_if_identity(roots, id, &companion_identity)?;
    }
    repository_check(injector, FailurePoint::RepositoryBeforeControlCleanup)?;
    remove_owned_directory(&transactions, id, &transaction_identity)?;
    drop(transactions);
    drop(base);
    cleanup_empty_repository_control(roots, location)
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

fn ensure_repository_mutation_root_is_live(
    roots: &RepositoryKernelRoots,
    class: RepositoryRootClass,
) -> Result<()> {
    match class {
        RepositoryRootClass::Worktree => ensure_ambient_directory_matches_capability(
            roots.layout.worktree_root(),
            &roots.worktree,
        ),
        RepositoryRootClass::Data => {
            ensure_repository_data_parent_is_live(roots)?;
            let data =
                roots
                    .data
                    .as_ref()
                    .ok_or_else(|| FileTransactionError::UnsupportedFilesystem {
                        operation: "data root is not published".into(),
                    })?;
            let live = roots
                .data_parent
                .symlink_metadata(&roots.data_leaf)
                .map_err(|error| {
                    if error.kind() == ErrorKind::NotFound {
                        anyhow::Error::new(FileTransactionError::UnexpectedOccupant {
                            path: roots.layout.data_root().display().to_string(),
                        })
                    } else {
                        anyhow::Error::new(error)
                    }
                })?;
            if live.is_symlink()
                || !live.is_dir()
                || capability_metadata_identity(&live)?
                    != capability_metadata_identity(&data.dir_metadata()?)?
            {
                return Err(FileTransactionError::UnexpectedOccupant {
                    path: roots.layout.data_root().display().to_string(),
                }
                .into());
            }
            Ok(())
        }
    }
}

fn ensure_repository_data_parent_is_live(roots: &RepositoryKernelRoots) -> Result<()> {
    let parent =
        roots
            .layout
            .data_root()
            .parent()
            .ok_or_else(|| FileTransactionError::InvalidPath {
                path: roots.layout.data_root().display().to_string(),
            })?;
    ensure_ambient_directory_matches_capability(parent, &roots.data_parent)
}

fn ensure_repository_commit_roots_are_live(
    roots: &RepositoryKernelRoots,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    if journal
        .actions
        .iter()
        .any(|action| action.path.root == RepositoryRootClass::Worktree)
    {
        ensure_repository_mutation_root_is_live(roots, RepositoryRootClass::Worktree)?;
    }
    if journal
        .actions
        .iter()
        .any(|action| action.path.root == RepositoryRootClass::Data)
    {
        if journal.data_root_was_absent {
            ensure_repository_published_data_root_is_live(roots, journal)?;
        } else {
            ensure_repository_mutation_root_is_live(roots, RepositoryRootClass::Data)?;
        }
    }
    Ok(())
}

fn ensure_repository_published_data_root_is_live(
    roots: &RepositoryKernelRoots,
    journal: &RepositoryTransactionJournal,
) -> Result<()> {
    ensure_repository_data_parent_is_live(roots)?;
    let expected = journal.data_stage_identity.as_ref().ok_or_else(|| {
        FileTransactionError::UnexpectedOccupant {
            path: roots.layout.data_root().display().to_string(),
        }
    })?;
    let actual = inspect_repository_leaf(&roots.data_parent, &roots.data_leaf)?;
    if actual.identity() != Some(expected) {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: roots.layout.data_root().display().to_string(),
        }
        .into());
    }
    Ok(())
}

fn ensure_ambient_directory_matches_capability(path: &Path, directory: &Dir) -> Result<()> {
    let live = std::fs::symlink_metadata(path)?;
    if !live.is_dir()
        || ambient_metadata_identity(path)?
            != capability_metadata_identity(&directory.dir_metadata()?)?
    {
        return Err(FileTransactionError::UnexpectedOccupant {
            path: path.display().to_string(),
        }
        .into());
    }
    Ok(())
}

#[cfg(unix)]
fn capability_metadata_identity(metadata: &cap_std::fs::Metadata) -> Result<String> {
    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn capability_metadata_identity(metadata: &cap_std::fs::Metadata) -> Result<String> {
    let volume =
        cap_primitives::fs::_WindowsByHandle::volume_serial_number(metadata).ok_or_else(|| {
            FileTransactionError::UnsupportedFilesystem {
                operation: "Windows volume identity".into(),
            }
        })?;
    let index = cap_primitives::fs::_WindowsByHandle::file_index(metadata).ok_or_else(|| {
        FileTransactionError::UnsupportedFilesystem {
            operation: "Windows file identity".into(),
        }
    })?;
    Ok(format!("{volume}:{index}"))
}

#[cfg(not(any(unix, windows)))]
fn capability_metadata_identity(metadata: &cap_std::fs::Metadata) -> Result<String> {
    Ok(format!(
        "{}:{}",
        metadata.len(),
        metadata.permissions().readonly()
    ))
}

#[cfg(unix)]
fn ambient_metadata_identity(path: &Path) -> Result<String> {
    use std::os::unix::fs::MetadataExt as _;
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(format!("{}:{}", metadata.dev(), metadata.ino()))
}

/// A `std::fs::Metadata`-only conversion leaves `volume_serial_number`/
/// `file_index` `None` on Windows (they come from the open handle, not the
/// stat), so this opens the path itself: `FILE_FLAG_OPEN_REPARSE_POINT`
/// preserves the no-follow semantics the caller relies on, and
/// `FILE_FLAG_BACKUP_SEMANTICS` is required to open a directory handle at all.
#[cfg(windows)]
fn ambient_metadata_identity(path: &Path) -> Result<String> {
    use std::os::windows::fs::OpenOptionsExt as _;

    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    let metadata = cap_primitives::fs::Metadata::from_file(&file)?;
    let volume =
        cap_primitives::fs::_WindowsByHandle::volume_serial_number(&metadata).ok_or_else(|| {
            FileTransactionError::UnsupportedFilesystem {
                operation: "Windows volume identity".into(),
            }
        })?;
    let index = cap_primitives::fs::_WindowsByHandle::file_index(&metadata).ok_or_else(|| {
        FileTransactionError::UnsupportedFilesystem {
            operation: "Windows file identity".into(),
        }
    })?;
    Ok(format!("{volume}:{index}"))
}

#[cfg(not(any(unix, windows)))]
fn ambient_metadata_identity(path: &Path) -> Result<String> {
    let metadata = std::fs::symlink_metadata(path)?;
    Ok(format!(
        "{}:{}",
        metadata.len(),
        metadata.permissions().readonly()
    ))
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
    let object = capability_metadata_identity(metadata)?;
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

fn repository_file_content_matches_expected(
    expected: &ExpectedPreimage,
    actual: &RepositoryEntry,
) -> bool {
    match (expected, actual) {
        (
            ExpectedPreimage::File {
                identity: expected_identity,
                mode: expected_mode,
            },
            RepositoryEntry::File {
                identity: actual_identity,
                mode: actual_mode,
                ..
            },
        ) => {
            expected_identity.sha256() == actual_identity.sha256()
                && expected_identity.byte_size() == actual_identity.byte_size()
                && expected_mode == actual_mode
        }
        _ => repository_matches_expected(expected, actual),
    }
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
    let transaction_identity = inspect_repository_root(&transaction)?
        .identity()
        .cloned()
        .ok_or_else(|| FileTransactionError::UnexpectedOccupant {
            path: format!("transaction control {id}"),
        })?;
    let stages = open_or_create_dir(&transaction, "stages")?;
    let backups = open_or_create_dir(&transaction, "backups")?;
    Ok(ControlDirs {
        base,
        transactions,
        transaction,
        transaction_identity,
        stages,
        backups,
        companion: None,
    })
}

fn validate_transaction_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.starts_with("cleanup-")
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
    options._cap_fs_ext_maybe_dir(true);
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
    use tempfile::TempDir;

    #[test]
    fn test_verified_directory_cleanup_keeps_raced_name_occupant() {
        let temp = TempDir::new().unwrap();
        let parent = Dir::open_ambient_dir(temp.path(), cap_std::ambient_authority()).unwrap();
        let parent = sync_capable_directory(&parent).unwrap();
        parent.create_dir("cleanup-owned").unwrap();
        let owned = open_existing_dir(&parent, "cleanup-owned").unwrap();
        stage_bytes(&owned, "owned", b"owned").unwrap();

        parent
            .rename("cleanup-owned", &parent, "moved-owned")
            .unwrap();
        parent.create_dir("cleanup-owned").unwrap();
        let bystander = open_existing_dir(&parent, "cleanup-owned").unwrap();
        stage_bytes(&bystander, "keep", b"bystander").unwrap();
        drop(bystander);

        remove_verified_directory(&parent, owned).unwrap();
        assert!(parent.read("cleanup-owned/keep").is_ok());
        assert!(parent.symlink_metadata("moved-owned").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn test_owner_digest_uses_only_explicit_layout_paths() {
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

        // The kernel does not reopen either path to canonicalize it. Real layout
        // discovery rejects symlinked root spellings before this point; manually
        // constructed distinct authorities therefore stay distinct here.
        assert_ne!(
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
                stage: ControlName::new(format!("mode-{relative}")).unwrap(),
                backup: ControlName::new(format!("backup-{relative}")).unwrap(),
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

    fn journal_kind(tag: ActionTag) -> RepositoryJournalActionKind {
        let stage = ControlName::new("stage").unwrap();
        let backup = ControlName::new("backup").unwrap();
        match tag {
            ActionTag::CreateDirectory => RepositoryJournalActionKind::CreateDirectory {
                mode: FileMode::Executable,
                stage,
            },
            ActionTag::WriteFile => RepositoryJournalActionKind::WriteFile {
                mode: FileMode::Regular,
                stage,
                backup,
            },
            ActionTag::SetMode => RepositoryJournalActionKind::SetMode {
                mode: FileMode::Executable,
                stage,
                backup,
            },
            ActionTag::DeleteFile => RepositoryJournalActionKind::DeleteFile { backup },
        }
    }

    fn assert_journal_action_mismatch(
        error: FileTransactionError,
        wanted_index: usize,
        wanted_expected: ActionTag,
        wanted_found: ActionTag,
    ) {
        match error {
            FileTransactionError::JournalActionMismatch {
                index,
                expected,
                found,
            } => {
                assert_eq!(index, wanted_index);
                assert_eq!(expected, wanted_expected);
                assert_eq!(found, wanted_found);
            }
            other => panic!("expected JournalActionMismatch, got {other:?}"),
        }
    }

    #[test]
    fn test_journal_action_extractors_reject_mismatched_kind() {
        // Each control-name extractor is total: a journal action whose kind
        // disagrees with the semantic action at its index aborts with a typed
        // JournalActionMismatch carrying the expected and found tags, never a
        // panic. Drift is structurally unreachable through RepositoryDelta::new,
        // so these functions are exercised directly.
        assert_journal_action_mismatch(
            create_directory_stage_name(&journal_kind(ActionTag::WriteFile), 3).unwrap_err(),
            3,
            ActionTag::CreateDirectory,
            ActionTag::WriteFile,
        );
        assert_journal_action_mismatch(
            write_file_control_names(&journal_kind(ActionTag::DeleteFile), 1).unwrap_err(),
            1,
            ActionTag::WriteFile,
            ActionTag::DeleteFile,
        );
        assert_journal_action_mismatch(
            set_mode_control_names(&journal_kind(ActionTag::CreateDirectory), 0).unwrap_err(),
            0,
            ActionTag::SetMode,
            ActionTag::CreateDirectory,
        );
        assert_journal_action_mismatch(
            delete_file_backup_name(&journal_kind(ActionTag::SetMode), 7).unwrap_err(),
            7,
            ActionTag::DeleteFile,
            ActionTag::SetMode,
        );

        // The matching kind extracts its control-names cleanly.
        assert!(create_directory_stage_name(&journal_kind(ActionTag::CreateDirectory), 0).is_ok());
        assert!(write_file_control_names(&journal_kind(ActionTag::WriteFile), 0).is_ok());
        assert!(set_mode_control_names(&journal_kind(ActionTag::SetMode), 0).is_ok());
        assert!(delete_file_backup_name(&journal_kind(ActionTag::DeleteFile), 0).is_ok());
    }

    #[test]
    fn test_set_mode_final_file_identity_rejects_non_file_preimage() {
        // The former :759 destructure: a SetMode action must carry a File
        // preimage. A non-file preimage aborts journal construction with a typed
        // JournalActionMismatch (routed to control-only teardown in the kernel)
        // instead of an unreachable! panic.
        let identity = EntryIdentity::for_bytes("7:1", b"file").unwrap();
        let file = ExpectedPreimage::File {
            identity: identity.clone(),
            mode: FileMode::Regular,
        };
        assert_eq!(set_mode_final_file_identity(&file, 2).unwrap(), identity);

        assert_journal_action_mismatch(
            set_mode_final_file_identity(&ExpectedPreimage::Absent, 4).unwrap_err(),
            4,
            ActionTag::SetMode,
            ActionTag::WriteFile,
        );
        assert_journal_action_mismatch(
            set_mode_final_file_identity(
                &ExpectedPreimage::Directory {
                    identity: EntryIdentity::for_bytes("7:2", b"dir").unwrap(),
                    mode: FileMode::Executable,
                },
                5,
            )
            .unwrap_err(),
            5,
            ActionTag::SetMode,
            ActionTag::CreateDirectory,
        );
    }

    #[test]
    fn test_validate_repository_journal_rejects_inconsistent_stage_and_action_shape() {
        use crate::repository_state::RootRelativePath;

        let identity = EntryIdentity::for_bytes("7:99", b"file").unwrap();
        let action = RepositoryJournalAction {
            path: RepositoryJournalPath {
                root: RepositoryRootClass::Worktree,
                relative: RootRelativePath::parse("target").unwrap(),
            },
            owner: "owner".into(),
            expected: ExpectedPreimage::Absent,
            final_identity: RepositoryFinalIdentity::File {
                identity: identity.clone(),
                mode: FileMode::Executable,
            },
            action: RepositoryJournalActionKind::SetMode {
                mode: FileMode::Executable,
                stage: ControlName::new("mode").unwrap(),
                backup: ControlName::new("backup").unwrap(),
            },
            progress: RepositoryActionProgress::Planned,
        };
        let journal = RepositoryTransactionJournal {
            version: REPOSITORY_JOURNAL_VERSION,
            transaction_id: "txn".into(),
            layout_digest: "layout".into(),
            owner_digest: "owner".into(),
            plan_hash: "plan".into(),
            data_root_was_absent: true,
            data_stage: None,
            data_stage_identity: Some(identity),
            decision: TransactionDecision::Prepared,
            actions: vec![action],
        };

        assert!(validate_repository_journal(&journal, "txn").is_err());
        let mut invalid_action = journal;
        invalid_action.data_root_was_absent = false;
        invalid_action.data_stage_identity = None;
        assert!(validate_repository_journal(&invalid_action, "txn").is_err());
    }
}
