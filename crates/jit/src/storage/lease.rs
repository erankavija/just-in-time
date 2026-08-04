//! Lease expiration and staleness against a supplied clock.
//!
//! A lease records when it was acquired, when it expires, and when it last beat.
//! Every question about those instants — has it expired, has it gone stale — is
//! answered against a [`Clock`] the caller supplies, so the answer is a property
//! of the lease and the instant rather than of when the process got around to
//! asking. Production supplies [`SystemClock`](super::clock::SystemClock); a
//! test supplies a clock it moves itself, and reaches any boundary without
//! waiting for one.
//!
//! # Design Principles
//!
//! - **Supplied time**: expiry and staleness read the caller's clock
//! - **Wall-clock state**: instants are `DateTime<Utc>`, serializable and readable
//! - **Lazy expiration**: check and evict expired leases during claim operations
//! - **Staleness for TTL=0**: indefinite leases marked stale but not auto-evicted
//!
//! # Example
//!
//! ```no_run
//! use jit::storage::clock::SystemClock;
//! use jit::storage::lease::Lease;
//!
//! let clock = SystemClock;
//!
//! // A finite lease with a 600 second TTL
//! let lease = Lease::new(
//!     "01ABC123".to_string(),
//!     "issue-001".to_string(),
//!     "agent:agent-1".to_string(),
//!     "wt:abc123".to_string(),
//!     "main".to_string(),
//!     600,
//!     &clock,
//! );
//! assert!(!lease.is_expired(&clock));
//!
//! // An indefinite lease (TTL=0)
//! let indefinite = Lease::new(
//!     "01XYZ789".to_string(),
//!     "issue-002".to_string(),
//!     "agent:agent-2".to_string(),
//!     "wt:def456".to_string(),
//!     "feature-branch".to_string(),
//!     0,
//!     &clock,
//! );
//! assert!(!indefinite.is_expired(&clock));
//! ```

use super::clock::Clock;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde::{Deserialize, Serialize};

/// Default staleness threshold for indefinite leases (1 hour)
pub const DEFAULT_STALE_THRESHOLD_SECS: u64 = 3600;

/// A lease on an issue, expiring against a supplied clock.
///
/// Every instant it holds is wall-clock `DateTime<Utc>`, so a serialized lease
/// round-trips complete and answers the same questions after a reload.
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
    /// When lease was acquired
    pub acquired_at: DateTime<Utc>,
    /// When lease expires. None if TTL=0
    pub expires_at: Option<DateTime<Utc>>,
    /// Last heartbeat timestamp (for staleness checks)
    pub last_beat: DateTime<Utc>,
}

impl Lease {
    /// Create a lease acquired at `clock`'s current instant.
    ///
    /// # Arguments
    ///
    /// * `lease_id` - Unique ULID identifier
    /// * `issue_id` - Issue being claimed
    /// * `agent_id` - Agent identifier (format: "type:identifier")
    /// * `worktree_id` - Worktree identifier (format: "wt:hash")
    /// * `branch` - Branch name
    /// * `ttl_secs` - Time-to-live in seconds (0 = indefinite)
    /// * `clock` - Source of the acquisition instant
    pub fn new(
        lease_id: String,
        issue_id: String,
        agent_id: String,
        worktree_id: String,
        branch: String,
        ttl_secs: u64,
        clock: &dyn Clock,
    ) -> Self {
        let now = clock.now();
        Self {
            lease_id,
            issue_id,
            agent_id,
            worktree_id,
            branch,
            ttl_secs,
            acquired_at: now,
            expires_at: (ttl_secs > 0).then(|| now + ChronoDuration::seconds(ttl_secs as i64)),
            last_beat: now,
        }
    }

    /// Whether this lease has expired at `clock`'s current instant.
    ///
    /// # Returns
    ///
    /// - `true` if TTL > 0 and that instant has reached `expires_at`
    /// - `false` if TTL = 0 (indefinite lease never expires)
    /// - `false` if TTL > 0 with no recorded expiry
    pub fn is_expired(&self, clock: &dyn Clock) -> bool {
        match self.expires_at {
            Some(expires_at) if self.ttl_secs > 0 => clock.now() >= expires_at,
            _ => false,
        }
    }

