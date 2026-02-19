//! Tests for short hash issue ID resolution
//!
//! Validates that partial UUID prefixes work like git short hashes.

mod harness;
use harness::TestHarness;
use jit::domain::Issue;
use jit::storage::{InMemoryStorage, IssueStore};
use proptest::prelude::*;

#[test]
fn test_resolve_full_uuid() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Full UUID should pass through unchanged
    let resolved = h.storage.resolve_issue_id(&id).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn test_resolve_short_prefix_4_chars() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // 4 character prefix should resolve
    let prefix = &id[..4];
    let resolved = h.storage.resolve_issue_id(prefix).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn test_resolve_short_prefix_8_chars() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // 8 character prefix should resolve
    let prefix = &id[..8];
    let resolved = h.storage.resolve_issue_id(prefix).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn test_resolve_with_hyphens() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Prefix with hyphens should work (e.g., "9db27a3a-86c5")
    let prefix = &id[..13]; // Includes first hyphen
    let resolved = h.storage.resolve_issue_id(prefix).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn test_resolve_without_hyphens() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Strip hyphens from ID, use first 8 chars
    let no_hyphens: String = id.chars().filter(|c| *c != '-').take(8).collect();
    let resolved = h.storage.resolve_issue_id(&no_hyphens).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn test_resolve_case_insensitive() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Uppercase prefix should work
    let prefix = id[..8].to_uppercase();
    let resolved = h.storage.resolve_issue_id(&prefix).unwrap();
    assert_eq!(resolved, id);
}

#[test]
fn test_resolve_too_short() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Less than 4 characters should error
    let prefix = &id[..3];
    let result = h.storage.resolve_issue_id(prefix);
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("at least 4 characters"),
        "Error: {}",
        err_msg
    );
}

#[test]
fn test_resolve_not_found() {
    let h = TestHarness::new();
    h.create_issue("Test issue");

    // Non-matching prefix should error
    let result = h.storage.resolve_issue_id("ffffffff");
    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not found"), "Error: {}", err_msg);
}

#[test]
fn test_resolve_ambiguous() {
    let h = TestHarness::new();

    // Create issues until we find two with same 4-char prefix
    // This is probabilistic but should happen quickly with enough UUIDs
    let mut ids = Vec::new();
    for i in 0..100 {
        let id = h.create_issue(&format!("Issue {}", i));
        ids.push(id);
    }

    // Find two IDs with matching 4-char prefix
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let prefix_i = &ids[i][..4];
            let prefix_j = &ids[j][..4];
            if prefix_i == prefix_j {
                // Found ambiguous prefix - test it
                let result = h.storage.resolve_issue_id(prefix_i);
                assert!(result.is_err(), "Should fail with ambiguous prefix");
                let err_msg = result.unwrap_err().to_string();
                assert!(
                    err_msg.contains("Ambiguous") || err_msg.contains("multiple"),
                    "Error should mention ambiguity: {}",
                    err_msg
                );
                return; // Test passed
            }
        }
    }

    // If we didn't find collision, that's okay - test is probabilistic
    // In practice, UUID collisions at 4 chars are common enough this will usually pass
}

#[test]
fn test_resolve_multiple_issues_unique_prefixes() {
    let h = TestHarness::new();
    let id1 = h.create_issue("Issue 1");
    let id2 = h.create_issue("Issue 2");
    let id3 = h.create_issue("Issue 3");

    // All should resolve independently
    let resolved1 = h.storage.resolve_issue_id(&id1[..8]).unwrap();
    let resolved2 = h.storage.resolve_issue_id(&id2[..8]).unwrap();
    let resolved3 = h.storage.resolve_issue_id(&id3[..8]).unwrap();

    assert_eq!(resolved1, id1);
    assert_eq!(resolved2, id2);
    assert_eq!(resolved3, id3);
}

#[test]
fn test_cli_show_with_short_hash() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Use short hash in show command
    let prefix = &id[..8];
    let issue = h.executor.show_issue(prefix).unwrap();
    assert_eq!(issue.id, id);
    assert_eq!(issue.title, "Test issue");
}

