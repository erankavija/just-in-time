//! Versioned wire format for durable file-set transaction recovery.

use super::transaction_action::{JournalActionKind, TargetIdentity};
use serde::{Deserialize, Serialize};

pub(crate) const JOURNAL_VERSION: u32 = 1;
pub(crate) const JOURNAL_FILE: &str = "journal.json";

/// Durable transaction decision. Prepared transactions roll back; committed
/// transactions retain their final targets; rolled-back transactions only need
/// terminal-residue cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionDecision {
    Prepared,
    Committed,
    RolledBack,
}

/// One identity-checked forward/reverse action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct JournalAction {
    pub(crate) action: JournalActionKind,
    pub(crate) original: TargetIdentity,
}

/// Complete recovery state. Paths are repository-relative and opaque stage or
/// backup names are relative to the transaction control directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TransactionJournal {
    pub(crate) version: u32,
    pub(crate) transaction_id: String,
    pub(crate) plan_hash: String,
    pub(crate) fresh_root: bool,
    pub(crate) decision: TransactionDecision,
    pub(crate) actions: Vec<JournalAction>,
}
