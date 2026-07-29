# Implementation Plan: Transitive Reduction Validation

## Overview

Ensure the dependency DAG always maintains its transitive reduction form - no redundant edges that are already reachable via other paths.

## Problem Statement

**Current Issue:**
- Code has transitive reduction logic in `add_dependency()` that applies when adding NEW edges
- BUT existing dependencies are NOT validated or cleaned up retroactively
- Found actual redundancy: Epic "AI Agent Validation" (4a00b2b0) has redundant edge to task "241b7002"
  - Direct path: `4a00b2b0 → 241b7002` (redundant)
  - Transitive path: `4a00b2b0 → 4cb2682e → 14303b30 → 241b7002` (already reachable)

**Root Cause:**
1. Dependencies added before reduction logic existed
2. Sequential additions can create redundancy (add A→C, then B→C, then A→B leaves A→C redundant)
3. No validation or cleanup command for existing graphs

## Current State Analysis

### What Works ✅
- `add_dependency()` has transitive reduction logic (lines 16-20, 30-39 in `dependency.rs`)
- `is_transitive()` checks if new edge is redundant
- `compute_transitive_reduction()` calculates minimal edge set
- Tests exist: `test_compute_transitive_reduction_*`

### What's Broken ❌
- No validation for existing dependencies
- No cleanup command for retroactive fixes
- No CI/validation enforcement of invariant
- Existing redundant edges in production data

### Example of Redundancy

```
Epic: AI Agent Validation (4a00b2b0)
├─ Direct dependencies:
│  ├─ 241b7002 (Add --stage option) ← REDUNDANT
│  ├─ 4cb2682e (Execute validation)
│  └─ ...
│
└─ Transitive path to 241b7002:
   4a00b2b0 → 4cb2682e → 14303b30 → 241b7002
```

The edge `4a00b2b0 → 241b7002` is redundant because 241b7002 is already reachable via the path through 4cb2682e and 14303b30.

## Task Breakdown

### 1. Add Transitive Reduction Validation Check

**Location**: `crates/jit/src/commands/validate.rs`

**Function**:
```rust
fn validate_transitive_reduction(&self) -> Result<Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    let all_issues = self.storage.list_issues()?;
    let issue_refs: Vec<&Issue> = all_issues.iter().collect();
    let graph = DependencyGraph::new(&issue_refs);
    
    for issue in &all_issues {
        if issue.dependencies.is_empty() {
            continue;
        }
        
        let reduced = graph.compute_transitive_reduction(&issue.id);
        let current: HashSet<_> = issue.dependencies.iter().collect();
        let reduced_set: HashSet<_> = reduced.iter().collect();
        
        // Find redundant edges (in current but not in reduction)
        for dep_id in &issue.dependencies {
            if !reduced_set.contains(dep_id) {
                // This edge is redundant - find the transitive path
                let path = graph.find_shortest_path(&issue.id, dep_id);
                issues.push(ValidationIssue {
                    severity: Severity::Warning,
                    code: "REDUNDANT_DEPENDENCY".to_string(),
                    message: format!(
                        "Issue {} has redundant dependency on {} (reachable via path: {})",
                        &issue.id[..8], &dep_id[..8], 
                        path.join(" → ")
                    ),
                    issue_id: Some(issue.id.clone()),
                    fixable: true,
                });
            }
        }
    }
    
    Ok(issues)
}
```

**Auto-fix Implementation**:
```rust
fn fix_transitive_reduction(&self, issue_id: &str) -> Result<()> {
    let all_issues = self.storage.list_issues()?;
    let issue_refs: Vec<&Issue> = all_issues.iter().collect();
    let graph = DependencyGraph::new(&issue_refs);
    
    let mut issue = self.storage.load_issue(issue_id)?;
    let reduced = graph.compute_transitive_reduction(issue_id);
    
    if issue.dependencies.len() != reduced.len() {
        let old_deps = issue.dependencies.clone();
        issue.dependencies = reduced.into_iter().collect();
        self.storage.save_issue(&issue)?;
        
        // Log event
        let event = Event::new_custom(
            "dependency.transitive_reduction".to_string(),
            json!({
                "issue_id": issue_id,
                "old_count": old_deps.len(),
                "new_count": issue.dependencies.len(),
                "removed": old_deps.iter()
                    .filter(|d| !issue.dependencies.contains(d))
                    .collect::<Vec<_>>()
            })
        );
        self.storage.append_event(&event)?;
    }
    
    Ok(())
}
```

### 2. Integrate into `jit validate` Command

