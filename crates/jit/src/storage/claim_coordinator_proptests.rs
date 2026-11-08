//! Property-based tests for claim coordination invariants
//!
//! These tests use `proptest` to verify system invariants across randomly
//! generated operation sequences, catching edge cases that example-based
//! tests might miss.

use super::*;
use proptest::prelude::*;
use std::sync::{Arc, Barrier};
use std::thread;
use tempfile::TempDir;

// Helper to set up a test coordinator
fn setup_coordinator(temp_dir: &TempDir) -> ClaimCoordinator {
    let mut paths = crate::test_utils::primary_checkout_paths(temp_dir.path());
    // The directory is no checkout, so every acquisition's branch probe would
    // fork git to learn there is none; no property here observes the branch.
    paths.git_repository = false;

    // These properties exercise coordination, not crash durability.
    let coordinator = ClaimCoordinator::new(
        paths,
        contending_locker(),
        "wt:proptest".parse().unwrap(),
        "agent:proptest".parse().unwrap(),
    )
    .with_fsync(false);

    coordinator.init().unwrap();
    coordinator
}

// The lock wait decides how long a caller queues before asking the coordinator
// again, not what it is told. A zero wait makes every refusal an immediate retry
// in `acquire_claim_when_reached` rather than a sleep between lock polls.
fn contending_locker() -> FileLocker {
    FileLocker::new(std::time::Duration::ZERO)
}

// Bound filesystem work while retaining varied generated inputs.
fn io_config() -> ProptestConfig {
    ProptestConfig::with_cases(64)
}

// Generator for valid issue IDs
fn issue_id_strategy() -> impl Strategy<Value = IssueId> {
    any::<u128>().prop_map(crate::test_utils::issue_id)
}

// Generator for TTL values (mix of finite and indefinite)
fn ttl_strategy() -> impl Strategy<Value = u64> {
    prop_oneof![
        Just(0),        // Indefinite
        Just(60),       // 1 minute
        Just(600),      // 10 minutes
        Just(3600),     // 1 hour
        60u64..7200u64, // Random 1min-2hr
    ]
}

// Property 2: Lease count invariant
// Number of active leases = acquires - (releases + evictions)
proptest! {
    #![proptest_config(io_config())]
    #[test]
    fn test_lease_count_invariant(
        issue_ids in prop::collection::hash_set(issue_id_strategy(), 1..10),
        ttl in ttl_strategy()
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let mut acquired_leases = Vec::new();

        // Acquire leases
        for issue_id in &issue_ids {
            let (lease, ()) = coordinator.acquire_claim_synchronized(
                issue_id, ttl, Some("test lease"),
                ClaimAcquireLimits::new(100, 100), |_| Ok(()),
            ).unwrap();
            acquired_leases.push(lease);
        }

        let acquired_count = acquired_leases.len();

        // Release some leases (about half)
        let released_count = acquired_count / 2;
        for lease in acquired_leases.iter().take(released_count) {
            coordinator.force_evict_lease(&IssueIdPrefix::from(&lease.issue_id), "test release").unwrap();
        }

        let stored = read_leases(&coordinator.paths).unwrap().len();
        let expected_active = acquired_count - released_count;

        prop_assert_eq!(stored, expected_active,
            "Active leases ({}) should equal acquired ({}) minus released ({})",
            stored, acquired_count, released_count);
    }
}

// Property 3: Sequence numbers are strictly increasing with no gaps
proptest! {
    #![proptest_config(io_config())]
    #[test]
    fn test_sequence_numbers_monotonic(
        operations in prop::collection::vec(
            (issue_id_strategy(), ttl_strategy()),
            2..20
        )
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Perform operations
        for (issue_id, ttl) in operations {
            let (lease, ()) = coordinator.acquire_claim_synchronized(
                &issue_id, ttl, Some("test lease"),
                ClaimAcquireLimits::new(100, 100), |_| Ok(()),
            ).unwrap();
            coordinator.force_evict_lease(&IssueIdPrefix::from(&lease.issue_id), "test release").unwrap();
        }

        let content = std::fs::read_to_string(
            coordinator.paths.shared_jit.join("claims.jsonl"),
        ).unwrap();
        let sequences: Vec<_> = content.lines()
            .map(|line| serde_json::from_str::<ClaimLogEntry>(line).unwrap().seq)
            .collect();

        prop_assert!(!sequences.is_empty(), "acquisition must append a log entry");
        prop_assert_eq!(sequences[0], 1);
        for pair in sequences.windows(2) {
            prop_assert_eq!(pair[1], pair[0] + 1,
                "sequence numbers must increase without gaps in append order");
        }
    }
}

// Property 4: Concurrent claims - exactly one succeeds per issue
proptest! {
    #![proptest_config(io_config())]
    #[test]
    fn test_concurrent_claims_exclusive(
        thread_count in 2u8..21u8,  // 2-20 threads
        issue_id in issue_id_strategy()
    ) {
        let temp_dir = Arc::new(TempDir::new().unwrap());
        let coordinator = setup_coordinator(&temp_dir);
        let barrier = Arc::new(Barrier::new(thread_count as usize));
        let claimants = Claimants::new();

        // Spawn threads all trying to claim the same issue
        let handles: Vec<_> = (0..thread_count)
            .map(|i| {
                let coord = ClaimCoordinator::new(
                    coordinator.paths.clone(),
                    contending_locker(),
                    format!("wt:claimant{i}").parse().unwrap(),
                    format!("agent:claimant{i}").parse().unwrap(),
                ).with_fsync(false);
                let barrier = Arc::clone(&barrier);
                let claimants = Arc::clone(&claimants);
                let issue = issue_id.clone();
                thread::spawn(move || {
                    barrier.wait();
                    acquire_claim_when_reached(&coord, &issue, 600, &claimants)
                })
            })
            .collect();

        // Collect results
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        let granted = results.iter().filter(|outcome| outcome.is_ok()).count();
        let refusals: Vec<_> = results
            .iter()
            .filter_map(|outcome| outcome.as_ref().err())
            .collect();

        prop_assert_eq!(granted, 1,
            "one claimant holds the issue");
        prop_assert_eq!(refusals.len(), (thread_count - 1) as usize,
            "every other claimant is turned away");
        prop_assert!(
            refusals
                .iter()
                .all(|error| error.to_string().contains("already claimed")),
            "each one is refused by the coordinator, rather than running out of \
             time to ask it"
        );
    }
}

