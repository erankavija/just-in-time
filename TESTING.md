# JIT Testing Strategy

This document describes the testing approach for the Just-In-Time issue tracker.

## Test Structure

We use a **three-layer testing approach**:

```
┌─────────────────────────────────────────┐
│   Integration Tests (Process-based)    │  ← 31 tests, end-to-end via CLI
├─────────────────────────────────────────┤
│   Harness Tests (In-process)           │  ← 14 tests, fast & reliable
├─────────────────────────────────────────┤
│   Unit Tests (Business Logic)          │  ← 78 tests, comprehensive
└─────────────────────────────────────────┘

Total: 123 tests (as of 2025-11-30)
```

## Test Types

### 1. Unit Tests (`cli/src/**/*.rs`)

Located in source files as `#[cfg(test)] mod tests`.

**Coverage:**
- `commands.rs`: Command execution logic (92 tests)
- `graph.rs`: DAG operations and cycle detection (16 tests)
- `storage.rs`: File I/O and persistence (14 tests)
- `domain.rs`: Core types and business logic

**Benefits:**
- Fast (run in milliseconds)
- Test individual functions in isolation
- High code coverage
- Easy to debug

**Example:**
```rust
#[test]
fn test_add_dependency_rejects_cycle() {
    let storage = setup_test_storage();
    let executor = CommandExecutor::new(storage);
    
    let id1 = executor.create_issue("Task 1", "", Priority::Normal, vec![]).unwrap();
    let id2 = executor.create_issue("Task 2", "", Priority::Normal, vec![]).unwrap();
    
    executor.add_dependency(&id2, &id1).unwrap();
    let result = executor.add_dependency(&id1, &id2);
    
    assert!(result.is_err());
}
```

### 2. Harness Tests (`cli/tests/harness_demo.rs`)

In-process tests using the `TestHarness` helper.

**Benefits:**
- 10-100x faster than process tests
- No PATH/binary dependencies
- Easy to debug (same process)
- Reliable (no timing issues)
- Clean, readable test code

**Usage:**
```rust
#[test]
fn test_harness_query_ready() {
    let h = TestHarness::new();  // Isolated environment
    
    // Create issues directly
    let ready_id = h.create_ready_issue("Ready task");
    let assigned_id = h.create_ready_issue("Assigned task");
    h.executor.claim_issue(&assigned_id, "agent:worker-1".to_string()).unwrap();
    
    // Query using executor
    let ready = h.executor.query_ready().unwrap();
    
    assert_eq!(ready.len(), 1);
    assert_eq!(ready[0].id, ready_id);
}
```

**Test Harness API:**
```rust
impl TestHarness {
    // Setup
    fn new() -> Self                                  // Isolated temp directory
    
    // Issue creation helpers
    fn create_issue(&self, title: &str) -> String
    fn create_ready_issue(&self, title: &str) -> String
    fn create_issue_with_priority(&self, title: &str, priority: Priority) -> String
    fn create_issue_with_gates(&self, title: &str, gates: Vec<String>) -> String
    
    // Gate setup
    fn add_gate(&self, key: &str, title: &str, description: &str, auto: bool)
    
    // Queries
    fn all_issues(&self) -> Vec<Issue>
    fn get_issue(&self, id: &str) -> Issue
    fn data_dir(&self) -> PathBuf
    
    // Direct access
    pub executor: CommandExecutor  // For any command
    pub storage: Storage            // For direct storage ops
}
```

**Coverage:**
- Query commands (ready, blocked, by assignee, by priority)
- Issue lifecycle (create, update, claim, release, delete)
- Dependencies (add, cycle detection, blocking)
- Gates (pass, fail, blocking)
- Complex workflows (multiple dependencies + gates)
- Performance (scales to 100+ issues)
- **CLI consistency** (JSON output, argument order) - 8 tests

### 3. Integration Tests (`jit/tests/integration_test.rs`, `jit/tests/query_tests.rs`, `jit/tests/test_cli_consistency.rs`)

Process-based tests that spawn the `jit` binary.

**Benefits:**
- Test actual CLI interface
- Catch argument parsing issues
- Verify output formatting
- End-to-end validation

**When to use:**
- Critical user-facing commands
- Output format validation
- CLI flag combinations
- Regression tests for bugs

