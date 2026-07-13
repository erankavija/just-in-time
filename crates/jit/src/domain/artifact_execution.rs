//! Stable records produced by dependency-aware archive execution.

use crate::domain::artifact_plan::{
    ContentIdentity, PendingDeletion, PlanTarget, PlanWarning, ReferenceChange,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One new or adopted destination made durable by an archive execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchivePublication {
    /// Source artifact, or `None` for the container ownership marker.
    pub source: Option<String>,
    /// Repository-relative destination path.
    pub destination: String,
    /// Exact published bytes.
    pub content_identity: ContentIdentity,
    /// True when identical occupied content was adopted without claiming publication.
    pub adopted: bool,
}

/// Structured result returned by `jit archive ... --execute`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchiveExecutionResult {
    /// Result wire version.
    pub schema_version: u32,
    /// Recomputed target.
    pub target: PlanTarget,
    /// Mirror root used by execution.
    pub destination_root: String,
    /// New and newly reconciled publications recorded by this execution.
    pub publications: Vec<ArchivePublication>,
    /// New and newly reconciled durable reference changes.
    pub reference_changes: Vec<ReferenceChange>,
    /// Removals recorded before their attempts.
    pub planned_deletions: Vec<PendingDeletion>,
    /// Sources actually removed.
    pub deleted_sources: Vec<String>,
    /// Planning and non-fatal deletion diagnostics.
    pub warnings: Vec<PlanWarning>,
    /// Whether this run appended an archive commit event.
    pub event_appended: bool,
    /// Whether the event repaired previously unrecorded adopted state.
    pub reconciling: bool,
}
