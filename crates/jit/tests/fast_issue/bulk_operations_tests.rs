//! Tests for bulk operations (multiple gates, dependencies, labels in single command)
use jit::declarations::GateMode;
use jit::storage::IssueStore;
use jit::test_harness::TestHarness;
use jit::test_utils::add_reducing_edge;
use jit::test_utils::issue_id;

// ========================================
// GATE BULK OPERATIONS TESTS
// ========================================

#[test]
fn test_add_multiple_gates_at_once() {
    let h = TestHarness::new();
    h.add_gate(
        &"tests".parse().unwrap(),
        "Tests",
        "Run tests",
        GateMode::Manual,
        None,
    );
    h.add_gate(
        &"clippy".parse().unwrap(),
        "Clippy",
        "Run Clippy",
        GateMode::Manual,
        None,
    );
    h.add_gate(
        &"fmt".parse().unwrap(),
        "Format",
        "Check formatting",
        GateMode::Manual,
        None,
    );

    let issue_id = h.create_issue("Test Issue");

    h.executor
        .gates()
        .add_gates(
            &issue_id,
            &[
                "tests".parse().unwrap(),
                "clippy".parse().unwrap(),
                "fmt".parse().unwrap(),
            ],
        )
        .unwrap();

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 3);
    assert!(loaded.gates_required.contains(&"tests".parse().unwrap()));
    assert!(loaded.gates_required.contains(&"clippy".parse().unwrap()));
    assert!(loaded.gates_required.contains(&"fmt".parse().unwrap()));
}

#[test]
fn test_add_gates_some_already_exist() {
    let h = TestHarness::new();
    h.add_gate(
        &"tests".parse().unwrap(),
        "Tests",
        "Run tests",
        GateMode::Manual,
        None,
    );
    h.add_gate(
        &"clippy".parse().unwrap(),
        "Clippy",
        "Run Clippy",
        GateMode::Manual,
        None,
    );

    let issue_id = h.create_issue("Test Issue");

    // Add one gate first
    h.executor
        .gates()
        .add_gate(&issue_id, &"tests".parse().unwrap())
        .unwrap();

    // Try to add tests again plus new ones
    let (result, _warnings) = h
        .executor
        .gates()
        .add_gates(
            &issue_id,
            &["tests".parse().unwrap(), "clippy".parse().unwrap()],
        )
        .unwrap();

    assert_eq!(result.added.len(), 1); // Only clippy
    assert_eq!(result.already_exist.len(), 1); // tests

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 2);
}

#[test]
fn test_add_gates_atomic_failure_invalid_gate() {
    let h = TestHarness::new();
    h.add_gate(
        &"tests".parse().unwrap(),
        "Tests",
        "Run tests",
        GateMode::Manual,
        None,
    );

    let issue_id = h.create_issue("Test Issue");

    // Try to add gates, one doesn't exist
    let result = h.executor.gates().add_gates(
        &issue_id,
        &["tests".parse().unwrap(), "nonexistent".parse().unwrap()],
    );

    // Should fail entirely
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("not found in registry"));

    // No gates should have been added
    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 0);
}

#[test]
fn test_add_single_gate_still_works() {
    let h = TestHarness::new();
    h.add_gate(
        &"tests".parse().unwrap(),
        "Tests",
        "Run tests",
        GateMode::Manual,
        None,
    );

    let issue_id = h.create_issue("Test Issue");

    // Single gate via new bulk API
    h.executor
        .gates()
        .add_gates(&issue_id, &["tests".parse().unwrap()])
        .unwrap();

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 1);
    assert!(loaded.gates_required.contains(&"tests".parse().unwrap()));
}

#[test]
fn test_remove_multiple_gates() {
    let h = TestHarness::new();
    h.add_gate(
        &"tests".parse().unwrap(),
        "Tests",
        "Run tests",
        GateMode::Manual,
        None,
    );
    h.add_gate(
        &"clippy".parse().unwrap(),
        "Clippy",
        "Run Clippy",
        GateMode::Manual,
        None,
    );
    h.add_gate(
        &"fmt".parse().unwrap(),
        "Format",
        "Check formatting",
        GateMode::Manual,
        None,
    );

    let issue_id = h.create_issue("Test Issue");

    // Add multiple gates
    h.executor
        .gates()
        .add_gates(
            &issue_id,
            &[
                "tests".parse().unwrap(),
                "clippy".parse().unwrap(),
                "fmt".parse().unwrap(),
            ],
        )
        .unwrap();

    // Remove two
    h.executor
        .gates()
        .remove_gates(
            &issue_id,
            &["tests".parse().unwrap(), "clippy".parse().unwrap()],
        )
        .unwrap();

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 1);
    assert!(loaded.gates_required.contains(&"fmt".parse().unwrap()));
}

#[test]
fn test_remove_gates_some_not_present() {
    let h = TestHarness::new();
    h.add_gate(
        &"tests".parse().unwrap(),
        "Tests",
        "Run tests",
        GateMode::Manual,
        None,
    );
    h.add_gate(
        &"clippy".parse().unwrap(),
        "Clippy",
        "Run Clippy",
        GateMode::Manual,
        None,
    );

    let issue_id = h.create_issue("Test Issue");

    h.executor
        .gates()
        .add_gates(&issue_id, &["tests".parse().unwrap()])
        .unwrap();

    let (result, _warnings) = h
        .executor
        .gates()
        .remove_gates(
            &issue_id,
            &["tests".parse().unwrap(), "clippy".parse().unwrap()],
        )
        .unwrap();

    assert_eq!(result.removed.len(), 1); // tests
    assert_eq!(result.not_found.len(), 1); // clippy
}

