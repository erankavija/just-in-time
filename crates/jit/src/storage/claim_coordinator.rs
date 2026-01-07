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
use super::worktree_paths::WorktreePaths;

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
        // 1. Acquire exclusive lock
        let lock_path = self.paths.shared_jit.join("locks/claims.lock");
        fs::create_dir_all(lock_path.parent().unwrap())?;
        let _guard = self.locker.lock_exclusive(&lock_path)?;

        // 2. Load index and evict expired leases
        let mut index = self.load_claims_index()?;
        self.evict_expired(&mut index)?;

        // 3. Check availability
        if let Some(existing) = index.find_lease(issue_id) {
            if existing.ttl_secs == 0 {
                bail!(
                    "Issue {} already claimed by {} (indefinite lease, last beat: {})",
                    issue_id,
                    existing.agent_id,
                    existing.last_beat
                );
            } else {
                bail!(
                    "Issue {} already claimed by {} until {}",
                    issue_id,
                    existing.agent_id,
                    existing.expires_at.unwrap()
                );
            }
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
    fn load_claims_index(&self) -> Result<ClaimsIndex> {
        let index_path = self.paths.shared_jit.join("claims.index.json");

        if !index_path.exists() {
            return Ok(ClaimsIndex {
                schema_version: 1,
                generated_at: Utc::now(),
                last_seq: 0,
                stale_threshold_secs: 3600, // 1 hour default
                leases: Vec::new(),
            });
        }

        let content = fs::read_to_string(&index_path).context("Failed to read claims index")?;
        serde_json::from_str(&content).context("Failed to parse claims index")
    }

    /// Write claims index atomically (temp + rename)
    fn write_index_atomic(&self, index: &ClaimsIndex) -> Result<()> {
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
    fn evict_expired(&self, index: &mut ClaimsIndex) -> Result<()> {
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
            bail!("git rev-parse failed");
        }

        Ok(String::from_utf8(output.stdout)?.trim().to_string())
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

        let barrier = Arc::new(Barrier::new(5));
        let successes = Arc::new(std::sync::Mutex::new(Vec::new()));

        let handles: Vec<_> = (0..5)
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
}
