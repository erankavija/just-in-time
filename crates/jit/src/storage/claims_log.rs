//! Append-only JSONL log for claim operations.
//!
//! This module provides the foundational audit log for lease-based claim coordination.
//! All claim operations (acquire, renew, release, evictions) are recorded as immutable
//! entries in a JSONL file, providing a complete audit trail.
//!
//! # Design Principles
//!
//! - **Append-only**: Entries are never modified or deleted
//! - **Durability**: fsync after every append ensures crash consistency
//! - **Atomicity**: Uses file locking to prevent concurrent write conflicts
//! - **Ordered**: Sequence numbers provide total ordering of operations
//!
//! # Example
//!
//! ```no_run
//! use jit::storage::claims_log::{ClaimsLog, ClaimOperation};
//! use std::path::Path;
//!
//! let log = ClaimsLog::new(Path::new(".git/jit"));
//! let entry = ClaimOperation::Acquire {
//!     lease_id: "01ABC123".to_string(),
//!     issue_id: "issue-001".to_string(),
//!     agent_id: "agent:agent-1".to_string(),
//!     worktree_id: "wt:abc123".to_string(),
//!     branch: "main".to_string(),
//!     ttl_secs: 600,
//!     acquired_at: chrono::Utc::now(),
//!     expires_at: Some(chrono::Utc::now()),
//! };
//!
//! log.append(&entry).unwrap();
//! let entries = log.read_all().unwrap();
//! ```

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Schema version for claims log entries
const SCHEMA_VERSION: u32 = 1;

/// A claim operation entry in the audit log
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum ClaimOperation {
    /// Acquire a new lease on an issue
    Acquire {
        lease_id: String,
        issue_id: String,
        agent_id: String,
        worktree_id: String,
        branch: String,
        ttl_secs: u64,
        acquired_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
    },
    /// Renew an existing lease (extend TTL or update heartbeat)
    Renew {
        lease_id: String,
        ttl_secs: u64,
        renewed_at: DateTime<Utc>,
        expires_at: Option<DateTime<Utc>>,
    },
    /// Explicitly release a lease
    Release {
        lease_id: String,
        released_at: DateTime<Utc>,
        released_by: String,
    },
    /// Automatically evict an expired lease
    AutoEvict {
        lease_id: String,
        evicted_at: DateTime<Utc>,
        reason: String,
    },
    /// Force evict a lease (admin operation)
    ForceEvict {
        lease_id: String,
        evicted_at: DateTime<Utc>,
        by: String,
        reason: String,
    },
    /// Heartbeat update for indefinite lease (TTL=0)
    Heartbeat { lease_id: String, at: DateTime<Utc> },
}

/// A complete log entry with metadata
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ClaimLogEntry {
    /// Schema version for forward compatibility
    pub schema_version: u32,
    /// Monotonic sequence number for total ordering
    pub sequence: u64,
    /// Timestamp when entry was written
    pub timestamp: DateTime<Utc>,
    /// The claim operation
    #[serde(flatten)]
    pub operation: ClaimOperation,
}

/// Append-only JSONL log for claim operations
#[derive(Debug, Clone)]
pub struct ClaimsLog {
    /// Path to the control plane directory (.git/jit/)
    control_plane_dir: PathBuf,
}

impl ClaimsLog {
    /// Create a new claims log instance
    ///
    /// # Arguments
    ///
    /// * `control_plane_dir` - Path to `.git/jit/` directory
    pub fn new(control_plane_dir: &Path) -> Self {
        Self {
            control_plane_dir: control_plane_dir.to_path_buf(),
        }
    }

    /// Get the path to the claims.jsonl file
    fn log_path(&self) -> PathBuf {
        self.control_plane_dir.join("claims.jsonl")
    }

    /// Get the path to the claims lock file
    fn lock_path(&self) -> PathBuf {
        self.control_plane_dir.join("locks").join("claims.lock")
    }

