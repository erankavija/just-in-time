//! CLI fixtures address lease storage relative to `WorktreePaths::shared_jit`.

use crate::domain::identity::IssueId;
use crate::storage::claim_coordinator::{ClaimsIndex, Lease};
use anyhow::{Context, Result};
use chrono::Utc;
use std::fs;
use std::path::{Path, PathBuf};

pub enum LeaseCorruption {
    MalformedJson,
    InvalidSchema,
}

/// The current aggregate index holds every issue's lease.
pub fn lease_path(shared_jit: &Path, _issue_id: &IssueId) -> PathBuf {
    shared_jit.join("claims.index.json")
}

pub fn history_path(shared_jit: &Path) -> PathBuf {
    shared_jit.join("claims.jsonl")
}

/// Plant a lease without expiry filtering or appending history.
pub fn plant_lease(shared_jit: &Path, lease: &Lease) -> Result<()> {
    let path = lease_path(shared_jit, &lease.issue_id);
    let mut index = read_index(&path)?;
    index
        .leases
        .retain(|stored| stored.issue_id != lease.issue_id);
    index.leases.push(lease.clone());
    index.last_seq += 1;
    index.generated_at = Utc::now();
    fs::create_dir_all(shared_jit)?;
    fs::write(path, serde_json::to_vec_pretty(&index)?).context("plant fixture lease")
}

/// Read stored state, including expired and stale leases.
pub fn read_lease(shared_jit: &Path, issue_id: &IssueId) -> Result<Option<Lease>> {
    Ok(read_index(&lease_path(shared_jit, issue_id))?
        .leases
        .into_iter()
        .find(|lease| lease.issue_id == *issue_id))
}

pub fn corrupt_lease(
    shared_jit: &Path,
    issue_id: &IssueId,
    corruption: LeaseCorruption,
) -> Result<()> {
    let bytes = match corruption {
        LeaseCorruption::MalformedJson => b"{".to_vec(),
        LeaseCorruption::InvalidSchema => serde_json::to_vec(&ClaimsIndex {
            schema_version: 2,
            ..empty_index()
        })?,
    };
    fs::create_dir_all(shared_jit)?;
    fs::write(lease_path(shared_jit, issue_id), bytes).context("corrupt fixture lease storage")
}

fn read_index(path: &Path) -> Result<ClaimsIndex> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).context("parse fixture lease storage"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(empty_index()),
        Err(error) => Err(error).context("read fixture lease storage"),
    }
}

fn empty_index() -> ClaimsIndex {
    ClaimsIndex {
        schema_version: 1,
        generated_at: Utc::now(),
        ..ClaimsIndex::default()
    }
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
            stale: false,
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
    }
}
