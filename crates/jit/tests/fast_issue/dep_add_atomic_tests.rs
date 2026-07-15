//! Tests for atomic variadic `jit dep add` (jit:c8518f2a).
//!
//! `add_dependencies_with_policy` validates every requested edge against the
//! would-be-final graph — every edge of the call applied at once — before
//! writing anything. A batch with any rejected edge leaves the dependency set
//! and the event log completely unchanged (REQ-01, REQ-03), and the resulting
//! error names every rejected edge, not only the first (REQ-02).
use crate::harness::TestHarness;
use jit::errors::DependencyBatchRejectedError;
use jit::storage::IssueStore;

/// REQ-01 / REQ-03: `dep add A D C` where D is a valid new edge and C is
/// self-redundant (already reachable through A's other dependencies) rejects
/// the WHOLE batch — D must not be added — and logs no event for either edge.
#[test]
fn test_self_redundant_sibling_blocks_valid_edge_and_logs_nothing() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");

    // A already reaches C via A -> B -> C, so a direct A -> C is self-redundant.
    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&b, &c).unwrap();

    let events_before = h.storage.read_events().unwrap().len();

    let result = h.executor.add_dependencies_with_policy(
        &a,
        &[d.clone(), c.clone()],
        jit::commands::RedundancyPolicy::Reject,
    );

    let err = result.expect_err("a self-redundant sibling must reject the whole batch");
    let batch = err
        .downcast_ref::<DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");
    assert_eq!(batch.rejected().len(), 1, "only C is rejected, not D");
    assert_eq!(batch.rejected()[0].0, c);

    // D — perfectly valid on its own — was NOT added.
    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(
        !loaded_a.dependencies.contains(&d),
        "the valid sibling edge must not be persisted when another edge is rejected"
    );
    assert!(!loaded_a.dependencies.contains(&c));
    assert_eq!(loaded_a.dependencies, vec![b.clone()]);

    // No event was logged for either edge (REQ-03).
    let events_after = h.storage.read_events().unwrap();
    assert_eq!(
        events_after.len(),
        events_before,
        "a rejected batch must not append any event"
    );
}

/// REQ-02: a batch with TWO independently-invalid edges (different reasons)
/// names both in the error, not only the first.
#[test]
fn test_multiple_invalid_edges_are_all_named() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    // Pre-existing B -> C: a fresh A -> C is self-redundant only once A -> B
    // is also requested, but here we pair it with an entirely different
    // failure class (an unresolvable id) to prove both get named.
    h.executor.add_dependency(&b, &c).unwrap();

    let result = h.executor.add_dependencies_with_policy(
        &a,
        &[b.clone(), c.clone(), "nonexistent".to_string()],
        jit::commands::RedundancyPolicy::Reject,
    );

    let err = result.expect_err("multiple invalid edges must reject the batch");
    let batch = err
        .downcast_ref::<DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");

    let named: Vec<&str> = batch.rejected().iter().map(|(to, _)| to.as_str()).collect();
    assert!(
        named.contains(&"nonexistent"),
        "the unresolvable id must be named: {named:?}"
    );
    assert!(
        named.contains(&c.as_str()),
        "the redundant edge must ALSO be named, not just the first failure: {named:?}"
    );
    assert!(
        batch.rejected().len() >= 2,
        "expected at least two distinct rejected edges, got {named:?}"
    );

    // Nothing persisted.
    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(loaded_a.dependencies.is_empty());
}

/// REQ-02 precision: a shadow-type redundancy must be attributed ONLY to the
/// specific edge responsible, never to an innocent sibling in the same batch.
/// Existing X -> Y and X -> Z; `dep add Y Z W` adds Y -> Z (shadows the
/// pre-existing X -> Z, since X now reaches Z via X -> Y -> Z) and Y -> W (an
/// entirely unrelated, perfectly valid edge with no shadow of its own). The
/// whole batch is still rejected (all-or-nothing: W does not get applied
/// either), but `rejected()` must name ONLY Z, not W.
#[test]
fn test_shadow_redundancy_attributed_only_to_responsible_edge_not_innocent_sibling() {
    let h = TestHarness::new();
    let x = h.create_issue("X");
    let y = h.create_issue("Y");
    let z = h.create_issue("Z");
    let w = h.create_issue("W");

    h.executor.add_dependency(&x, &y).unwrap();
    h.executor.add_dependency(&x, &z).unwrap();

    let result = h.executor.add_dependencies_with_policy(
        &y,
        &[z.clone(), w.clone()],
        jit::commands::RedundancyPolicy::Reject,
    );

    let err = result.expect_err("Y -> Z shadows the pre-existing X -> Z edge");
    let batch = err
        .downcast_ref::<DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");

    let named: Vec<&str> = batch.rejected().iter().map(|(to, _)| to.as_str()).collect();
    assert_eq!(
        named,
        vec![z.as_str()],
        "only the responsible edge (Z) may be named; the innocent sibling (W) must NOT appear: {named:?}"
    );

    // All-or-nothing: W was not applied either, even though it has no shadow
    // of its own.
    let loaded_y = h.storage.load_issue(&y).unwrap();
    assert!(
        loaded_y.dependencies.is_empty(),
        "W must not be persisted when its sibling Z is rejected"
    );
}

