//! Stable records produced by dependency-aware archive execution.

use crate::domain::artifact_plan::{
    ContentIdentity, PendingDeletion, PlanTarget, PlanWarning, ReferenceChange,
};
use crate::domain::{Event, EventBody};
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

/// The durable payload of one archive execution, shared by the event that
/// records it and by the folds that read prior executions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveExecution {
    /// Container or document selected for archival.
    pub target: PlanTarget,
    /// Repository-relative mirror root used by the operation.
    pub destination_root: String,
    /// New or adopted publications made durable before this record.
    pub publications: Vec<ArchivePublication>,
    /// Exact issue-document reference changes made durable before this record.
    pub reference_changes: Vec<ReferenceChange>,
    /// Identity-guarded removals attempted after this record.
    pub planned_deletions: Vec<PendingDeletion>,
    /// Whether the record reconciles durable state left by an earlier failed append.
    pub reconciling: bool,
}

/// Ordered execution evidence for one archive target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ArchiveEvidenceRecord {
    pub target: PlanTarget,
    pub executions: Vec<ArchiveExecution>,
}

/// The archive executions among `events`, in log order.
pub fn archive_executions(events: &[Event]) -> Vec<ArchiveExecution> {
    events
        .iter()
        .filter_map(|event| match event.body() {
            EventBody::ArtifactArchiveExecuted(execution) => Some(execution.clone()),
            _ => None,
        })
        .collect()
}

/// Structured result returned by `jit archive ... --execute`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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
    pub plan_warnings: Vec<PlanWarning>,
    /// Whether this run appended an archive commit event.
    pub event_appended: bool,
    /// Whether the event repaired previously unrecorded adopted state.
    pub reconciling: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{archive_execution, stamped_event};

    #[test]
    fn test_archive_execution_event_body_serializes_the_payload_flat_beside_its_tag() {
        let execution = archive_execution(
            PlanTarget::Document {
                path: "a.md".into(),
            },
            "arc",
            &[("a.md", "arc/a.md")],
        );
        let body = serde_json::to_value(EventBody::draft_artifact_archive_executed(
            execution.clone(),
        ))
        .unwrap();
        let mut expected = serde_json::to_value(&execution).unwrap();
        expected["type"] = "artifact_archive_executed".into();
        assert_eq!(body, expected);
    }

    #[test]
    fn test_archive_executions_keeps_only_archive_events_in_log_order() {
        let first = archive_execution(
            PlanTarget::Document {
                path: "a.md".into(),
            },
            "arc",
            &[],
        );
        let second = archive_execution(
            PlanTarget::Document {
                path: "b.md".into(),
            },
            "arc",
            &[],
        );
        let at = chrono::DateTime::UNIX_EPOCH;
        let events = [
            stamped_event(
                EventBody::draft_artifact_archive_executed(first.clone()),
                "1",
                at,
            ),
            stamped_event(
                EventBody::IssueDeleted {
                    issue_id: crate::domain::identity::IssueId::from_uuid(uuid::Uuid::from_u128(1)),
                },
                "2",
                at,
            ),
            stamped_event(
                EventBody::draft_artifact_archive_executed(second.clone()),
                "3",
                at,
            ),
        ];
        assert_eq!(archive_executions(&events), [first, second]);
    }
}
