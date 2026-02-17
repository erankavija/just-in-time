//! Tests for transitive reduction validation
//!
//! Ensures the dependency DAG maintains its transitive reduction form with no
//! redundant edges. Tests detection, auto-fix, and edge cases.

mod harness;
use harness::TestHarness;
use jit::storage::IssueStore;

#[test]
fn test_detect_transitive_redundancy() {
    let h = TestHarness::new();

    // Create A → B → C and add redundant A → C
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Manually add redundant edge (bypassing reduction logic)
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(issue_a).unwrap();

    // Validate should detect it
    let result = h.executor.validate_silent();
    assert!(result.is_err(), "Should detect redundant dependency");
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("redundant") || err_msg.contains("transitive"),
        "Error should mention redundancy: {}",
        err_msg
    );
}

#[test]
fn test_fix_transitive_redundancy() {
    let h = TestHarness::new();

    // Create A → B → C and add redundant A → C
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Add redundant edge
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(issue_a).unwrap();

    // Fix with validate --fix
    let mut executor = h.executor;
    let (fixes_applied, _messages) = executor.validate_with_fix(true, false).unwrap();
    assert!(fixes_applied > 0, "Should apply at least one fix");

    // Verify C was removed from A's dependencies
    let fixed_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(
        fixed_a.dependencies.len(),
        1,
        "Should have only one dependency"
    );
    assert!(fixed_a.dependencies.contains(&b), "Should keep A→B");
    assert!(!fixed_a.dependencies.contains(&c), "Should remove A→C");
}

#[test]
fn test_validate_reports_all_redundancies() {
    let h = TestHarness::new();

    // Create multiple redundancies:
    // A → B → C and A → C (redundant)
    // A → B → D and A → D (redundant)
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");

    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&b, &d).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Add redundant edges
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    issue_a.dependencies.push(d.clone());
    h.storage.save_issue(issue_a).unwrap();

    // Validate should detect both
    let result = h.executor.validate_silent();
    assert!(result.is_err(), "Should detect redundant dependencies");

    // Fix and verify both removed
    let mut executor = h.executor;
    let (fixes_applied, _messages) = executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes_applied, 2, "Should fix both redundancies");

    let fixed_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(fixed_a.dependencies.len(), 1, "Should have only B");
    assert!(fixed_a.dependencies.contains(&b));
}

#[test]
fn test_no_false_positives_diamond_pattern() {
    let h = TestHarness::new();

    // Create diamond: A → B, A → C, B → D, C → D
    // Both paths to D are necessary (not redundant at A level)
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");

    h.executor.add_dependency(&b, &d).unwrap();
    h.executor.add_dependency(&c, &d).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&a, &c).unwrap();

    // Validation should pass - no redundancies
    let result = h.executor.validate_silent();
    assert!(result.is_ok(), "Diamond pattern should be valid");
}

#[test]
fn test_no_false_positives_independent_deps() {
    let h = TestHarness::new();

    // A → B and A → C with no connection between B and C
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&a, &c).unwrap();

    // Validation should pass
    let result = h.executor.validate_silent();
    assert!(result.is_ok(), "Independent dependencies should be valid");
}

#[test]
fn test_dry_run_does_not_modify() {
    let h = TestHarness::new();

    // Create A → B → C and redundant A → C
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(issue_a).unwrap();

    // Dry run
    let mut executor = h.executor;
    let (fixes_count, _messages) = executor.validate_with_fix(true, true).unwrap();
    assert!(fixes_count > 0, "Should report fixes available");

    // Verify nothing was modified
    let unchanged_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(
        unchanged_a.dependencies.len(),
        2,
        "Should still have both deps"
    );
}

#[test]
fn test_complex_chain_reduction() {
    let h = TestHarness::new();

    // Create A → B → C → D and add redundant A → D
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");

    h.executor.add_dependency(&c, &d).unwrap();
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Add redundant edge (A can reach D via B→C→D)
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(d.clone());
    h.storage.save_issue(issue_a).unwrap();

    // Fix
    let mut executor = h.executor;
    let _ = executor.validate_with_fix(true, false).unwrap();

    // Verify
    let fixed_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(fixed_a.dependencies.len(), 1);
    assert!(fixed_a.dependencies.contains(&b));
    assert!(!fixed_a.dependencies.contains(&d));
}

#[test]
fn test_multiple_issues_with_redundancies() {
    let h = TestHarness::new();

    // Issue A has redundancy: A → B → C and A → C
    // Issue D has redundancy: D → E → F and D → F
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");
    let e = h.create_issue("E");
    let f = h.create_issue("F");

    // Setup chains
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&e, &f).unwrap();
    h.executor.add_dependency(&d, &e).unwrap();

    // Add redundant edges
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(issue_a).unwrap();

    let mut issue_d = h.storage.load_issue(&d).unwrap();
    issue_d.dependencies.push(f.clone());
    h.storage.save_issue(issue_d).unwrap();

    // Fix both
    let mut executor = h.executor;
    let (fixes_applied, _messages) = executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes_applied, 2, "Should fix both issues");

    // Verify both fixed
    let fixed_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(fixed_a.dependencies.len(), 1);
    assert!(fixed_a.dependencies.contains(&b));

    let fixed_d = h.storage.load_issue(&d).unwrap();
    assert_eq!(fixed_d.dependencies.len(), 1);
    assert!(fixed_d.dependencies.contains(&e));
}
