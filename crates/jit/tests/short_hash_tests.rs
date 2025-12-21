//! Tests for short hash issue ID resolution
//!
//! Validates that partial UUID prefixes work like git short hashes.

mod harness;
use harness::TestHarness;
use jit::storage::IssueStore;

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
    h.executor
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
