# Generic DAG Refactoring Plan

**Status**: ✅ Completed  
**Date**: 2025-12-02  
**Goal**: Refactor DependencyGraph to be generic and promote separation of concerns

## Summary

Successfully refactored the dependency graph implementation to be generic and reusable:

- ✅ **Generic Graph**: `DependencyGraph<'a, T: GraphNode>` works with any node type
- ✅ **Clean Separation**: Graph algorithms separated from Issue-specific visualization
- ✅ **New Module**: Created `visualization.rs` for Issue graph exports (DOT, Mermaid)
- ✅ **Tests**: Added 5 new generic graph tests (102 total, all passing)
- ✅ **Zero Warnings**: Passes clippy and fmt checks
- ✅ **Web UI Ready**: Clean abstractions prepared for future API consumption

## Motivation

Current `DependencyGraph` is tightly coupled to `Issue` domain type:
- Stores `HashMap<String, &'a Issue>` directly
- Export functions access Issue-specific fields (title, state)
- Cannot be reused for other dependency relationships
- Violates separation of concerns (graph algorithms mixed with domain logic)

**Long-term goal**: Prepare for web UI that can render and edit dependency graphs independently of specific formats.

## Design Principles

1. **DAG = Pure Data Structure** - No domain logic, no visualization
2. **Separation of Concerns** - Graph algorithms separate from Issue business logic  
3. **Web UI Ready** - Clean abstractions for future API consumption
4. **Reusable** - Can be used for any dependency relationship (tasks, packages, builds, etc.)

## Architecture

### Module Structure
```
crates/jit/src/
  ├── graph.rs          # Generic DependencyGraph<T: GraphNode>
  ├── domain.rs         # Issue + GraphNode impl
  ├── visualization.rs  # NEW: Issue-specific DOT/Mermaid exports
  ├── commands.rs       # Uses graph + visualization
  └── lib.rs            # Exports public API
```

### Core Abstractions

#### 1. GraphNode Trait (graph.rs)
```rust
/// Trait for types that can participate in a dependency graph
pub trait GraphNode {
    /// Unique identifier for this node
    fn id(&self) -> &str;
    
    /// IDs of nodes this node depends on
    fn dependencies(&self) -> &[String];
}
```

#### 2. Generic DependencyGraph (graph.rs)
```rust
/// Generic dependency graph with cycle detection
pub struct DependencyGraph<'a, T: GraphNode> {
    nodes: HashMap<String, &'a T>,
}

impl<'a, T: GraphNode> DependencyGraph<'a, T> {
    pub fn new(nodes: &[&'a T]) -> Self
    pub fn validate_add_dependency(&self, from: &str, to: &str) -> Result<(), GraphError>
    pub fn get_roots(&self) -> Vec<&'a T>
    pub fn get_dependents(&self, node_id: &str) -> Vec<&'a T>
    pub fn get_transitive_dependents(&self, node_id: &str) -> Vec<&'a T>
    pub fn validate_dag(&self) -> Result<(), GraphError>
    // ... other pure graph operations
}
```

**No visualization methods** - export_dot/export_mermaid removed from here.

#### 3. GraphNode Implementation (domain.rs)
```rust
impl GraphNode for Issue {
    fn id(&self) -> &str {
        &self.id
    }
    
    fn dependencies(&self) -> &[String] {
        &self.dependencies
    }
}
```

#### 4. Visualization Module (visualization.rs - NEW)
```rust
/// Export Issue dependency graph as DOT format for Graphviz
pub fn export_dot(graph: &DependencyGraph<Issue>) -> String

/// Export Issue dependency graph as Mermaid format
pub fn export_mermaid(graph: &DependencyGraph<Issue>) -> String
```

These functions:
- Take `&DependencyGraph<Issue>` (not `&[&Issue]`)
- Access Issue-specific fields (title, state) for rendering
- Can be extended with more formats without touching graph.rs

### Error Handling

```rust
#[derive(Debug, Error, PartialEq)]
pub enum GraphError {
    #[error("Node not found: {id}")]
    NodeNotFound { id: String },
    
    #[error("Cycle detected: adding dependency would create a cycle")]
    CycleDetected,
}
```

Generic "node" terminology - callers provide context in error messages.

## Implementation Steps (TDD)

### ✅ Step 1: Write Tests for Generic Graph (COMPLETED)
- ✅ Created dummy `TestNode` struct
- ✅ Wrote 5 tests proving graph works with non-Issue types
- ✅ Tests for generic cycle detection, traversal, validation

