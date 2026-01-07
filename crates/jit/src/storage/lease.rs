//! Lease management with monotonic time semantics for automatic expiration.
//!
//! This module implements time-based lease expiration using monotonic clocks to avoid
//! issues with system time adjustments (NTP, manual changes). Supports both finite
//! leases (with TTL) and indefinite leases (TTL=0) with staleness detection.
//!
//! # Design Principles
//!
//! - **Monotonic time for expiry**: Use `Instant` for TTL checks, immune to wall-clock changes
//! - **Wall-clock for audit**: Store `DateTime<Utc>` for human-readable timestamps
//! - **Lazy expiration**: Check and evict expired leases during claim operations
//! - **Staleness for TTL=0**: Indefinite leases marked stale but not auto-evicted
//!
//! # Example
//!
//! ```no_run
//! use jit::storage::lease::Lease;
//! use chrono::Utc;
//!
//! // Create a finite lease with 600 second TTL
//! let lease = Lease::new(
//!     "01ABC123".to_string(),
//!     "issue-001".to_string(),
//!     "agent:agent-1".to_string(),
//!     "wt:abc123".to_string(),
//!     "main".to_string(),
//!     600,
//! );
//!
//! // Check if expired (uses monotonic time)
//! assert!(!lease.is_expired());
//!
//! // Create indefinite lease (TTL=0)
//! let indefinite = Lease::new(
//!     "01XYZ789".to_string(),
//!     "issue-002".to_string(),
//!     "agent:agent-2".to_string(),
//!     "wt:def456".to_string(),
//!     "feature-branch".to_string(),
//!     0,
//! );
//! assert!(!indefinite.is_expired()); // Never expires
//! ```

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Default staleness threshold for indefinite leases (1 hour)
pub const DEFAULT_STALE_THRESHOLD_SECS: u64 = 3600;

/// A lease on an issue with time-based expiration.
///
/// Uses dual-clock approach:
/// - `Instant` (monotonic) for reliable TTL checks
/// - `DateTime<Utc>` (wall-clock) for audit trail
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    /// Unique lease identifier (ULID)
    pub lease_id: String,
    /// Issue being claimed
    pub issue_id: String,
    /// Agent holding the lease
    pub agent_id: String,
    /// Worktree where work is happening
    pub worktree_id: String,
    /// Branch where work is happening
    pub branch: String,
    /// Time-to-live in seconds (0 = indefinite)
    pub ttl_secs: u64,
    /// When lease was acquired (wall-clock, for audit)
    pub acquired_at: DateTime<Utc>,
    /// When lease expires (wall-clock, for audit). None if TTL=0
    pub expires_at: Option<DateTime<Utc>>,
    /// Last heartbeat timestamp (for staleness checks)
    pub last_beat: DateTime<Utc>,

    /// Monotonic clock reference (not serialized, reconstructed on load)
    #[serde(skip)]
    acquired_instant: Option<Instant>,
}

impl Lease {
    /// Create a new lease with current timestamps.
    ///
    /// # Arguments
    ///
    /// * `lease_id` - Unique ULID identifier
    /// * `issue_id` - Issue being claimed
    /// * `agent_id` - Agent identifier (format: "type:identifier")
    /// * `worktree_id` - Worktree identifier (format: "wt:hash")
    /// * `branch` - Branch name
    /// * `ttl_secs` - Time-to-live in seconds (0 = indefinite)
    pub fn new(
        lease_id: String,
        issue_id: String,
        agent_id: String,
        worktree_id: String,
        branch: String,
        ttl_secs: u64,
    ) -> Self {
        let now_utc = Utc::now();
        let now_instant = Instant::now();

        let expires_at = if ttl_secs > 0 {
            Some(now_utc + ChronoDuration::seconds(ttl_secs as i64))
        } else {
            None
        };

        Self {
            lease_id,
            issue_id,
            agent_id,
            worktree_id,
            branch,
            ttl_secs,
            acquired_at: now_utc,
            expires_at,
            last_beat: now_utc,
            acquired_instant: Some(now_instant),
        }
    }