/// REQ-02: a cycle-failing edge must NOT suppress redundancy analysis for its
/// siblings. Existing A -> B -> C and D -> A; `dep add A D C` tries to add
/// A -> D (cycle: D already depends on A) and A -> C (self-redundant: A
/// already reaches C via A -> B). Both must be named, not just the cycle.
#[test]
fn test_cycle_failure_does_not_suppress_sibling_redundancy_check() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");

    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&d, &a).unwrap();

    let result = h.executor.add_dependencies_with_policy(
        &a,
        &[d.clone(), c.clone()],
        jit::commands::RedundancyPolicy::Reject,
    );

    let err = result.expect_err("a cycle-failing edge must not mask a sibling's redundancy");
    let batch = err
        .downcast_ref::<DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");

    let named: Vec<&str> = batch.rejected().iter().map(|(to, _)| to.as_str()).collect();
    assert!(
        named.contains(&d.as_str()),
        "the cycle edge (D) must be named: {named:?}"
    );
    assert!(
        named.contains(&c.as_str()),
        "the redundant edge (C) must ALSO be named, not suppressed by the cycle: {named:?}"
    );
    assert_eq!(
        named.len(),
        2,
        "exactly these two edges are rejected: {named:?}"
    );

    // All-or-nothing: nothing persisted.
    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(loaded_a.dependencies, vec![b.clone()]);
}

/// REQ-02 (no false positives): when the ONLY failure in a batch is a cycle,
/// the redundancy pass for the other, perfectly valid sibling edge still runs
/// and finds nothing — the valid edge must not be spuriously rejected.
#[test]
fn test_cycle_only_failure_runs_redundancy_pass_without_spurious_rejection() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");
    let e = h.create_issue("E");

    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&d, &a).unwrap();

    // D -> A would cycle (D already depends on A); E is a fresh, unrelated
    // issue with no redundancy of its own.
    let result = h.executor.add_dependencies_with_policy(
        &a,
        &[d.clone(), e.clone()],
        jit::commands::RedundancyPolicy::Reject,
    );

    let err = result.expect_err("the cycle edge must still fail the batch");
    let batch = err
        .downcast_ref::<DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");

    let named: Vec<&str> = batch.rejected().iter().map(|(to, _)| to.as_str()).collect();
    assert_eq!(
        named,
        vec![d.as_str()],
        "only the cycle edge (D) is rejected; the valid sibling (E) must not be spuriously flagged: {named:?}"
    );

    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(loaded_a.dependencies, vec![b.clone()]);
}

/// REQ-01 combination case: A -> B and A -> C are each fine in isolation
/// against the pre-existing graph, but adding them TOGETHER makes A -> C
/// redundant (A now reaches C via A -> B -> C, since B -> C already exists).
/// The would-be-final-graph check must catch this even though neither edge
/// alone would have been rejected — validating edges independently against
/// the graph as it stood before this call would miss it.
///
/// (A true mutual cycle formed only by combining two edges — "A -> B plus
/// B -> A in one call" — cannot arise from this API: every edge in a `dep add`
/// batch shares the same `from`, and cycle validity of a from -> to edge
/// depends only on whether `from` is reachable from `to` via the PRE-EXISTING
/// graph, never on `from`'s own other outgoing edges. The redundancy version
/// below is the real-world manifestation of the same "check the combination,
/// not each edge in isolation" requirement.)
#[test]
fn test_combination_redundancy_neither_edge_alone_would_trigger() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    // Pre-existing B -> C. A has no dependencies yet, so A -> C alone would
    // NOT be redundant (A has no other route to C yet).
    h.executor.add_dependency(&b, &c).unwrap();

    // Confirm A -> C is fine in isolation against the pre-existing graph.
    let (result, _) = h
        .executor
        .add_dependency_with_policy(&a, &c, jit::commands::RedundancyPolicy::Reject)
        .expect("A -> C alone is not redundant against the pre-existing graph");
    assert_eq!(result, jit::commands::DependencyAddResult::Added);
    // Undo it so the batch call below starts from a clean slate.
    h.executor.remove_dependency(&a, &c).unwrap();

    let result = h.executor.add_dependencies_with_policy(
        &a,
        &[b.clone(), c.clone()],
        jit::commands::RedundancyPolicy::Reject,
    );

    let err = result
        .expect_err("A -> C becomes redundant only in combination with A -> B in the same call");
    let batch = err
        .downcast_ref::<DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");
    assert!(batch.rejected().iter().any(|(to, _)| to == &c));

    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(
        loaded_a.dependencies.is_empty(),
        "all-or-nothing: A -> B must not persist either"
    );
}

