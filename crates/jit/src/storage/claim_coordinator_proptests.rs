//! Property-based tests for claim coordination invariants
//!
//! These tests use `proptest` to verify system invariants across randomly
//! generated operation sequences, catching edge cases that example-based
//! tests might miss.

use super::*;
use proptest::prelude::*;
use std::sync::Arc;
use std::thread;
use tempfile::TempDir;

// Helper to set up a test coordinator
fn setup_coordinator(temp_dir: &TempDir) -> ClaimCoordinator {
    let paths = WorktreePaths {
        common_dir: temp_dir.path().join(".git"),
        worktree_root: temp_dir.path().to_path_buf(),
        local_jit: temp_dir.path().join(".jit"),
        shared_jit: temp_dir.path().join(".git/jit"),
    };

    let locker = FileLocker::new(std::time::Duration::from_secs(5));
    let coordinator = ClaimCoordinator::new(
        paths,
        locker,
        "wt:proptest".to_string(),
        "agent:proptest".to_string(),
    );

    coordinator.init().unwrap();
    coordinator
}

// Generator for valid issue IDs
fn issue_id_strategy() -> impl Strategy<Value = String> {
    "[a-z]{3}-[0-9]{3}".prop_map(|s| s.to_string())
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

// Property 1: Index rebuild produces consistent results
// The same log rebuilt twice produces the same set of active issues
proptest! {
    #[test]
    fn prop_index_rebuild_idempotent(
        issue_ids in prop::collection::vec(issue_id_strategy(), 1..10),
        ttls in prop::collection::vec(ttl_strategy(), 1..10)
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Acquire multiple leases
        let issue_count = issue_ids.len().min(ttls.len());
        for i in 0..issue_count {
            let _ = coordinator.acquire_claim(&issue_ids[i], ttls[i]);
        }

        // Rebuild index twice
        let index1 = coordinator.rebuild_index_from_log().unwrap();
        let index2 = coordinator.rebuild_index_from_log().unwrap();

        // Indexes should have same structure
        prop_assert_eq!(index1.leases.len(), index2.leases.len());
        prop_assert_eq!(index1.last_seq, index2.last_seq);

        // Same set of issue IDs (order may differ)
        let mut issues1: Vec<_> = index1.leases.iter().map(|l| &l.issue_id).collect();
        let mut issues2: Vec<_> = index2.leases.iter().map(|l| &l.issue_id).collect();
        issues1.sort();
        issues2.sort();
        prop_assert_eq!(issues1, issues2,
            "Rebuilt indexes should contain same set of issues");
    }
}

// Property 2: Lease count invariant
// Number of active leases = acquires - (releases + evictions)
proptest! {
    #[test]
    fn prop_lease_count_invariant(
        issue_ids in prop::collection::vec(issue_id_strategy(), 1..10),
        ttl in ttl_strategy()
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let mut acquired_leases = Vec::new();

        // Acquire leases
        for issue_id in &issue_ids {
            if let Ok(lease) = coordinator.acquire_claim(issue_id, ttl) {
                acquired_leases.push(lease);
            }
        }

        let acquired_count = acquired_leases.len();

        // Release some leases (about half)
        let mut released_count = 0;
        for lease in acquired_leases.iter().take(acquired_count / 2) {
            if coordinator.release_lease(&lease.lease_id).is_ok() {
                released_count += 1;
            }
        }

        // Rebuild index and verify count
        let index = coordinator.rebuild_index_from_log().unwrap();
        let expected_active = acquired_count - released_count;

        prop_assert_eq!(index.leases.len(), expected_active,
            "Active leases ({}) should equal acquired ({}) minus released ({})",
            index.leases.len(), acquired_count, released_count);
    }
}

// Property 3: Sequence numbers are strictly increasing with no gaps
proptest! {
    #[test]
    fn prop_sequence_numbers_monotonic(
        operations in prop::collection::vec(
            (issue_id_strategy(), ttl_strategy()),
            2..20
        )
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        // Perform operations
        for (issue_id, ttl) in operations {
            let _ = coordinator.acquire_claim(&issue_id, ttl);
        }

        // Read log and verify sequences
        let log_path = coordinator.paths.shared_jit.join("claims.jsonl");
        if !log_path.exists() {
            return Ok(());
        }

        let content = std::fs::read_to_string(&log_path).unwrap();
        let mut sequences: Vec<u64> = content
            .lines()
            .filter_map(|line| {
                serde_json::from_str::<serde_json::Value>(line).ok()
                    .and_then(|v| v.get("sequence")?.as_u64())
            })
            .collect();

        if sequences.is_empty() {
            return Ok(());
        }

        sequences.sort_unstable();

        // Check strictly increasing
        for i in 1..sequences.len() {
            prop_assert!(sequences[i] > sequences[i-1],
                "Sequence numbers must be strictly increasing: {} should be > {}",
                sequences[i], sequences[i-1]);
        }

        // Check no gaps (assuming starts at 1)
        if !sequences.is_empty() {
            prop_assert_eq!(sequences[0], 1, "First sequence should be 1");
            prop_assert_eq!(sequences[sequences.len() - 1], sequences.len() as u64,
                "Last sequence should equal count (no gaps)");
        }
    }
}

// Property 4: Concurrent claims - exactly one succeeds per issue
proptest! {
    #[test]
    fn prop_concurrent_claims_exclusive(
        thread_count in 2u8..21u8,  // 2-20 threads
        issue_id in issue_id_strategy()
    ) {
        let temp_dir = Arc::new(TempDir::new().unwrap());
        let coordinator = Arc::new(setup_coordinator(&temp_dir));

        // Spawn threads all trying to claim the same issue
        let handles: Vec<_> = (0..thread_count)
            .map(|_| {
                let coord = Arc::clone(&coordinator);
                let issue = issue_id.clone();
                thread::spawn(move || {
                    coord.acquire_claim(&issue, 600)
                })
            })
            .collect();

        // Collect results
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        // Count successes and failures
        let successes = results.iter().filter(|r| r.is_ok()).count();
        let failures = results.iter().filter(|r| r.is_err()).count();

        prop_assert_eq!(successes, 1,
            "Exactly one thread should acquire the claim");
        prop_assert_eq!(failures, (thread_count - 1) as usize,
            "All other threads should fail");
    }
}

// Property 5: No data loss in index rebuild
// Every acquired lease appears in rebuilt index (unless expired/released)
proptest! {
    #[test]
    fn prop_no_data_loss_in_rebuild(
        issue_ids in prop::collection::hash_set(issue_id_strategy(), 1..10)
    ) {
        let temp_dir = TempDir::new().unwrap();
        let coordinator = setup_coordinator(&temp_dir);

        let mut lease_ids = Vec::new();

        // Acquire leases with long TTL (won't expire during test)
        for issue_id in &issue_ids {
            if let Ok(lease) = coordinator.acquire_claim(issue_id, 3600) {
                lease_ids.push(lease.lease_id.clone());
            }
        }

        // Rebuild index
        let index = coordinator.rebuild_index_from_log().unwrap();

        // All acquired leases should be in index
        for lease_id in &lease_ids {
            prop_assert!(
                index.leases.iter().any(|l| l.lease_id == *lease_id),
                "Lease {} should be in rebuilt index", lease_id
            );
        }

        prop_assert_eq!(index.leases.len(), lease_ids.len(),
            "Index should contain exactly the acquired leases");
    }
}

// Note: Expired lease filtering is tested in example-based tests
// (test_rebuild_index_filters_expired_leases) to avoid slow property tests
// that would need to sleep for expiration.

// Property 7: Concurrent claims on different issues succeed
proptest! {
    #[test]
    fn prop_concurrent_different_issues_succeed(
        issue_ids in prop::collection::hash_set(issue_id_strategy(), 2..10)
    ) {
        let temp_dir = Arc::new(TempDir::new().unwrap());
        let coordinator = Arc::new(setup_coordinator(&temp_dir));

        let issue_vec: Vec<_> = issue_ids.into_iter().collect();
        let thread_count = issue_vec.len();

        // Each thread claims a different issue
        let handles: Vec<_> = issue_vec
            .into_iter()
            .map(|issue_id| {
                let coord = Arc::clone(&coordinator);
                thread::spawn(move || {
                    coord.acquire_claim(&issue_id, 600)
                })
            })
            .collect();

        // All should succeed
        let results: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect();

        let successes = results.iter().filter(|r| r.is_ok()).count();

        prop_assert_eq!(successes, thread_count,
            "All threads claiming different issues should succeed");
    }
}
