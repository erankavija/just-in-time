//! CLI fixtures address lease storage relative to `WorktreePaths::shared_jit`.

use crate::domain::identity::IssueId;
use crate::storage::claim_coordinator::Lease;
use crate::storage::WorktreePaths;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub enum LeaseCorruption {
    MalformedJson,
}

/// The issue's lease record.
pub fn lease_path(shared_jit: &Path, issue_id: &IssueId) -> PathBuf {
    WorktreePaths::lease_path(shared_jit, issue_id)
}

pub fn history_path(shared_jit: &Path) -> PathBuf {
    shared_jit.join("claims.jsonl")
}

/// Plant a lease without expiry filtering or appending history.
pub fn plant_lease(shared_jit: &Path, lease: &Lease) -> Result<()> {
    write_record(
        shared_jit,
        &lease.issue_id,
        &serde_json::to_vec_pretty(lease)?,
    )
    .context("plant fixture lease")
}

/// Read stored state, including expired and stale leases.
pub fn read_lease(shared_jit: &Path, issue_id: &IssueId) -> Result<Option<Lease>> {
    match fs::read(lease_path(shared_jit, issue_id)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .context("parse fixture lease record"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).context("read fixture lease record"),
    }
}

pub fn corrupt_lease(
    shared_jit: &Path,
    issue_id: &IssueId,
    corruption: LeaseCorruption,
) -> Result<()> {
    let bytes: &[u8] = match corruption {
        LeaseCorruption::MalformedJson => b"{",
    };
    write_record(shared_jit, issue_id, bytes).context("corrupt fixture lease record")
}

fn write_record(shared_jit: &Path, issue_id: &IssueId, bytes: &[u8]) -> Result<()> {
    fs::create_dir_all(WorktreePaths::leases_dir(shared_jit))?;
    Ok(fs::write(lease_path(shared_jit, issue_id), bytes)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::claim_coordinator::Lease;
    use crate::test_utils::issue_id;
    use chrono::{Duration, Utc};

    #[test]
    fn test_plant_lease_replaces_only_its_issue_in_the_supplied_store() {
        let dir = tempfile::tempdir().unwrap();
        let shared_jit = dir.path().join("control-plane");
        let now = Utc::now();
        let mut lease = Lease {
            lease_id: "00000000-0000-4000-8000-000000000001".parse().unwrap(),
            issue_id: issue_id(1),
            agent_id: "agent:fixture".parse().unwrap(),
            worktree_id: "fixture-worktree".parse().unwrap(),
            branch: None,
            ttl_secs: 60,
            acquired_at: now - Duration::hours(1),
            expires_at: Some(now - Duration::minutes(59)),
            last_beat: now - Duration::hours(1),
        };
        let other = Lease {
            issue_id: issue_id(2),
            ..lease.clone()
        };
        plant_lease(&shared_jit, &lease).unwrap();
        plant_lease(&shared_jit, &other).unwrap();

        lease.branch = Some("updated".into());
        plant_lease(&shared_jit, &lease).unwrap();

        assert_eq!(
            read_lease(&shared_jit, &lease.issue_id).unwrap(),
            Some(lease)
        );
        assert_eq!(
            read_lease(&shared_jit, &other.issue_id).unwrap(),
            Some(other)
        );
        assert_eq!(read_lease(&shared_jit, &issue_id(3)).unwrap(), None);
        assert!(lease_path(&shared_jit, &issue_id(1)).starts_with(&shared_jit));
        assert_ne!(
            lease_path(&shared_jit, &issue_id(1)),
            lease_path(&shared_jit, &issue_id(2))
        );
    }
}
