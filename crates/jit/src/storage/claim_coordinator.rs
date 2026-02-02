//! Claim coordination for multi-agent parallel work
//!
//! This module implements atomic, race-free claim acquisition using file locks
//! to ensure only one agent can hold a lease on an issue at a time.
//!
//! # Design
//!
//! - Uses exclusive file locks (`fs4` crate) for atomicity
//! - Claims stored in append-only JSONL log for audit trail
//! - Claims index provides derived view of active leases
//! - Monotonic time semantics for expiration (immune to NTP)
//!
//! See design doc: `dev/design/worktree-parallel-work.md` - "Claim Acquisition Algorithm"

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

use super::lock::FileLocker;
use super::lock_cleanup;
use super::temp_cleanup;
use super::worktree_paths::WorktreePaths;
use crate::errors;

/// A lease granting exclusive write access to an issue
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lease {
    /// Unique lease identifier (UUID)
    pub lease_id: String,
    /// Issue being claimed
    pub issue_id: String,
    /// Agent holding the lease
    pub agent_id: String,
    /// Worktree where lease was acquired
    pub worktree_id: String,
    /// Git branch (informational)
    pub branch: Option<String>,
    /// Time-to-live in seconds (0 = indefinite)
    pub ttl_secs: u64,
    /// When the lease was acquired
    pub acquired_at: DateTime<Utc>,
    /// When the lease expires (None if TTL=0)
    pub expires_at: Option<DateTime<Utc>>,
    /// Last heartbeat timestamp (for indefinite leases)
    pub last_beat: DateTime<Utc>,
}

/// Claim operation types for audit log
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ClaimOp {
    /// Acquire a new lease
    Acquire {
        #[serde(flatten)]
        lease: Lease,
    },
    /// Renew an existing lease
    Renew {
        lease_id: String,
        new_expires_at: Option<DateTime<Utc>>,
        new_last_beat: DateTime<Utc>,
    },
    /// Release a lease explicitly
    Release {
        lease_id: String,
        released_at: DateTime<Utc>,
    },
    /// Automatically evict expired lease
    AutoEvict {
        lease_id: String,
        evicted_at: DateTime<Utc>,
        reason: String,
    },
    /// Force evict a lease (admin operation)
    ForceEvict {
        lease_id: String,
        evicted_at: DateTime<Utc>,
        reason: String,
    },
}

/// Claim operation with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimLogEntry {
    /// Schema version for future compatibility
    pub schema_version: u32,
    /// Sequence number for total ordering
    pub seq: u64,
    /// Timestamp of operation
    pub timestamp: DateTime<Utc>,
    /// The operation
    #[serde(flatten)]
    pub operation: ClaimOp,
}

/// Index of active leases (derived from log)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaimsIndex {
    /// Schema version
    pub schema_version: u32,
    /// When this index was generated
    pub generated_at: DateTime<Utc>,
    /// Last sequence number processed
    pub last_seq: u64,
    /// Stale threshold in seconds (for indefinite leases)
    pub stale_threshold_secs: u64,
    /// Active leases by issue ID
    pub leases: Vec<Lease>,
    /// Detected sequence gaps (for diagnostics)
    #[serde(default)]
    pub sequence_gaps: Vec<u64>,
}

impl ClaimsIndex {
    /// Find active lease for an issue
    pub fn find_lease(&self, issue_id: &str) -> Option<&Lease> {
        self.leases.iter().find(|l| l.issue_id == issue_id)
    }

    /// Add a lease to the index
    pub fn add_lease(&mut self, lease: Lease) {
        // Remove any existing lease for this issue
        self.leases.retain(|l| l.issue_id != lease.issue_id);
        self.leases.push(lease);
    }

    /// Remove a lease from the index
    pub fn remove_lease(&mut self, lease_id: &str) {
        self.leases.retain(|l| l.lease_id != lease_id);
    }

    /// Check if lease is stale (for indefinite leases)
    pub fn is_stale(&self, lease: &Lease) -> bool {
        if lease.ttl_secs > 0 {
            return false; // Finite leases use expiration, not staleness
        }

        let elapsed = Utc::now().signed_duration_since(lease.last_beat);
        elapsed.num_seconds() as u64 > self.stale_threshold_secs
    }

    /// Find a lease by lease_id
    pub fn find_lease_by_id(&self, lease_id: &str) -> Option<&Lease> {
        self.leases.iter().find(|l| l.lease_id == lease_id)
    }
}

/// Claim coordinator for atomic lease operations
pub struct ClaimCoordinator {
    paths: WorktreePaths,
    locker: FileLocker,
    worktree_id: String,
    agent_id: String,
}

impl ClaimCoordinator {
    /// Create a new claim coordinator
    ///
    /// # Arguments
    ///
    /// * `paths` - Worktree paths (for control plane location)
    /// * `locker` - File locker for atomic operations
    /// * `worktree_id` - Current worktree ID
    /// * `agent_id` - Current agent ID
    pub fn new(
        paths: WorktreePaths,
        locker: FileLocker,
        worktree_id: String,
        agent_id: String,
    ) -> Self {
        Self {
            paths,
            locker,
            worktree_id,
            agent_id,
        }
    }