    /// Append a claim operation to the log
    ///
    /// This operation is atomic and durable:
    /// - Acquires exclusive lock to prevent concurrent writes
    /// - Reads current sequence number
    /// - Appends new entry with next sequence
    /// - Fsyncs file to ensure durability
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Control plane directory doesn't exist
    /// - Lock cannot be acquired
    /// - File I/O fails
    /// - fsync fails
    pub fn append(&self, operation: &ClaimOperation) -> Result<u64> {
        // Ensure control plane directory exists
        std::fs::create_dir_all(&self.control_plane_dir)
            .context("Failed to create control plane directory")?;
        std::fs::create_dir_all(self.control_plane_dir.join("locks"))
            .context("Failed to create locks directory")?;

        // Acquire lock for atomic append
        let locker = crate::storage::FileLocker::new(std::time::Duration::from_secs(5));
        let _lock = locker
            .lock_exclusive(&self.lock_path())
            .context("Failed to acquire claims lock")?;

        // Determine next sequence number
        let sequence = self.next_sequence()?;

        // Create entry
        let entry = ClaimLogEntry {
            schema_version: SCHEMA_VERSION,
            sequence,
            timestamp: Utc::now(),
            operation: operation.clone(),
        };

        // Append to file
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())
            .context("Failed to open claims log")?;

        let json = serde_json::to_string(&entry).context("Failed to serialize claim entry")?;
        writeln!(file, "{}", json).context("Failed to write claim entry")?;

        // Ensure durability with fsync
        file.sync_all().context("Failed to fsync claims log")?;

