//! Tests for transitive reduction validation
//!
//! Ensures the dependency DAG maintains its transitive reduction form with no
//! redundant edges. Tests detection, auto-fix, and edge cases.

mod harness;
use harness::TestHarness;
use jit::storage::IssueStore;

#[test]
fn test_fix_transitive_reduction_logs_event() {
    let h = TestHarness::new();

    // Create A → B → C and add redundant A → C
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Manually inject redundant edge
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(issue_a).unwrap();

    let events_before = h.storage.read_events().unwrap().len();

    // Apply fix
    let mut executor = h.executor;
    let (fixes_applied, _) = executor.validate_with_fix(true, false).unwrap();
    assert_eq!(fixes_applied, 1);

    // Verify a DependencyReduced event was logged
    let events = h.storage.read_events().unwrap();
    assert_eq!(events.len(), events_before + 1, "Should log one event");

    let event = &events[events_before];
    assert_eq!(event.get_type(), "dependency_reduced");
    assert_eq!(event.get_issue_id(), a);

    if let jit::domain::Event::DependencyReduced {
        issue_id,
        old_count,
        new_count,
        removed_deps,
        ..
    } = event
    {
        assert_eq!(issue_id, &a);
        assert_eq!(*old_count, 2);
        assert_eq!(*new_count, 1);
        assert_eq!(removed_deps, &[c]);
    } else {
        panic!("Expected DependencyReduced event");
    }
}

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

// ============================================================================
// Write-time redundancy guard on `jit dep add` (jit:7a50e021)
// ============================================================================
//
// Cycle detection is enforced at write time (INV-DAG-ACYCLIC); these tests pin
// the same treatment for the transitive-reduction property: a `dep add` that
// would shadow an existing edge (or is itself redundant) is rejected by default
// and only applied under an explicit `--reduce`, so a silent write can never
// surface as a distant `jit validate` failure.

use jit::commands::RedundancyPolicy;

/// REQ-01: adding an edge that makes a PRE-EXISTING edge redundant is rejected
/// under the default (Reject) policy, with the offending edge pair named.
#[test]
fn test_dep_add_rejects_edge_that_shadows_existing_edge() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    // A → C and A → B exist (no redundancy yet).
    h.executor.add_dependency(&a, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Adding B → C makes A → C redundant (A reaches C via A → B → C).
    let err = h
        .executor
        .add_dependency_with_policy(&b, &c, RedundancyPolicy::Reject)
        .expect_err("redundant add must be rejected");
    let msg = err.to_string();
    // The offending edge pair (A → C) must be named.
    assert!(
        msg.contains(&a[..8]) && msg.contains(&c[..8]),
        "error must name the shadowed edge A→C, got: {msg}"
    );
    assert!(
        err.downcast_ref::<jit::errors::RedundantDependencyError>()
            .is_some(),
        "must be a typed RedundantDependencyError"
    );

    // Nothing was written: A still directly depends on C, B does not depend on C.
    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(loaded_a.dependencies.contains(&c));
    let loaded_b = h.storage.load_issue(&b).unwrap();
    assert!(!loaded_b.dependencies.contains(&c));
}

/// REQ-01: adding an edge that is ITSELF already reachable through existing
/// edges is rejected under the default policy, naming the pair.
#[test]
fn test_dep_add_rejects_self_redundant_edge() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    // B → C and A → B exist.
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // A → C is itself redundant (A reaches C via A → B → C).
    let err = h
        .executor
        .add_dependency_with_policy(&a, &c, RedundancyPolicy::Reject)
        .expect_err("self-redundant add must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains(&a[..8]) && msg.contains(&c[..8]),
        "error must name the redundant edge A→C, got: {msg}"
    );

    // A must NOT have gained the redundant direct edge to C.
    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(!loaded_a.dependencies.contains(&c));
    assert!(loaded_a.dependencies.contains(&b));
}

/// REQ-02: `--reduce` makes the add succeed and drops the now-redundant edge in
/// the same operation, leaving the graph transitively reduced.
#[test]
fn test_dep_add_reduce_drops_shadowed_edge() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    h.executor.add_dependency(&a, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();

    // Add B → C with --reduce: succeeds and drops the shadowed A → C.
    let (result, _warnings) = h
        .executor
        .add_dependency_with_policy(&b, &c, RedundancyPolicy::Reduce)
        .expect("reduce add must succeed");
    assert_eq!(result, jit::commands::DependencyAddResult::Added);

    // The new edge B → C is present.
    let loaded_b = h.storage.load_issue(&b).unwrap();
    assert!(
        loaded_b.dependencies.contains(&c),
        "new edge B→C must be present"
    );

    // The shadowed edge A → C is gone; A still depends on B.
    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(
        !loaded_a.dependencies.contains(&c),
        "shadowed edge A→C must be dropped"
    );
    assert!(loaded_a.dependencies.contains(&b));

    // Graph is transitively reduced.
    assert!(
        h.executor.validate_silent().is_ok(),
        "graph must be transitively reduced after --reduce add"
    );
}

/// REQ-03 (Background scenario): X → Y and X → Z exist; adding Y → Z makes
/// X → Z redundant. With --reduce the add succeeds, X → Z is dropped, and a
/// later `jit validate` reports no transitive-reduction violation.
#[test]
fn test_dep_add_reduce_background_scenario_leaves_validate_clean() {
    let h = TestHarness::new();
    let x = h.create_issue("X");
    let y = h.create_issue("Y");
    let z = h.create_issue("Z");

    h.executor.add_dependency(&x, &y).unwrap();
    h.executor.add_dependency(&x, &z).unwrap();

    // Adding Y → Z makes the pre-existing X → Z redundant (X reaches Z via X→Y→Z).
    let (result, _warnings) = h
        .executor
        .add_dependency_with_policy(&y, &z, RedundancyPolicy::Reduce)
        .expect("reduce add must succeed");
    assert_eq!(result, jit::commands::DependencyAddResult::Added);

    let loaded_x = h.storage.load_issue(&x).unwrap();
    assert!(
        !loaded_x.dependencies.contains(&z),
        "redundant X→Z must be dropped"
    );
    assert!(loaded_x.dependencies.contains(&y));
    let loaded_y = h.storage.load_issue(&y).unwrap();
    assert!(
        loaded_y.dependencies.contains(&z),
        "new edge Y→Z must be present"
    );

    // A later validate must be clean — no distant violation possible.
    assert!(
        h.executor.validate_silent().is_ok(),
        "validate must be clean after the reduce add"
    );
}

/// REQ-03: a plain, non-redundant `dep add` still succeeds and leaves validate
/// clean (existing behavior preserved for non-redundant edges).
#[test]
fn test_dep_add_non_redundant_edge_still_added_and_validate_clean() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");

    let (result, _warnings) = h
        .executor
        .add_dependency_with_policy(&a, &b, RedundancyPolicy::Reject)
        .expect("non-redundant add must succeed even under Reject");
    assert_eq!(result, jit::commands::DependencyAddResult::Added);

    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(loaded_a.dependencies.contains(&b));
    assert!(h.executor.validate_silent().is_ok());
}
