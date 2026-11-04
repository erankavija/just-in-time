//! The `jit claim` and `jit recover` responses and their human renderings.

use super::{HumanRender, OutputMode};
use crate::commands::claim::{ListedLease, RecoveryReport, ReleasedLeaseInfo};
use crate::domain::identity::{IssueId, LeaseId};
use crate::domain::list_envelope::{CollectionKey, ListEnvelope};
use crate::storage::Lease;
use schemars::JsonSchema;
use serde::Serialize;
use std::fmt::Write;

/// Response for `claim acquire`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimAcquireResponse {
    pub lease_id: LeaseId,
    /// The issue as the caller named it.
    pub issue_id: String,
    pub ttl_secs: u64,
}

impl HumanRender for ClaimAcquireResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!(
            "Acquired lease: {}\n  Issue: {}\n  TTL: {} seconds\n",
            self.lease_id, self.issue_id, self.ttl_secs
        )
    }
}

/// Response for `claim release`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimReleaseResponse {
    #[serde(flatten)]
    pub released: ReleasedLeaseInfo,
}

impl HumanRender for ClaimReleaseResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let released = &self.released;
        format!(
            "Released lease: {}\n  Issue: {}\n  Previous owner: {}\n  Released by: {}\n",
            released.lease_id, released.issue_id, released.previous_owner, released.actor
        )
    }
}

/// Response for `claim renew`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimRenewResponse {
    pub lease: Lease,
    /// Seconds the lease was extended by; rendered in the human output only.
    #[serde(skip)]
    pub extension: u64,
}

impl HumanRender for ClaimRenewResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let lease = &self.lease;
        let mut text = format!(
            "Renewed lease: {}\n  Issue: {}\n  Extended by: {} seconds\n",
            lease.lease_id, lease.issue_id, self.extension
        );
        if let Some(expires_at) = lease.expires_at {
            let _ = writeln!(text, "  New expiry: {}", expires_at.to_rfc3339());
        }
        text
    }
}

/// Response for `claim heartbeat`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimHeartbeatResponse {
    pub lease_id: LeaseId,
    pub issue_id: IssueId,
}

impl HumanRender for ClaimHeartbeatResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!(
            "Heartbeat sent: {}\n  Issue: {}\n",
            self.lease_id, self.issue_id
        )
    }
}

/// Response for `claim force-evict`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ClaimForceEvictResponse {
    pub lease_id: LeaseId,
    pub issue_id: IssueId,
    pub reason: String,
}

impl HumanRender for ClaimForceEvictResponse {
    fn render_human(&self, _: OutputMode) -> String {
        format!(
            "Force-evicted lease: {}\n  Issue: {}\n  Reason: {}\n",
            self.lease_id, self.issue_id, self.reason
        )
    }
}

/// Collection key of [`LeaseListResponse`].
pub enum LeasesKey {}

impl CollectionKey for LeasesKey {
    const KEY: &'static str = "leases";
    const SCHEMA_NAME: Option<&'static str> = Some("LeaseListResponse");
}

/// The unserialized extras of a [`LeaseListResponse`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, JsonSchema)]
pub struct LeaseListingKind {
    /// `claim list` rather than `claim status`; selects the human wording.
    #[serde(skip)]
    pub all: bool,
}

/// Response for `claim status` and `claim list`.
pub type LeaseListResponse = ListEnvelope<LeasesKey, ListedLease, LeaseListingKind>;

impl LeaseListResponse {
    pub fn listing(leases: Vec<ListedLease>, all: bool) -> Self {
        Self::with_extras(leases, LeaseListingKind { all })
    }
}

impl HumanRender for LeaseListResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let (none, heading) = if self.extras.all {
            ("No active leases.", "All active leases")
        } else {
            ("No active leases found.", "Active leases")
        };
        if self.items.is_empty() {
            return format!("{none}\n");
        }
        let mut text = format!("{heading} ({}):\n\n", self.count());
        self.items
            .iter()
            .for_each(|listed| render_lease(&mut text, listed));
        text
    }
}

/// Append one lease to `text` for the plain-text listings.
fn render_lease(text: &mut String, listed: &ListedLease) {
    let lease = &listed.lease;
    let now = chrono::Utc::now();
    let _ = writeln!(text, "Lease: {}", lease.lease_id);
    let _ = writeln!(text, "  Issue:    {}", lease.issue_id);
    let _ = writeln!(text, "  Agent:    {}", lease.agent_id);
    let _ = writeln!(text, "  Worktree: {}", lease.worktree_id);
    if let Some(branch) = &lease.branch {
        let _ = writeln!(text, "  Branch:   {branch}");
    }
    let _ = writeln!(text, "  Acquired: {}", lease.acquired_at);
    if lease.ttl_secs > 0 {
        if let Some(expires_at) = lease.expires_at {
            let remaining = expires_at.signed_duration_since(now);
            let _ = writeln!(
                text,
                "  Expires:  {expires_at} ({} seconds remaining)",
                remaining.num_seconds().max(0)
            );
        }
    } else {
        let since_beat = now.signed_duration_since(lease.last_beat);
        let _ = writeln!(text, "  TTL:      indefinite");
        let _ = writeln!(
            text,
            "  Last beat: {} ({} seconds ago)",
            lease.last_beat,
            since_beat.num_seconds()
        );
        if listed.stale {
            let _ = writeln!(
                text,
                "  STALE: Lease marked stale (no heartbeat for {} minutes)",
                since_beat.num_minutes()
            );
            let _ = writeln!(
                text,
                "     Use 'jit claim heartbeat {}' to refresh",
                lease.issue_id.short_id()
            );
        }
    }
    text.push('\n');
}

/// Response for `recover`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RecoverResponse {
    pub success: bool,
    #[serde(flatten)]
    pub report: RecoveryReport,
}

impl HumanRender for RecoverResponse {
    fn render_human(&self, _: OutputMode) -> String {
        let report = &self.report;
        format!(
            "Recovery complete:\n  • Redo records applied: {}\n  • Stale locks cleaned: {}\n  \
             • Expired leases evicted: {}\n  • Stale leases reclaimed: {}\n  \
             • Temp files removed: {}\n  • Derived-state targets repaired: {}\n",
            report.redo_records_applied,
            report.stale_locks_cleaned,
            report.expired_leases_evicted,
            report.stale_leases_reclaimed,
            report.temp_files_removed,
            report.derived_state_repaired
        )
    }
}