        Ok(sequence)
    }

    /// Read all claim entries from the log
    ///
    /// Returns entries in chronological order (sequence order).
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - File cannot be opened (returns empty vec if file doesn't exist)
    /// - JSON deserialization fails (indicates corruption)
    pub fn read_all(&self) -> Result<Vec<ClaimLogEntry>> {
        let log_path = self.log_path();

        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&log_path).context("Failed to open claims log")?;
        let reader = BufReader::new(file);

        let mut entries = Vec::new();
        for (line_num, line) in reader.lines().enumerate() {
            let line = line.context("Failed to read line from claims log")?;
            let entry: ClaimLogEntry = serde_json::from_str(&line)
                .with_context(|| format!("Failed to parse claim entry at line {}", line_num + 1))?;
            entries.push(entry);
        }

        Ok(entries)
    }

    /// Get the next sequence number
    ///
    /// Reads all entries and returns max(sequence) + 1, or 1 if empty.
    fn next_sequence(&self) -> Result<u64> {
        let entries = self.read_all()?;
        Ok(entries.iter().map(|e| e.sequence).max().unwrap_or(0) + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup_test_log() -> (TempDir, ClaimsLog) {
        let temp_dir = TempDir::new().unwrap();
        let control_plane = temp_dir.path().join(".git/jit");
        std::fs::create_dir_all(&control_plane).unwrap();
        let log = ClaimsLog::new(&control_plane);
        (temp_dir, log)
    }

    #[test]
    fn test_append_and_read_acquire_operation() {
        let (_temp, log) = setup_test_log();

        let operation = ClaimOperation::Acquire {
            lease_id: "01ABC123".to_string(),
            issue_id: "issue-001".to_string(),
            agent_id: "agent:agent-1".to_string(),
            worktree_id: "wt:abc123".to_string(),
            branch: "main".to_string(),
            ttl_secs: 600,
            acquired_at: Utc::now(),
            expires_at: Some(Utc::now()),
        };

        let seq = log.append(&operation).unwrap();
        assert_eq!(seq, 1);

        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[0].schema_version, SCHEMA_VERSION);
        assert_eq!(entries[0].operation, operation);
    }

    #[test]
    fn test_append_multiple_operations() {
        let (_temp, log) = setup_test_log();

        let op1 = ClaimOperation::Acquire {
            lease_id: "01ABC123".to_string(),
            issue_id: "issue-001".to_string(),
            agent_id: "agent:agent-1".to_string(),
            worktree_id: "wt:abc123".to_string(),
            branch: "main".to_string(),
            ttl_secs: 600,
            acquired_at: Utc::now(),
            expires_at: Some(Utc::now()),
        };

        let op2 = ClaimOperation::Renew {
            lease_id: "01ABC123".to_string(),
            ttl_secs: 600,
            renewed_at: Utc::now(),
            expires_at: Some(Utc::now()),
        };

        let op3 = ClaimOperation::Release {
            lease_id: "01ABC123".to_string(),
            released_at: Utc::now(),
            released_by: "agent:agent-1".to_string(),
        };

        log.append(&op1).unwrap();
        log.append(&op2).unwrap();
        log.append(&op3).unwrap();

        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].sequence, 1);
        assert_eq!(entries[1].sequence, 2);
        assert_eq!(entries[2].sequence, 3);
    }

    #[test]
    fn test_sequence_numbers_monotonic() {
        let (_temp, log) = setup_test_log();

        let mut sequences = Vec::new();
        for i in 0..10 {
            let op = ClaimOperation::Heartbeat {
                lease_id: format!("lease-{}", i),
                at: Utc::now(),
            };
            let seq = log.append(&op).unwrap();
            sequences.push(seq);
        }

        // Verify sequences are strictly increasing
        for i in 1..sequences.len() {
            assert!(sequences[i] > sequences[i - 1]);
        }

        // Verify no gaps
        for (i, &seq) in sequences.iter().enumerate() {
            assert_eq!(seq, (i + 1) as u64);
        }
    }

    #[test]
    fn test_read_empty_log() {
        let (_temp, log) = setup_test_log();

        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn test_all_operation_types_serialize() {
        let (_temp, log) = setup_test_log();
        let now = Utc::now();

        let operations = vec![
            ClaimOperation::Acquire {
                lease_id: "01ABC".to_string(),
                issue_id: "issue-1".to_string(),
                agent_id: "agent:a1".to_string(),
                worktree_id: "wt:123".to_string(),
                branch: "main".to_string(),
                ttl_secs: 600,
                acquired_at: now,
                expires_at: Some(now),
            },
            ClaimOperation::Renew {
                lease_id: "01ABC".to_string(),
                ttl_secs: 600,
                renewed_at: now,
                expires_at: Some(now),
            },
            ClaimOperation::Release {
                lease_id: "01ABC".to_string(),
                released_at: now,
                released_by: "agent:a1".to_string(),
            },
            ClaimOperation::AutoEvict {
                lease_id: "01ABC".to_string(),
                evicted_at: now,
                reason: "expired".to_string(),
            },
            ClaimOperation::ForceEvict {
                lease_id: "01ABC".to_string(),
                evicted_at: now,
                by: "admin:alice".to_string(),
                reason: "stale".to_string(),
            },
            ClaimOperation::Heartbeat {
                lease_id: "01ABC".to_string(),
                at: now,
            },
        ];

        for op in &operations {
            log.append(op).unwrap();
        }

        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), operations.len());

        for (i, entry) in entries.iter().enumerate() {
            assert_eq!(entry.operation, operations[i]);
        }
    }

    #[test]
    fn test_concurrent_append_safety() {
        use std::sync::Arc;
        use std::thread;

        let temp_dir = TempDir::new().unwrap();
        let control_plane = temp_dir.path().join(".git/jit");
        std::fs::create_dir_all(&control_plane).unwrap();
        let log = Arc::new(ClaimsLog::new(&control_plane));

        let mut handles = vec![];
        for i in 0..10 {
            let log_clone = Arc::clone(&log);
            let handle = thread::spawn(move || {
                let op = ClaimOperation::Heartbeat {
                    lease_id: format!("lease-{}", i),
                    at: Utc::now(),
                };
                log_clone.append(&op).unwrap()
            });
            handles.push(handle);
        }

        let mut sequences: Vec<u64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        sequences.sort_unstable();

        // All sequences should be unique
        let unique_count = sequences
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len();
        assert_eq!(unique_count, 10);

        // Final log should have exactly 10 entries
        let entries = log.read_all().unwrap();
        assert_eq!(entries.len(), 10);

        // All sequence numbers from 1 to 10 should be present
        let mut entry_sequences: Vec<u64> = entries.iter().map(|e| e.sequence).collect();
        entry_sequences.sort_unstable();
        assert_eq!(entry_sequences, (1..=10).collect::<Vec<u64>>());
    }

    #[test]
    fn test_fsync_durability() {
        let (_temp, log) = setup_test_log();

        let op = ClaimOperation::Acquire {
            lease_id: "01ABC".to_string(),
            issue_id: "issue-1".to_string(),
            agent_id: "agent:a1".to_string(),
            worktree_id: "wt:123".to_string(),
            branch: "main".to_string(),
            ttl_secs: 600,
            acquired_at: Utc::now(),
            expires_at: Some(Utc::now()),
        };

        log.append(&op).unwrap();

        // Verify file exists and is readable
        assert!(log.log_path().exists());

        // Read directly from file to verify fsync worked
        let content = std::fs::read_to_string(log.log_path()).unwrap();
        assert!(content.contains("\"op\":\"acquire\""));
    }
}