### ✅ Step 2: Extract & Generify Core Graph (COMPLETED)
- ✅ Added `GraphNode` trait to graph.rs
- ✅ Made `DependencyGraph<'a, T: GraphNode>` generic
- ✅ Updated all methods to work with `T`
- ✅ Changed `IssueNotFound` → `NodeNotFound { id: String }`
- ✅ **Removed** `export_dot()` and `export_mermaid()` from graph.rs

### ✅ Step 3: Create visualization.rs Module (COMPLETED)
- ✅ Created new file: `crates/jit/src/visualization.rs`
- ✅ Moved export logic from graph.rs
- ✅ Functions take `&DependencyGraph<Issue>`
- ✅ Access Issue fields directly (title, state, etc.)
- ✅ Moved export tests to this module

### ✅ Step 4: Implement GraphNode for Issue (COMPLETED)
```rust
// In domain.rs
impl GraphNode for Issue {
    fn id(&self) -> &str { &self.id }
    fn dependencies(&self) -> &[String] { &self.dependencies }
}
```

### ✅ Step 5: Update Commands & Call Sites (COMPLETED)
- ✅ Updated commands.rs to use `DependencyGraph<Issue>`
- ✅ Call `visualization::export_dot(&graph)` instead of `graph.export_dot()`
- ✅ Error context maintained with new NodeNotFound error type

### ✅ Step 6: Update Tests (COMPLETED)
- ✅ Updated type annotations where needed
- ✅ All 102 tests pass (97 existing + 5 new generic tests)
- ✅ Added new tests for generic functionality

### ✅ Step 7: Documentation & Cleanup (COMPLETED)
- ✅ Updated doc comments on graph.rs (emphasize generic nature)
- ✅ Added examples in visualization.rs
- ✅ Ran clippy, fmt (zero warnings)
- ✅ Updated lib.rs and main.rs exports

## Files Modified

| File | Changes | Estimated Lines |
|------|---------|-----------------|
| `graph.rs` | Add trait, generify, remove exports | ~60 modified, ~90 removed |
| `visualization.rs` | **NEW** - Issue graph exports | ~120 new |
| `domain.rs` | Add GraphNode impl | ~10 new |
| `commands.rs` | Update call sites | ~20 modified |
| `lib.rs` | Export visualization module | ~1 new |
| Tests | Type annotations, move tests | ~40 modified |

## Benefits

### Immediate
✅ **Reusable**: DAG can be used for any dependency relationship  
✅ **Testable**: Graph logic can be tested with simple mock types  
✅ **Maintainable**: Clear separation between graph algorithms and domain logic  
✅ **Extensible**: Easy to add new node types or export formats  
✅ **Functional**: Pure functions in core graph module

### Future (Web UI)
- **GET /api/graph** → Build `DependencyGraph<Issue>`, serialize nodes + edges
- **Render client-side** → No coupling to DOT/Mermaid formats
- **Export on-demand** → Call visualization functions for downloads
- **Validate edits** → Use same `validate_add_dependency()` in API layer
- **Multiple views** → Same graph data, different visualizations

## Testing Strategy

1. **Generic graph tests**: Use dummy `TestNode` to prove genericity
2. **Issue integration tests**: Ensure existing Issue-based functionality works
3. **Visualization tests**: Test DOT/Mermaid export formats
4. **Command tests**: Ensure CLI commands work with refactored code

Target: Maintain 100% of existing test coverage (16 graph tests + command tests).

## Success Criteria

- [x] All existing tests pass (102/102 ✓)
- [x] New tests for generic graph functionality (5 new tests)
- [x] `cargo clippy` passes with zero warnings
- [x] `cargo fmt` applied
- [x] Documentation updated
- [x] Graph can be instantiated with non-Issue types (TestNode validates)
- [x] Visualization logic separated from graph algorithms

## Actual Implementation Results

### Files Changed
- `graph.rs`: ~100 lines modified (added trait, generified, removed exports)
- `visualization.rs`: **NEW** - 228 lines (Issue graph exports + 4 tests)
- `domain.rs`: +8 lines (GraphNode impl for Issue)
- `commands.rs`: 2 lines modified (call visualization module)
- `lib.rs`: +1 line (export visualization module)
- `main.rs`: +1 line (add visualization module)

### Test Results
- **Before**: 97 tests passing
- **After**: 102 tests passing (+5 generic graph tests)
- **Coverage**: All graph operations, exports, and Issue integration tested
- **Execution Time**: <0.01s for all graph+visualization tests

### Code Quality
- ✅ Zero clippy warnings
- ✅ Code formatted with `cargo fmt`
- ✅ Full documentation with examples
- ✅ Maintains functional programming style

## References

- Original issue: Need for generic DAG abstraction
- Related: Future web UI development
- See: `docs/design.md` for overall architecture
- See: `ROADMAP.md` Phase 3 for integration with other features
