//! Declarative actions accepted by the durable file-set transaction kernel.

use serde::{Deserialize, Serialize};

/// One repository-relative mutation in a durable file-set transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionAction {
    /// Ensure a directory exists, creating it with the requested Unix mode.
    CreateDirectory {
        /// Repository-relative directory path.
        path: String,
        /// Unix permission bits. Ignored on Windows.
        unix_mode: Option<u32>,
    },
    /// Create or replace a regular file with these complete bytes.
    WriteFile {
        /// Repository-relative file path.
        path: String,
        /// Complete final contents.
        contents: Vec<u8>,
        /// Unix permission bits applied before publication. Ignored on Windows.
        unix_mode: Option<u32>,
    },
    /// Change only the Unix permission bits of an existing regular file.
    SetMode {
        /// Repository-relative file path.
        path: String,
        /// Final Unix permission bits. This action is a no-op on Windows.
        unix_mode: u32,
    },
}

impl TransactionAction {
    /// The repository-relative target of this action.
    pub fn path(&self) -> &str {
        match self {
            Self::CreateDirectory { path, .. }
            | Self::WriteFile { path, .. }
            | Self::SetMode { path, .. } => path,
        }
    }
}

/// Stable identity used to reject target races during publication and recovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FileIdentity {
    pub(crate) sha256: String,
    pub(crate) byte_size: u64,
    pub(crate) unix_mode: Option<u32>,
}

/// The original kind and identity of a target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TargetIdentity {
    Absent,
    Directory,
    File { identity: FileIdentity },
}

/// Journal-safe action data; file contents live only in synchronized stages.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum JournalActionKind {
    CreateDirectory {
        path: String,
        unix_mode: Option<u32>,
    },
    WriteFile {
        path: String,
        unix_mode: Option<u32>,
        final_identity: FileIdentity,
        stage_name: String,
        backup_name: String,
    },
    SetMode {
        path: String,
        unix_mode: u32,
        original_mode: Option<u32>,
        final_identity: FileIdentity,
    },
}

impl JournalActionKind {
    pub(crate) fn path(&self) -> &str {
        match self {
            Self::CreateDirectory { path, .. }
            | Self::WriteFile { path, .. }
            | Self::SetMode { path, .. } => path,
        }
    }
}
