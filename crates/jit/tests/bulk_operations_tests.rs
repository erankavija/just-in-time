//! Tests for bulk operations (multiple gates, dependencies, labels in single command)

mod harness;
use harness::TestHarness;
use jit::storage::IssueStore;

// ========================================
// GATE BULK OPERATIONS TESTS
// ========================================

#[test]
fn test_add_multiple_gates_at_once() {
    let h = TestHarness::new();
    h.add_gate("tests", "Tests", "Run tests", false);
    h.add_gate("clippy", "Clippy", "Run Clippy", false);
    h.add_gate("fmt", "Format", "Check formatting", false);

    let issue_id = h.create_issue("Test Issue");

    h.executor
        .add_gates(
            &issue_id,
            &["tests".to_string(), "clippy".to_string(), "fmt".to_string()],
        )
        .unwrap();

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 3);
    assert!(loaded.gates_required.contains(&"tests".to_string()));
    assert!(loaded.gates_required.contains(&"clippy".to_string()));
    assert!(loaded.gates_required.contains(&"fmt".to_string()));
}

#[test]
fn test_add_gates_some_already_exist() {
    let h = TestHarness::new();
    h.add_gate("tests", "Tests", "Run tests", false);
    h.add_gate("clippy", "Clippy", "Run Clippy", false);

    let issue_id = h.create_issue("Test Issue");

    // Add one gate first
    h.executor.add_gate(&issue_id, "tests".to_string()).unwrap();

    // Try to add tests again plus new ones
    let (result, _warnings) = h
        .executor
        .add_gates(&issue_id, &["tests".to_string(), "clippy".to_string()])
        .unwrap();

    assert_eq!(result.added.len(), 1); // Only clippy
    assert_eq!(result.already_exist.len(), 1); // tests

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 2);
}

#[test]
fn test_add_gates_atomic_failure_invalid_gate() {
    let h = TestHarness::new();
    h.add_gate("tests", "Tests", "Run tests", false);

    let issue_id = h.create_issue("Test Issue");

    // Try to add gates, one doesn't exist
    let result = h
        .executor
        .add_gates(&issue_id, &["tests".to_string(), "nonexistent".to_string()]);

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
    h.add_gate("tests", "Tests", "Run tests", false);

    let issue_id = h.create_issue("Test Issue");

    // Single gate via new bulk API
    h.executor
        .add_gates(&issue_id, &["tests".to_string()])
        .unwrap();

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 1);
    assert!(loaded.gates_required.contains(&"tests".to_string()));
}

#[test]
fn test_remove_multiple_gates() {
    let h = TestHarness::new();
    h.add_gate("tests", "Tests", "Run tests", false);
    h.add_gate("clippy", "Clippy", "Run Clippy", false);
    h.add_gate("fmt", "Format", "Check formatting", false);

    let issue_id = h.create_issue("Test Issue");

    // Add multiple gates
    h.executor
        .add_gates(
            &issue_id,
            &["tests".to_string(), "clippy".to_string(), "fmt".to_string()],
        )
        .unwrap();

    // Remove two
    h.executor
        .remove_gates(&issue_id, &["tests".to_string(), "clippy".to_string()])
        .unwrap();

    let loaded = h.storage.load_issue(&issue_id).unwrap();
    assert_eq!(loaded.gates_required.len(), 1);
    assert!(loaded.gates_required.contains(&"fmt".to_string()));
}

#[test]
fn test_remove_gates_some_not_present() {
    let h = TestHarness::new();
    h.add_gate("tests", "Tests", "Run tests", false);
    h.add_gate("clippy", "Clippy", "Run Clippy", false);

    let issue_id = h.create_issue("Test Issue");

    h.executor
        .add_gates(&issue_id, &["tests".to_string()])
        .unwrap();

    let (result, _warnings) = h
        .executor
        .remove_gates(&issue_id, &["tests".to_string(), "clippy".to_string()])
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
        .add_dependencies(&from, &[to1.clone(), to2.clone(), to3.clone()])
        .unwrap();

    assert_eq!(result.added.len(), 3);
    assert!(result.errors.is_empty());

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
    h.executor.add_dependency(&from, &to1).unwrap();

    // Try to add to1 again plus to2
    let result = h
        .executor
        .add_dependencies(&from, &[to1.clone(), to2.clone()])
        .unwrap();

    assert_eq!(result.added.len(), 1); // to2
    assert_eq!(result.already_exist.len(), 1); // to1
    assert!(result.errors.is_empty());
}

#[test]
fn test_add_dependencies_partial_failure_nonexistent() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to1 = h.create_issue("Dep 1");

    let result = h
        .executor
        .add_dependencies(&from, &[to1.clone(), "nonexistent".to_string()])
        .unwrap();

    assert_eq!(result.added.len(), 1); // to1
    assert_eq!(result.errors.len(), 1); // nonexistent
    assert!(result.errors[0].1.contains("not found"));
}

#[test]
fn test_add_dependencies_cycle_detection() {
    let h = TestHarness::new();
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");

    // Create A → B → C
    h.executor.add_dependency(&a, &b).unwrap();
    h.executor.add_dependency(&b, &c).unwrap();

    // Try to add C → A (would create cycle)
    let result = h
        .executor
        .add_dependencies(&c, std::slice::from_ref(&a))
        .unwrap();

    assert_eq!(result.added.len(), 0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].1.contains("cycle"));
}

#[test]
fn test_add_single_dependency_still_works() {
    let h = TestHarness::new();
    let from = h.create_issue("Parent");
    let to = h.create_issue("Dep");

    h.executor
        .add_dependencies(&from, std::slice::from_ref(&to))
        .unwrap();

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
        .add_dependencies(&from, &[to1.clone(), to2.clone(), to3.clone()])
        .unwrap();

    // Remove two
    h.executor
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

    h.executor
        .add_dependencies(&from, std::slice::from_ref(&to1))
        .unwrap();

    let result = h
        .executor
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

    let result = h.executor.add_gates(&issue_id, &[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("at least one"));
}

#[test]
fn test_add_dependencies_empty_list_error() {
    let h = TestHarness::new();
    let issue_id = h.create_issue("Test Issue");

    let result = h.executor.add_dependencies(&issue_id, &[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("at least one"));
}