**Update**: `crates/jit/src/commands/validate.rs`

Add to existing validation checks:
```rust
pub fn validate(&self, fix: bool, dry_run: bool) -> Result<ValidationReport> {
    let mut all_issues = Vec::new();
    
    // ... existing validations ...
    
    // NEW: Transitive reduction validation
    let transitive_issues = self.validate_transitive_reduction()?;
    all_issues.extend(transitive_issues);
    
    // Apply fixes if requested
    if fix {
        for issue in &all_issues {
            if issue.fixable && issue.code == "REDUNDANT_DEPENDENCY" {
                if !dry_run {
                    self.fix_transitive_reduction(
                        issue.issue_id.as_ref().unwrap()
                    )?;
                }
            }
        }
    }
    
    Ok(ValidationReport { issues: all_issues })
}
```

### 3. Add Helper Method to Find Paths

**Location**: `crates/jit/src/graph.rs`

```rust
impl<'a, T: GraphNode> DependencyGraph<'a, T> {
    /// Find shortest path between two nodes (for reporting)
    pub fn find_shortest_path(&self, from: &str, to: &str) -> Vec<String> {
        let mut queue = VecDeque::new();
        let mut visited = HashMap::new();
        
        queue.push_back((from, vec![from.to_string()]));
        
        while let Some((current, path)) = queue.pop_front() {
            if current == to && path.len() > 1 {
                return path;
            }
            
            if visited.contains_key(current) {
                continue;
            }
            visited.insert(current, ());
            
            if let Some(node) = self.nodes.get(current) {
                for dep in node.dependencies() {
                    // Skip direct edge from start
                    if current == from && dep == to {
                        continue;
                    }
                    
                    let mut new_path = path.clone();
                    new_path.push(dep.clone());
                    queue.push_back((dep.as_str(), new_path));
                }
            }
        }
        
        vec![]
    }
}
```

### 4. Add Tests

**File**: `crates/jit/tests/transitive_reduction_validation_tests.rs`

```rust
#[test]
fn test_detect_transitive_redundancy() {
    let h = TestHarness::new();
    
    // Create A → B → C and A → C (redundant)
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();
    
    // Manually add redundant edge (bypassing reduction logic)
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(&issue_a).unwrap();
    
    // Validate should detect it
    let result = h.executor.validate(false, false).unwrap();
    assert!(result.issues.iter().any(|i| 
        i.code == "REDUNDANT_DEPENDENCY" && 
        i.issue_id == Some(a.clone())
    ));
}

#[test]
fn test_fix_transitive_redundancy() {
    let h = TestHarness::new();
    
    // Same setup
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();
    
    // Add redundant edge
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    h.storage.save_issue(&issue_a).unwrap();
    
    // Fix with validate --fix
    h.executor.validate(true, false).unwrap();
    
    // Verify C was removed
    let fixed_a = h.storage.load_issue(&a).unwrap();
    assert_eq!(fixed_a.dependencies.len(), 1);
    assert!(fixed_a.dependencies.contains(&b));
    assert!(!fixed_a.dependencies.contains(&c));
}

#[test]
fn test_validate_reports_all_redundancies() {
    let h = TestHarness::new();
    
    // Create multiple issues with redundancies
    let a = h.create_issue("A");
    let b = h.create_issue("B");
    let c = h.create_issue("C");
    let d = h.create_issue("D");
    
    // A → B → C and A → C (redundant)
    // B → D and A → D (via A → B → D, redundant)
    
    h.executor.add_dependency(&b, &c).unwrap();
    h.executor.add_dependency(&b, &d).unwrap();
    h.executor.add_dependency(&a, &b).unwrap();
    
    let mut issue_a = h.storage.load_issue(&a).unwrap();
    issue_a.dependencies.push(c.clone());
    issue_a.dependencies.push(d.clone());
    h.storage.save_issue(&issue_a).unwrap();
    
    let result = h.executor.validate(false, false).unwrap();
    let redundancies: Vec<_> = result.issues.iter()
        .filter(|i| i.code == "REDUNDANT_DEPENDENCY")
        .collect();
    
    assert_eq!(redundancies.len(), 2); // A→C and A→D
}

#[test]
fn test_no_false_positives() {
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
    
    let result = h.executor.validate(false, false).unwrap();
    let redundancies: Vec<_> = result.issues.iter()
        .filter(|i| i.code == "REDUNDANT_DEPENDENCY")
        .collect();
    
    assert_eq!(redundancies.len(), 0);
}
```

### 5. CLI Integration