// Property 7: Concurrent claims on different issues succeed
proptest! {
    #![proptest_config(io_config())]
    #[test]
    fn test_concurrent_different_issues_succeed(
        issue_ids in prop::collection::hash_set(issue_id_strategy(), 2..10)
    ) {
        let temp_dir = Arc::new(TempDir::new().unwrap());
        let coordinator = Arc::new(setup_coordinator(&temp_dir));
        let claimants = Claimants::new();
        let barrier = Arc::new(Barrier::new(issue_ids.len()));

        // Each thread claims a different issue
        let handles: Vec<_> = issue_ids
            .iter()
            .cloned()
            .map(|issue_id| {
                let coord = Arc::clone(&coordinator);
                let barrier = Arc::clone(&barrier);
                let claimants = Arc::clone(&claimants);
                thread::spawn(move || {
                    barrier.wait();
                    acquire_claim_when_reached(&coord, &issue_id, 600, &claimants)
                })
            })
            .collect();

        let granted: std::collections::HashSet<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .filter_map(|outcome| outcome.ok())
            .map(|lease| lease.issue_id)
            .collect();

        prop_assert_eq!(granted, issue_ids,
            "claims on distinct issues do not compete, so each one is granted \
             however often its claimant was refused the lock");
    }
}

// Property 8: Staleness computation is consistent
// An indefinite lease is stale iff now - last_beat > CLAIM_STALE_THRESHOLD_SECS
proptest! {
    #[test]
    fn test_staleness_consistent_with_threshold(
        secs_since_beat in 0u64..2 * crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS
    ) {
        let now = chrono::Utc::now();
        let lease = Lease {
            lease_id: uuid::Uuid::new_v4().to_string().parse().unwrap(),
            issue_id: crate::test_utils::issue_id(53),
            agent_id: "agent:test".parse().unwrap(),
            worktree_id: "wt:test".parse().unwrap(),
            branch: None,
            ttl_secs: 0, // Indefinite
            acquired_at: now - chrono::Duration::seconds(secs_since_beat as i64),
            expires_at: None,
            last_beat: now - chrono::Duration::seconds(secs_since_beat as i64),
        };

        let is_stale = lease.is_stale(now);

        let expected = secs_since_beat > crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS;
        prop_assert_eq!(is_stale, expected,
            "Staleness({}) should equal (secs_since_beat {} > built-in threshold)",
            is_stale, secs_since_beat);
    }
}

// Property 8b: Recovery reclaims exactly the indefinite leases listings report stale
proptest! {
    #![proptest_config(io_config())]
    #[test]
    fn test_evict_lapsed_reclaims_an_indefinite_lease_iff_it_is_stale(
        secs_since_beat in 0u64..2 * crate::runtime_defaults::CLAIM_STALE_THRESHOLD_SECS
    ) {
        use crate::storage::clock::FixedClock;

        let temp_dir = TempDir::new().unwrap();
        let base = chrono::Utc::now();
        let clock = Arc::new(FixedClock::new(base));
        let coordinator = setup_coordinator(&temp_dir).with_clock(clock.clone());
        let lease = coordinator.acquire_claim_synchronized(
            &crate::test_utils::issue_id(54), 0, Some("test lease"),
            ClaimAcquireLimits::new(1, 1), |_| Ok(()),
        ).unwrap().0;
        let now = base + chrono::Duration::seconds(secs_since_beat as i64);
        clock.set(now);

        let lapsed = coordinator.evict_lapsed().unwrap();

        let stale = lease.is_stale(now);
        prop_assert_eq!(lapsed.reclaimed == 1, stale);
        prop_assert_eq!(read_leases(&coordinator.paths).unwrap().is_empty(), stale);
    }
}

// Property 9: Finite leases are never stale (use expiration instead)
proptest! {
    #[test]
    fn test_finite_leases_never_stale(
        ttl in 1u64..7200u64,
        secs_since_beat in 0u64..100000u64
    ) {
        let now = chrono::Utc::now();
        let lease = Lease {
            lease_id: uuid::Uuid::new_v4().to_string().parse().unwrap(),
            issue_id: crate::test_utils::issue_id(53),
            agent_id: "agent:test".parse().unwrap(),
            worktree_id: "wt:test".parse().unwrap(),
            branch: None,
            ttl_secs: ttl,
            acquired_at: now - chrono::Duration::seconds(secs_since_beat as i64),
            expires_at: Some(now + chrono::Duration::seconds(3600)),
            last_beat: now - chrono::Duration::seconds(secs_since_beat as i64),
        };

        let is_stale = lease.is_stale(now);

        // Finite leases should never be stale (they use expiration instead)
        prop_assert!(!is_stale,
            "Finite lease (ttl={}) should never be stale, regardless of last_beat ({}s ago)",
            ttl, secs_since_beat);
    }
}