    /// Whether this indefinite lease has gone stale at `clock`'s current instant.
    ///
    /// An indefinite lease (TTL=0) is stale once `stale_threshold_secs` have
    /// passed since its last heartbeat. Stale leases block structural edits
    /// without being auto-evicted.
    ///
    /// # Returns
    ///
    /// - `true` if TTL = 0 and `now - last_beat >= stale_threshold_secs`
    /// - `false` otherwise, including every finite lease, which expires instead
    pub fn is_stale(&self, stale_threshold_secs: u64, clock: &dyn Clock) -> bool {
        if self.ttl_secs > 0 {
            return false;
        }

        let elapsed_secs = clock
            .now()
            .signed_duration_since(self.last_beat)
            .num_seconds()
            .max(0) as u64;

        elapsed_secs >= stale_threshold_secs
    }

    /// Move the last heartbeat to `clock`'s current instant.
    ///
    /// Renews an indefinite lease (TTL=0) without changing expiry.
    pub fn update_heartbeat(&mut self, clock: &dyn Clock) {
        self.last_beat = clock.now();
    }

    /// Renew a finite lease so it expires `additional_ttl_secs` after `clock`'s
    /// current instant.
    ///
    /// An indefinite lease has no expiry to extend, so this beats it instead.
    pub fn renew(&mut self, additional_ttl_secs: u64, clock: &dyn Clock) {
        if self.ttl_secs == 0 {
            self.update_heartbeat(clock);
            return;
        }

        let now = clock.now();
        self.expires_at = Some(now + ChronoDuration::seconds(additional_ttl_secs as i64));
        self.last_beat = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::clock::FixedClock;

    /// A lease of `ttl_secs` acquired at the clock's current instant.
    fn lease_at(clock: &FixedClock, ttl_secs: u64) -> Lease {
        Lease::new(
            "01LEASE".to_string(),
            "issue-001".to_string(),
            "agent:test".to_string(),
            "wt:test".to_string(),
            "main".to_string(),
            ttl_secs,
            clock,
        )
    }

    /// A clock a test moves itself, with the instant it starts at read back
    /// from the clock so both sides of an assertion share its resolution.
    fn test_clock() -> (FixedClock, DateTime<Utc>) {
        let clock = FixedClock::new(Utc::now());
        let base = clock.now();
        (clock, base)
    }

    #[test]
    fn test_new_records_the_acquisition_instant_and_derives_expiry_from_the_ttl() {
        let (clock, base) = test_clock();
        let ttl_secs = 600;

        let lease = lease_at(&clock, ttl_secs);

        assert_eq!(lease.acquired_at, base);
        assert_eq!(lease.last_beat, base);
        assert_eq!(
            lease.expires_at,
            Some(base + ChronoDuration::seconds(ttl_secs as i64)),
            "a finite lease expires its whole TTL after it was acquired"
        );
    }

    #[test]
    fn test_new_indefinite_lease_records_no_expiry() {
        let (clock, _) = test_clock();

        let lease = lease_at(&clock, 0);

        assert!(
            lease.expires_at.is_none(),
            "an indefinite lease has no instant to expire at"
        );
        assert!(!lease.is_expired(&clock));
        assert!(!lease.is_stale(DEFAULT_STALE_THRESHOLD_SECS, &clock));
    }

    #[test]
    fn test_is_expired_turns_over_exactly_at_the_recorded_expiry() {
        let (clock, base) = test_clock();
        let ttl_secs = 600;
        let lease = lease_at(&clock, ttl_secs);
        let expiry = lease.expires_at.unwrap();

        clock.set(expiry - ChronoDuration::milliseconds(1));
        assert!(
            !lease.is_expired(&clock),
            "a lease is live for every instant before its expiry"
        );

        clock.set(expiry);
        assert!(
            lease.is_expired(&clock),
            "a lease is expired from its expiry onward"
        );

        clock.set(base + ChronoDuration::seconds(ttl_secs as i64 * 100));
        assert!(lease.is_expired(&clock), "and stays expired after it");
    }

    #[test]
    fn test_is_expired_is_false_for_an_indefinite_lease_at_any_instant() {
        let (clock, base) = test_clock();
        let lease = lease_at(&clock, 0);

        clock.set(base + ChronoDuration::days(365));

        assert!(!lease.is_expired(&clock));
    }

    #[test]
    fn test_is_expired_is_false_for_a_finite_lease_with_no_recorded_expiry() {
        let (clock, base) = test_clock();
        let mut lease = lease_at(&clock, 600);
        lease.expires_at = None;

        clock.set(base + ChronoDuration::days(365));

        assert!(
            !lease.is_expired(&clock),
            "with nothing recorded to expire at, no instant expires it"
        );
    }

    #[test]
    fn test_is_stale_turns_over_at_the_threshold_and_a_heartbeat_resets_it() {
        let (clock, base) = test_clock();
        let threshold_secs = 60;
        let mut lease = lease_at(&clock, 0);

        clock.set(base + ChronoDuration::seconds(threshold_secs as i64 - 1));
        assert!(
            !lease.is_stale(threshold_secs, &clock),
            "a lease is fresh for every instant short of the threshold"
        );

        clock.set(base + ChronoDuration::seconds(threshold_secs as i64));
        assert!(
            lease.is_stale(threshold_secs, &clock),
            "a lease is stale once the threshold has passed since its last beat"
        );

        lease.update_heartbeat(&clock);
        assert!(
            !lease.is_stale(threshold_secs, &clock),
            "a heartbeat measures staleness from the instant it was taken"
        );
    }

    #[test]
    fn test_is_stale_is_false_for_a_finite_lease_however_far_the_clock_moves() {
        let (clock, base) = test_clock();
        let lease = lease_at(&clock, 600);

        clock.set(base + ChronoDuration::days(365));

        assert!(
            !lease.is_stale(0, &clock),
            "a finite lease expires rather than going stale"
        );
    }

    #[test]
    fn test_serialization_roundtrip_preserves_the_lease_and_its_verdicts() {
        let (clock, base) = test_clock();
        let original = lease_at(&clock, 600);

        let restored: Lease = serde_json::from_str(&serde_json::to_string(&original).unwrap())
            .expect("a lease round-trips through JSON");

        assert_eq!(restored.acquired_at, original.acquired_at);
        assert_eq!(restored.expires_at, original.expires_at);
        assert_eq!(restored.last_beat, original.last_beat);
        clock.set(base + ChronoDuration::seconds(599));
        assert_eq!(restored.is_expired(&clock), original.is_expired(&clock));
        clock.set(base + ChronoDuration::seconds(600));
        assert_eq!(
            restored.is_expired(&clock),
            original.is_expired(&clock),
            "a reloaded lease expires at the same instant as the one it was written from"
        );
    }

    #[test]
    fn test_renew_moves_a_finite_lease_expiry_to_the_renewal_instant_plus_the_ttl() {
        let (clock, base) = test_clock();
        let mut lease = lease_at(&clock, 1);
        let renewed_at = base + ChronoDuration::milliseconds(900);
        let additional_ttl_secs = 10;

        clock.set(renewed_at);
        lease.renew(additional_ttl_secs, &clock);

        assert_eq!(
            lease.expires_at,
            Some(renewed_at + ChronoDuration::seconds(additional_ttl_secs as i64))
        );
        clock.set(base + ChronoDuration::seconds(1));
        assert!(
            !lease.is_expired(&clock),
            "the instant the original TTL would have expired at is inside the renewed one"
        );
    }

    #[test]
    fn test_renew_beats_an_indefinite_lease_rather_than_giving_it_an_expiry() {
        let (clock, base) = test_clock();
        let mut lease = lease_at(&clock, 0);
        let renewed_at = base + ChronoDuration::seconds(30);

        clock.set(renewed_at);
        lease.renew(0, &clock);

        assert_eq!(lease.last_beat, renewed_at);
        assert!(lease.expires_at.is_none());
        assert!(!lease.is_stale(60, &clock));
    }

    #[test]
    fn test_update_heartbeat_records_the_clock_instant() {
        let (clock, base) = test_clock();
        let mut lease = lease_at(&clock, 0);
        let beat_at = base + ChronoDuration::seconds(30);

        clock.set(beat_at);
        lease.update_heartbeat(&clock);

        assert_eq!(lease.last_beat, beat_at);
    }
}
