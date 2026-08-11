//! Versioned wire format for durable file-set transaction recovery.

use crate::repository_state::{
    EntryIdentity, ExpectedPreimage, FileMode, RepositoryRootClass, RootRelativePath,
};
use serde::{Deserialize, Serialize};

// Version 3 marks the single-barrier cutover that removed per-action progress
// state. Version-2 journals are a different durable representation that this
// code does not interpret, so recovery rejects them rather than reading them.
pub(crate) const REPOSITORY_JOURNAL_VERSION: u32 = 3;
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

/// The four-value action classification shared by the semantic action enum
/// (`RepositoryAction`) and its durable journal counterpart
/// (`RepositoryJournalActionKind`). The two enums carry disjoint payloads and
/// stay separate types; this tag is the common vocabulary the kernel uses to
/// report a journal/delta alignment mismatch without conflating the enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ActionTag {
    CreateDirectory,
    WriteFile,
    SetMode,
    DeleteFile,
}

impl std::fmt::Display for ActionTag {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::CreateDirectory => "create_directory",
            Self::WriteFile => "write_file",
            Self::SetMode => "set_mode",
            Self::DeleteFile => "delete_file",
        })
    }
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

impl RepositoryJournalActionKind {
    /// The four-value action tag of this durable journal action kind.
    pub(crate) fn tag(&self) -> ActionTag {
        match self {
            Self::CreateDirectory { .. } => ActionTag::CreateDirectory,
            Self::WriteFile { .. } => ActionTag::WriteFile,
            Self::SetMode { .. } => ActionTag::SetMode,
            Self::DeleteFile { .. } => ActionTag::DeleteFile,
        }
    }
}

/// One durable action, described by both of its endpoints: the preimage it
/// requires and the identity it publishes. Recovery converges from those two
/// identities alone, so an action carries no progress state — the record is
/// written once, complete, before the first live mutation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepositoryJournalAction {
    pub(crate) path: RepositoryJournalPath,
    pub(crate) owner: String,
    pub(crate) expected: ExpectedPreimage,
    pub(crate) final_identity: RepositoryFinalIdentity,
    pub(crate) action: RepositoryJournalActionKind,
}

/// Durable layout-aware recovery authority. `layout_digest` binds every
/// relative path to the exact selected roots supplied at session open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
