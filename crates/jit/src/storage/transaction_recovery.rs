//! Typed recovery state and failure/race injection for the transaction kernel.

use super::transaction_journal::ActionTag;

/// Observable recovery state returned after prepare/commit/rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RecoveryState {
    /// A prepared journal remains and must be rolled back before another write.
    Prepared,
    /// A committed journal remains; final targets are authoritative and cleanup
    /// must finish before another write.
    Committed,
}

/// Stable injection points covering every durability boundary and action edge.
///
/// The shared `Repository` prefix is intentional: every variant names a step
/// of the one repository-write transaction kernel, not an accidental
/// collision. `test-support`-off builds narrow this type's own exposure to
/// `pub(crate)` (see `storage::mod`), which lets clippy's default
/// `avoid-breaking-exported-api` guard stop suppressing `enum_variant_names`
/// for it; silence that lint explicitly rather than rename call sites.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FailurePoint {
    RepositoryRecoveryExternal,
    RepositoryRecoveryInternal,
    RepositoryBeforeControlCreation,
    RepositoryCreateControl,
    RepositoryBeforeInitialJournal,
    RepositoryBeforeDataStageJournal,
    RepositoryBeforePreparedJournal { action: usize },
    RepositoryBeforePublishedJournal { action: usize },
    RepositorySyncInitialJournal,
    RepositoryCreateCompanion,
    RepositorySweepCompanions,
    RepositoryPrepareIntent,
    RepositoryPrepareAction { action: usize },
    RepositoryStageAction { action: usize },
    RepositorySyncStage { action: usize },
    RepositorySyncBackup { action: usize },
    RepositorySyncPreparedAction { action: usize },
    RepositoryBeforeAction { action: usize },
    RepositoryBeforeTargetMutation { action: usize },
    RepositoryAfterRootBindingCheck { action: usize },
    RepositoryBeforeDeleteRename { action: usize },
    RepositorySyncTargetParent { action: usize },
    RepositoryVerifyFinalIdentity { action: usize },
    RepositoryAfterAction { action: usize },
    RepositoryBeforeDataRootPublication,
    RepositoryAfterDataParentBindingCheck,
    RepositoryAfterDataRootPublication,
    RepositoryBeforeCommitDecision,
    RepositoryAfterCommit,
    RepositoryBeforeReverseAction { action: usize },
    RepositorySyncRollbackJournal { action: usize },
    RepositoryBeforeRollbackDecision,
    RepositoryBeforeStageCleanup,
    RepositoryBeforeCompanionCleanup,
    RepositoryBeforeControlCleanup,
    RepositoryCleanup,
}

/// Test seam for deterministic I/O interruption and race injection.
pub trait TransactionFailureInjector: Send + Sync {
    /// Fail or mutate the fixture at one precise kernel boundary.
    fn check(&self, point: &FailurePoint) -> std::io::Result<()>;
}

/// Production injector: every boundary proceeds normally.
#[derive(Debug, Default)]
pub struct NoTransactionFailures;

impl TransactionFailureInjector for NoTransactionFailures {
    fn check(&self, _point: &FailurePoint) -> std::io::Result<()> {
        Ok(())
    }
}

/// Failure that means a durable journal must be recovered before further writes.
#[derive(Debug, thiserror::Error)]
#[error("transaction {transaction_id} requires recovery from {state:?}: {source}")]
pub(crate) struct RecoveryRequiredError {
    /// Transaction whose journal remains authoritative.
    pub transaction_id: String,
    /// Durable state recorded in that journal.
    pub state: RecoveryState,
    /// Failure that prevented convergence or cleanup.
    #[source]
    pub source: anyhow::Error,
}

/// Typed storage failures specific to durable file-set publication.
#[derive(Debug, thiserror::Error)]
pub(crate) enum FileTransactionError {
    #[error("invalid transaction target path: {path}")]
    InvalidPath { path: String },
    #[error("duplicate transaction target: {path}")]
    DuplicateTarget { path: String },
    #[error("transaction target traverses a symbolic link or junction: {path}")]
    SymlinkComponent { path: String },
    #[error("transaction target has an unsupported filesystem object: {path}")]
    UnsupportedTarget { path: String },
    #[error("transaction target changed after preflight: {path}")]
    UnexpectedOccupant { path: String },
    #[error("reserved bootstrap path is occupied by non-protocol state")]
    UnexpectedBootstrapOccupant,
    #[error("filesystem operation required for durable transactions is unsupported: {operation}")]
    UnsupportedFilesystem { operation: String },
    #[error("transaction journal does not match the selected repository layout")]
    LayoutMismatch,
    #[error("selected data-root destination became occupied: {path}")]
    OccupiedDataRoot { path: String },
    #[error("filesystem object kind is unsupported for transaction capture: {path}")]
    UnsupportedObjectKind { path: String },
    #[error(
        "transaction journal actions resolve to one physical identity (a hard-link alias): {path}"
    )]
    AliasedTarget { path: String },
    /// The semantic delta and its durable journal action drifted out of
    /// alignment at `index`: a total extraction expected the `expected` action
    /// tag but the journal action carried `found`. Structurally unreachable for
    /// a delta built through `RepositoryDelta::new` (its journal is derived from
    /// the same validated actions); it replaces the former mid-publication
    /// `unreachable!` so any drift aborts with no partial write instead of a
    /// panic.
    #[error("journal action {index} kind mismatch: expected {expected}, found {found}")]
    JournalActionMismatch {
        index: usize,
        expected: ActionTag,
        found: ActionTag,
    },
}