**Update**: `crates/jit/src/cli.rs` and `crates/jit/src/main.rs`

Validate command already exists, just needs to call the new validation:
```bash
jit validate                    # Check for issues
jit validate --fix              # Auto-fix redundancies
jit validate --fix --dry-run    # Show what would be fixed
```

### 6. Update Documentation

**Files to update**:
- `docs/design.md` - Add transitive reduction invariant
- `README.md` - Mention in DAG features
- `TESTING.md` - Add validation testing section

**Content**:
```markdown
## Dependency DAG Invariants

The dependency graph maintains these invariants:

1. **Acyclic**: No cycles allowed (enforced at add time)
2. **Transitive Reduction**: Only minimal edges stored
   - If A→C is reachable via A→B→C, then edge A→C is redundant
   - System automatically maintains reduction form
   - Use `jit validate --fix` to clean up any violations

### Example

```
BAD (redundant edge):          GOOD (transitive reduction):
A → B → C                      A → B → C
A → C (redundant!)             
```
```

### 7. Property-Based Tests

**File**: `crates/jit/tests/transitive_reduction_property_tests.rs`

```rust
use proptest::prelude::*;

proptest! {
    #[test]
    fn test_any_dag_reduces_correctly(
        edges in prop::collection::vec((0usize..10, 0usize..10), 10..30)
    ) {
        // Filter to create valid DAG (no self-loops, no cycles)
        // Build graph, compute reduction
        // Verify reduction is minimal and preserves reachability
    }
}
```

## Implementation Approach (TDD)

1. **Write tests first** for validation detection
2. **Implement `validate_transitive_reduction()`** 
3. **Add `find_shortest_path()` helper**
4. **Implement `fix_transitive_reduction()`**
5. **Integrate into `validate` command**
6. **Run on actual repository** to find/fix issues
7. **Add property-based tests**
8. **Update documentation**
9. **Run full test suite**
10. **Run clippy and fmt**

## Success Criteria

✅ `jit validate` detects existing redundant edges  
✅ `jit validate --fix` removes redundant edges while preserving reachability  
✅ All existing tests pass  
✅ New tests cover detection, fixing, and edge cases  
✅ No false positives (diamond patterns preserved)  
✅ Property tests verify correctness  
✅ Actual repository cleaned up  
✅ Documentation updated  
✅ Zero clippy warnings  

## Migration Path

### One-Time Cleanup

```bash
# 1. Detect issues
jit validate --check-transitive-reduction

# 2. Preview fixes
jit validate --fix --dry-run

# 3. Apply fixes
jit validate --fix

# 4. Verify
jit validate
```

### Future Prevention

- CI validation step: `jit validate` in GitHub Actions
- Pre-commit hook option
- Regular validation runs

## Performance Considerations

- Validation requires loading all issues (already done in `validate`)
- Transitive closure computation is O(V + E) per issue with BFS
- For 100 issues with avg 5 deps each: ~500 graph traversals
- Acceptable for validation command (not hot path)
- Can optimize with memoization if needed

## Edge Cases

1. **Diamond patterns**: A→B, A→C, B→D, C→D
   - Both paths to D needed at A level
   - Not redundant ✅

2. **Self-loops**: Prevented by cycle detection ✅

3. **Empty dependencies**: Skip validation ✅

4. **Concurrent modifications**: Use existing file locking ✅

5. **Large graphs**: Memoize transitive closure if performance issue

## Dependencies

No new crate dependencies required - uses existing:
- `anyhow` for error handling
- `serde_json` for event logging
- Graph utilities already in `graph.rs`

## Estimated Effort

- Validation logic: 3 hours
- Path finding helper: 1 hour
- Fix implementation: 2 hours  
- Tests (unit + integration): 3 hours
- Property tests: 2 hours
- Documentation: 1 hour
- Cleanup current repo: 1 hour
- **Total: 13 hours**

## Related Issues

This addresses a core DAG invariant violation and should be high priority to maintain data integrity.

## Why This Matters

**Data Integrity**: DAG must be in canonical form (transitive reduction)  
**Performance**: Redundant edges waste storage and computation  
**Correctness**: Dependency semantics unclear with redundant edges  
**Observability**: Graph visualization confusing with extra edges  
**Maintenance**: Future algorithms assume reduction form  

## References

- Transitive Reduction: https://en.wikipedia.org/wiki/Transitive_reduction
- Existing code: `crates/jit/src/graph.rs` lines 246-295
- Existing tests: `crates/jit/src/graph.rs` lines 644-690