// ========================================
// DEPENDENCY BULK OPERATIONS TESTS
// ========================================

#[test]
fn test_add_multiple_dependencies() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to1 = h.create_issue("Dep 1");
    let to2 = h.create_issue("Dep 2");
    let to3 = h.create_issue("Dep 3");

    let result = h
        .executor
        .graph()
        .add_dependencies_with_policy(
            &from,
            &[to1.clone(), to2.clone(), to3.clone()],
            jit::commands::RedundancyPolicy::Reduce,
        )
        .unwrap();

    assert_eq!(result.added.len(), 3);

    let loaded = h.storage.load_issue(&from).unwrap();
    assert_eq!(loaded.dependencies.len(), 3);
    assert!(loaded.dependencies.contains(&to1));
    assert!(loaded.dependencies.contains(&to2));
    assert!(loaded.dependencies.contains(&to3));
}

#[test]
fn test_add_dependencies_some_already_exist() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to1 = h.create_issue("Dep 1");
    let to2 = h.create_issue("Dep 2");

    // Add one dependency first
    add_reducing_edge(&h.executor, &from, &to1).unwrap();

    // Try to add to1 again plus to2
    let result = h
        .executor
        .graph()
        .add_dependencies_with_policy(
            &from,
            &[to1.clone(), to2.clone()],
            jit::commands::RedundancyPolicy::Reduce,
        )
        .unwrap();

    assert_eq!(result.added.len(), 1); // to2
    assert_eq!(result.already_exist.len(), 1); // to1
}

/// jit:c8518f2a REQ-01: a batch with an unresolvable target is atomic — the
/// resolvable sibling must NOT be persisted either. Mirrors the already-atomic
/// `add_gates` contract (`test_add_gates_atomic_failure_invalid_gate`).
#[test]
fn test_add_dependencies_atomic_failure_nonexistent() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to1 = h.create_issue("Dep 1");
    let absent = issue_id(1);

    let result = h.executor.graph().add_dependencies_with_policy(
        &from,
        &[to1.clone(), absent.clone()],
        jit::commands::RedundancyPolicy::Reduce,
    );

    let err = result.expect_err("an unresolvable target must fail the whole batch");
    let batch = err
        .downcast_ref::<jit::errors::DependencyBatchRejectedError>()
        .expect("must be a typed DependencyBatchRejectedError");
    assert_eq!(batch.rejected().len(), 1);
    assert_eq!(batch.rejected()[0].0, absent.as_str());
    assert!(batch.rejected()[0].1.to_string().contains("not found"));

    // Nothing was persisted, including the sibling edge that would have
    // succeeded on its own.
    let loaded = h.storage.load_issue(&from).unwrap();
    assert!(loaded.dependencies.is_empty());
}

#[test]
fn test_add_single_dependency_still_works() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to = h.create_issue("Dep");

    add_reducing_edge(&h.executor, &from, &to).unwrap();

    let loaded = h.storage.load_issue(&from).unwrap();
    assert_eq!(loaded.dependencies.len(), 1);
    assert!(loaded.dependencies.contains(&to));
}

#[test]
fn test_remove_multiple_dependencies() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to1 = h.create_issue("Dep 1");
    let to2 = h.create_issue("Dep 2");
    let to3 = h.create_issue("Dep 3");

    // Add three dependencies
    h.executor
        .graph()
        .add_dependencies_with_policy(
            &from,
            &[to1.clone(), to2.clone(), to3.clone()],
            jit::commands::RedundancyPolicy::Reduce,
        )
        .unwrap();

    // Remove two
    h.executor
        .graph()
        .remove_dependencies(&from, &[to1.clone(), to2.clone()])
        .unwrap();

    let loaded = h.storage.load_issue(&from).unwrap();
    assert_eq!(loaded.dependencies.len(), 1);
    assert!(loaded.dependencies.contains(&to3));
}

#[test]
fn test_remove_dependencies_some_not_present() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to1 = h.create_issue("Dep 1");
    let to2 = h.create_issue("Dep 2");

    add_reducing_edge(&h.executor, &from, &to1).unwrap();

    let result = h
        .executor
        .graph()
        .remove_dependencies(&from, &[to1.clone(), to2.clone()])
        .unwrap();

    assert_eq!(result.removed.len(), 1); // to1
    assert_eq!(result.not_found.len(), 1); // to2
}

// ========================================
// EMPTY INPUT VALIDATION TESTS
// ========================================

#[test]
fn test_add_gates_empty_list_error() {
    let h = TestHarness::new();
    let issue_id = h.create_issue("Test Issue");

    let result = h.executor.gates().add_gates(&issue_id, &[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("at least one"));
}

#[test]
fn test_add_dependencies_empty_list_error() {
    let h = TestHarness::new();
    let issue_id = h.create_issue("Test Issue");

    let result = h.executor.graph().add_dependencies_with_policy(
        &issue_id,
        &[],
        jit::commands::RedundancyPolicy::Reduce,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("at least one"));
}