**Example:**
```rust
#[test]
fn test_create_and_query() {
    let temp = setup_test_repo();
    let jit = jit_binary();
    
    // Create issue via CLI
    let output = Command::new(&jit)
        .args(["issue", "create", "-t", "Task", "--priority", "high"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    
    assert!(output.status.success());
    
    // Query via CLI
    let output = Command::new(&jit)
        .args(["query", "ready", "--json"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json["issues"].is_array());
}
```

## Running Tests

```bash
# All tests (unit + harness + integration)
cargo test

# Fast tests only (unit + harness, ~0.02s)
cargo test --lib
cargo test --test harness_demo

# Integration tests only (slower, ~0.02s)
cargo test --test integration_test
cargo test --test query_tests

# Specific test
cargo test test_harness_query_ready

# With output
cargo test -- --nocapture

# Single-threaded (for debugging)
cargo test -- --test-threads=1
```

## Test Organization

```
jit/
├── src/
│   ├── commands.rs        # Unit tests for business logic
│   ├── graph.rs           # Graph algorithms tests
│   ├── storage.rs         # File I/O tests
│   └── domain.rs          # Domain model tests
├── tests/
│   ├── harness.rs             # TestHarness implementation
│   ├── harness_demo.rs        # 8 harness-based tests
│   ├── integration_test.rs    # 16 CLI integration tests
│   ├── query_tests.rs         # 7 query integration tests
│   ├── test_cli_consistency.rs # 8 CLI consistency tests
│   └── test_no_coordinator.rs  # 6 post-refactor validation tests
└── Cargo.toml

dispatch/
├── tests/
│   └── test_dispatch.rs   # 3 orchestrator tests (using jit CLI)
└── Cargo.toml
```

## Best Practices

### When Writing Tests

1. **Use harness for new features**: Default to `TestHarness` for fast, reliable tests
2. **Keep integration tests minimal**: Only test critical user-facing scenarios
3. **Test edge cases in unit tests**: Cycles, empty inputs, concurrent operations
4. **Use descriptive names**: `test_query_ready_excludes_assigned_issues`
5. **Follow TDD**: Write test first, implement minimal code to pass

### Test Naming Convention

```rust
// Good names
test_query_ready_returns_only_unassigned()
test_add_dependency_rejects_cycle()
test_gate_blocks_until_passed()

// Bad names
test_query()
test_dependencies()
test_it_works()
```

### Assertions

```rust
// Prefer specific assertions
assert_eq!(ready.len(), 1);
assert_eq!(ready[0].id, expected_id);

// Over generic ones
assert!(ready.len() > 0);
assert!(ready.contains(&expected));
```

## Performance

```
Benchmark Results:
- Unit tests:        ~0.00s (78 tests)
- Harness tests:     ~0.02s (8 tests)  ← 100x faster than process tests
- Integration tests: ~0.02s (23 tests)

Total test time: ~0.04s
```

## Adding New Tests

### For a New Command

1. **Start with unit test** in the command's module
2. **Add harness test** for workflow testing
3. **Optionally add integration test** if user-facing

Example for a new `jit issue bulk-update` command:

```rust
// 1. Unit test in commands.rs
#[test]
fn test_bulk_update_issues() {
    let executor = setup();
    let ids = vec!["id1", "id2"];
    executor.bulk_update(&ids, State::Done).unwrap();
    // ...assertions...
}

// 2. Harness test in harness_demo.rs
#[test]
fn test_harness_bulk_update() {
    let h = TestHarness::new();
    let id1 = h.create_issue("Task 1");
    let id2 = h.create_issue("Task 2");
    
    h.executor.bulk_update(&[id1, id2], State::Done).unwrap();
    
    assert_eq!(h.get_issue(&id1).state, State::Done);
}

// 3. Integration test in integration_test.rs (optional)
#[test]
fn test_cli_bulk_update() {
    let temp = setup_test_repo();
    Command::new(jit_binary())
        .args(["issue", "bulk-update", "--state", "done", "id1", "id2"])
        .current_dir(temp.path())
        .assert()
        .success();
}
```

## Future Improvements

1. **Snapshot testing**: For CLI output validation
2. **Property-based tests**: Using `proptest` for DAG operations
3. **Benchmarks**: Track query performance over time
4. **Coverage tracking**: Aim for >85% code coverage
5. **Mutation testing**: Verify test quality

## See Also

- [Copilot Instructions](../.github/copilot-instructions.md) - TDD guidelines
- [ROADMAP.md](../ROADMAP.md) - Testing requirements per phase
- [cli/tests/harness_demo.rs](../cli/tests/harness_demo.rs) - Example tests
