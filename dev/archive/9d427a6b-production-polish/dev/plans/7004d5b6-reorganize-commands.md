# Plan: Reorganize commands module to separate CLI from domain logic

**Issue:** `7004d5b6` — Reorganize commands module to separate CLI from domain logic  
**Epic:** production-polish  
**Priority:** low  
**Downstream dependents:** 3 (epics: docs-lifecycle-p2, production-polish; milestone: v1.0)

## Problem Statement

The `commands/` module mixes CLI orchestration (output formatting, flag parsing) with pure domain query logic. This prevents using jit as a Rust library and makes module boundaries unclear. The query functions in `commands/query.rs` are already pure domain code—they just live in the wrong place.

## Proposed Approach: Option 1 — `domain/` directory module

Promote `domain.rs` to a `domain/` directory module with `types.rs` (existing data structures) and `queries.rs` (relocated query functions). This aligns with the existing pattern where `query/` is already a directory module.

### Target Structure

```
crates/jit/src/
├── commands/               # Pure CLI orchestration (no domain logic)
│   ├── mod.rs              # CommandExecutor (delegates to domain)
│   ├── issue.rs            # Issue commands (no eprintln!, returns warnings)
│   ├── query.rs            # REMOVED — functions moved to domain/queries.rs
│   └── ...                 # Other command modules unchanged
├── domain/                 # Domain types + operations
│   ├── mod.rs              # Re-exports from submodules
│   ├── types.rs            # Issue, State, Priority, Gate, Event, etc.
│   └── queries.rs          # query_ready, query_blocked, query_available, etc.
├── query/                  # Boolean query filter DSL (unchanged)
└── lib.rs                  # Updated exports
```

## Detailed Workplan

### Phase 1: Restructure `domain.rs` → `domain/` directory

- [ ] **1.1** Create `crates/jit/src/domain/` directory
- [ ] **1.2** Move `domain.rs` contents → `domain/types.rs`
- [ ] **1.3** Create `domain/mod.rs` that re-exports everything from `types.rs` (preserves all existing `use crate::domain::*` paths)
- [ ] **1.4** Verify: `cargo check --workspace` passes — zero import breakage

### Phase 2: Relocate query functions

- [ ] **2.1** Create `domain/queries.rs` with all 13 query functions from `commands/query.rs`
- [ ] **2.2** These functions are methods on `CommandExecutor<S: IssueStore>`. Two approaches:
  - **Option A (minimal change):** Keep them as `CommandExecutor` methods but in `domain/queries.rs` via `impl<S: IssueStore> CommandExecutor<S>` (Rust allows impl blocks in separate files if the type is accessible). This requires `CommandExecutor` to be importable from the domain module — circular dependency risk.
  - **Option B (cleaner):** Convert query functions to **free functions** that take `&[Issue]` and config as parameters. `CommandExecutor` methods become thin wrappers that call `storage.list_issues()` then delegate to free functions. This gives true library-usable domain logic.
  - **Recommendation: Option B** — it achieves the actual goal of library-usable domain operations.
- [ ] **2.3** Update `CommandExecutor` methods in `commands/query.rs` to delegate to the new free functions (or remove `commands/query.rs` entirely and put thin wrappers in `commands/mod.rs`)
- [ ] **2.4** Update `domain/mod.rs` to re-export query functions
- [ ] **2.5** Verify: `cargo check --workspace` passes

### Phase 3: Fix CLI output leaks (`eprintln!`)

- [ ] **3.1** In `commands/issue.rs`: `create_issue()` and `update_issue()` currently print validation warnings with `eprintln!`. Change these to **return warnings** alongside the result (e.g., return `Result<(Issue, Vec<String>)>` or a `CommandResult` struct). Move `eprintln!` to the call site in `main.rs`.
- [ ] **3.2** In `commands/mod.rs`: `require_active_lease()` prints a warning via `eprintln!` in Warn mode. Change to return a `LeaseCheckResult` enum or `Result<Option<String>>` where the `Option<String>` is the warning message. Move printing to CLI layer.
- [ ] **3.3** Verify: `cargo check --workspace` passes

