//! Versioned wire format for durable file-set transaction recovery.

use crate::repository_state::{
    EntryIdentity, ExpectedPreimage, FileMode, RepositoryRootClass, RootRelativePath,
};
use serde::{Deserialize, Serialize};

pub(crate) const REPOSITORY_JOURNAL_VERSION: u32 = 2;
pub(crate) const JOURNAL_FILE: &str = "journal.json";

/// Durable transaction decision. Prepared transactions roll back; committed
/// transactions retain their final targets; rolled-back transactions only need
/// terminal-residue cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransactionDecision {
    Prepared,
    Committed,
    RolledBack,
}

/// A validated opaque name below one transaction-control directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub(crate) struct ControlName(String);

impl ControlName {
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, String> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err("transaction control name is not one safe path component".into());
        }
        Ok(Self(value))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ControlName {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ControlName> for String {
    fn from(value: ControlName) -> Self {
        value.0
    }
}

/// Layout-qualified journal path. Absolute and inferred-root spellings are not
/// representable in the durable protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepositoryJournalPath {
    pub(crate) root: RepositoryRootClass,
    pub(crate) relative: RootRelativePath,
}

/// Final target identity used to distinguish our publication from a raced
/// post-crash occupant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RepositoryFinalIdentity {
    Absent,
    Directory {
        identity: EntryIdentity,
        mode: FileMode,
    },
    File {
        identity: EntryIdentity,
        mode: FileMode,
    },
}

/// Exact repository action vocabulary persisted by the layout-aware kernel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum RepositoryJournalActionKind {
    CreateDirectory {
        mode: FileMode,
        stage: ControlName,
    },
    WriteFile {
        mode: FileMode,
        stage: ControlName,
        backup: ControlName,
    },
    SetMode {
        mode: FileMode,
        stage: ControlName,
        backup: ControlName,
    },
    DeleteFile {
        backup: ControlName,
    },
}

/// Durable per-action preparation/publication progress.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RepositoryActionProgress {
    #[default]
    Planned,
    Prepared,
    BackupReady,
    Published,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepositoryJournalAction {
    pub(crate) path: RepositoryJournalPath,
    pub(crate) owner: String,
    pub(crate) expected: ExpectedPreimage,
    pub(crate) final_identity: RepositoryFinalIdentity,
    pub(crate) action: RepositoryJournalActionKind,
    pub(crate) progress: RepositoryActionProgress,
}

/// Durable layout-aware recovery authority. `layout_digest` binds every
/// relative path to the exact selected roots supplied at session open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RepositoryTransactionJournal {
    pub(crate) version: u32,
    pub(crate) transaction_id: String,
    pub(crate) layout_digest: String,
    /// Stable owner identity: a digest of the worktree and data-root PATHS, which
    /// (unlike `layout_digest`) does not change when an absent data root becomes
    /// present. Recovery under the shared worktree bootstrap namespace uses it to
    /// tell its own transactions from those of a different data root.
    #[serde(default)]
    pub(crate) owner_digest: String,
    pub(crate) plan_hash: String,
    pub(crate) data_root_was_absent: bool,
    pub(crate) data_stage: Option<ControlName>,
    pub(crate) data_stage_identity: Option<EntryIdentity>,
    pub(crate) decision: TransactionDecision,
    pub(crate) actions: Vec<RepositoryJournalAction>,
}