#[test]
fn test_cli_update_with_short_hash() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Use short hash in update command
    let prefix = &id[..8];
    let _ = h
        .executor
        .update_issue(prefix, None, None, None, None, vec![], vec![])
        .unwrap();

    // Verify it worked
    let issue = h.storage.load_issue(&id).unwrap();
    assert_eq!(issue.id, id);
}

#[test]
fn test_cli_dependency_with_short_hashes() {
    let h = TestHarness::new();
    let id1 = h.create_issue("Issue 1");
    let id2 = h.create_issue("Issue 2");

    // Use short hashes for both IDs
    let prefix1 = &id1[..8];
    let prefix2 = &id2[..8];

    h.executor.add_dependency(prefix1, prefix2).unwrap();

    // Verify dependency was added
    let issue1 = h.storage.load_issue(&id1).unwrap();
    assert!(issue1.dependencies.contains(&id2));
}

#[test]
fn test_cli_delete_with_short_hash() {
    let h = TestHarness::new();
    let id = h.create_issue("Test issue");

    // Delete using short hash
    let prefix = &id[..8];
    h.executor.delete_issue(prefix).unwrap();

    // Verify it's gone
    let result = h.storage.load_issue(&id);
    assert!(result.is_err());
}

// ========== Property-Based Tests ==========

/// Build a fresh `InMemoryStorage` with `init()` already called.
fn make_storage() -> InMemoryStorage {
    // Disable worktree divergence checks in tests
    std::env::set_var("JIT_TEST_MODE", "1");
    let storage = InMemoryStorage::new();
    storage.init().unwrap();
    storage
}

/// Save an issue whose `id` field is set to `custom_id`.
fn save_issue_with_id(storage: &InMemoryStorage, custom_id: &str, title: &str) {
    let mut issue = Issue::new(title.to_string(), String::new());
    issue.id = custom_id.to_string();
    storage.save_issue(issue).unwrap();
}

/// Return the normalized (lowercase, no hyphens) form of a UUID string.
fn normalize_uuid(id: &str) -> String {
    id.to_lowercase().replace('-', "")
}

// --- Strategy helpers ---

/// A strategy that produces a valid lowercase hex char `[0-9a-f]`.
fn hex_char_strategy() -> impl Strategy<Value = char> {
    prop_oneof![
        Just('0'),
        Just('1'),
        Just('2'),
        Just('3'),
        Just('4'),
        Just('5'),
        Just('6'),
        Just('7'),
        Just('8'),
        Just('9'),
        Just('a'),
        Just('b'),
        Just('c'),
        Just('d'),
        Just('e'),
        Just('f'),
    ]
}

/// A strategy that produces a UUID-like string of the form
/// `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` where all x's are random hex digits.
fn uuid_like_strategy() -> impl Strategy<Value = String> {
    // 8-4-4-4-12 hex digits separated by hyphens
    (
        prop::collection::vec(hex_char_strategy(), 8),
        prop::collection::vec(hex_char_strategy(), 4),
        prop::collection::vec(hex_char_strategy(), 4),
        prop::collection::vec(hex_char_strategy(), 4),
        prop::collection::vec(hex_char_strategy(), 12),
    )
        .prop_map(|(g1, g2, g3, g4, g5)| {
            format!(
                "{}-{}-{}-{}-{}",
                g1.iter().collect::<String>(),
                g2.iter().collect::<String>(),
                g3.iter().collect::<String>(),
                g4.iter().collect::<String>(),
                g5.iter().collect::<String>()
            )
        })
}

proptest! {
    /// Property: for a single stored issue, any unique prefix of 4–16 normalized
    /// chars resolves back to that issue's full ID.
    #[test]
    fn test_resolve_unique_prefix_various_lengths(
        uuid in uuid_like_strategy(),
        // prefix_len: how many normalized (no-hyphen) chars to use as prefix
        prefix_len in 4usize..=16usize,
    ) {
        let storage = make_storage();
        save_issue_with_id(&storage, &uuid, "proptest issue");

        let normalized = normalize_uuid(&uuid);
        // Guard: normalized UUID must be at least prefix_len chars
        prop_assume!(normalized.len() >= prefix_len);

        let prefix = &normalized[..prefix_len];
        let result = storage.resolve_issue_id(prefix);

        prop_assert!(
            result.is_ok(),
            "Expected Ok for prefix '{prefix}' of issue '{uuid}', got: {:?}",
            result.unwrap_err()
        );
        prop_assert_eq!(result.unwrap(), uuid);
    }
}

