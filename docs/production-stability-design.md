# Production Stability Design

## Overview

This document outlines the design for production stability features required for v1.0: bulk operations, comprehensive error recovery, and performance benchmarks. These features ensure the system can handle real-world workloads reliably and efficiently.

## Motivation

A production-ready issue tracker must handle:
- **Scale**: Large repositories with hundreds or thousands of issues
- **Reliability**: Graceful error handling and recovery from failures
- **Efficiency**: Batch operations for managing multiple issues simultaneously

## 1. Bulk Operations Support

### Design Goals

Enable efficient batch operations on multiple issues matching filter criteria, reducing overhead for both human users and AI agents managing large workspaces.

### Command Structure

```bash
# Bulk update state
jit issue bulk-update --filter 'label:epic:phase5.2' --state ready

# Bulk add labels
jit issue bulk-update --filter 'state:ready AND priority:high' --add-label 'sprint:current'

# Bulk assign
jit issue bulk-update --filter 'state:ready AND label:component:cli' --assignee 'agent:worker-1'

# Preview mode (dry-run)
jit issue bulk-update --filter '...' --state ready --dry-run
```

### Filter Syntax

Support boolean query language:
- Label filters: `label:epic:foo`, `label:component:*`
- State filters: `state:ready`, `state:backlog OR state:ready`
- Priority filters: `priority:high`
- Assignee filters: `assignee:none`, `assignee:agent:*`
- Combination: `(state:ready AND label:epic:foo) OR priority:critical`

### Implementation Approach

1. Parse filter expression into AST
2. Evaluate against all issues (leverage existing query infrastructure)
3. Apply operation to matching issues
4. Use atomic file operations (existing infrastructure)
5. Log bulk operation events
6. Return summary with counts (modified, skipped, errors)

### JSON Output

```json
{
  "operation": "bulk-update",
  "filter": "label:epic:phase5.2",
  "changes": {"state": "ready"},
  "matched": 5,
  "modified": 4,
  "skipped": 1,
  "errors": [],
  "issue_ids": ["id1", "id2", "id3", "id4"]
}
```

### Error Handling

- Partial failures: Continue processing remaining issues
- Rollback not required (atomic per-issue writes)
- Report all errors in summary
- Exit code indicates success (0) or partial failure (1)

## 2. Comprehensive Error Recovery

### Design Goals

Ensure system can detect, report, and recover from all common failure modes without data loss or corruption.

### Error Categories

#### Storage Errors

1. **Partial writes**: System crash during file write
   - Detection: File size validation, JSON parse validation
   - Recovery: Restore from `.tmp` files if newer, otherwise flag for manual intervention

2. **Corrupted JSON**: Invalid syntax or schema violations
   - Detection: Parse errors, schema validation
   - Recovery: Attempt to parse with lenient parser, log corruption, offer repair mode

3. **Lock timeouts**: Deadlock or stale lock files
   - Detection: Lock acquisition timeout (current: file locking with fs4)
   - Recovery: Automatic stale lock cleanup (age-based), force-unlock command

4. **Missing files**: Referenced files don't exist
   - Detection: File not found errors
   - Recovery: Graceful degradation, repair command to remove dead references

#### Concurrent Access Errors

1. **Race conditions**: Multiple agents modifying same issue
   - Detection: Already handled by file locking
   - Recovery: Retry with exponential backoff

2. **Stale reads**: Reading old data after external modification
   - Detection: Timestamp validation
   - Recovery: Reload and retry operation

#### Network Errors (MCP Server)

1. **Connection failures**: Client disconnects
   - Detection: Socket errors
   - Recovery: Graceful shutdown, log incomplete operations

2. **Timeout errors**: Long-running operations
   - Detection: Operation timeout
   - Recovery: Return partial results, allow resumption

### Implementation Strategy

1. **Audit Phase**: Systematic review of all I/O operations for error handling
2. **Test Coverage**: Add failure injection tests for each error category
3. **Repair Commands**: Add `jit validate --fix` to detect and repair common issues
4. **Logging**: Comprehensive error logging with context
5. **Documentation**: Error recovery runbook for operators

### Validation & Repair

```bash
# Detect issues
jit validate

# Auto-fix common problems
jit validate --fix

# Dry-run mode
jit validate --fix --dry-run
```

Validation checks:
- All JSON files parseable
- All references (dependencies, documents) valid
- No orphaned lock files
- DAG property maintained
- Label hierarchy consistency

## 3. Performance Benchmarks

### Design Goals

Establish performance baselines and ensure system performs acceptably at scale (1000+ issues).

### Benchmark Scenarios

#### Scenario 1: Large Repository Simulation

- **Setup**: Generate 1000 issues with realistic dependency graph
- **Operations**:
  - List all issues
  - Query by state (various states)
  - Query by label (various labels)
  - Graph export (full DAG)
  - Find roots
  - Find downstream dependents (various depths)
  - Bulk operations on 100 issues

#### Scenario 2: Complex Queries

- **Setup**: 500 issues with diverse labels
- **Operations**:
  - Strategic query (all strategic labels)
  - Label wildcard queries
  - Multi-label filters
  - Full-text search across all issues

#### Scenario 3: Gate Operations

- **Setup**: 100 issues with 5 gates each
- **Operations**:
  - Check all gates on single issue
  - Precheck execution (fast and slow gates)
  - Postcheck execution
  - Gate history queries

### Performance Targets

- **Query operations**: <100ms for 1000 issues
- **Graph export**: <500ms for 1000 issues
- **Bulk operations**: <1s for 100 issues
- **Gate execution**: Dominated by checker runtime (framework overhead <10ms)

### Measurement Approach

1. Use criterion.rs for Rust benchmarks
2. Measure wall-clock time and memory usage
3. Generate plots showing performance vs. scale
4. Document results in `docs/performance.md`

### Continuous Monitoring

- Add benchmark CI job (GitHub Actions)
- Track performance over time
- Alert on regressions >10%

## Testing Strategy

### Bulk Operations

- Unit tests: Filter parsing, expression evaluation
- Integration tests: Bulk update various scenarios
- Property tests: Invariants maintained after bulk operations
- Error tests: Partial failures, invalid filters

### Error Recovery

- Failure injection tests for each error category
- Validation tests with broken repositories
- Repair tests with auto-fix scenarios

### Performance

- Benchmark suite with multiple scales (10, 100, 1000 issues)
- Regression tests comparing against baseline
- Memory leak detection (valgrind or similar)

## Documentation

- User guide: Bulk operations syntax and examples
- Operations guide: Error recovery procedures
- Performance guide: Benchmarks and optimization tips

## Implementation Plan

1. **Bulk operations** (1-2 days): Filter parser, bulk update command, tests
2. **Error recovery** (2-3 days): Audit, validation command, repair logic, comprehensive tests
3. **Performance benchmarks** (1-2 days): Benchmark suite, CI integration, documentation

**Total effort**: 4-7 days of focused development