/// REQ-01/02/03 all-valid batch: every edge applies, exactly one
/// `issue_updated` event covers the whole batch (not one per edge).
#[test]
fn test_all_valid_batch_applies_everything_with_one_event() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    let events_before = h.storage.read_events().unwrap().len();

    let result = h
        .executor
        .add_dependencies(&a, &[b.clone(), c.clone()])
        .unwrap();
    assert_eq!(result.added.len(), 2);

    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(loaded_a.dependencies.contains(&b));
    assert!(loaded_a.dependencies.contains(&c));

    // A fresh, dependency-free issue is `ready`; gaining two unmet
    // dependencies in the same batch demotes it to `backlog` (one
    // `IssueStateChanged` event) alongside the single `IssueUpdated`
    // (`dependency-add`) event covering BOTH new edges — not one
    // `IssueUpdated` per edge, since the whole batch applied in one pass.
    let events_after = h.storage.read_events().unwrap();
    let new_events = &events_after[events_before..];
    assert_eq!(
        new_events.len(),
        2,
        "expected exactly the demotion event plus one batch dependency-add event, got {new_events:?}"
    );
    let updated = new_events
        .iter()
        .filter(|e| e.get_type() == "issue_updated")
        .count();
    assert_eq!(
        updated, 1,
        "one atomic batch add covering two edges should log exactly one issue_updated event, got {new_events:?}"
    );
    assert!(new_events.iter().all(|e| e.get_issue_id() == a));
}

/// `--reduce` interplay: a batch mixing a valid edge with a self-redundant
/// one succeeds as a whole under `RedundancyPolicy::Reduce` — the valid edge
/// is added, the redundant one is silently skipped (not an error).
#[test]
fn test_reduce_policy_variadic_skips_redundant_and_adds_valid() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");

    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&b, &c).unwrap();

    // D is a fresh, non-redundant edge; C is self-redundant (A already
    // reaches C via A -> B -> C).
    let result = h
        .executor
        .add_dependencies_with_policy(
            &a,
            &[d.clone(), c.clone()],
            jit::commands::RedundancyPolicy::Reduce,
        )
        .expect("--reduce must let the valid sibling through");

    assert_eq!(result.added, vec![d.clone()]);
    assert_eq!(result.skipped.len(), 1);
    assert_eq!(result.skipped[0].0, c);

    let loaded_a = h.storage.load_issue(&a).unwrap();
    assert!(loaded_a.dependencies.contains(&d));
    assert!(loaded_a.dependencies.contains(&b));
    assert!(!loaded_a.dependencies.contains(&c));

    assert!(h.executor.validate_silent().is_ok());
}

/// `--reduce` interplay: a batch shadowing a PRE-EXISTING edge on another
/// issue still succeeds under `Reduce`, dropping the shadowed edge as a
/// `dependency_reduced` event on that OTHER issue.
#[test]
fn test_reduce_policy_drops_shadowed_edge_on_other_issue() {
    let h = TestHarness::new();
    let x = h.create_issue("X");
    let y = h.create_issue("Y");
    let z = h.create_issue("Z");

    h.executor.add_dependency(&x, &y).unwrap();
    h.executor.add_dependency(&x, &z).unwrap();

    // Y -> Z makes the pre-existing X -> Z redundant (X reaches Z via X->Y->Z).
    let result = h
        .executor
        .add_dependencies_with_policy(
            &y,
            std::slice::from_ref(&z),
            jit::commands::RedundancyPolicy::Reduce,
        )
        .expect("--reduce must succeed and drop the shadowed edge");
    assert_eq!(result.added, vec![z.clone()]);

    let loaded_x = h.storage.load_issue(&x).unwrap();
    assert!(!loaded_x.dependencies.contains(&z));
    assert!(loaded_x.dependencies.contains(&y));

    let events = h.storage.read_events().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.get_type() == "dependency_reduced" && e.get_issue_id() == x),
        "expected a dependency_reduced event on X"
    );

    assert!(h.executor.validate_silent().is_ok());
}