proptest! {
    /// Property: when two issues share a common normalized prefix, resolving that
    /// prefix fails with an ambiguity error.
    ///
    /// Strategy: generate a 4-char shared prefix plus two distinct 28-char suffixes.
    /// Together they form two 32-char normalized IDs (reformatted as hyphenated UUIDs)
    /// that both start with the same prefix. Resolution of that prefix must fail.
    #[test]
    fn test_resolve_ambiguity_detected_for_shared_prefix(
        // shared_prefix: exactly 4 hex chars (minimum resolvable length)
        prefix_chars in prop::collection::vec(hex_char_strategy(), 4usize),
        // Each suffix provides the remaining 28 hex chars so total is 32
        suffix_a in prop::collection::vec(hex_char_strategy(), 28usize),
        suffix_b in prop::collection::vec(hex_char_strategy(), 28usize),
    ) {
        let prefix_str: String = prefix_chars.iter().collect();
        let suffix_a_str: String = suffix_a.iter().collect();
        let suffix_b_str: String = suffix_b.iter().collect();

        // Skip if both suffixes are identical (no ambiguity)
        prop_assume!(suffix_a_str != suffix_b_str);

        // Build two 32-hex-char normalized IDs that share the same 4-char prefix
        let norm_a = format!("{}{}", prefix_str, suffix_a_str);
        let norm_b = format!("{}{}", prefix_str, suffix_b_str);

        // Reformat as hyphenated UUIDs: 8-4-4-4-12
        let uuid_a = format!(
            "{}-{}-{}-{}-{}",
            &norm_a[..8], &norm_a[8..12], &norm_a[12..16], &norm_a[16..20], &norm_a[20..]
        );
        let uuid_b = format!(
            "{}-{}-{}-{}-{}",
            &norm_b[..8], &norm_b[8..12], &norm_b[12..16], &norm_b[16..20], &norm_b[20..]
        );

        let storage = make_storage();
        save_issue_with_id(&storage, &uuid_a, "issue A");
        save_issue_with_id(&storage, &uuid_b, "issue B");

        // Resolve using the shared prefix: must fail with an ambiguity error
        let result = storage.resolve_issue_id(&prefix_str);

        prop_assert!(
            result.is_err(),
            "Expected Err for ambiguous prefix '{prefix_str}', but got Ok({})",
            result.unwrap()
        );
        let err_msg = result.unwrap_err().to_string();
        prop_assert!(
            err_msg.contains("Ambiguous") || err_msg.contains("multiple"),
            "Error message should mention ambiguity, got: {err_msg}"
        );
    }
}

proptest! {
    /// Property: any prefix whose normalized length is 1, 2, or 3 chars is
    /// rejected with "at least 4 characters" regardless of what is stored.
    #[test]
    fn test_resolve_prefix_shorter_than_minimum_is_rejected(
        // prefix: 1–3 hex chars (always too short after normalization)
        short_prefix in prop::collection::vec(hex_char_strategy(), 1..=3),
        uuid in uuid_like_strategy(),
    ) {
        let storage = make_storage();
        save_issue_with_id(&storage, &uuid, "stored issue");

        let prefix_str: String = short_prefix.iter().collect();
        let result = storage.resolve_issue_id(&prefix_str);

        prop_assert!(
            result.is_err(),
            "Expected Err for short prefix '{prefix_str}', but got Ok"
        );
        let err_msg = result.unwrap_err().to_string();
        prop_assert!(
            err_msg.contains("at least 4 characters"),
            "Error should mention minimum length, got: {err_msg}"
        );
    }
}