    /// Reconstruct lease from serialized data with monotonic time approximation.
    ///
    /// Since `Instant` cannot be serialized, we reconstruct it from the UTC timestamp
    /// by calculating elapsed time and subtracting from current instant.
    ///
    /// This is a conservative approximation that may extend lease lifetime slightly
    /// but never shortens it (safety-first approach).
    pub fn from_serde(lease: Lease) -> Self {
        let elapsed_secs = Utc::now()
            .signed_duration_since(lease.acquired_at)
            .num_seconds()
            .max(0) as u64;

        let acquired_instant = Instant::now()
            .checked_sub(std::time::Duration::from_secs(elapsed_secs))
            .or(Some(Instant::now()));

        Self {
            acquired_instant,
            ..lease
        }
    }

    /// Check if this lease has expired (for finite leases only).
    ///
    /// Uses monotonic `Instant` for reliable expiry checks immune to NTP adjustments.
    /// Falls back to wall-clock comparison if `Instant` is unavailable.
    ///
    /// # Returns
    ///
    /// - `true` if TTL > 0 and lease has expired
    /// - `false` if TTL = 0 (indefinite lease never expires)
    /// - `false` if lease is still valid
    pub fn is_expired(&self) -> bool {
        if self.ttl_secs == 0 {
            return false; // Indefinite leases never expire
        }

        match self.acquired_instant {
            Some(instant) => instant.elapsed().as_secs() >= self.ttl_secs,
            None => {
                // Fallback to wall-clock (less reliable but safe)
                match self.expires_at {
                    Some(expires_at) => Utc::now() >= expires_at,
                    None => false,
                }
            }
        }
    }

    /// Check if this lease is stale (for indefinite leases).
    ///
    /// An indefinite lease (TTL=0) is stale if too much time has passed since
    /// the last heartbeat. Stale leases block structural edits but aren't auto-evicted.
    ///
    /// # Arguments
    ///
    /// * `stale_threshold_secs` - Maximum time since last heartbeat before marked stale
    ///
    /// # Returns
    ///
    /// - `true` if TTL = 0 and `now - last_beat > stale_threshold_secs`
    /// - `false` otherwise
    pub fn is_stale(&self, stale_threshold_secs: u64) -> bool {
        if self.ttl_secs > 0 {
            return false; // Finite leases use expiry, not staleness
        }

        let elapsed_secs = Utc::now()
            .signed_duration_since(self.last_beat)
            .num_seconds()
            .max(0) as u64;

        elapsed_secs >= stale_threshold_secs
    }

    /// Update last heartbeat timestamp to current time.
    ///
    /// Used for renewing indefinite leases (TTL=0) without changing expiry.
    pub fn update_heartbeat(&mut self) {
        self.last_beat = Utc::now();
    }