    /// Initialize control plane directories
    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(self.paths.shared_jit.join("locks"))
            .context("Failed to create locks directory")?;
        fs::create_dir_all(self.paths.shared_jit.join("heartbeat"))
            .context("Failed to create heartbeat directory")?;
        Ok(())
    }

    /// Acquire a claim on an issue (atomic operation)
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Issue to claim
    /// * `ttl_secs` - Time-to-live in seconds (0 = indefinite)
    ///
    /// # Returns
    ///
    /// The acquired lease on success
    ///
    /// # Errors
    ///
    /// - Issue already claimed by another agent
    /// - Lock timeout
    /// - I/O errors
    pub fn acquire_claim(&self, issue_id: &str, ttl_secs: u64) -> Result<Lease> {
        // 1. Acquire exclusive lock with metadata for diagnostics
        let lock_path = self.paths.shared_jit.join("locks/claims.lock");
        fs::create_dir_all(lock_path.parent().unwrap())?;
        let _guard = self
            .locker
            .lock_exclusive_with_metadata(&lock_path, &self.agent_id)?;

        // 2. Load index and evict expired leases
        let mut index = self.load_claims_index()?;
        self.evict_expired(&mut index)?;

        // 3. Check availability
        if let Some(existing) = index.find_lease(issue_id) {
            let expires_info = if existing.ttl_secs == 0 {
                format!("(indefinite lease, last beat: {})", existing.last_beat)
            } else {
                format!("until {}", existing.expires_at.unwrap())
            };

            bail!(
                "{}",
                errors::already_claimed(issue_id, &existing.agent_id, &expires_info)
            );
        }

        // 4. Create new lease
        let now = Utc::now();
        let lease = Lease {
            lease_id: Uuid::new_v4().to_string(),
            issue_id: issue_id.to_string(),
            agent_id: self.agent_id.clone(),
            worktree_id: self.worktree_id.clone(),
            branch: self.get_current_branch().ok(),
            ttl_secs,
            acquired_at: now,
            expires_at: if ttl_secs > 0 {
                Some(now + Duration::seconds(ttl_secs as i64))
            } else {
                None
            },
            last_beat: now,
        };

        // 5. Append to audit log
        let op = ClaimOp::Acquire {
            lease: lease.clone(),
        };
        self.append_claim_op(&op)?;

        // 6. Update index atomically
        index.add_lease(lease.clone());
        self.write_index_atomic(&index)?;

        // 7. Lock released via RAII
        Ok(lease)
    }

    /// Load claims index (or create empty if missing)
    pub fn load_claims_index(&self) -> Result<ClaimsIndex> {
        let index_path = self.paths.shared_jit.join("claims.index.json");

        if !index_path.exists() {
            return Ok(ClaimsIndex {
                schema_version: 1,
                generated_at: Utc::now(),
                last_seq: 0,
                stale_threshold_secs: 3600, // 1 hour default
                leases: Vec::new(),
                sequence_gaps: Vec::new(),
            });
        }

        let content = fs::read_to_string(&index_path).context("Failed to read claims index")?;
        serde_json::from_str(&content).context("Failed to parse claims index")
    }

    /// Write claims index atomically (temp + rename)
    pub fn write_index_atomic(&self, index: &ClaimsIndex) -> Result<()> {
        let index_path = self.paths.shared_jit.join("claims.index.json");
        let temp_path = index_path.with_extension("tmp");

        let json = serde_json::to_string_pretty(index)?;
        fs::write(&temp_path, json)?;

        // Fsync for durability
        let file = fs::File::open(&temp_path)?;
        file.sync_all()?;
        drop(file);

        // Atomic rename
        fs::rename(temp_path, index_path)?;

        Ok(())
    }

    /// Evict expired leases from index
    pub fn evict_expired(&self, index: &mut ClaimsIndex) -> Result<()> {
        let now = Utc::now();
        let expired: Vec<_> = index
            .leases
            .iter()
            .filter(|l| {
                // Only evict finite leases that are expired
                if l.ttl_secs > 0 {
                    if let Some(expires_at) = l.expires_at {
                        return now > expires_at;
                    }
                }
                false
            })
            .cloned()
            .collect();

        for lease in expired {
            // Log eviction
            let op = ClaimOp::AutoEvict {
                lease_id: lease.lease_id.clone(),
                evicted_at: now,
                reason: format!("Lease expired at {}", lease.expires_at.unwrap()),
            };
            self.append_claim_op(&op)?;

            // Remove from index
            index.remove_lease(&lease.lease_id);
        }

        Ok(())
    }

    /// Append a claim operation to the audit log
    fn append_claim_op(&self, op: &ClaimOp) -> Result<()> {
        let log_path = self.paths.shared_jit.join("claims.jsonl");

        // Get next sequence number
        let seq = self.get_next_seq(&log_path)?;

        let entry = ClaimLogEntry {
            schema_version: 1,
            seq,
            timestamp: Utc::now(),
            operation: op.clone(),
        };

        let json = serde_json::to_string(&entry)?;
        let line = format!("{}\n", json);

        // Append to log
        use std::io::Write;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        file.write_all(line.as_bytes())?;
        file.sync_all()?; // Fsync for durability

        Ok(())
    }

    /// Get next sequence number from log
    fn get_next_seq(&self, log_path: &PathBuf) -> Result<u64> {
        if !log_path.exists() {
            return Ok(1);
        }

        let content = fs::read_to_string(log_path)?;
        let max_seq = content
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<ClaimLogEntry>(line)
                    .ok()
                    .map(|e| e.seq)
            })
            .max()
            .unwrap_or(0);

        Ok(max_seq + 1)
    }

    /// Get current git branch
    fn get_current_branch(&self) -> Result<String> {
        use std::process::Command;

        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .output()
            .context("Failed to get current branch")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!(
                "{}",
                errors::git_command_failed("git rev-parse --abbrev-ref HEAD", &stderr)
            );
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
    }

    /// Renew an existing lease
    ///
    /// # Arguments
    ///
    /// * `lease_id` - Lease to renew
    /// * `extension_secs` - How many seconds to extend the lease by
    ///
    /// # Returns
    ///
    /// Updated lease on success
    ///
    /// # Errors
    ///
    /// - Lease not found
    /// - Lease not owned by current agent
    /// - Lease already expired
    pub fn renew_lease(&self, lease_id: &str, extension_secs: u64) -> Result<Lease> {
        // 1. Acquire exclusive lock with metadata
        let lock_path = self.paths.shared_jit.join("locks/claims.lock");
        let _guard = self
            .locker
            .lock_exclusive_with_metadata(&lock_path, &self.agent_id)?;

        // 2. Load index and evict expired
        let mut index = self.load_claims_index()?;
        self.evict_expired(&mut index)?;

        // 3. Find the lease
        let lease = index
            .leases
            .iter()
            .find(|l| l.lease_id == lease_id)
            .ok_or_else(|| anyhow::anyhow!("{}", errors::lease_not_found(lease_id)))?;

        // 4. Verify ownership
        if lease.agent_id != self.agent_id {
            bail!(
                "{}",
                errors::not_owner(
                    &format!("lease {}", lease_id),
                    &lease.agent_id,
                    &self.agent_id
                )
            );
        }

        // 5. Calculate new expiry/heartbeat
        let now = Utc::now();
        let (new_expires_at, new_last_beat) = if lease.ttl_secs > 0 {
            // Finite lease: extend expiry by extension_secs
            let new_expiry = now + Duration::seconds(extension_secs as i64);
            (Some(new_expiry), lease.last_beat)
        } else {
            // Indefinite lease: update heartbeat
            (None, now)
        };

        // 6. Log renewal
        let op = ClaimOp::Renew {
            lease_id: lease_id.to_string(),
            new_expires_at,
            new_last_beat,
        };
        self.append_claim_op(&op)?;

        // 7. Update index
        let mut updated_lease = lease.clone();
        updated_lease.expires_at = new_expires_at;
        updated_lease.last_beat = new_last_beat;
        index.add_lease(updated_lease.clone());
        self.write_index_atomic(&index)?;

        Ok(updated_lease)
    }

    /// Release a lease explicitly
    ///
    /// # Arguments
    ///
    /// * `lease_id` - Lease to release
    ///
    /// # Errors
    ///
    /// - Lease not found
    /// - Lease not owned by current agent
    pub fn release_lease(&self, lease_id: &str) -> Result<()> {
        // 1. Acquire exclusive lock with metadata
        let lock_path = self.paths.shared_jit.join("locks/claims.lock");
        let _guard = self
            .locker
            .lock_exclusive_with_metadata(&lock_path, &self.agent_id)?;

        // 2. Load index
        let mut index = self.load_claims_index()?;

        // 3. Find the lease
        let lease = index
            .leases
            .iter()
            .find(|l| l.lease_id == lease_id)
            .ok_or_else(|| anyhow::anyhow!("{}", errors::lease_not_found(lease_id)))?;

        // 4. Verify ownership
        if lease.agent_id != self.agent_id {
            bail!(
                "{}",
                errors::not_owner(
                    &format!("lease {}", lease_id),
                    &lease.agent_id,
                    &self.agent_id
                )
            );
        }

        // 5. Log release
        let op = ClaimOp::Release {
            lease_id: lease_id.to_string(),
            released_at: Utc::now(),
        };
        self.append_claim_op(&op)?;

        // 6. Remove from index
        index.remove_lease(lease_id);
        self.write_index_atomic(&index)?;

        Ok(())
    }

    /// Force evict a lease (admin operation)
    ///
    /// # Arguments
    ///
    /// * `lease_id` - Lease to evict
    /// * `reason` - Reason for eviction
    ///
    /// # Errors
    ///
    /// - Lease not found
    pub fn force_evict_lease(&self, lease_id: &str, reason: &str) -> Result<()> {
        // 1. Acquire exclusive lock with metadata
        let lock_path = self.paths.shared_jit.join("locks/claims.lock");
        let _guard = self
            .locker
            .lock_exclusive_with_metadata(&lock_path, &self.agent_id)?;

        // 2. Load index
        let mut index = self.load_claims_index()?;

        // 3. Verify lease exists
        if !index.leases.iter().any(|l| l.lease_id == lease_id) {
            bail!("{}", errors::lease_not_found(lease_id));
        }

        // 4. Log eviction
        let op = ClaimOp::ForceEvict {
            lease_id: lease_id.to_string(),
            evicted_at: Utc::now(),
            reason: reason.to_string(),
        };
        self.append_claim_op(&op)?;

        // 5. Remove from index
        index.remove_lease(lease_id);
        self.write_index_atomic(&index)?;

        Ok(())
    }

    /// Get active leases with optional filtering
    ///
    /// # Arguments
    ///
    /// * `issue_id` - Optional filter by issue ID
    /// * `agent_id` - Optional filter by agent ID
    ///
    /// # Returns
    ///
    /// Vector of active leases matching the filters
    ///
    /// # Errors
    ///
    /// Returns an error if the index cannot be loaded
    pub fn get_active_leases(
        &self,
        issue_id: Option<&str>,
        agent_id: Option<&str>,
    ) -> Result<Vec<Lease>> {
        // 1. Acquire exclusive lock with metadata (needed for evict_expired which writes)
        let lock_path = self.paths.shared_jit.join("locks/claims.lock");
        let _guard = self
            .locker
            .lock_exclusive_with_metadata(&lock_path, &self.agent_id)?;

        // 2. Load index and evict expired leases
        let mut index = self.load_claims_index()?;
        self.evict_expired(&mut index)?;

        // 3. Filter leases
        let mut leases: Vec<Lease> = index
            .leases
            .into_iter()
            .filter(|lease| {
                if let Some(iid) = issue_id {
                    if lease.issue_id != iid {
                        return false;
                    }
                }
                if let Some(aid) = agent_id {
                    if lease.agent_id != aid {
                        return false;
                    }
                }
                true
            })
            .collect();

        // 4. Sort by acquired_at (most recent first)
        leases.sort_by(|a, b| b.acquired_at.cmp(&a.acquired_at));

        Ok(leases)
    }

    /// Rebuild claims index from JSONL log
    ///
    /// Reconstructs the index by replaying all operations from the append-only log.
    /// This is used for recovery from corruption or to ensure consistency.
    ///
    /// # Algorithm
    ///
    /// 1. Read all entries from claims.jsonl in sequence order
    /// 2. Apply operations to build active leases map:
    ///    - Acquire: add lease
    ///    - Renew: update expires_at and last_beat
    ///    - Release/AutoEvict/ForceEvict: remove lease
    /// 3. Filter out expired finite leases
    /// 4. Track highest sequence number
    ///
    /// # Returns
    ///
    /// A new ClaimsIndex with all active, non-expired leases
    ///
    /// # Errors
    ///
    /// Returns an error if the log file cannot be read or parsed
    pub fn rebuild_index_from_log(&self) -> Result<ClaimsIndex> {
        use std::collections::HashMap;
        use std::io::{BufRead, BufReader};

        let log_path = self.paths.shared_jit.join("claims.jsonl");
        let mut active: HashMap<String, Lease> = HashMap::new();
        let mut max_seq = 0u64;
        let mut expected_seq = 1u64;
        let mut sequence_gaps = Vec::new();

        if log_path.exists() {
            let file = fs::File::open(&log_path).context("Failed to open claims log")?;
            let reader = BufReader::new(file);

            for line in reader.lines() {
                let line = line.context("Failed to read line from claims log")?;
                if line.trim().is_empty() {
                    continue;
                }

                let entry: ClaimLogEntry =
                    serde_json::from_str(&line).context("Failed to parse claim log entry")?;

                // Check for sequence gaps
                if entry.seq != expected_seq {
                    // Detect missing sequences
                    for missing in expected_seq..entry.seq {
                        eprintln!(
                            "Warning: Sequence gap detected - missing sequence {}",
                            missing
                        );
                        sequence_gaps.push(missing);
                    }
                }
                expected_seq = entry.seq + 1;
                max_seq = max_seq.max(entry.seq);

                match entry.operation {
                    ClaimOp::Acquire { lease } => {
                        active.insert(lease.lease_id.clone(), lease);
                    }
                    ClaimOp::Renew {
                        lease_id,
                        new_expires_at,
                        new_last_beat,
                    } => {
                        if let Some(lease) = active.get_mut(&lease_id) {
                            lease.expires_at = new_expires_at;
                            lease.last_beat = new_last_beat;
                        }
                    }
                    ClaimOp::Release { lease_id, .. }
                    | ClaimOp::AutoEvict { lease_id, .. }
                    | ClaimOp::ForceEvict { lease_id, .. } => {
                        active.remove(&lease_id);
                    }
                }
            }
        }

        // Filter out expired finite leases
        let now = Utc::now();
        active.retain(|_, lease| {
            if lease.ttl_secs > 0 {
                // Finite lease - check expiration
                lease.expires_at.is_some_and(|exp| exp > now)
            } else {
                // Indefinite lease - keep it (staleness doesn't remove, just warns)
                true
            }
        });

        Ok(ClaimsIndex {
            schema_version: 1,
            generated_at: Utc::now(),
            last_seq: max_seq,
            stale_threshold_secs: 3600, // 1 hour default
            leases: active.into_values().collect(),
            sequence_gaps,
        })
    }

    /// Startup recovery routine - runs on every jit command
    ///
    /// Performs automatic recovery:
    /// 1. Cleanup stale locks from dead processes
    /// 2. Rebuild index if corrupted or inconsistent
    /// 3. Evict expired leases
    ///
    /// This is safe to call repeatedly and has minimal overhead when no recovery needed.
    pub fn startup_recovery(&self) -> Result<()> {
        // 1. Clean up stale locks
        let lock_dir = self.paths.shared_jit.join("locks");
        lock_cleanup::cleanup_stale_locks(&lock_dir)?;

        // 2. Rebuild index if corrupted
        if !self.verify_index_consistency()? {
            eprintln!("Warning: Claims index inconsistent, rebuilding from log...");
            let index = self.rebuild_index_from_log()?;
            self.write_index_atomic(&index)?;
        }

        // 3. Evict expired leases
        let mut index = self.load_claims_index()?;
        self.evict_expired(&mut index)?;
        self.write_index_atomic(&index)?;

        // 4. Clean up orphaned temp files (1 hour threshold)
        let jit_data_dir = &self.paths.local_jit;
        if let Err(e) = temp_cleanup::cleanup_orphaned_temp_files(jit_data_dir, 3600) {
            // Log but don't fail - temp file cleanup is best-effort
            eprintln!("Warning: Failed to cleanup orphaned temp files: {}", e);
        }

        Ok(())
    }

    /// Verify index consistency
    ///
    /// Returns true if index is valid, false if it needs rebuilding.
    ///
    /// Note: Does NOT check for expired leases - those are handled by evict_expired().
    /// Expired leases are normal state, not corruption. Treating them as corruption
    /// would trigger unnecessary full index rebuilds.
    pub fn verify_index_consistency(&self) -> Result<bool> {
        use std::collections::HashSet;

        let index_path = self.paths.shared_jit.join("claims.index.json");
        if !index_path.exists() {
            return Ok(false);
        }

        // Try to parse index
        let index: ClaimsIndex = match fs::read_to_string(&index_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(idx) => idx,
            None => return Ok(false), // Corrupted JSON
        };

        // Check for duplicates (actual corruption)
        let mut seen_issues = HashSet::new();
        for lease in &index.leases {
            if !seen_issues.insert(&lease.issue_id) {
                eprintln!(
                    "Error: Duplicate active lease for issue: {}",
                    lease.issue_id
                );
                return Ok(false);
            }
        }

        // All checks passed
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use std::time::Duration as StdDuration;
    use tempfile::TempDir;

    fn setup_coordinator(temp_dir: &TempDir) -> ClaimCoordinator {
        let paths = WorktreePaths {
            common_dir: temp_dir.path().join(".git"),
            worktree_root: temp_dir.path().to_path_buf(),
            local_jit: temp_dir.path().join(".jit"),
            shared_jit: temp_dir.path().join(".git/jit"),
        };

        let locker = FileLocker::new(StdDuration::from_secs(5));
        let coordinator = ClaimCoordinator::new(
            paths,
            locker,
            "wt:test123".to_string(),
            "agent:test".to_string(),
        );

        coordinator.init().unwrap();
        coordinator
    }

    #[test]
    fn test_acquire_claim_succeeds() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let lease = coordinator.acquire_claim("issue-001", 600).unwrap();

        assert_eq!(lease.issue_id, "issue-001");
        assert_eq!(lease.agent_id, "agent:test");
        assert_eq!(lease.worktree_id, "wt:test123");
        assert_eq!(lease.ttl_secs, 600);
        assert!(lease.expires_at.is_some());
    }

    #[test]
    fn test_acquire_claim_indefinite_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let lease = coordinator.acquire_claim("issue-002", 0).unwrap();

        assert_eq!(lease.ttl_secs, 0);
        assert!(lease.expires_at.is_none());
        assert_eq!(lease.last_beat, lease.acquired_at);
    }

    #[test]
    fn test_acquire_claim_already_claimed() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // First claim succeeds
        coordinator.acquire_claim("issue-003", 600).unwrap();

        // Second claim fails
        let result = coordinator.acquire_claim("issue-003", 600);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already claimed"));
    }

    #[test]
    fn test_concurrent_claim_attempts_serialize() {
        let temp_dir = TempDir::new().unwrap();
        let temp_path = Arc::new(temp_dir.path().to_path_buf());

        let barrier = Arc::new(Barrier::new(20)); // Increased from 5 to 20
        let successes = Arc::new(std::sync::Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..20) // Increased from 5 to 20
            .map(|i| {
                let temp_path = Arc::clone(&temp_path);
                let barrier = Arc::clone(&barrier);
                let successes = Arc::clone(&successes);

                thread::spawn(move || {
                    let paths = WorktreePaths {
                        common_dir: temp_path.join(".git"),
                        worktree_root: temp_path.to_path_buf(),
                        local_jit: temp_path.join(".jit"),
                        shared_jit: temp_path.join(".git/jit"),
                    };

                    let locker = FileLocker::new(StdDuration::from_secs(10));
                    let coordinator = ClaimCoordinator::new(
                        paths,
                        locker,
                        format!("wt:thread{}", i),
                        format!("agent:thread{}", i),
                    );

                    coordinator.init().unwrap();

                    // Synchronize start
                    barrier.wait();

                    // All try to claim same issue
                    if let Ok(lease) = coordinator.acquire_claim("issue-race", 600) {
                        successes.lock().unwrap().push(lease);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().unwrap();
        }

        // Exactly one should succeed
        let successes = successes.lock().unwrap();
        assert_eq!(
            successes.len(),
            1,
            "Exactly one thread should acquire the claim"
        );
    }

    #[test]
    fn test_claims_log_persisted() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        coordinator.acquire_claim("issue-004", 600).unwrap();

        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        assert!(log_path.exists());

        let content = fs::read_to_string(log_path).unwrap();
        assert!(content.contains("issue-004"));
        assert!(content.contains("acquire"));
    }

    #[test]
    fn test_claims_index_updated() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        coordinator.acquire_claim("issue-005", 600).unwrap();

        let index_path = temp_dir.path().join(".git/jit/claims.index.json");
        assert!(index_path.exists());

        let index = coordinator.load_claims_index().unwrap();
        assert_eq!(index.leases.len(), 1);
        assert_eq!(index.leases[0].issue_id, "issue-005");
    }

    #[test]
    fn test_expired_lease_auto_evicted() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create a lease that's already expired
        let mut index = coordinator.load_claims_index().unwrap();
        let expired_lease = Lease {
            lease_id: Uuid::new_v4().to_string(),
            issue_id: "issue-006".to_string(),
            agent_id: "agent:old".to_string(),
            worktree_id: "wt:old".to_string(),
            branch: None,
            ttl_secs: 60,
            acquired_at: Utc::now() - Duration::seconds(120),
            expires_at: Some(Utc::now() - Duration::seconds(60)),
            last_beat: Utc::now() - Duration::seconds(120),
        };
        index.add_lease(expired_lease);
        coordinator.write_index_atomic(&index).unwrap();

        // Now try to claim - should evict expired and succeed
        let lease = coordinator.acquire_claim("issue-006", 600).unwrap();
        assert_eq!(lease.agent_id, "agent:test");
    }

    #[test]
    fn test_renew_finite_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire a finite lease
        let lease = coordinator.acquire_claim("issue-007", 600).unwrap();
        let original_expiry = lease.expires_at.unwrap();

        // Wait a bit
        thread::sleep(StdDuration::from_millis(100));

        // Renew the lease
        let renewed = coordinator.renew_lease(&lease.lease_id, 600).unwrap();

        // Expiry should be extended
        assert!(renewed.expires_at.unwrap() > original_expiry);
        assert_eq!(renewed.issue_id, lease.issue_id);
        assert_eq!(renewed.agent_id, lease.agent_id);
    }

    #[test]
    fn test_renew_indefinite_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire an indefinite lease
        let lease = coordinator.acquire_claim("issue-008", 0).unwrap();
        let original_heartbeat = lease.last_beat;

        // Wait a bit
        thread::sleep(StdDuration::from_millis(100));

        // Renew (heartbeat) the lease
        let renewed = coordinator.renew_lease(&lease.lease_id, 0).unwrap();

        // Heartbeat should be updated
        assert!(renewed.last_beat > original_heartbeat);
        assert_eq!(renewed.ttl_secs, 0);
        assert!(renewed.expires_at.is_none());
    }

    #[test]
    fn test_renew_nonexistent_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let result = coordinator.renew_lease("fake-lease-id", 600);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_renew_lease_not_owned() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator1 = setup_coordinator(&temp_dir);

        // Agent 1 acquires lease
        let lease = coordinator1.acquire_claim("issue-009", 600).unwrap();

        // Create coordinator for different agent
        let paths = WorktreePaths {
            common_dir: temp_dir.path().join(".git"),
            worktree_root: temp_dir.path().to_path_buf(),
            local_jit: temp_dir.path().join(".jit"),
            shared_jit: temp_dir.path().join(".git/jit"),
        };
        let coordinator2 = ClaimCoordinator::new(
            paths,
            FileLocker::new(StdDuration::from_secs(5)),
            "wt:other".to_string(),
            "agent:other".to_string(),
        );

        // Agent 2 tries to renew - should fail
        let result = coordinator2.renew_lease(&lease.lease_id, 600);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owned by"));
    }

    #[test]
    fn test_release_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire and release
        let lease = coordinator.acquire_claim("issue-010", 600).unwrap();
        coordinator.release_lease(&lease.lease_id).unwrap();

        // Verify lease is gone from index
        let index = coordinator.load_claims_index().unwrap();
        assert!(index.find_lease("issue-010").is_none());

        // Should be able to claim again
        let new_lease = coordinator.acquire_claim("issue-010", 600).unwrap();
        assert_ne!(new_lease.lease_id, lease.lease_id); // Different lease
    }

    #[test]
    fn test_release_nonexistent_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let result = coordinator.release_lease("fake-lease-id");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_release_lease_not_owned() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator1 = setup_coordinator(&temp_dir);

        // Agent 1 acquires lease
        let lease = coordinator1.acquire_claim("issue-011", 600).unwrap();

        // Create coordinator for different agent
        let paths = WorktreePaths {
            common_dir: temp_dir.path().join(".git"),
            worktree_root: temp_dir.path().to_path_buf(),
            local_jit: temp_dir.path().join(".jit"),
            shared_jit: temp_dir.path().join(".git/jit"),
        };
        let coordinator2 = ClaimCoordinator::new(
            paths,
            FileLocker::new(StdDuration::from_secs(5)),
            "wt:other".to_string(),
            "agent:other".to_string(),
        );

        // Agent 2 tries to release - should fail
        let result = coordinator2.release_lease(&lease.lease_id);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("owned by"));
    }

    #[test]
    fn test_force_evict_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator1 = setup_coordinator(&temp_dir);

        // Agent 1 acquires lease
        let lease = coordinator1.acquire_claim("issue-012", 600).unwrap();

        // Create admin coordinator
        let paths = WorktreePaths {
            common_dir: temp_dir.path().join(".git"),
            worktree_root: temp_dir.path().to_path_buf(),
            local_jit: temp_dir.path().join(".jit"),
            shared_jit: temp_dir.path().join(".git/jit"),
        };
        let admin_coordinator = ClaimCoordinator::new(
            paths,
            FileLocker::new(StdDuration::from_secs(5)),
            "wt:admin".to_string(),
            "admin:alice".to_string(),
        );

        // Admin force evicts (no ownership check)
        admin_coordinator
            .force_evict_lease(&lease.lease_id, "stale after crash")
            .unwrap();

        // Verify lease is gone
        let index = coordinator1.load_claims_index().unwrap();
        assert!(index.find_lease("issue-012").is_none());

        // Should be able to claim again
        coordinator1.acquire_claim("issue-012", 600).unwrap();
    }

    #[test]
    fn test_force_evict_nonexistent_lease() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let result = coordinator.force_evict_lease("fake-lease-id", "test");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_operations_logged_to_audit_trail() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Perform various operations
        let lease = coordinator.acquire_claim("issue-013", 600).unwrap();
        coordinator.renew_lease(&lease.lease_id, 600).unwrap();
        coordinator.release_lease(&lease.lease_id).unwrap();

        // Check audit log
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        let content = fs::read_to_string(log_path).unwrap();

        assert!(content.contains("acquire"));
        assert!(content.contains("renew"));
        assert!(content.contains("release"));

        // Verify sequence ordering
        let lines: Vec<_> = content.lines().collect();
        assert!(lines.len() >= 3);
    }

    #[test]
    fn test_rebuild_index_from_empty_log() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let index = coordinator.rebuild_index_from_log().unwrap();

        assert_eq!(index.schema_version, 1);
        assert_eq!(index.last_seq, 0);
        assert_eq!(index.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_with_acquire_operations() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create some leases
        coordinator.acquire_claim("issue-rebuild-1", 600).unwrap();
        coordinator.acquire_claim("issue-rebuild-2", 600).unwrap();

        // Rebuild from log
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        assert_eq!(rebuilt.leases.len(), 2);
        assert!(rebuilt
            .leases
            .iter()
            .any(|l| l.issue_id == "issue-rebuild-1"));
        assert!(rebuilt
            .leases
            .iter()
            .any(|l| l.issue_id == "issue-rebuild-2"));
        assert_eq!(rebuilt.last_seq, 2);
    }

    #[test]
    fn test_rebuild_index_with_release_operation() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire and release
        let lease = coordinator.acquire_claim("issue-rebuild-3", 600).unwrap();
        coordinator.release_lease(&lease.lease_id).unwrap();

        // Rebuild should show no active leases
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_filters_expired_leases() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create lease that expires immediately
        coordinator.acquire_claim("issue-rebuild-4", 1).unwrap();

        // Wait for expiry
        thread::sleep(StdDuration::from_millis(1100));

        // Rebuild should filter out expired
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_with_renew_operation() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let lease = coordinator.acquire_claim("issue-rebuild-5", 600).unwrap();
        let original_expiry = lease.expires_at.unwrap();

        // Renew
        thread::sleep(StdDuration::from_millis(100));
        coordinator.renew_lease(&lease.lease_id, 600).unwrap();

        // Rebuild and verify updated expiry
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 1);
        let renewed = &rebuilt.leases[0];
        assert!(renewed.expires_at.unwrap() > original_expiry);
    }

    #[test]
    fn test_rebuild_index_with_evict_operations() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let lease = coordinator.acquire_claim("issue-rebuild-6", 600).unwrap();

        // Force evict
        coordinator
            .force_evict_lease(&lease.lease_id, "test eviction")
            .unwrap();

        // Rebuild should show no active leases
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();
        assert_eq!(rebuilt.leases.len(), 0);
    }

    #[test]
    fn test_rebuild_index_preserves_indefinite_leases() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create indefinite lease
        let lease = coordinator.acquire_claim("issue-rebuild-7", 0).unwrap();

        // Rebuild
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        assert_eq!(rebuilt.leases.len(), 1);
        let preserved = &rebuilt.leases[0];
        assert_eq!(preserved.ttl_secs, 0);
        assert!(preserved.expires_at.is_none());
        assert_eq!(preserved.lease_id, lease.lease_id);
    }

    #[test]
    fn test_rebuild_index_sequence_tracking() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Multiple operations
        coordinator.acquire_claim("issue-rebuild-8", 600).unwrap();
        coordinator.acquire_claim("issue-rebuild-9", 600).unwrap();
        coordinator.acquire_claim("issue-rebuild-10", 600).unwrap();

        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        // Should track last sequence number
        assert_eq!(rebuilt.last_seq, 3);
    }

    #[test]
    fn test_rebuild_detects_sequence_gaps() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create log with gap: sequences 1, 2, 4 (missing 3)
        let log_path = temp_dir.path().join(".git/jit/claims.jsonl");
        fs::create_dir_all(log_path.parent().unwrap()).unwrap();

        let entry1 = ClaimLogEntry {
            schema_version: 1,
            seq: 1,
            timestamp: Utc::now(),
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "lease-1".to_string(),
                    issue_id: "issue-1".to_string(),
                    agent_id: "agent:test".to_string(),
                    worktree_id: "wt:main".to_string(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::seconds(600)),
                    last_beat: Utc::now(),
                },
            },
        };

        let entry2 = ClaimLogEntry {
            schema_version: 1,
            seq: 2,
            timestamp: Utc::now(),
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "lease-2".to_string(),
                    issue_id: "issue-2".to_string(),
                    agent_id: "agent:test".to_string(),
                    worktree_id: "wt:main".to_string(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::seconds(600)),
                    last_beat: Utc::now(),
                },
            },
        };

        // Skip seq 3, jump to 4
        let entry4 = ClaimLogEntry {
            schema_version: 1,
            seq: 4,
            timestamp: Utc::now(),
            operation: ClaimOp::Acquire {
                lease: Lease {
                    lease_id: "lease-3".to_string(),
                    issue_id: "issue-3".to_string(),
                    agent_id: "agent:test".to_string(),
                    worktree_id: "wt:main".to_string(),
                    branch: None,
                    ttl_secs: 600,
                    acquired_at: Utc::now(),
                    expires_at: Some(Utc::now() + Duration::seconds(600)),
                    last_beat: Utc::now(),
                },
            },
        };

        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();

        use std::io::Write;
        writeln!(file, "{}", serde_json::to_string(&entry1).unwrap()).unwrap();
        writeln!(file, "{}", serde_json::to_string(&entry2).unwrap()).unwrap();
        writeln!(file, "{}", serde_json::to_string(&entry4).unwrap()).unwrap();
        drop(file);

        // Should succeed but detect gap
        let rebuilt = coordinator.rebuild_index_from_log().unwrap();

        // Should still work despite gap
        assert_eq!(rebuilt.leases.len(), 3);
        assert_eq!(rebuilt.last_seq, 4);
        assert!(
            rebuilt.sequence_gaps.contains(&3),
            "Should detect missing sequence 3"
        );
    }

    #[test]
    fn test_staleness_check() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let index = coordinator.load_claims_index().unwrap();

        // Stale indefinite lease (old heartbeat)
        let stale_lease = Lease {
            lease_id: Uuid::new_v4().to_string(),
            issue_id: "test".to_string(),
            agent_id: "agent:test".to_string(),
            worktree_id: "wt:test".to_string(),
            branch: None,
            ttl_secs: 0,
            acquired_at: Utc::now() - Duration::seconds(7200),
            expires_at: None,
            last_beat: Utc::now() - Duration::seconds(7200),
        };
        assert!(index.is_stale(&stale_lease));

        // Fresh indefinite lease (recent heartbeat)
        let fresh_lease = Lease {
            lease_id: Uuid::new_v4().to_string(),
            issue_id: "test".to_string(),
            agent_id: "agent:test".to_string(),
            worktree_id: "wt:test".to_string(),
            branch: None,
            ttl_secs: 0,
            acquired_at: Utc::now() - Duration::seconds(60),
            expires_at: None,
            last_beat: Utc::now() - Duration::seconds(60),
        };
        assert!(!index.is_stale(&fresh_lease));

        // Finite lease - never stale (uses expiration instead)
        let finite_lease = Lease {
            lease_id: Uuid::new_v4().to_string(),
            issue_id: "test".to_string(),
            agent_id: "agent:test".to_string(),
            worktree_id: "wt:test".to_string(),
            branch: None,
            ttl_secs: 600,
            acquired_at: Utc::now() - Duration::seconds(7200),
            expires_at: Some(Utc::now() + Duration::seconds(600)),
            last_beat: Utc::now() - Duration::seconds(7200),
        };
        assert!(!index.is_stale(&finite_lease));
    }

    #[test]
    fn test_startup_recovery_runs_successfully() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Should succeed without error
        coordinator.startup_recovery().unwrap();
    }

    #[test]
    fn test_startup_recovery_rebuilds_corrupted_index() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire a claim to create log entries and index
        let _lease = coordinator.acquire_claim("issue-001", 600).unwrap();

        // Verify index exists
        let index_path = temp_dir.path().join(".git/jit/claims.index.json");
        assert!(index_path.exists(), "Index should exist after claim");

        // Corrupt the index by writing invalid JSON
        fs::write(&index_path, "invalid json{{{").unwrap();

        // Recovery should rebuild from log
        coordinator.startup_recovery().unwrap();

        // Index should be valid and contain the lease
        let index = coordinator.load_claims_index().unwrap();
        assert_eq!(index.leases.len(), 1);
        assert_eq!(index.leases[0].issue_id, "issue-001");
    }

    #[test]
    fn test_startup_recovery_evicts_expired_leases() {
        use std::thread;
        use std::time::Duration as StdDuration;

        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create a lease with 1-second TTL
        coordinator.acquire_claim("issue-001", 1).unwrap();

        // Wait for it to expire
        thread::sleep(StdDuration::from_secs(2));

        // Recovery should evict the expired lease
        coordinator.startup_recovery().unwrap();

        // Index should be empty
        let index = coordinator.load_claims_index().unwrap();
        assert_eq!(index.leases.len(), 0, "Expired lease should be evicted");
    }

    #[test]
    fn test_startup_recovery_cleans_stale_locks() {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Create a lock directory in the correct location (shared control plane)
        let lock_dir = temp_dir.path().join(".git/jit/locks");
        fs::create_dir_all(&lock_dir).unwrap();

        // Create an unheld lock file (will be cleaned)
        let lock_path = lock_dir.join("stale.lock");
        fs::write(&lock_path, "").unwrap();

        // Create metadata with dead process PID
        let meta_path = lock_dir.join("stale.lock.meta");
        let metadata = crate::storage::lock::LockMetadata {
            pid: u32::MAX - 1,
            agent_id: "agent:dead".to_string(),
            created_at: Utc::now(),
            last_updated: Utc::now(),
        };
        fs::write(&meta_path, serde_json::to_string_pretty(&metadata).unwrap()).unwrap();

        // Recovery should remove stale lock
        coordinator.startup_recovery().unwrap();

        assert!(!lock_path.exists(), "Stale lock should be removed");
        assert!(!meta_path.exists(), "Stale lock metadata should be removed");
    }
}

// Include property-based tests
#[cfg(test)]
#[path = "claim_coordinator_proptests.rs"]
mod proptests;