### Phase 4: Update `lib.rs` exports

- [ ] **4.1** Add public re-exports for domain query functions so library consumers can use them directly:
  ```rust
  pub use domain::queries::{query_ready, query_blocked, query_available, ...};
  ```
- [ ] **4.2** Verify existing re-exports (`Issue`, `Priority`, `State`, `CommandExecutor`) still work

### Phase 5: Update tests and verify

- [ ] **5.1** Update import paths in integration tests (9 files in `crates/jit/tests/`):
  - `query_tests.rs`, `label_query_tests.rs`, `label_query_json_tests.rs`
  - `label_strategic_tests.rs`, `query_json_tests.rs`, `schema_tests.rs`
  - `cross_worktree_integration_tests.rs`, `test_no_coordinator.rs`, `harness_demo.rs`
- [ ] **5.2** Update inline tests in `commands/mod.rs` and `commands/issue.rs` if affected
- [ ] **5.3** Run full test suite: `cargo test --workspace --quiet`
- [ ] **5.4** Run quality gates: `cargo clippy --workspace --all-targets` and `cargo fmt --all --check`

### Phase 6: Cleanup and documentation

- [ ] **6.1** Remove `commands/query.rs` if all logic has been relocated (or leave as thin delegating wrappers if import compatibility is needed)
- [ ] **6.2** Add doc comments to `domain/queries.rs` explaining the module's purpose
- [ ] **6.3** Update `domain/mod.rs` module-level doc comment to describe the domain layer
- [ ] **6.4** Verify no dead code warnings: `cargo clippy --workspace --all-targets`

## Key Design Decisions

### Query function signatures (Phase 2, Option B)

Current (method on CommandExecutor):
```rust
impl<S: IssueStore> CommandExecutor<S> {
    pub fn query_ready(&self) -> Result<Vec<Issue>> {
        let issues = self.storage.list_issues()?;
        // ... filter logic
    }
}
```

Proposed (free function in domain):
```rust
// domain/queries.rs
pub fn query_ready(issues: &[Issue]) -> Vec<Issue> {
    issues.iter()
        .filter(|i| i.state == State::Ready && i.assignee.is_none() && !i.is_blocked())
        .cloned()
        .collect()
}
```

Note: Some functions like `query_strategic` need config access (`strategic_types` from `ConfigManager`). These can take the config value as a parameter rather than needing the whole `ConfigManager`.

### Warning return pattern (Phase 3)

Current:
```rust
// commands/issue.rs
for warning in warnings {
    eprintln!("⚠️  Warning: {}", warning);
}
```

Proposed:
```rust
// commands/issue.rs — return warnings
pub fn create_issue(&self, ...) -> Result<(Issue, Vec<String>)> { ... }

// main.rs — CLI layer prints
let (issue, warnings) = executor.create_issue(...)?;
for warning in &warnings {
    eprintln!("⚠️  Warning: {}", warning);
}
```

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Breaking existing `use crate::domain::X` imports | Phase 1 preserves all paths via `domain/mod.rs` re-exports |
| Query functions that need storage access can't be free functions | `CommandExecutor` wrappers call `storage.list_issues()` then delegate |
| `query_strategic` needs config (strategic_types) | Pass `&[String]` parameter instead of `ConfigManager` |
| `query_by_label` uses `crate::labels::matches_pattern` | This is already a domain utility; no change needed |
| Large number of test files to update | Tests access queries via `executor.query_*()` — if we keep `CommandExecutor` wrappers, most tests need zero changes |

## Scope Boundaries

**In scope:**
- Moving query logic from `commands/query.rs` to `domain/queries.rs`
- Promoting `domain.rs` to `domain/` directory
- Fixing `eprintln!` leaks in `commands/issue.rs` and `commands/mod.rs`
- Updating `lib.rs` exports

**Out of scope:**
- Refactoring `CommandExecutor` itself (stays in `commands/mod.rs`)
- Moving non-query domain logic (gate checking, dependency operations)
- Changing the `query/` filter DSL module
- Changing CLI argument parsing or output formatting
- Adding new features or tests beyond what's needed for the refactor
