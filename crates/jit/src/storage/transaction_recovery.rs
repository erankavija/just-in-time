//! Typed recovery state and failure/race injection for the transaction kernel.

use super::transaction_journal::TransactionDecision;

/// Observable recovery state returned after prepare/commit/rollback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecoveryState {
    /// No durable transaction residue remains.
    Clean,
    /// A prepared journal remains and must be rolled back before another write.
    Prepared,
    /// A committed journal remains; final targets are authoritative and cleanup
    /// must finish before another write.
    Committed,
    /// Rollback is terminal; only machine-local residue cleanup remains.
    RolledBack,
}

impl From<TransactionDecision> for RecoveryState {
    fn from(value: TransactionDecision) -> Self {
        match value {
            TransactionDecision::Prepared => Self::Prepared,
            TransactionDecision::Committed => Self::Committed,
            TransactionDecision::RolledBack => Self::RolledBack,
        }
    }
}

/// Stable injection points covering every durability boundary and action edge.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FailurePoint {
    CreateExternalControl,
    CreateInternalControl,
    CreateJournal,
    Stage { action: usize },
    SyncStage { action: usize },
    SyncJournal { decision: RecoveryState },
    BeforeAction { action: usize },
    AfterParentOpen { action: usize },
    BeforeModeMutation { action: usize },
    AfterRenameAside { action: usize },
    AfterPublish { action: usize },
    SyncTargetParent { action: usize },
    ReverseAction { action: usize },
    BeforeReverseModeMutation { action: usize },
    SyncReverseParent { action: usize },
    BeforeFreshRootRemoval,
    AfterFreshRootRemoval,
    CleanupTerminalResidue,
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
pub struct RecoveryRequiredError {
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
pub enum FileTransactionError {
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
    #[error("transaction staging and target are on different volumes: {path}")]
    CrossVolume { path: String },
    #[error("filesystem operation required for durable transactions is unsupported: {operation}")]
    UnsupportedFilesystem { operation: String },
}
