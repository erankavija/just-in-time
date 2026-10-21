//! Tests for short hash issue ID resolution
//!
//! Validates that partial UUID prefixes work like git short hashes.
use jit::storage::{InMemoryStorage, IssueStore};
use jit::test_harness::TestHarness;
use proptest::prelude::*;

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
        .update_issue(
            prefix,
            None,
            None,
            None,
            None,
            vec![],
            vec![],
            None,
            None,
            false,
        )
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

/// Build a fresh in-memory storage fixture.
fn make_storage() -> InMemoryStorage {
    // Disable worktree divergence checks in tests
    jit::test_mode::enable();
    InMemoryStorage::new()
}

/// Seed an issue whose `id` field is set to `custom_id`.
fn seed_issue_with_id(storage: &InMemoryStorage, custom_id: &str, title: &str) {
    let mut issue = crate::fixture_issue(title.to_string(), String::new());
    issue.id = custom_id.to_string();
    jit::test_harness::seed_memory_issue(storage, &issue);
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
        seed_issue_with_id(&storage, &uuid_a, "issue A");
        seed_issue_with_id(&storage, &uuid_b, "issue B");

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