    /// Renew a finite lease by extending its TTL.
    ///
    /// Updates `expires_at` by adding TTL duration from current time.
    /// Also updates monotonic instant reference.
    ///
    /// # Arguments
    ///
    /// * `additional_ttl_secs` - Additional seconds to extend the lease
    pub fn renew(&mut self, additional_ttl_secs: u64) {
        if self.ttl_secs == 0 {
            // For indefinite leases, just update heartbeat
            self.update_heartbeat();
            return;
        }

        let now_utc = Utc::now();
        let now_instant = Instant::now();

        self.expires_at = Some(now_utc + ChronoDuration::seconds(additional_ttl_secs as i64));
        self.last_beat = now_utc;
        self.acquired_instant = Some(now_instant);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn test_new_lease_finite_ttl() {
        let lease = Lease::new(
            "01ABC123".to_string(),
            "issue-001".to_string(),
            "agent:agent-1".to_string(),
            "wt:abc123".to_string(),
            "main".to_string(),
            600,
        );

        assert_eq!(lease.lease_id, "01ABC123");
        assert_eq!(lease.issue_id, "issue-001");
        assert_eq!(lease.ttl_secs, 600);
        assert!(lease.expires_at.is_some());
        assert!(!lease.is_expired());
        assert!(!lease.is_stale(DEFAULT_STALE_THRESHOLD_SECS));
    }

    #[test]
    fn test_new_lease_indefinite_ttl() {
        let lease = Lease::new(
            "01XYZ789".to_string(),
            "issue-002".to_string(),
            "agent:agent-2".to_string(),
            "wt:def456".to_string(),
            "feature-branch".to_string(),
            0,
        );

        assert_eq!(lease.ttl_secs, 0);
        assert!(lease.expires_at.is_none());
        assert!(!lease.is_expired());
        assert!(!lease.is_stale(DEFAULT_STALE_THRESHOLD_SECS));
    }

    #[test]
    fn test_finite_lease_expiration_monotonic() {
        // Create lease with 1 second TTL
        let lease = Lease::new(
            "01EXPIRE".to_string(),
            "issue-exp".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            1,
        );

        assert!(!lease.is_expired());

        // Wait for expiration (with extra margin for reliability)
        thread::sleep(Duration::from_millis(1200));

        assert!(lease.is_expired());
    }

    #[test]
    fn test_indefinite_lease_never_expires() {
        let lease = Lease::new(
            "01FOREVER".to_string(),
            "issue-forever".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            0,
        );

        // Even after waiting, indefinite lease doesn't expire
        thread::sleep(Duration::from_millis(100));
        assert!(!lease.is_expired());
    }

    #[test]
    fn test_indefinite_lease_staleness() {
        let mut lease = Lease::new(
            "01STALE".to_string(),
            "issue-stale".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            0,
        );

        // Initially not stale
        assert!(!lease.is_stale(1));

        // Wait to become stale (with extra margin for reliability)
        thread::sleep(Duration::from_millis(1200));
        assert!(lease.is_stale(1)); // 1 second threshold

        // Update heartbeat refreshes staleness
        lease.update_heartbeat();
        assert!(!lease.is_stale(1));
    }

    #[test]
    fn test_finite_lease_not_stale() {
        let lease = Lease::new(
            "01FINITE".to_string(),
            "issue-finite".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            600,
        );

        // Finite leases never marked stale (they expire instead)
        assert!(!lease.is_stale(0));
    }

    #[test]
    fn test_lease_serialization_roundtrip() {
        let original = Lease::new(
            "01SERIAL".to_string(),
            "issue-serial".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            600,
        );

        // Serialize to JSON
        let json = serde_json::to_string(&original).unwrap();

        // Deserialize back
        let deserialized: Lease = serde_json::from_str(&json).unwrap();

        // Instant should be None after deserialization
        assert!(deserialized.acquired_instant.is_none());

        // Reconstruct instant
        let reconstructed = Lease::from_serde(deserialized);
        assert!(reconstructed.acquired_instant.is_some());

        // Should still report not expired
        assert!(!reconstructed.is_expired());
    }

    #[test]
    fn test_instant_reconstruction_approximation() {
        let lease = Lease::new(
            "01APPROX".to_string(),
            "issue-approx".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            10,
        );

        // Serialize and deserialize
        let json = serde_json::to_string(&lease).unwrap();
        let deserialized: Lease = serde_json::from_str(&json).unwrap();

        // Wait a bit
        thread::sleep(Duration::from_millis(100));

        // Reconstruct
        let reconstructed = Lease::from_serde(deserialized);

        // Should still be valid (conservative approximation)
        assert!(!reconstructed.is_expired());
    }

    #[test]
    fn test_renew_finite_lease() {
        let mut lease = Lease::new(
            "01RENEW".to_string(),
            "issue-renew".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            1,
        );

        // Wait almost to expiry
        thread::sleep(Duration::from_millis(900));

        // Renew with additional 10 seconds
        lease.renew(10);

        // Should not be expired now
        assert!(!lease.is_expired());
    }

    #[test]
    fn test_renew_indefinite_lease_updates_heartbeat() {
        let mut lease = Lease::new(
            "01RENEW-INF".to_string(),
            "issue-renew-inf".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            0,
        );

        let original_heartbeat = lease.last_beat;
        thread::sleep(Duration::from_millis(100));

        lease.renew(0);

        assert!(lease.last_beat > original_heartbeat);
        assert!(!lease.is_stale(1));
    }

    #[test]
    fn test_fallback_to_wall_clock_when_instant_missing() {
        let mut lease = Lease::new(
            "01FALLBACK".to_string(),
            "issue-fallback".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            1,
        );

        // Remove instant to force fallback
        lease.acquired_instant = None;

        assert!(!lease.is_expired());

        // Wait for expiration
        thread::sleep(Duration::from_millis(1100));

        // Should detect expiry via wall-clock fallback
        assert!(lease.is_expired());
    }

    #[test]
    fn test_update_heartbeat() {
        let mut lease = Lease::new(
            "01HEARTBEAT".to_string(),
            "issue-heartbeat".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            0,
        );

        let original_heartbeat = lease.last_beat;
        thread::sleep(Duration::from_millis(100));

        lease.update_heartbeat();

        assert!(lease.last_beat > original_heartbeat);
    }
}
