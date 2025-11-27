# Test Coverage Report

**Last Updated:** 2025-11-27  
**Overall Coverage:** 40.34% (313/776 lines)

## Coverage by Module

| Module | Coverage | Lines Covered | Total Lines | Status |
|--------|----------|---------------|-------------|--------|
| commands.rs | 40.15% | 104/259 | 259 | ⚠️ NEEDS IMPROVEMENT |
| coordinator.rs | 0.00% | 0/136 | 136 | ❌ NO TESTS |
| domain.rs | 66.00% | 33/50 | 50 | ✅ GOOD |
| graph.rs | 85.45% | 94/110 | 110 | ✅ EXCELLENT |
| main.rs | 0.00% | 0/132 | 132 | ❌ NO TESTS (CLI entry point) |
| storage.rs | 92.13% | 82/89 | 89 | ✅ EXCELLENT |

## Target Goals

- **Phase 3 Completion:** >80% overall coverage
- **Phase 4 Completion:** >90% overall coverage
- **Critical Modules:** storage.rs, domain.rs, graph.rs should be >90%
- **Command Modules:** commands.rs should be >80%
- **Coordinator:** coordinator.rs should be >70% (complex daemon logic)

## Missing Test Coverage

### commands.rs (40.15% - PRIORITY HIGH)

**Untested Functions:**
- `delete_issue()` - Line 141
- `assign_issue()` - Line 145
- `unassign_issue()` - Line 182
- `add_dependency()` - Line 217 (validation tested, command not tested)
- `remove_dependency()` - Line 236
- `add_gate()` - Line 243
- `pass_gate()` - Line 252 (basic tested, edge cases missing)
- `fail_gate()` - Line 280 (basic tested, edge cases missing)
- `show_graph()` - Line 308
- `show_downstream()` - Line 331
- `show_roots()` - Line 340
- `validate()` - Line 349
- `status()` - Line 359
- `list_gates()` - Line 385
- `add_gate_definition()` - Line 390
- `remove_gate_definition()` - Line 419
- `show_gate_definition()` - Line 431
- `export_graph()` - Line 478

**Partially Tested Functions:**
- `update_issue()` - Basic test exists, state transitions need more coverage
- `claim_issue()` - Basic test exists, edge cases missing
- `claim_next()` - Priority tested, filtering not tested

### coordinator.rs (0.00% - PRIORITY HIGH)

**Completely Untested:**
- All coordinator daemon logic
- Agent pool management
- Dispatch algorithms
- Status monitoring
- Configuration loading

**Action Required:**
- Add integration tests for coordinator
- Mock agent execution
- Test dispatch logic
- Test concurrent agent handling

### domain.rs (66.00% - PRIORITY MEDIUM)

**Missing Coverage:**
- Event creation for all event types (lines 243-304)
- Some edge cases in blocking logic

**Action Required:**
- Add tests for all event types
- Test edge cases in `is_blocked()` logic

### graph.rs (85.45% - GOOD)

**Missing Coverage:**
- Error handling paths (lines 28, 39, 70, 86, 95)
- Some edge cases in export functions

**Action Required:**
- Add error scenario tests
- Test malformed graph exports

### storage.rs (92.13% - EXCELLENT)

**Missing Coverage:**
- Some error paths (lines 166, 187-188, 192-193, 197-198)

**Action Required:**
- Add file I/O error tests
- Test disk full scenarios

## TDD Enforcement

Going forward, **ALL** new features must:

1. **Write tests first** - Red/Green/Refactor cycle
2. **Achieve >80% coverage** for the feature
3. **Include edge case tests** - errors, boundaries, invalid inputs
4. **Document test coverage** in PR description

### Test Writing Guidelines

```rust
#[test]
fn test_function_name_expected_behavior() {
    // Arrange - set up test data
    let (_temp, executor) = setup();
    
    // Act - execute the function
    let result = executor.function_name(args).unwrap();
    
    // Assert - verify expectations
    assert_eq!(result.field, expected_value);
}

#[test]
fn test_function_name_error_scenario() {
    let (_temp, executor) = setup();
    
    // Test error conditions
    let result = executor.function_name(invalid_args);
    assert!(result.is_err());
}
```

## How to Run Coverage

```bash
# Install tarpaulin (one time)
cargo install cargo-tarpaulin

# Run coverage
cd cli && cargo tarpaulin --out Stdout

# Generate HTML report
cd cli && cargo tarpaulin --out Html
# Open tarpaulin-report.html in browser
```

## Next Steps

1. **Immediate:** Backfill tests for commands.rs critical functions
2. **Phase 3:** Add coordinator tests before new features
3. **Phase 4:** Achieve >90% coverage before production
